#!/usr/bin/env python3
"""Checks the Maven Central artifacts of the Java binding.

    python scripts/release/check_jar.py --version 0.1.0 bindings/java/target [--platforms LIST]

TARGET_DIR is Maven's output folder after `mvn -P release verify`. The
checks: diffyman-VERSION.jar holds the classes, both license texts under
META-INF and native/<platform>/<library> for every platform (all five by
default: macos-aarch64, macos-x86_64, linux-x86_64, linux-aarch64,
windows-x86_64; --platforms takes a comma-separated subset for a local
trial), and nothing else native; diffyman-VERSION-sources.jar and
diffyman-VERSION-javadoc.jar exist and are not empty. Signatures (.asc)
are checked by Maven Central itself.

Python 3.11 or later, standard library only.
"""

from __future__ import annotations

import argparse
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LIBRARIES = {
    "macos-aarch64": "libdiffyman_java.dylib",
    "macos-x86_64": "libdiffyman_java.dylib",
    "linux-x86_64": "libdiffyman_java.so",
    "linux-aarch64": "libdiffyman_java.so",
    "windows-x86_64": "diffyman_java.dll",
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--version", required=True)
    parser.add_argument("--platforms", default=",".join(LIBRARIES))
    parser.add_argument("target_dir", type=Path)
    args = parser.parse_args()
    platforms = [p for p in args.platforms.split(",") if p]
    problems = []
    unknown = sorted(set(platforms) - set(LIBRARIES))
    if unknown:
        problems.append(f"unknown platforms: {', '.join(unknown)}")
    jar = args.target_dir / f"diffyman-{args.version}.jar"
    if not jar.is_file():
        print(f"check_jar.py: no {jar}", file=sys.stderr)
        return 1
    with zipfile.ZipFile(jar) as archive:
        names = set(archive.namelist())
        for name in ("sh/reddy/diffyman/Diffyman.class", "sh/reddy/diffyman/NativeLibrary.class"):
            if name not in names:
                problems.append(f"{jar.name}: no {name}")
        for name in ("LICENSE-MIT", "LICENSE-APACHE"):
            inside = f"META-INF/{name}"
            if inside not in names:
                problems.append(f"{jar.name}: no {inside}")
            elif archive.read(inside) != (ROOT / name).read_bytes():
                problems.append(f"{jar.name}: {inside} differs from the repository's")
        expected = {f"native/{p}/{LIBRARIES[p]}" for p in platforms if p in LIBRARIES}
        native = {n for n in names if n.startswith("native/") and not n.endswith("/")}
        for name in sorted(expected - native):
            problems.append(f"{jar.name}: no {name}")
        for name in sorted(native - expected):
            problems.append(f"{jar.name}: unexpected {name}")
        for name in sorted(expected & native):
            if archive.getinfo(name).file_size < 100_000:
                problems.append(f"{jar.name}: {name} is too small to be the library")
    for classifier in ("sources", "javadoc"):
        extra = args.target_dir / f"diffyman-{args.version}-{classifier}.jar"
        if not extra.is_file():
            problems.append(f"no {extra.name}")
            continue
        with zipfile.ZipFile(extra) as archive:
            wanted = "sh/reddy/diffyman/Diffyman.java" if classifier == "sources" else "index.html"
            if wanted not in archive.namelist():
                problems.append(f"{extra.name}: no {wanted}")
    for problem in problems:
        print(f"check_jar.py: {problem}", file=sys.stderr)
    if problems:
        return 1
    print(f"{jar.name}: classes, licenses and {len(expected)} native libraries; sources and javadoc jars present")
    return 0


if __name__ == "__main__":
    sys.exit(main())
