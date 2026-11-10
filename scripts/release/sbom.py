#!/usr/bin/env python3
"""CycloneDX SBOMs of release artifacts, from cargo-cyclonedx.

    python scripts/release/sbom.py --target TRIPLE --out-dir DIR [--name NAME] CRATE_DIR [...]

Runs `cargo cyclonedx` (JSON, CycloneDX 1.5, without build-only
dependencies) for the given target (`all` for every target), then copies the
SBOM of each CRATE_DIR (such as crates/diffyman-cli) to
DIR/NAME.cdx.json, or DIR/<package>-<version>.cdx.json without --name
(one crate only with --name). cargo-cyclonedx names workspace crates by
their Cargo package ID, which holds the checkout's absolute path; that path
is replaced by /diffyman, so the SBOM names no build-machine folder. The
SBOM files cargo-cyclonedx writes next to every manifest are removed.
SOURCE_DATE_EPOCH, when set, is the SBOM's timestamp.

Needs cargo-cyclonedx on PATH. Python 3.11 or later, standard library only.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GENERATED = "diffyman-release-sbom"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--target", required=True, help="a target triple, or `all`")
    parser.add_argument("--out-dir", required=True, type=Path)
    parser.add_argument("--name", help="the output file name, without .cdx.json")
    parser.add_argument("crates", nargs="+", type=Path, help="crate folders, relative to the repository")
    args = parser.parse_args()
    if args.name and len(args.crates) != 1:
        parser.error("--name takes exactly one crate")
    subprocess.run(
        [
            "cargo", "cyclonedx", "--manifest-path", str(ROOT / "Cargo.toml"), "--format", "json",
            "--spec-version", "1.5", "--no-build-deps", "--target", args.target,
            "--override-filename", GENERATED,
        ],
        cwd=ROOT,
        check=True,
    )
    workspace = ROOT.as_posix().lstrip("/")  # "Users/..." or "D:/a/..."
    packages = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout)["packages"]
    versions = {p["name"]: p["version"] for p in packages}
    args.out_dir.mkdir(parents=True, exist_ok=True)
    try:
        for crate in args.crates:
            source = ROOT / crate / f"{GENERATED}.json"
            name = tomllib.loads((ROOT / crate / "Cargo.toml").read_text(encoding="utf-8"))["package"]["name"]
            text = source.read_text(encoding="utf-8")
            text = text.replace(f"file:///{workspace}", "file:///diffyman").replace(f"file://{workspace}", "file:///diffyman")
            if workspace in text:
                print(f"sbom.py: {source} still names {workspace}", file=sys.stderr)
                return 1
            out = args.out_dir / f"{args.name or name + '-' + versions[name]}.cdx.json"
            out.write_text(text, encoding="utf-8")
            print(out)
    finally:
        for pattern in (f"crates/*/{GENERATED}.json", f"bindings/*/{GENERATED}.json"):
            for leftover in ROOT.glob(pattern):
                leftover.unlink()
    return 0


if __name__ == "__main__":
    sys.exit(main())
