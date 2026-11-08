"""Spike C size matrix for the diffyman WASM binding (`--target web`).

Builds the binding once per Cargo profile variant (opt-level x LTO), then runs
wasm-opt at -O3, -Os and -Oz on each raw module and reports raw, gzip -9 and
brotli (quality 11) sizes. Every variant is loaded in Node and must produce the
same Diff Document for a fixed input pair, so a size win can never hide a
broken module.

Run from the repository root (needs wasm-pack, Node 22+ and a wasm-opt binary):

    scripts/with-rustup uv run --no-project --python 3.12 --with brotli==1.1.0 \
        python bindings/wasm/bench/size.py --wasm-opt PATH/TO/wasm-opt

Outputs go to ~/diffyman-data/tmp/wasm-size (outside the repository).
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

import brotli

HERE = Path(__file__).resolve().parent
CRATE = HERE.parent
CHECK = HERE / "check_web_pkg.mjs"

# (name, opt-level, lto, strip). The workspace release profile is opt-level 3,
# thin LTO, codegen-units = 1, no strip; wasm32-unknown-unknown already uses
# panic = "abort". `strip = true` drops the "name" section, which is most of
# what wasm-opt removes, but it also drops `target_features`, so wasm-bindgen
# falls back to a different (return-pointer) ABI. The "name section removed"
# rows get the same saving without that side effect.
VARIANTS = [
    ("o3-thin", "3", "thin", False),
    ("o3-fat", "3", "fat", False),
    ("o3-thin-strip", "3", "thin", True),
    ("os-thin", "s", "thin", False),
    ("os-fat", "s", "fat", False),
    ("os-thin-strip", "s", "thin", True),
    ("oz-thin", "z", "thin", False),
    ("oz-fat", "z", "fat", False),
    ("oz-thin-strip", "z", "thin", True),
]
WASM_OPT_LEVELS = ["-O3", "-Os", "-Oz"]
# The features rustc enables by default for wasm32-unknown-unknown. wasm-opt
# normally reads them from the `target_features` custom section, but
# `strip = true` removes that section, so they are passed explicitly.
WASM_FEATURES = [
    "--enable-bulk-memory", "--enable-bulk-memory-opt", "--enable-nontrapping-float-to-int",
    "--enable-sign-ext", "--enable-mutable-globals", "--enable-multivalue", "--enable-reference-types",
    "--enable-call-indirect-overlong",
]


def leb128(data: bytes, pos: int) -> tuple[int, int]:
    value = shift = 0
    while True:
        byte = data[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if byte < 0x80:
            return value, pos


def remove_custom_sections(src: Path, dst: Path, names: set[str]) -> None:
    """Drops the named custom sections, as `wasm-bindgen --remove-name-section
    --remove-producers-section` does. Everything else is copied byte for byte."""
    data = src.read_bytes()
    out = bytearray(data[:8])  # magic and version
    pos = 8
    while pos < len(data):
        start = pos
        section_id = data[pos]
        size, body = leb128(data, pos + 1)
        end = body + size
        if section_id == 0:
            name_len, name_start = leb128(data, body)
            if data[name_start:name_start + name_len].decode() in names:
                pos = end
                continue
        out += data[start:end]
        pos = end
    dst.write_bytes(bytes(out))


def sizes(path: Path) -> dict[str, int]:
    data = path.read_bytes()
    return {
        "raw": len(data),
        "gzip9": len(gzip.compress(data, compresslevel=9, mtime=0)),
        "brotli11": len(brotli.compress(data, quality=11)),
    }


def build(name: str, opt: str, lto: str, strip: bool, out_root: Path) -> tuple[Path, float]:
    out = out_root / name
    if out.exists():
        shutil.rmtree(out)
    cmd = [
        "wasm-pack", "build", str(CRATE), "--release", "--target", "web", "--no-opt",
        "--out-dir", str(out), "--",
        "--config", f'profile.release.opt-level="{opt}"' if opt in ("s", "z") else f"profile.release.opt-level={opt}",
        "--config", f'profile.release.lto="{lto}"',
        "--config", f"profile.release.strip={'true' if strip else 'false'}",
    ]
    start = time.monotonic()
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return out, time.monotonic() - start


def check(pkg: Path, wasm: Path) -> str:
    result = subprocess.run(
        ["node", str(CHECK), str(pkg), str(wasm)], check=True, capture_output=True, text=True
    )
    return result.stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--wasm-opt", default=shutil.which("wasm-opt"), help="wasm-opt binary")
    parser.add_argument("--out", type=Path, default=Path.home() / "diffyman-data" / "tmp" / "wasm-size")
    args = parser.parse_args()
    if not args.wasm_opt:
        print("wasm-opt not found; pass --wasm-opt", file=sys.stderr)
        return 2
    args.out.mkdir(parents=True, exist_ok=True)
    version = subprocess.run([args.wasm_opt, "--version"], capture_output=True, text=True).stdout.strip()

    rows = []
    digests = set()
    for name, opt, lto, strip in VARIANTS:
        pkg, build_s = build(name, opt, lto, strip, args.out)
        raw = pkg / "diffyman_wasm_bg.wasm"
        files = [("none", raw)]
        nonames = pkg / "no-names.wasm"
        remove_custom_sections(raw, nonames, {"name", "producers"})
        files.append(("none, name section removed", nonames))
        for level in WASM_OPT_LEVELS:
            dst = pkg / f"opt{level}.wasm"
            subprocess.run([args.wasm_opt, level, *WASM_FEATURES, str(raw), "-o", str(dst)], check=True)
            files.append((level, dst))
        for level, path in files:
            digest = check(pkg, path)
            digests.add(digest)
            row = {"variant": name, "optLevel": opt, "lto": lto, "strip": strip, "wasmOpt": level,
                   "buildSeconds": round(build_s, 1), "outputSha256": digest[:16],
                   "moduleSha256": hashlib.sha256(path.read_bytes()).hexdigest()[:16], **sizes(path)}
            rows.append(row)
            print(json.dumps(row), flush=True)

    (args.out / "results.json").write_text(json.dumps({"wasmOpt": version, "rows": rows}, indent=2))
    print(f"\nwasm-opt: {version}")
    print("| Profile | wasm-opt | Raw | gzip -9 | brotli 11 |")
    print("|---|---|---|---|---|")
    for r in rows:
        strip = ", strip" if r["strip"] else ""
        print(f"| opt-level {r['optLevel']}, lto {r['lto']}{strip} | {r['wasmOpt']} | "
              f"{r['raw'] / 1000:.0f} KB | {r['gzip9'] / 1000:.0f} KB | {r['brotli11'] / 1000:.0f} KB |")
    best = min(rows, key=lambda r: r["brotli11"])
    print(f"\nsmallest (brotli): {best['variant']} {best['wasmOpt']}: {best}")
    print(f"distinct outputs across all modules: {len(digests)} (must be 1)")
    return 0 if len(digests) == 1 else 1


if __name__ == "__main__":
    sys.exit(main())
