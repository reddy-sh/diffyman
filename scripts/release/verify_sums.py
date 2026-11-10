#!/usr/bin/env python3
"""Checks downloaded release artifacts against the run's SHA256SUMS.

    python3 scripts/release/verify_sums.py --sums release/SHA256SUMS --root release ARTIFACT [...]

ROOT is the folder the workflow artifacts were downloaded into, one subfolder
per artifact (actions/download-artifact without merge-multiple), as the
`checksums` job of .github/workflows/release.yml lays them out before it
writes SHA256SUMS. Each ARTIFACT is an artifact name or a glob such as
`release-python-*`. The check fails unless every file under the matching
subfolders is listed in SHA256SUMS with the same SHA-256, and every line of
SHA256SUMS for those artifacts names a file that is there. So a publish job
uploads exactly the bytes that were built, checksummed and attested, and
nothing else.

Python 3.11 or later, standard library only.
"""

from __future__ import annotations

import argparse
import fnmatch
import hashlib
import re
import sys
from pathlib import Path

LINE = re.compile(r"^([0-9a-f]{64})  (\S.*)$")


class Problem(Exception):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def read_sums(path: Path) -> dict[str, str]:
    """Relative path -> SHA-256, from `sha256sum` output."""
    sums: dict[str, str] = {}
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        match = LINE.match(line)
        if match is None:
            raise Problem(f"{path}:{number}: not a sha256sum line")
        digest, name = match.groups()
        if name in sums:
            raise Problem(f"{path}:{number}: {name} is listed twice")
        sums[name] = digest
    return sums


def verify(sums_path: Path, root: Path, patterns: list[str]) -> list[str]:
    """The relative paths checked; raises Problem on the first difference."""
    sums = read_sums(sums_path)

    def wanted(name: str) -> bool:
        return any(fnmatch.fnmatchcase(name.split("/", 1)[0], pattern) for pattern in patterns)

    listed = {name for name in sums if wanted(name)}
    present = {
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file() and wanted(path.relative_to(root).as_posix())
    }
    if not listed:
        raise Problem(f"SHA256SUMS lists no file of {', '.join(patterns)}")
    missing = sorted(listed - present)
    if missing:
        raise Problem(f"listed in SHA256SUMS but not downloaded: {', '.join(missing)}")
    extra = sorted(present - listed)
    if extra:
        raise Problem(f"not listed in SHA256SUMS: {', '.join(extra)}")
    for name in sorted(listed):
        actual = sha256(root / name)
        if actual != sums[name]:
            raise Problem(f"{name}: SHA-256 {actual}, SHA256SUMS says {sums[name]}")
    return sorted(listed)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--sums", required=True, type=Path, help="the SHA256SUMS file")
    parser.add_argument("--root", required=True, type=Path, help="the folder with one subfolder per artifact")
    parser.add_argument("artifacts", nargs="+", help="artifact names or globs, such as release-maven")
    args = parser.parse_args()
    try:
        checked = verify(args.sums, args.root, args.artifacts)
    except (Problem, OSError) as problem:
        print(f"verify_sums.py: {problem}", file=sys.stderr)
        return 1
    for name in checked:
        print(f"OK {name}")
    print(f"{len(checked)} files match SHA256SUMS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
