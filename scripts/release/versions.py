#!/usr/bin/env python3
"""The release version in every manifest, and the crates.io publish order.

    python3 scripts/release/versions.py check [--expect 0.1.0]
    python3 scripts/release/versions.py set 0.1.0
    python3 scripts/release/versions.py crates
    python3 scripts/release/versions.py lock-version wasm-bindgen

`check` exits 1 when the manifests disagree (or differ from --expect). It
reads:

- Cargo: the workspace version in Cargo.toml (every member inherits it, so
  the CLI's --version and diffyman::ENGINE_VERSION follow), the `=X`
  requirements of the workspace crates in [workspace.dependencies], and the
  workspace crates in Cargo.lock and fuzz/Cargo.lock;
- Python: bindings/python takes its version from Cargo (dynamic);
- npm: the package.json and package-lock.json of bindings/node,
  bindings/wasm/react and examples/amplify-react, including the linked local
  packages recorded in the lockfiles (the WASM package has the crate's
  version, since wasm-pack writes it);
- Maven: the project version in bindings/java/pom.xml;
- docs/api/openapi.json (`info.version`, from the server crate's version);
- the jar file names and Maven coordinates quoted in the Java docs.

`set` rewrites all of them, changing nothing else in each file, then runs
`check`. `crates` prints the workspace crates that are published to
crates.io (`publish` not false), one per line, each after the crates it
depends on (normal, build and versioned dev-dependencies). `lock-version`
prints the version of a crates.io package in Cargo.lock, which must be
there exactly once (the release installs wasm-bindgen-cli at the version of
the wasm-bindgen crate).

Python 3.11 or later, standard library only.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
from collections.abc import Callable
from pathlib import Path
from typing import Any

Json = dict[str, Any]

ROOT = Path(__file__).resolve().parents[2]
SEMVER = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")

NPM_PACKAGES = ["bindings/node", "bindings/wasm/react", "examples/amplify-react"]
POM = "bindings/java/pom.xml"
OPENAPI = "docs/api/openapi.json"
# Text that names the version: (file, pattern with one group for the version).
TEXT_REFERENCES: list[tuple[str, str]] = [
    ("docs/quickstart/java.md", r"diffyman-(\d+\.\d+\.\d+)\.jar"),
    ("docs/quickstart/java.md", r"<artifactId>diffyman</artifactId>\s*<version>(\d+\.\d+\.\d+)</version>"),
    ("docs/quickstart/check_examples.py", r"diffyman-(\d+\.\d+\.\d+)\.jar"),
    ("tests/conformance/JavaDiff.java", r"diffyman-(\d+\.\d+\.\d+)\.jar"),
    ("bindings/java/README.md", r"diffyman-(\d+\.\d+\.\d+)\.jar"),
    ("bindings/java/README.md", r"sh\.reddy\.diffyman:diffyman:(\d+\.\d+\.\d+)"),
]


class Problem(Exception):
    pass


def read(rel: str) -> str:
    return (ROOT / rel).read_text(encoding="utf-8")


def write(rel: str, text: str) -> None:
    path = ROOT / rel
    if path.read_text(encoding="utf-8") != text:
        path.write_text(text, encoding="utf-8", newline="")
        print(f"updated {rel}")


# --- Cargo -----------------------------------------------------------------


def workspace_members() -> dict[str, str]:
    """Package name -> manifest directory (relative) of every workspace member."""
    manifest = tomllib.loads(read("Cargo.toml"))
    members: dict[str, str] = {}
    for pattern in manifest["workspace"]["members"]:
        for directory in sorted(ROOT.glob(pattern)):
            cargo = directory / "Cargo.toml"
            if not cargo.is_file():
                continue
            package = tomllib.loads(cargo.read_text(encoding="utf-8"))["package"]
            members[package["name"]] = str(directory.relative_to(ROOT))
    return members


def cargo_versions(members: dict[str, str]) -> list[tuple[str, str]]:
    found: list[tuple[str, str]] = []
    manifest = tomllib.loads(read("Cargo.toml"))
    found.append(("Cargo.toml [workspace.package] version", manifest["workspace"]["package"]["version"]))
    for name, spec in manifest["workspace"]["dependencies"].items():
        if isinstance(spec, dict) and "path" in spec:
            requirement = spec.get("version", "")
            if not requirement.startswith("="):
                raise Problem(f"Cargo.toml: workspace dependency {name} needs an exact requirement (version = \"=X\")")
            found.append((f"Cargo.toml [workspace.dependencies] {name}", requirement[1:]))
    for name, directory in members.items():
        package = tomllib.loads(read(f"{directory}/Cargo.toml"))["package"]
        if package.get("version") != {"workspace": True}:
            raise Problem(f"{directory}/Cargo.toml: use version.workspace = true")
    for lock in ("Cargo.lock", "fuzz/Cargo.lock"):
        for package in tomllib.loads(read(lock))["package"]:
            if package["name"] in members and "source" not in package:
                found.append((f"{lock} {package['name']}", package["version"]))
    return found


def set_cargo(members: dict[str, str], version: str) -> None:
    text = read("Cargo.toml")
    text, count = re.subn(
        r'(?m)^(\[workspace\.package\][^\[]*?^version = ")[^"]+(")',
        rf"\g<1>{version}\g<2>",
        text,
    )
    if count != 1:
        raise Problem("Cargo.toml: no version in [workspace.package]")
    text = re.sub(
        r'(?m)^(\S+ = \{ path = "(?:crates|bindings)/[^"]+", version = "=)[^"]+(")',
        rf"\g<1>{version}\g<2>",
        text,
    )
    write("Cargo.toml", text)
    for lock in ("Cargo.lock", "fuzz/Cargo.lock"):
        lock_text = read(lock)
        for name in members:
            # A workspace package has no `source` line after its version.
            lock_text = re.sub(
                rf'(?m)^(\[\[package\]\]\nname = "{re.escape(name)}"\nversion = ")[^"]+("\n(?!source))',
                rf"\g<1>{version}\g<2>",
                lock_text,
            )
        write(lock, lock_text)


def python_versions() -> list[tuple[str, str]]:
    project = tomllib.loads(read("bindings/python/pyproject.toml"))["project"]
    if "version" in project or "version" not in project.get("dynamic", []):
        raise Problem("bindings/python/pyproject.toml: the version must be dynamic (taken from Cargo.toml)")
    return []


# --- npm -------------------------------------------------------------------


def dump_json(data: object, newline: bool) -> str:
    # The layout npm writes: JSON.stringify(data, null, 2).
    return json.dumps(data, indent=2, ensure_ascii=False) + ("\n" if newline else "")


def load_json(rel: str) -> tuple[Json, bool]:
    text = read(rel)
    data = json.loads(text)
    newline = text.endswith("\n")
    if dump_json(data, newline) != text:
        raise Problem(f"{rel}: not in npm's JSON layout; run npm install once, then retry")
    return data, newline


DEPENDENCY_FIELDS = ("dependencies", "devDependencies", "peerDependencies", "optionalDependencies")
EXACT = re.compile(r"^\d+\.\d+\.\d+$")


def is_ours(name: object) -> bool:
    """A package built from this repository (the WASM package has the crate's version)."""
    return isinstance(name, str) and name.startswith(("@diffyman/", "diffyman-"))


def version_fields(entry: Json, where: str) -> list[tuple[str, Json, str]]:
    """(description, holder, key) of every version field of ours in a manifest or lock entry."""
    fields = [(where, entry, "version")]
    for field in DEPENDENCY_FIELDS:
        for name, spec in entry.get(field, {}).items():
            # Exact versions of our own packages; `file:` links and ranges stay.
            if is_ours(name) and isinstance(spec, str) and EXACT.match(spec):
                fields.append((f"{where} {field}[{name!r}]", entry[field], name))
    return fields


def npm_fields(directory: str, package: Json, lock: Json) -> list[tuple[str, Json, str]]:
    fields = version_fields(package, f"{directory}/package.json")
    fields.append((f"{directory}/package-lock.json version", lock, "version"))
    for key, entry in lock["packages"].items():
        if key == "" or (not key.startswith("node_modules/") and is_ours(entry.get("name"))):
            fields += version_fields(entry, f"{directory}/package-lock.json packages[{key!r}]")
    return fields


def npm_versions() -> list[tuple[str, str]]:
    found: list[tuple[str, str]] = []
    for directory in NPM_PACKAGES:
        package = json.loads(read(f"{directory}/package.json"))
        lock = json.loads(read(f"{directory}/package-lock.json"))
        found += [(where, holder.get(key, "")) for where, holder, key in npm_fields(directory, package, lock)]
    return found


def set_npm(version: str) -> None:
    for directory in NPM_PACKAGES:
        package, package_newline = load_json(f"{directory}/package.json")
        lock, lock_newline = load_json(f"{directory}/package-lock.json")
        for _, holder, key in npm_fields(directory, package, lock):
            holder[key] = version
        write(f"{directory}/package.json", dump_json(package, package_newline))
        write(f"{directory}/package-lock.json", dump_json(lock, lock_newline))


# --- Maven, OpenAPI, docs ----------------------------------------------------

POM_VERSION = re.compile(r"(<artifactId>diffyman</artifactId>(?:\s*<!--.*?-->)?\s*<version>)([^<]+)(</version>)", re.DOTALL)
# `info.version`: the 4-space-indented key inside the top-level "info" object.
OPENAPI_VERSION = re.compile(r'(\n  "info": \{\n(?: {4,}.*\n)*? {4}"version": ")([^"]+)(")')


def one_match(pattern: re.Pattern[str], text: str, rel: str) -> re.Match[str]:
    match = pattern.search(text)
    if match is None:
        raise Problem(f"{rel}: version not found")
    return match


def other_versions() -> list[tuple[str, str]]:
    found = [
        (POM, one_match(POM_VERSION, read(POM), POM).group(2)),
        (f"{OPENAPI} info.version", json.loads(read(OPENAPI))["info"]["version"]),
    ]
    for rel, pattern in TEXT_REFERENCES:
        matches = re.findall(pattern, read(rel))
        if not matches:
            raise Problem(f"{rel}: expected text matching {pattern}")
        found.extend((f"{rel} {pattern}", value) for value in matches)
    return found


def set_others(version: str) -> None:
    def replace(match: re.Match[str]) -> str:
        return match.group(1) + version + match.group(3)

    text = read(POM)
    one_match(POM_VERSION, text, POM)
    write(POM, POM_VERSION.sub(replace, text, count=1))
    text = read(OPENAPI)
    one_match(OPENAPI_VERSION, text, OPENAPI)
    write(OPENAPI, OPENAPI_VERSION.sub(replace, text, count=1))
    by_file: dict[str, list[str]] = {}
    for rel, pattern in TEXT_REFERENCES:
        by_file.setdefault(rel, []).append(pattern)
    for rel, patterns in by_file.items():
        text = read(rel)
        for pattern in patterns:
            text = re.sub(pattern, lambda m: m.group(0).replace(m.group(1), version), text)
        write(rel, text)


# --- commands ------------------------------------------------------------------


def collect() -> list[tuple[str, str]]:
    members = workspace_members()
    found: list[tuple[str, str]] = []
    readers: list[Callable[[], list[tuple[str, str]]]] = [
        lambda: cargo_versions(members),
        python_versions,
        npm_versions,
        other_versions,
    ]
    for reader in readers:
        found.extend(reader())
    return found


def check(expect: str | None) -> int:
    found = collect()
    version = expect or found[0][1]
    wrong = [(where, value) for where, value in found if value != version]
    if wrong:
        print(f"versions disagree (expected {version}):", file=sys.stderr)
        for where, value in wrong:
            print(f"  {where}: {value or '(none)'}", file=sys.stderr)
        return 1
    print(f"{version}: {len(found)} version fields agree")
    return 0


def set_version(version: str) -> int:
    members = workspace_members()
    set_cargo(members, version)
    set_npm(version)
    set_others(version)
    return check(version)


def publish_order() -> list[str]:
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked", "--offline"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    packages = {p["name"]: p for p in metadata["packages"]}
    published = {name for name, p in packages.items() if p.get("publish") is None or p["publish"]}
    needs: dict[str, set[str]] = {}
    for name in published:
        deps = set()
        for dep in packages[name]["dependencies"]:
            if dep["name"] not in packages or dep.get("path") is None:
                continue
            if dep["kind"] == "dev" and dep.get("req") in (None, "*"):
                continue  # Unversioned dev-dependencies are dropped by `cargo publish`.
            if dep["name"] not in published:
                raise Problem(f"{name} depends on {dep['name']}, which has publish = false")
            deps.add(dep["name"])
        deps.discard(name)
        needs[name] = deps
    order: list[str] = []
    while needs:
        ready = sorted(name for name, deps in needs.items() if deps <= set(order))
        if not ready:
            raise Problem(f"dependency cycle among {sorted(needs)}")
        order.extend(ready)
        for name in ready:
            del needs[name]
    return order


def lock_version(name: str) -> str:
    versions: list[str] = [str(p["version"]) for p in tomllib.loads(read("Cargo.lock"))["package"] if p["name"] == name and "source" in p]
    if len(versions) != 1:
        raise Problem(f"Cargo.lock has {len(versions)} versions of {name} from a registry ({', '.join(versions)}), expected one")
    return versions[0]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    check_parser = sub.add_parser("check", help="the manifests agree (and equal --expect)")
    check_parser.add_argument("--expect", help="the version they must all have")
    set_parser = sub.add_parser("set", help="write a version into every manifest")
    set_parser.add_argument("version")
    sub.add_parser("crates", help="the crates.io publish order")
    lock_parser = sub.add_parser("lock-version", help="the Cargo.lock version of a crates.io package")
    lock_parser.add_argument("name")
    args = parser.parse_args()
    try:
        for value in (getattr(args, "expect", None), getattr(args, "version", None)):
            if value is not None and not SEMVER.match(value):
                raise Problem(f"not a release version (MAJOR.MINOR.PATCH): {value!r}")
        if args.command == "check":
            return check(args.expect)
        if args.command == "set":
            return set_version(args.version)
        if args.command == "lock-version":
            print(lock_version(args.name))
            return 0
        print("\n".join(publish_order()))
        return 0
    except Problem as problem:
        print(f"versions.py: {problem}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
