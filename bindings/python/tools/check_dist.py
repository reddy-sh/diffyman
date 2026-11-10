"""Check built wheels and sdists of the ``diffyman`` package before a release.

    python tools/check_dist.py dist/diffyman-0.1.0-cp311-abi3-*.whl dist/diffyman-0.1.0.tar.gz

Standard library only; it reads the archives and never installs them. Each
file is checked for what PyPI shows and what users get:

- the name and version (the workspace version in the root ``Cargo.toml``);
- core metadata: ``Requires-Python: >=3.11``, ``License-Expression`` and a
  ``License-File`` for each license text, the homepage
  ``https://reddy.sh`` and the repository and documentation URLs;
- both license texts, equal to the repository's;
- a wheel: the ``cp311-abi3`` tag (one wheel per platform for CPython 3.11
  and newer), not pure Python, exactly one extension module next to the
  Python sources, ``py.typed`` and no bytecode;
- an sdist: ``pyproject.toml``, the Rust sources it builds from, no test
  corpus and no built extension;
- no build machine path inside any file (``/Users/``, ``/home/``,
  ``C:\\Users\\``, a Cargo home that was not remapped): build with
  ``RUSTFLAGS="--remap-path-prefix=$CARGO_HOME=/cargo
  --remap-path-prefix=$PWD=."`` so that panic locations name crates, not
  folders. The CycloneDX SBOM maturin writes into ``.dist-info/sboms`` names
  the build folder in its ``bom-ref`` values and is checked only with
  ``--strict-sbom``.

Exit status 0 when every file passes, 1 otherwise.
"""

from __future__ import annotations

import argparse
import email.parser
import email.policy
import pathlib
import re
import sys
import tarfile
import tomllib
import zipfile
from collections.abc import Iterator

HERE = pathlib.Path(__file__).resolve().parent
PACKAGE = HERE.parent
REPO = PACKAGE.parents[1]
LICENSES = ("LICENSE-MIT", "LICENSE-APACHE")
# Paths of a build machine: macOS and Linux home folders, Windows profiles and
# a Cargo home that was not remapped (~/.cargo, or /usr/local/cargo in the
# manylinux images). A remapped one reads /cargo/registry/src/...
HOME_PATHS = re.compile(rb"/Users/|/home/|[A-Za-z]:\\Users\\|\.cargo[/\\]registry|/usr/local/cargo/")
WHEEL_NAME = re.compile(r"^diffyman-(?P<version>[^-]+)-cp311-abi3-(?P<platform>[A-Za-z0-9_.]+)\.whl$")
SDIST_NAME = re.compile(r"^diffyman-(?P<version>[^-]+)\.tar\.gz$")


def workspace_version() -> str:
    manifest = tomllib.loads((REPO / "Cargo.toml").read_text(encoding="utf-8"))
    version = manifest["workspace"]["package"]["version"]
    assert isinstance(version, str)
    return version


def check_metadata(text: str, version: str, problems: list[str]) -> None:
    """Core metadata (PKG-INFO or METADATA)."""
    message = email.parser.Parser(policy=email.policy.compat32).parsestr(text)
    expected = {
        "Name": "diffyman",
        "Version": version,
        "Requires-Python": ">=3.11",
        "License-Expression": "MIT OR Apache-2.0",
    }
    for field, value in expected.items():
        if message.get(field) != value:
            problems.append(f"metadata {field} is {message.get(field)!r}, not {value!r}")
    files = message.get_all("License-File") or []
    for name in LICENSES:
        if name not in files:
            problems.append(f"metadata has no License-File: {name}")
    if message.get("License") is not None:
        problems.append("metadata has a legacy License field next to License-Expression")
    urls = dict(
        (label.strip(), url.strip())
        for label, _, url in (entry.partition(",") for entry in message.get_all("Project-URL") or [])
    )
    expected_urls = {
        "Homepage": "https://reddy.sh",
        "Repository": "https://github.com/reddy-sh/diffyman",
        "Documentation": "https://reddy-sh.github.io/diffyman",
    }
    for label, url in expected_urls.items():
        if urls.get(label) != url:
            problems.append(f"metadata Project-URL {label} is {urls.get(label)!r}, not {url!r}")
    version_parts = (message.get("Metadata-Version") or "0").split(".")
    if tuple(int(part) for part in version_parts) < (2, 4):
        problems.append(f"Metadata-Version {message.get('Metadata-Version')} is older than 2.4 (PEP 639)")


def check_license(data: bytes, name: str, where: str, problems: list[str]) -> None:
    if data != (REPO / name).read_bytes():
        problems.append(f"{where} differs from the repository's {name}")


def check_paths(members: Iterator[tuple[str, bytes]], strict_sbom: bool, problems: list[str]) -> None:
    for name, data in members:
        if not strict_sbom and "/sboms/" in name:
            continue
        if name.endswith("/tools/check_dist.py"):
            continue  # this script, which spells the patterns out

        found = sorted({m.group(0).decode("latin-1") for m in HOME_PATHS.finditer(data)})
        if found:
            count = len(HOME_PATHS.findall(data))
            problems.append(f"{name}: {count} build machine paths ({', '.join(found)})")


