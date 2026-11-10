#!/usr/bin/env python3
"""Lays out the npm packages of a release, ready for `npm publish`.

    python scripts/release/stage_npm.py --version 0.1.0 --out DIR \
        --node-addons ADDONS_DIR --wasm-pkg bindings/wasm/pkg-web [--react bindings/wasm/react]

Writes one folder per package under DIR and DIR/publish-order.txt, the
folder names in the order they must be published:

- node-<platform>: @diffyman/node-<platform>, one prebuilt addon each
  (darwin-arm64, darwin-x64, linux-x64-gnu, linux-arm64-gnu,
  win32-x64-msvc), and node: @diffyman/node with those packages as exact
  optionalDependencies. Both come from bindings/node/scripts/stage-npm-packages.mjs
  (which needs `npm ci` in bindings/node for the napi CLI); ADDONS_DIR holds the
  diffyman.<platform>.node files, in any subfolder;
- wasm: @diffyman/wasm, the wasm-pack output after
  bindings/wasm/scripts/finish-package.mjs;
- react: @diffyman/react (bindings/wasm/react, after `npm run build`), which
  imports @diffyman/wasm.

Each folder holds the files of the package's "files" list and its
package.json, which keeps "private": true and has no "scripts" or
"devDependencies": the publish job removes "private" with `npm pkg delete
private` just before `npm publish`, so nothing here can be published by
accident. The script then checks every folder: the version, homepage and
repository, README and license texts equal to the repository's, each
platform package's addon, the loader's package names, and that no file
names the unpublished `diffyman-wasm` module.

Python 3.11 or later, standard library only.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

Json = dict[str, Any]

ROOT = Path(__file__).resolve().parents[2]
NODE = ROOT / "bindings" / "node"
REPOSITORY = "git+https://github.com/reddy-sh/diffyman.git"
LICENSES = ["LICENSE-MIT", "LICENSE-APACHE"]
# The platform keys bindings/node/index.js loads, `@diffyman/node-${key}`.
PLATFORMS = ["darwin-arm64", "darwin-x64", "linux-x64-gnu", "linux-arm64-gnu", "win32-x64-msvc"]
OLD_WASM_SPECIFIER = re.compile(rb"""["']diffyman-wasm["'/]""")


class Problem(Exception):
    pass


def read_manifest(folder: Path) -> Json:
    manifest: Json = json.loads((folder / "package.json").read_text(encoding="utf-8"))
    return manifest


def write_manifest(folder: Path, manifest: Json) -> None:
    (folder / "package.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def copy_files(source: Path, folder: Path, files: list[str]) -> None:
    """Copies the entries of a "files" list (files or folders) from source."""
    for entry in files:
        path = source / entry
        if path.is_dir():
            shutil.copytree(path, folder / entry)
        elif path.is_file():
            (folder / entry).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, folder / entry)
        else:
            raise Problem(f"{source.name}: {entry} is listed in \"files\" but missing; build the package first")


def published_manifest(manifest: Json, version: str) -> Json:
    if manifest.get("version") != version:
        raise Problem(f"{manifest.get('name')}: version {manifest.get('version')}, expected {version}")
    manifest = {k: v for k, v in manifest.items() if k not in ("scripts", "devDependencies")}
    manifest["private"] = True
    repository = manifest.get("repository")
    if isinstance(repository, dict) and repository.get("url") in (
        "https://github.com/reddy-sh/diffyman",
        "https://github.com/reddy-sh/diffyman.git",
    ):
        # The form npm writes itself (it would rewrite the URL on publish, with a warning).
        manifest["repository"] = {**repository, "url": REPOSITORY}
    return manifest


def stage_node(out: Path, addons: Path, version: str, allow_missing: bool) -> list[str]:
    """@diffyman/node and its platform packages, through the binding's own stager."""
    if "@diffyman/node-${key}" not in (NODE / "index.js").read_text(encoding="utf-8"):
        raise Problem("bindings/node/index.js no longer loads @diffyman/node-<platform>; update stage_npm.py")
    with tempfile.TemporaryDirectory() as scratch:
        staged = Path(scratch) / "node"
        command = ["node", str(NODE / "scripts" / "stage-npm-packages.mjs"), "--out", str(staged), "--addons", str(addons.resolve())]
        if allow_missing:
            command.append("--allow-missing")
        result = subprocess.run(command, cwd=NODE, capture_output=True, text=True, check=False)
        if result.returncode != 0:
            raise Problem(f"stage-npm-packages.mjs failed: {result.stderr.strip()}")
        order = []
        for key in PLATFORMS:
            folder = staged / key
            if not folder.is_dir():
                raise Problem(f"stage-npm-packages.mjs staged no {key} package")
            manifest = read_manifest(folder)
            if manifest.get("name") != f"@diffyman/node-{key}" or manifest.get("main") != f"diffyman.{key}.node":
                raise Problem(f"{key}: package {manifest.get('name')!r} with main {manifest.get('main')!r}")
            if not (folder / manifest["main"]).is_file():
                if allow_missing:
                    continue  # A local trial without this platform's addon: not staged.
                raise Problem(f"{key}: no {manifest['main']}")
            write_manifest(folder, published_manifest(manifest, version))
            shutil.copytree(folder, out / f"node-{key}")
            order.append(f"node-{key}")
        main = published_manifest(read_manifest(staged / "node"), version)
        expected = {f"@diffyman/node-{key}": version for key in PLATFORMS}
        if main.get("optionalDependencies") != expected:
            raise Problem(f"@diffyman/node: optionalDependencies {main.get('optionalDependencies')}, expected {expected}")
        write_manifest(staged / "node", main)
        shutil.copytree(staged / "node", out / "node")
        order.append("node")
    return order


def stage_dir(source: Path, out: Path, name: str, expected: str, version: str) -> str:
    manifest = read_manifest(source)
    if manifest.get("name") != expected:
        raise Problem(f"{source}: package {manifest.get('name')!r}, expected {expected!r}")
    folder = out / name
    folder.mkdir()
    copy_files(source, folder, manifest["files"])
    write_manifest(folder, published_manifest(manifest, version))
    return folder.name


def check_folder(folder: Path, version: str) -> None:
    manifest = read_manifest(folder)
    if manifest["version"] != version or manifest.get("private") is not True:
        raise Problem(f"{folder.name}: wrong version or not private")
    if manifest.get("homepage") != "https://reddy.sh" or manifest.get("repository", {}).get("url") != REPOSITORY:
        raise Problem(f"{folder.name}: homepage must be https://reddy.sh and the repository {REPOSITORY}")
    if "scripts" in manifest:
        raise Problem(f"{folder.name}: has scripts")
    for name in ["README.md", *LICENSES]:
        if not (folder / name).is_file():
            raise Problem(f"{folder.name}: no {name}")
    for name in LICENSES:
        if (folder / name).read_bytes() != (ROOT / name).read_bytes():
            raise Problem(f"{folder.name}: {name} differs from the repository's")
    for entry in manifest.get("files", []):
        if not (folder / entry).exists():
            raise Problem(f"{folder.name}: {entry}, listed in \"files\", is missing")
    for path in folder.rglob("*"):
        if path.is_file() and path.suffix in (".js", ".mjs", ".cjs", ".ts", ".json", ".md") and OLD_WASM_SPECIFIER.search(path.read_bytes()):
            raise Problem(f"{path}: imports the unpublished module diffyman-wasm; use @diffyman/wasm")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--node-addons", required=True, type=Path)
    parser.add_argument("--wasm-pkg", required=True, type=Path)
    parser.add_argument("--react", type=Path, default=ROOT / "bindings" / "wasm" / "react")
    parser.add_argument("--allow-missing-platforms", action="store_true", help="for local trials: stage only the addons found")
    args = parser.parse_args()
    try:
        if args.out.exists() and any(args.out.iterdir()):
            raise Problem(f"{args.out} is not empty")
        args.out.mkdir(parents=True, exist_ok=True)
        order = stage_node(args.out, args.node_addons, args.version, args.allow_missing_platforms)
        order.append(stage_dir(args.wasm_pkg, args.out, "wasm", "@diffyman/wasm", args.version))
        order.append(stage_dir(args.react, args.out, "react", "@diffyman/react", args.version))
        for name in order:
            check_folder(args.out / name, args.version)
        (args.out / "publish-order.txt").write_text("\n".join(order) + "\n", encoding="utf-8")
        for name in order:
            print(f"{args.out / name}")
        return 0
    except Problem as problem:
        print(f"stage_npm.py: {problem}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
