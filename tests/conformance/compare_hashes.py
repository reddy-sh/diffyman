"""Compares hash tables written by run_bindings.py --hashes on several platforms.

Each table lists, per case, the SHA-256 of every surface's document with the
top-level "provenance" key removed, plus the golden file's. This checks that

  1. every table covers the same cases, and
  2. for each case, every hash in every table is the same: each surface on
     each platform wrote the same bytes, and they equal expected.json.

Usage: python tests/conformance/compare_hashes.py TABLE [TABLE ...]
(a folder is searched for *.tsv files).
"""

from __future__ import annotations

import argparse
import collections
import pathlib
import sys


def read_table(path: pathlib.Path) -> tuple[str, dict[str, set[str]], int]:
    """(platform, case -> hashes, number of hashed rows)."""
    platform = path.name
    hashes: dict[str, set[str]] = collections.defaultdict(set)
    rows = 0
    header_seen = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("# platform: "):
            platform = line.removeprefix("# platform: ")
            continue
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if not header_seen:
            if fields[:6] != ["case", "surface", "mode", "binding", "result", "sha256"]:
                raise SystemExit(f"{path}: not a run_bindings.py hash table")
            header_seen = True
            continue
        case, sha = fields[0], fields[5]
        hashes.setdefault(case, set())
        if sha != "-":
            hashes[case].add(sha)
            rows += 1
    if not header_seen:
        raise SystemExit(f"{path}: no header row")
    return platform, dict(hashes), rows


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("tables", nargs="+", type=pathlib.Path)
    args = parser.parse_args()

    paths: list[pathlib.Path] = []
    for item in args.tables:
        paths.extend(sorted(item.rglob("*.tsv")) if item.is_dir() else [item])
    if len(paths) < 2:
        raise SystemExit(f"need at least two tables to compare, found {len(paths)}")

    tables = [(path, *read_table(path)) for path in paths]
    problems = []
    all_cases = sorted(set().union(*(t[2].keys() for t in tables)))
    for path, platform, hashes, rows in tables:
        missing = [c for c in all_cases if c not in hashes]
        print(f"{path}: {platform}, {len(hashes)} cases, {rows} hashed documents")
        if missing:
            problems.append(f"{path} ({platform}) lacks {len(missing)} cases, such as {missing[0]}")

    for case in all_cases:
        seen: dict[str, list[str]] = collections.defaultdict(list)
        for _, platform, hashes, _ in tables:
            for sha in hashes.get(case, ()):
                seen[sha].append(platform)
        if len(seen) > 1:
            detail = "; ".join(f"{sha[:16]} on {', '.join(sorted(set(p)))}" for sha, p in sorted(seen.items()))
            problems.append(f"{case}: {len(seen)} different documents ({detail})")
        elif not seen:
            problems.append(f"{case}: no document on any platform")

    print()
    for problem in problems:
        print(f"FAIL {problem}")
    print(
        f"{len(all_cases)} cases across {len(tables)} platforms: "
        f"{'byte-identical everywhere' if not problems else f'{len(problems)} problems'}"
    )
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
