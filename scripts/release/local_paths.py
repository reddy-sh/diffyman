#!/usr/bin/env python3
"""Keep build-machine paths out of release binaries.

    python scripts/release/local_paths.py rustflags [--target TRIPLE] [--crt-static]
        [--install-name NAME] [--github-env]
    python scripts/release/local_paths.py check FILE_OR_ARCHIVE [...]

`rustflags` prints RUSTFLAGS with one --remap-path-prefix per build-machine
folder rustc could record in a binary (panic locations, file!(), debug info):
the home folder, CARGO_HOME (crate sources), RUSTUP_HOME, the checkout and
the folders the manylinux containers of maturin-action use. The linker
records paths of its own, so for `--target *-windows-msvc` it adds
/PDBALTPATH:%_PDB% (the PDB by file name only), with --crt-static also a
static C runtime (no Visual C++ runtime needed), and for `*-apple-darwin`
with --install-name the install name @rpath/NAME for a cdylib. With
--github-env it appends RUSTFLAGS=... to $GITHUB_ENV instead of printing it.
Pass --target to cargo as well, so these flags reach only the target's code
(not build scripts or proc-macros).

`check` reads each file (every file under a folder argument), and every
member of a .zip, .whl, .jar, .tar.gz, .tgz or .crate, and fails when one holds one
of this machine's folders (UTF-8 or UTF-16LE), or, in a binary file, a
folder of the CI images.

Python 3.11 or later, standard library only.
"""

from __future__ import annotations

import argparse
import os
import sys
import tarfile
import zipfile
from collections.abc import Iterator
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# Fixed folders of CI images: the GitHub runners' homes and checkout roots,
# and the Rust installs of the manylinux containers maturin-action runs.
CONTAINER = [("/root/.cargo", "/cargo"), ("/usr/local/cargo", "/cargo"), ("/root/.rustup", "/rustup"), ("/usr/local/rustup", "/rustup")]
KNOWN = ["/home/runner", "/Users/runner", "C:\\Users\\runneradmin", "D:\\a\\", *(folder for folder, _ in CONTAINER)]


def this_machine() -> list[tuple[str, str]]:
    """(folder, replacement) for this machine's home, Rust installs and checkout."""
    home = Path.home()
    cargo_home = Path(os.environ.get("CARGO_HOME") or home / ".cargo")
    rustup_home = Path(os.environ.get("RUSTUP_HOME") or home / ".rustup")
    workspace = Path(os.environ.get("GITHUB_WORKSPACE") or ROOT)
    return [(str(home), "/home"), (str(rustup_home), "/rustup"), (str(cargo_home), "/cargo"), (str(workspace), ".")]


def local_folders() -> list[tuple[str, str]]:
    """(folder, replacement), the most general first: rustc applies the last match."""
    seen: set[str] = set()
    unique = []
    for folder, replacement in CONTAINER + this_machine():
        if folder not in seen:
            seen.add(folder)
            unique.append((folder, replacement))
    return unique


def rustflags(target: str | None, crt_static: bool, install_name: str | None) -> str:
    flags = []
    for folder, replacement in local_folders():
        if any(c.isspace() for c in folder):
            raise SystemExit(f"local_paths.py: {folder!r} contains whitespace, which RUSTFLAGS cannot carry")
        flags.append(f"--remap-path-prefix={folder}={replacement}")
    target = target or ""
    if target.endswith("-windows-msvc"):
        # The linker records the PDB's absolute path in the binary; keep the file name only.
        flags.append("-Clink-arg=/PDBALTPATH:%_PDB%")
        if crt_static:
            flags.append("-Ctarget-feature=+crt-static")
    if target.endswith("-apple-darwin") and install_name:
        # A cdylib's install name is otherwise its absolute path in the target folder.
        flags.append(f"-Clink-arg=-Wl,-install_name,@rpath/{install_name}")
    extra = os.environ.get("RUSTFLAGS", "").strip()
    return " ".join(([extra] if extra else []) + flags)


def encodings(folders: set[str]) -> list[bytes]:
    found = []
    for folder in sorted(folders):
        if len(folder) < 4:
            continue
        found.append(folder.encode())
        found.append(folder.encode("utf-16-le"))
        if "\\" in folder:
            found.append(folder.replace("\\", "/").encode())
    return found


def needles() -> tuple[list[bytes], list[bytes]]:
    """(this machine's folders, checked everywhere; the CI image folders, checked in binaries)."""
    return encodings({folder for folder, _ in this_machine()}), encodings(set(KNOWN))


def is_text(data: bytes) -> bool:
    if b"\x00" in data:
        return False
    try:
        data.decode("utf-8")
    except UnicodeDecodeError:
        return False
    return True


def members(path: Path) -> Iterator[tuple[str, bytes]]:
    name = path.name.lower()
    if name.endswith((".zip", ".whl", ".jar")):
        with zipfile.ZipFile(path) as archive:
            for info in archive.infolist():
                if not info.is_dir():
                    yield f"{path}!{info.filename}", archive.read(info)
    elif name.endswith((".tar.gz", ".tgz", ".crate")):
        with tarfile.open(path, "r:gz") as tar:
            for entry in tar:
                if entry.isfile():
                    handle = tar.extractfile(entry)
                    assert handle is not None
                    yield f"{path}!{entry.name}", handle.read()
    else:
        yield str(path), path.read_bytes()


def check(paths: list[Path]) -> int:
    everywhere, in_binaries = needles()
    bad = 0
    count = 0
    files: list[Path] = []
    for path in paths:
        if path.is_dir():
            files.extend(sorted(p for p in path.rglob("*") if p.is_file()))
        elif path.is_file():
            files.append(path)
        else:
            print(f"local_paths.py: no such file or folder: {path}", file=sys.stderr)
            return 2
    for path in files:
        for where, data in members(path):
            count += 1
            # A source file may name a CI folder on purpose (a check like this
            # one); a build-machine folder of this run is never expected.
            patterns = everywhere if is_text(data) else everywhere + in_binaries
            hits = sorted({p.decode("utf-16-le") if b"\x00" in p else p.decode() for p in patterns if p in data})
            if hits:
                bad += 1
                print(f"{where}: contains {', '.join(hits)}", file=sys.stderr)
    if bad:
        print(f"local_paths.py: {bad} of {count} files contain a build-machine path", file=sys.stderr)
        return 1
    print(f"no build-machine path in {count} files")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    flags = sub.add_parser("rustflags", help="RUSTFLAGS that remap the build-machine folders")
    flags.add_argument("--target", help="the Rust target triple")
    flags.add_argument("--crt-static", action="store_true", help="Windows: link the C runtime statically")
    flags.add_argument("--install-name", help="macOS: the cdylib's install name, @rpath/NAME")
    flags.add_argument("--github-env", action="store_true", help="append RUSTFLAGS=... to $GITHUB_ENV")
    checker = sub.add_parser("check", help="fail when a file holds a build-machine folder")
    checker.add_argument("paths", nargs="+", type=Path)
    args = parser.parse_args()
    if args.command == "check":
        return check(args.paths)
    value = rustflags(args.target, args.crt_static, args.install_name)
    if args.github_env:
        with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as env:
            env.write(f"RUSTFLAGS={value}\n")
    else:
        print(value)
    return 0


if __name__ == "__main__":
    sys.exit(main())
