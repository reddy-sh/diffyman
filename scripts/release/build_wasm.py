#!/usr/bin/env python3
"""Builds the release package @diffyman/wasm (docs/adr/0008-wasm-target.md).

    python3 scripts/release/build_wasm.py --out bindings/wasm/pkg-web [-- CARGO_ARGS...]

The steps of ADR 0008's release build, without wasm-pack (which downloads
wasm-bindgen and wasm-opt binaries on its own):

1. `wasm-bindgen --version` must be the version of the wasm-bindgen crate in
   Cargo.lock. Install it first, built from source with its own lockfile:
   cargo install --locked wasm-bindgen-cli --version "$(python3
   scripts/release/versions.py lock-version wasm-bindgen)".
2. `cargo build --release --locked --lib -p diffyman-wasm --target
   wasm32-unknown-unknown` (the workspace release profile, unchanged).
3. `wasm-bindgen --target web --remove-name-section
   --remove-producers-section` into OUT, which is emptied first; no
   wasm-opt.
4. bindings/wasm/scripts/finish-package.mjs --target web OUT: the npm name,
   metadata, README and license texts.
5. Checks that the module has no `name` or `producers` section, and prints
   its size raw, gzip -9 and brotli 11 (when the `brotli` command exists).

Python 3.11 or later, standard library only; needs cargo, wasm-bindgen and
Node.js on PATH.
"""

from __future__ import annotations

import argparse
import gzip
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGET = "wasm32-unknown-unknown"
MODULE = "diffyman_wasm"
REMOVED = {"name", "producers"}

sys.path.insert(0, str(Path(__file__).resolve().parent))
import versions


class Problem(Exception):
    pass


def leb128(data: bytes, pos: int) -> tuple[int, int]:
    value = shift = 0
    while True:
        byte = data[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if byte < 0x80:
            return value, pos


def custom_sections(data: bytes) -> list[str]:
    """The names of a module's custom sections, in order."""
    if data[:4] != b"\0asm":
        raise Problem("not a WebAssembly module")
    names = []
    pos = 8
    while pos < len(data):
        section_id = data[pos]
        size, body = leb128(data, pos + 1)
        if section_id == 0:
            length, start = leb128(data, body)
            names.append(data[start : start + length].decode("utf-8", "replace"))
        pos = body + size
    return names


def sizes(path: Path) -> dict[str, int | None]:
    data = path.read_bytes()
    result: dict[str, int | None] = {"raw": len(data), "gzip": len(gzip.compress(data, compresslevel=9, mtime=0)), "brotli": None}
    if shutil.which("brotli"):
        compressed = subprocess.run(["brotli", "-q", "11", "-c", str(path)], check=True, capture_output=True).stdout
        result["brotli"] = len(compressed)
    return result


def run(*command: str) -> None:
    print("+ " + " ".join(command), flush=True)
    subprocess.run(command, cwd=ROOT, check=True)


def build(out: Path, cargo_args: list[str]) -> Path:
    wanted = versions.lock_version("wasm-bindgen")
    found = shutil.which("wasm-bindgen")
    if found is None:
        raise Problem(f"no wasm-bindgen on PATH; run: cargo install --locked wasm-bindgen-cli --version {wanted}")
    printed = subprocess.run([found, "--version"], check=True, capture_output=True, text=True).stdout.split()
    if printed[-1:] != [wanted]:
        raise Problem(f"{found} is {' '.join(printed)}, Cargo.lock has wasm-bindgen {wanted}: run cargo install --locked wasm-bindgen-cli --version {wanted}")
    run("cargo", "build", "--release", "--locked", "--lib", "-p", "diffyman-wasm", "--target", TARGET, *cargo_args)
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
    module = target_dir / TARGET / "release" / f"{MODULE}.wasm"
    if out.exists():
        shutil.rmtree(out)
    run(
        found, "--target", "web", "--out-dir", str(out), "--out-name", MODULE,
        "--remove-name-section", "--remove-producers-section", str(module),
    )
    run("node", str(ROOT / "bindings" / "wasm" / "scripts" / "finish-package.mjs"), "--target", "web", str(out))
    wasm = out / f"{MODULE}_bg.wasm"
    left = REMOVED & set(custom_sections(wasm.read_bytes()))
    if left:
        raise Problem(f"{wasm} still has the custom sections {sorted(left)}")
    return wasm


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--out", required=True, type=Path, help="the package folder to write (emptied first)")
    parser.add_argument("cargo_args", nargs="*", help="after --: more cargo build arguments")
    args = parser.parse_args()
    try:
        wasm = build(args.out.resolve(), args.cargo_args)
    except (Problem, versions.Problem, subprocess.CalledProcessError, OSError) as problem:
        print(f"build_wasm.py: {problem}", file=sys.stderr)
        return 1
    measured = sizes(wasm)
    brotli = f"{measured['brotli']:,}" if measured["brotli"] is not None else "(no brotli command)"
    print(f"{wasm.name}: {measured['raw']:,} bytes raw, {measured['gzip']:,} gzip -9, {brotli} brotli 11")
    print(f"custom sections left: {', '.join(custom_sections(wasm.read_bytes())) or 'none'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
