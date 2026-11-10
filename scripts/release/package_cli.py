#!/usr/bin/env python3
"""Packs a release build of the CLI into its archive.

    python scripts/release/package_cli.py --version 0.1.0 --target TRIPLE \
        --binary target/TRIPLE/release/diffyman --out DIR

Writes DIR/diffyman-VERSION-TRIPLE.tar.gz (DIR/...zip for Windows targets)
with one folder, diffyman-VERSION-TRIPLE/, holding the binary, README.md
and the two license texts, and prints the archive's path. The archive is
reproducible: entries sorted, owner 0, the time of SOURCE_DATE_EPOCH (the
commit time in the workflow; 0 when unset), and a gzip header without a
name or time. SHA256SUMS for every release file is written later, by the
workflow's checksums job.

Python 3.11 or later, standard library only.
"""

from __future__ import annotations

import argparse
import gzip
import io
import os
import re
import sys
import tarfile
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
README = ROOT / "crates" / "diffyman-cli" / "README.md"
LICENSES = [ROOT / "LICENSE-MIT", ROOT / "LICENSE-APACHE"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--version", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    if not re.fullmatch(r"\d+\.\d+\.\d+", args.version) or not re.fullmatch(r"[a-z0-9_]+(-[a-z0-9_]+){2,3}", args.target):
        print("package_cli.py: bad --version or --target", file=sys.stderr)
        return 2
    windows = "-windows-" in args.target
    exe = "diffyman.exe" if windows else "diffyman"
    if not args.binary.is_file():
        print(f"package_cli.py: no binary at {args.binary}", file=sys.stderr)
        return 2
    readme = README if README.is_file() else ROOT / "README.md"
    base = f"diffyman-{args.version}-{args.target}"
    entries = [(exe, args.binary.read_bytes(), 0o755)]
    entries += [(path.name, path.read_bytes(), 0o644) for path in [readme, *LICENSES]]
    entries.sort()
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
    args.out.mkdir(parents=True, exist_ok=True)
    if windows:
        archive = args.out / f"{base}.zip"
        stamp = time.gmtime(max(epoch, 315532800))[:6]  # zip times start in 1980
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as out:
            for name, data, mode in entries:
                member = zipfile.ZipInfo(f"{base}/{name}", date_time=stamp)
                member.external_attr = (0o100000 | mode) << 16
                member.compress_type = zipfile.ZIP_DEFLATED
                out.writestr(member, data)
    else:
        archive = args.out / f"{base}.tar.gz"
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT) as tar:
            folder = tarfile.TarInfo(base)
            folder.type, folder.mode, folder.mtime = tarfile.DIRTYPE, 0o755, epoch
            tar.addfile(folder)
            for name, data, mode in entries:
                info = tarfile.TarInfo(f"{base}/{name}")
                info.size, info.mode, info.mtime = len(data), mode, epoch
                tar.addfile(info, io.BytesIO(data))
        with archive.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0, compresslevel=9) as gz:
            gz.write(buffer.getvalue())
    print(archive)
    return 0


if __name__ == "__main__":
    sys.exit(main())