def check_wheel(path: pathlib.Path, version: str, strict_sbom: bool) -> list[str]:
    problems: list[str] = []
    match = WHEEL_NAME.match(path.name)
    if not match:
        return ["file name is not diffyman-<version>-cp311-abi3-<platform>.whl"]
    if match["version"] != version:
        problems.append(f"version {match['version']}, the workspace {version}")
    dist_info = f"diffyman-{version}.dist-info"
    with zipfile.ZipFile(path) as wheel:
        names = wheel.namelist()
        wheel_file = wheel.read(f"{dist_info}/WHEEL").decode("utf-8")
        if "Root-Is-Purelib: false" not in wheel_file:
            problems.append("WHEEL: Root-Is-Purelib is not false")
        tags = [line.split(":", 1)[1].strip() for line in wheel_file.splitlines() if line.startswith("Tag:")]
        if tags != [f"cp311-abi3-{match['platform']}"]:
            problems.append(f"WHEEL tags {tags}, not one cp311-abi3 tag for {match['platform']}")
        check_metadata(wheel.read(f"{dist_info}/METADATA").decode("utf-8"), version, problems)
        for name in LICENSES:
            member = f"{dist_info}/licenses/{name}"
            if member not in names:
                problems.append(f"no {member}")
            else:
                check_license(wheel.read(member), name, member, problems)
        package = sorted(n for n in names if n.startswith("diffyman/"))
        extensions = [n for n in package if n.endswith((".so", ".pyd"))]
        if len(extensions) != 1 or not re.fullmatch(r"diffyman/_native\.(abi3\.so|pyd)", extensions[0]):
            problems.append(f"extension modules {extensions}, not one diffyman/_native.abi3.so or .pyd")
        for required in ["diffyman/__init__.py", "diffyman/types.py", "diffyman/_native.pyi", "diffyman/py.typed"]:
            if required not in names:
                problems.append(f"no {required}")
        bytecode = [n for n in names if "__pycache__" in n or n.endswith(".pyc")]
        if bytecode:
            problems.append(f"bytecode in the wheel: {bytecode}")
        check_paths(((n, wheel.read(n)) for n in names), strict_sbom, problems)
    return problems


def check_sdist(path: pathlib.Path, version: str, strict_sbom: bool) -> list[str]:
    problems: list[str] = []
    match = SDIST_NAME.match(path.name)
    if not match:
        return ["file name is not diffyman-<version>.tar.gz"]
    if match["version"] != version:
        problems.append(f"version {match['version']}, the workspace {version}")
    root = f"diffyman-{version}"
    with tarfile.open(path, "r:gz") as sdist:
        members = {m.name: m for m in sdist.getmembers() if m.isfile()}

        def read(name: str) -> bytes:
            handle = sdist.extractfile(members[name])
            assert handle is not None
            with handle:
                return handle.read()

        if f"{root}/PKG-INFO" not in members:
            return problems + ["no PKG-INFO"]
        check_metadata(read(f"{root}/PKG-INFO").decode("utf-8"), version, problems)
        for name in LICENSES:
            member = f"{root}/{name}"
            if member not in members:
                problems.append(f"no {member}")
            else:
                check_license(read(member), name, member, problems)
        for required in ["pyproject.toml", "Cargo.toml", "Cargo.lock", "bindings/python/Cargo.toml",
                         "bindings/python/src/lib.rs", "crates/diffyman-engine/src/lib.rs",
                         "crates/diffyman-algo/src/line_diff/histogram/NOTICE", "python/diffyman/py.typed"]:
            if f"{root}/{required}" not in members:
                problems.append(f"no {root}/{required}")
        unwanted = [n for n in members if "/tests/corpus/" in n or n.endswith((".so", ".pyd", ".pyc"))]
        if unwanted:
            problems.append(f"{len(unwanted)} unwanted files, such as {unwanted[0]}")
        outside = [n for n in members if not n.startswith(f"{root}/")]
        if outside:
            problems.append(f"files outside {root}/: {outside[:3]}")
        check_paths(((n, read(n)) for n in members), strict_sbom, problems)
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("files", nargs="+", type=pathlib.Path, help="wheels (.whl) and sdists (.tar.gz)")
    parser.add_argument("--strict-sbom", action="store_true", help="also refuse build paths in the SBOM")
    args = parser.parse_args()
    version = workspace_version()
    failed = False
    for path in args.files:
        if path.name.endswith(".whl"):
            problems = check_wheel(path, version, args.strict_sbom)
        elif path.name.endswith(".tar.gz"):
            problems = check_sdist(path, version, args.strict_sbom)
        else:
            problems = ["neither a wheel nor an sdist"]
        size = f"{path.stat().st_size / 1e6:.2f} MB"
        if problems:
            failed = True
            print(f"FAIL {path.name} ({size})")
            for problem in problems:
                print(f"  - {problem}")
        else:
            print(f"ok   {path.name} ({size})")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
