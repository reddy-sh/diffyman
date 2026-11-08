#!/usr/bin/env python3
"""Generate the folder and archive cases under tests/corpus/tree/.

Each case holds `left` and `right` inputs that are folders (`left/`,
`right/`), archives (`left.zip`, `right.tar.gz`, ...) or plain files, plus
`options.json` and `case.md`. Re-running rewrites the inputs, options.json and
case.md of every case; it never touches expected.json.

Every byte is written by this script, never by a compression library, so
the fixtures are the same on every machine and Python version:

- zip and tar headers are packed field by field (timestamps 0, owner 0);
- deflate data is written with fixed Huffman codes (literals, and runs of
  zero bytes as length-258 matches), checked here against zlib;
- gzip members use the same deflate writer; zstd frames use raw blocks.

Folder inputs hold regular files only: git stores no empty folders and
symbolic links are not portable, so those cases live in the Rust tests.
Ignore-file cases use `.ignore` files, because a `.gitignore` inside the
corpus would hide fixture files from git itself.

Usage (from the repository root):

    uv run --python 3.12 python tests/corpus/tools/make_tree_cases.py

All content in this file is original and written for the diffyman corpus.
"""

from __future__ import annotations

import json
import shutil
import struct
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "tree"

TREE_DEFAULTS = {
    "followSymlinks": False,
    "ignoreFiles": False,
    "exclude": [],
    "detectMoves": True,
    "maxDepth": 3,
    "childBudget": 16777216,
}


def opts(adapter: str = "auto", **tree: object) -> dict:
    """A fully spelled-out options object (diff-document.md section 9)."""
    unknown = set(tree) - set(TREE_DEFAULTS)
    if unknown:
        raise SystemExit(f"unknown tree options {sorted(unknown)}")
    return {
        "algorithm": "histogram",
        "granularity": "line",
        "ignore": {"whitespace": "none", "case": False, "blankLines": False, "regex": []},
        "context": 3,
        "positionEncoding": "utf-8",
        "detectMoves": False,
        "adapter": adapter,
        "tree": {**TREE_DEFAULTS, **tree},
    }


def noise(seed: int, length: int) -> bytes:
    """Deterministic pseudo-random bytes (xorshift64, one byte per step)."""
    state = (seed | 1) & 0xFFFFFFFFFFFFFFFF
    out = bytearray(length)
    for i in range(length):
        state ^= (state << 13) & 0xFFFFFFFFFFFFFFFF
        state ^= state >> 7
        state ^= (state << 17) & 0xFFFFFFFFFFFFFFFF
        out[i] = (state >> 24) & 0xFF
    return bytes(out)


def lines(*text: str) -> bytes:
    return "".join(t + "\n" for t in text).encode("utf-8")


# --- deflate, gzip, zstd ---------------------------------------------------


class BitWriter:
    def __init__(self) -> None:
        self.out = bytearray()
        self.acc = 0
        self.n = 0

    def bits(self, value: int, count: int) -> None:
        """Write `count` bits of `value`, least significant first."""
        for i in range(count):
            self.acc |= ((value >> i) & 1) << self.n
            self.n += 1
            if self.n == 8:
                self.out.append(self.acc)
                self.acc, self.n = 0, 0

    def code(self, value: int, count: int) -> None:
        """Write a Huffman code, most significant bit first."""
        for i in reversed(range(count)):
            self.bits((value >> i) & 1, 1)

    def finish(self) -> bytes:
        if self.n:
            self.out.append(self.acc)
        return bytes(self.out)


def deflate(data: bytes) -> bytes:
    """Raw deflate with one fixed-Huffman block. Bytes are literals, except
    that a run of zero bytes after a zero literal becomes matches of length
    258 at distance 1 (13 bits for 258 bytes)."""
    w = BitWriter()
    w.bits(1, 1)  # BFINAL
    w.bits(1, 2)  # BTYPE = fixed Huffman

    def literal(b: int) -> None:
        if b < 144:
            w.code(0x30 + b, 8)
        else:
            w.code(0x190 + b - 144, 9)

    i = 0
    while i < len(data):
        b = data[i]
        literal(b)
        i += 1
        if b == 0:
            while i + 258 <= len(data) and data[i : i + 258] == bytes(258):
                w.code(0xC5, 8)  # length code 285 (258)
                w.code(0, 5)  # distance code 0 (1)
                i += 258
    w.code(0, 7)  # end of block
    return w.finish()


def gzip_member(data: bytes) -> bytes:
    header = b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x00\xff"
    trailer = struct.pack("<II", zlib.crc32(data) & 0xFFFFFFFF, len(data) & 0xFFFFFFFF)
    return header + deflate(data) + trailer


def zstd_skippable(nibble: int, data: bytes) -> bytes:
    """A zstd skippable frame (magic 0x184D2A50 + nibble), as zstd tools
    write for metadata."""
    return struct.pack("<II", 0x184D2A50 | nibble, len(data)) + data


def zstd_raw(data: bytes) -> bytes:
    """One zstd frame of raw blocks: single segment, 4-byte content size."""
    out = bytearray(b"\x28\xb5\x2f\xfd\xa0") + struct.pack("<I", len(data))
    chunks = [data[i : i + 100_000] for i in range(0, len(data), 100_000)] or [b""]
    for k, chunk in enumerate(chunks):
        header = (1 if k == len(chunks) - 1 else 0) | (len(chunk) << 3)
        out += struct.pack("<I", header)[:3] + chunk
    return bytes(out)


# --- zip -------------------------------------------------------------------


def zentry(name: str | bytes, data: bytes = b"", *, deflated: bool = False, flags: int = 0, mode: int | None = None) -> dict:
    return {
        "name": name.encode("utf-8") if isinstance(name, str) else name,
        "data": data,
        "deflated": deflated,
        "flags": flags,
        "mode": mode,
    }


def zip64_end_record(count: int, cd_size: int, cd_offset: int) -> bytes:
    """A zip64 end-of-central-directory record that declares `count` entries."""
    return b"PK\x06\x06" + struct.pack("<QHHIIQQQQ", 44, 45, 45, 0, 0, count, count, cd_size, cd_offset)


def zip_bytes(entries: list[dict], *, zip64_count: int | None = None) -> bytes:
    """A zip archive written field by field. `zip64_count` ends it the zip64
    way: a zip64 end record that claims that many entries (a forged count),
    its locator, and a zip32 end record whose fields defer to it."""
    out = bytearray()
    central = bytearray()
    for e in entries:
        stored = deflate(e["data"]) if e["deflated"] else e["data"]
        method = 8 if e["deflated"] else 0
        crc = zlib.crc32(e["data"]) & 0xFFFFFFFF
        made_by = 0x0314 if e["mode"] is not None else 0x0014
        external = (e["mode"] or 0) << 16
        fields = struct.pack(
            "<HHHHHIIIHH", 20, e["flags"], method, 0, 0x21, crc, len(stored), len(e["data"]), len(e["name"]), 0
        )
        offset = len(out)
        out += b"PK\x03\x04" + fields + e["name"] + stored
        central += (
            b"PK\x01\x02"
            + struct.pack("<H", made_by)
            + fields
            + struct.pack("<HHHII", 0, 0, 0, external, offset)
            + e["name"]
        )
    start = len(out)
    out += central
    if zip64_count is not None:
        record_at = len(out)
        out += zip64_end_record(zip64_count, len(central), start)
        out += b"PK\x06\x07" + struct.pack("<IQI", 0, record_at, 1)
        out += b"PK\x05\x06" + struct.pack("<HHHHIIH", 0, 0, 0xFFFF, 0xFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0)
        return bytes(out)
    count = len(entries)
    out += b"PK\x05\x06" + struct.pack("<HHHHIIH", 0, 0, count, count, len(central), start, 0)
    return bytes(out)


def zip_shared(names: list[str], data: bytes) -> bytes:
    """A zip whose central directory lists every name in `names`, all
    pointing at one stored local entry at offset 0 (overlapping entries)."""
    crc = zlib.crc32(data) & 0xFFFFFFFF
    first = names[0].encode("utf-8")

    def fields(name: bytes) -> bytes:
        return struct.pack("<HHHHHIIIHH", 20, 0, 0, 0, 0x21, crc, len(data), len(data), len(name), 0)

    out = bytearray(b"PK\x03\x04" + fields(first) + first + data)
    central = bytearray()
    for name in names:
        raw = name.encode("utf-8")
        central += b"PK\x01\x02" + struct.pack("<H", 0x0014) + fields(raw) + struct.pack("<HHHII", 0, 0, 0, 0, 0) + raw
    start = len(out)
    out += central
    out += b"PK\x05\x06" + struct.pack("<HHHHIIH", 0, 0, len(names), len(names), len(central), start, 0)
    return bytes(out)


# --- tar -------------------------------------------------------------------


def tar_header(name: bytes, size: int, flag: bytes = b"0", link: bytes = b"", prefix: bytes = b"") -> bytes:
    h = bytearray(512)
    h[0 : len(name)] = name
    h[100:108] = b"0000644\x00"
    h[108:116] = b"0000000\x00"
    h[116:124] = b"0000000\x00"
    h[124:136] = b"%011o\x00" % size
    h[136:148] = b"00000000000\x00"
    h[156:157] = flag
    h[157 : 157 + len(link)] = link
    h[257:263] = b"ustar\x00"
    h[263:265] = b"00"
    h[345 : 345 + len(prefix)] = prefix
    h[148:156] = b" " * 8
    h[148:156] = b"%06o\x00 " % sum(h)
    return bytes(h)


def pad(data: bytes) -> bytes:
    return data + bytes(-len(data) % 512)


def tar_bytes(entries: list[tuple]) -> bytes:
    """Entries: ("file", name, data), ("dir", name), ("symlink", name, target),
    ("hardlink", name, target), ("fifo", name), ("pax", records, entry) and
    ("longname", name, data). Names are str (UTF-8) or bytes."""

    def b(s: str | bytes) -> bytes:
        return s.encode("utf-8") if isinstance(s, str) else s

    out = bytearray()
    for e in entries:
        kind = e[0]
        if kind == "file":
            out += tar_header(b(e[1]), len(e[2])) + pad(e[2])
        elif kind == "dir":
            out += tar_header(b(e[1]), 0, b"5")
        elif kind == "symlink":
            out += tar_header(b(e[1]), 0, b"2", b(e[2]))
        elif kind == "hardlink":
            out += tar_header(b(e[1]), 0, b"1", b(e[2]))
        elif kind == "fifo":
            out += tar_header(b(e[1]), 0, b"6")
        elif kind == "pax":
            body = bytearray()
            for key, value in e[1]:
                record = b" " + key.encode() + b"=" + b(value) + b"\n"
                n = len(record) + 1
                while len(str(n)) + len(record) != n:
                    n = len(str(n)) + len(record)
                body += str(n).encode() + record
            out += tar_header(b"PaxHeaders/entry", len(body), b"x") + pad(bytes(body))
            out += tar_bytes([e[2]])[:-1024]
        elif kind == "longname":
            name = b(e[1]) + b"\x00"
            out += tar_header(b"././@LongLink", len(name), b"L") + pad(name)
            out += tar_header(b(e[1])[:99], len(e[2])) + pad(e[2])
        else:
            raise SystemExit(f"unknown tar entry {kind}")
    return bytes(out) + bytes(1024)


# --- cases -----------------------------------------------------------------

CASES: list[dict] = []


def case(slug: str, left, right, options: dict, about: str) -> None:
    """`left` and `right` are a dict (a folder: relative path -> bytes) or a
    (file name, bytes) pair."""
    CASES.append({"slug": slug, "left": left, "right": right, "options": options, "about": about})


BLOB = noise(7, 3000)
BLOB_EDITED = BLOB[:1500] + b"\xff" + BLOB[1501:]
APP = lines("def main():", "    print('hello')", "", "if __name__ == '__main__':", "    main()")
HELPERS = lines("def add(a, b):", "    return a + b")
SHARED = lines("shared text that appears twice")
INNER_LEFT = zip_bytes([zentry("a.txt", lines("hello")), zentry("keep.txt", lines("same"))])
INNER_RIGHT = zip_bytes([zentry("a.txt", lines("hello there")), zentry("keep.txt", lines("same")), zentry("b.txt", lines("new"))])
# Over the 100 bytes of a ustar name field, in a few components.
PAX_PATH = "pax-" + "p" * 70 + "/" + "q" * 70 + "/deep.txt"
GNU_PATH = "gnu-" + "g" * 70 + "/" + "h" * 70 + "/long.txt"
TAR_CONTENT = [
    ("dir", "docs/"),
    ("file", "docs/guide.txt", lines("step one", "step two")),
    ("file", "src/app.js", lines("console.log('app');")),
    ("symlink", "latest", "src/app.js"),
]

case(
    "001-identical-folders",
    {"README.md": lines("# Project"), "src/app.py": APP, "src/lib/helpers.py": HELPERS},
    {"README.md": lines("# Project"), "src/app.py": APP, "src/lib/helpers.py": HELPERS},
    opts(),
    "Two folders with the same files in nested folders. A correct result is `kind: \"tree\"`, "
    "`status: \"identical\"`, every entry `unchanged` (directories included), equal manifest digests "
    "in `left.sha256` and `right.sha256`, `mediaType: \"inode/directory\"` and similarity 1.",
)
case(
    "002-folder-changes",
    {
        "README.md": lines("# Project", "", "Old intro."),
        "src/app.py": APP,
        "src/helpers.py": HELPERS,
        "data/blob.bin": BLOB,
        "old.txt": lines("to be removed"),
    },
    {
        "README.md": lines("# Project", "", "New intro."),
        "src/app.py": APP.replace(b"hello", b"hello, world"),
        "src/helpers.py": HELPERS,
        "data/blob.bin": BLOB_EDITED,
        "new/feature.txt": lines("added"),
    },
    opts(),
    "Edited text files, an edited binary file, a removed file and an added file in a new folder. "
    "A correct result marks `README.md` and `src/app.py` `modified` with text child documents, "
    "`data/blob.bin` `modified` with a binary child document (one replaced byte at offset 1500), "
    "`old.txt` `removed`, `new` and `new/feature.txt` `added`, and the rest `unchanged`.",
)
case(
    "003-folder-renames",
    {
        "src/util.py": HELPERS,
        "assets/logo.bin": BLOB,
        "notes/a.txt": SHARED,
        "notes/b.txt": SHARED,
        "empty-old.txt": b"",
    },
    {
        "lib/util.py": HELPERS,
        "img/logo.bin": BLOB,
        "archive/x.txt": SHARED,
        "moved/b.txt": SHARED,
        "empty-new.txt": b"",
    },
    opts(),
    "Files moved to new paths with their content unchanged. A correct result has `moved` entries "
    "at the right paths with `fromPath` set: `lib/util.py` from `src/util.py`, `img/logo.bin` from "
    "`assets/logo.bin`, and for the two files with equal content `moved/b.txt` from `notes/b.txt` "
    "(same last component first) and `archive/x.txt` from `notes/a.txt`. Empty files never move: "
    "`empty-old.txt` is `removed` and `empty-new.txt` `added`. Moved files get no child document.",
)
case(
    "004-folder-type-change",
    {"config": lines("key=value"), "docs/index.txt": lines("index")},
    {"config/app.ini": lines("key=value"), "docs": lines("docs became a file")},
    opts(),
    "A file replaced by a folder and a folder replaced by a file. A correct result has `config` and "
    "`docs` as `typeChange` (left and right types differ), `config/app.ini` `added` and "
    "`docs/index.txt` `removed`.",
)
case(
    "005-folder-exclude",
    {
        "app.log": lines("left log"),
        "keep.log": lines("kept, left"),
        "build/out.bin": noise(3, 64),
        "src/main.c": lines("int main(void) { return 0; }"),
    },
    {
        "app.log": lines("right log"),
        "keep.log": lines("kept, right"),
        "build/out.bin": noise(4, 64),
        "src/main.c": lines("int main(void) { return 0; }"),
    },
    opts(exclude=["*.log", "build/", "!keep.log"]),
    "`tree.exclude` with a glob, a directory pattern and a negation. A correct result leaves out "
    "`app.log` and everything under `build/`, keeps `keep.log` (re-included) as `modified` with a "
    "child document, and lists `src` and `src/main.c` as `unchanged`.",
)
case(
    "006-folder-ignore-files",
    {
        ".ignore": lines("*.tmp", "cache/"),
        "a.tmp": lines("left temp"),
        "cache/data.bin": noise(5, 32),
        "src/.ignore": lines("!keep.tmp"),
        "src/keep.tmp": lines("kept, left"),
        "src/code.txt": lines("code"),
    },
    {
        ".ignore": lines("*.tmp", "cache/"),
        "a.tmp": lines("right temp"),
        "cache/data.bin": noise(6, 32),
        "src/.ignore": lines("!keep.tmp"),
        "src/keep.tmp": lines("kept, right"),
        "src/code.txt": lines("code"),
    },
    opts(ignoreFiles=True),
    "`tree.ignoreFiles` with `.ignore` files at the root and in a subfolder (a nested file can "
    "re-include). A correct result leaves out `a.tmp` and `cache/`, lists both `.ignore` files as "
    "`unchanged`, and `src/keep.tmp` as `modified`.",
)
case(
    "007-folder-no-moves",
    {"src/util.py": HELPERS, "notes/a.txt": SHARED},
    {"lib/util.py": HELPERS, "notes/c.txt": SHARED},
    opts(detectMoves=False),
    "The renames of 003 with `tree.detectMoves: false`. A correct result has no `moved` entry: the "
    "old paths are `removed` and the new paths `added`.",
)
case(
    "008-folder-child-budget",
    {"a.txt": lines("alpha"), "b.txt": lines("beta")},
    {"a.txt": lines("ALPHA"), "b.txt": lines("BETA")},
    opts(childBudget=1),
    "A child budget of one byte: no child document fits. A correct result has both files "
    "`modified` with `childDocument: null` and one `children-omitted` warning.",
)
case(
    "009-folder-children-off",
    {"a.txt": lines("alpha"), "b.txt": lines("beta")},
    {"a.txt": lines("ALPHA"), "b.txt": lines("BETA")},
    opts(childBudget=0),
    "`tree.childBudget: 0` turns child documents off. A correct result has both files `modified` "
    "with `childDocument: null` and no warning at all.",
)
case(
    "010-folder-vs-zip",
    {"a.txt": lines("one"), "sub/b.txt": lines("two")},
    ("right.zip", zip_bytes([zentry("a.txt", lines("one")), zentry("sub/b.txt", lines("two"))])),
    opts(),
    "A folder against a zip of the same files; the zip has no directory entries. A correct result "
    "is `identical`: `sub` is implied by `sub/b.txt` on the right, the right side has "
    "`mediaType: \"application/zip\"` and the zip's own size and SHA-256.",
)
case(
    "011-zip-changes",
    (
        "left.zip",
        zip_bytes(
            [
                zentry("docs/"),
                zentry("docs/guide.txt", lines("step one", "step two")),
                zentry("src/app.js", lines("console.log('app');")),
                zentry("latest", b"src/app.js", mode=0o120777),
                zentry("old/name.txt", lines("moving file")),
            ]
        ),
    ),
    (
        "right.zip",
        zip_bytes(
            [
                zentry("docs/guide.txt", lines("step one", "step 2", "step three")),
                zentry("src/app.js", lines("console.log('app');")),
                zentry("src/main.js", lines("import './app.js';")),
                zentry("latest", b"src/main.js", mode=0o120777),
                zentry("new/name.txt", lines("moving file")),
            ]
        ),
    ),
    opts(),
    "Two zips: an edited file, an added file, a moved file and a symbolic link whose target "
    "changed. A correct result has `docs/guide.txt` `modified` with a text child, `latest` a "
    "`modified` symlink (targets `src/app.js` and `src/main.js`, never resolved), `new/name.txt` "
    "`moved` from `old/name.txt`, and `docs` `unchanged` (explicit on the left, implied on the right).",
)
case(
    "012-tar-vs-targz",
    ("left.tar", tar_bytes(TAR_CONTENT)),
    ("right.tar.gz", gzip_member(tar_bytes(TAR_CONTENT))),
    opts(),
    "The same tar, plain and gzip-compressed. A correct result is `identical`, entry for entry, "
    "with `application/x-tar` and `application/gzip` media types and different archive hashes.",
)
case(
    "013-tarzst-vs-tar",
    (
        "left.tar.zst",
        zstd_raw(
            tar_bytes(
                [
                    ("file", "src/app.js", lines("console.log('app');")),
                    ("symlink", "latest", "src/app.js"),
                    ("hardlink", "copy.js", "src/app.js"),
                    ("fifo", "pipe"),
                    ("pax", [("path", PAX_PATH)], ("file", "ignored-name", lines("deep v1"))),
                    ("longname", GNU_PATH, lines("gnu v1")),
                ]
            )
        ),
    ),
    (
        "right.tar",
        tar_bytes(
            [
                ("file", "src/app.js", lines("console.log('app v2');")),
                ("symlink", "latest", "src/app.js"),
                ("hardlink", "copy.js", "src/other.js"),
                ("fifo", "pipe"),
                ("pax", [("path", PAX_PATH)], ("file", "ignored-name", lines("deep v2"))),
                ("longname", GNU_PATH, lines("gnu v1")),
            ]
        ),
    ),
    opts(),
    "A zstd-compressed tar against a plain one, with pax and GNU long names, links and a FIFO. "
    "A correct result reads the 154-byte pax path and the GNU long name, lists `latest` as an "
    "`unchanged` symlink, `copy.js` as a `modified` hardlink, `pipe` as `other`, and the edited "
    "files as `modified` with text children.",
)
case(
    "014-zip-traversal-names",
    (
        "left.zip",
        zip_bytes(
            [
                zentry("../evil.txt", lines("escape")),
                zentry("/etc/passwd", lines("root:x:0:0")),
                zentry("C:\\Windows\\win.ini", lines("[fonts]")),
                zentry("a/../../b.txt", lines("climb")),
                zentry("safe.txt", lines("safe")),
            ]
        ),
    ),
    ("right.zip", zip_bytes([zentry("safe.txt", lines("safe"))])),
    opts(),
    "Entry names that are absolute, start with a drive letter or climb with `..`. A correct result "
    "lists them as `removed` entries with paths as given after normalization (`../evil.txt`, "
    "`/etc/passwd`, `C:/Windows/win.ini`, `a/../../b.txt`, and their implied parents such as "
    "`..` and `/etc`), never resolved, with an `unsafe-path` warning for the left side.",
)
case(
    "015-zip-bomb",
    (
        "left.zip",
        zip_bytes([zentry("zeros.bin", bytes(2 << 20), deflated=True), zentry("readme.txt", lines("a"))]),
    ),
    (
        "right.zip",
        zip_bytes(
            [
                zentry("zeros.bin", bytes(2 << 20), deflated=True),
                zentry("more-zeros.bin", bytes(3 << 20), deflated=True),
                zentry("readme.txt", lines("b")),
            ]
        ),
    ),
    opts(),
    "A tiny zip bomb: 2 MiB and 3 MiB of zeros deflate to about 13 and 20 KB, over the ratio "
    "limit of 100. A correct result never inflates them: their `sha256` is `null` and `size` is "
    "the declared size, `zeros.bin` is `unchanged` (equal stored bytes), `more-zeros.bin` is "
    "`added`, `readme.txt` is `modified` with a child, there are `tree-limit` warnings for both "
    "sides, and the status is `partial`.",
)
case(
    "016-zip-duplicates",
    (
        "left.zip",
        zip_bytes(
            [
                zentry("a.txt", lines("first copy")),
                zentry("a.txt", lines("second copy")),
                zentry("./b.txt", lines("dot slash")),
                zentry("b.txt", lines("plain")),
                zentry("x", lines("a file named x")),
                zentry("x/y.txt", lines("below a file")),
            ]
        ),
    ),
    ("right.zip", zip_bytes([zentry("a.txt", lines("second copy")), zentry("b.txt", lines("plain")), zentry("x/y.txt", lines("below a file"))])),
    opts(),
    "Duplicate paths: the same name twice, `./b.txt` and `b.txt`, and a file `x` with an entry "
    "below it. A correct result keeps the later entry of each path (`a.txt` and `b.txt` are "
    "`unchanged`), keeps `x` as a file (a `typeChange` against the implied folder on the right), "
    "and gives a `duplicate-path` warning for the left side.",
)
case(
    "017-zip-encrypted",
    (
        "left.zip",
        zip_bytes(
            [
                zentry("secret.bin", noise(11, 40), flags=1),
                zentry("changed.bin", noise(20, 40), flags=1),
                zentry("plain.txt", lines("open")),
            ]
        ),
    ),
    (
        "right.zip",
        zip_bytes(
            [
                zentry("secret.bin", noise(11, 40), flags=1),
                zentry("changed.bin", noise(22, 40), flags=1),
                zentry("plain.txt", lines("open")),
            ]
        ),
    ),
    opts(),
    "Entries with the encryption flag. A correct result never decrypts them: `sha256` is `null`, "
    "`secret.bin` is `unchanged` (equal stored bytes), `changed.bin` `modified` without a child, "
    "`encrypted-entry` warnings for both sides, and status `partial`.",
)
case(
    "018-nested-zip",
    ("left.zip", zip_bytes([zentry("inner.zip", INNER_LEFT), zentry("top.txt", lines("top"))])),
    ("right.zip", zip_bytes([zentry("inner.zip", INNER_RIGHT), zentry("top.txt", lines("top"))])),
    opts(),
    "A zip inside a zip. A correct result has `inner.zip` `modified` with a child document of kind "
    "`tree` (depth 1), whose `a.txt` entry has its own text child and whose `b.txt` is `added`.",
)
case(
    "019-nested-depth-limit",
    ("left.zip", zip_bytes([zentry("inner.zip", INNER_LEFT), zentry("top.txt", lines("top"))])),
    ("right.zip", zip_bytes([zentry("inner.zip", INNER_RIGHT), zentry("top.txt", lines("top"))])),
    opts(maxDepth=0),
    "The inputs of 018 with `tree.maxDepth: 0`. A correct result compares `inner.zip` as bytes: its "
    "child document is kind `binary` with a `depth-limit` warning.",
)
case(
    "020-zip-corrupt",
    ("left.zip", zip_bytes([zentry("a.txt", lines("a"))])[:-30]),
    ("right.zip", zip_bytes([zentry("a.txt", lines("a"))])),
    opts(),
    "A zip whose end record is cut off. Its first bytes still say zip, so the pair is a tree, and "
    "a correct result is an error document with `invalid-archive` for the left side, file meta "
    "for both sides, and `tree: null`.",
)
case(
    "021-tree-adapter-on-text",
    ("left.txt", lines("plain text")),
    ("right.txt", lines("other text")),
    opts(adapter="tree"),
    "`adapter: \"tree\"` on two text files. A correct result is a `tree` error document with "
    "`invalid-archive` for both sides.",
)
case(
    "022-zip-forged-entry-count",
    ("left.zip", zip_bytes([zentry("a.txt", lines("a"))], zip64_count=1 << 40)),
    ("right.zip", zip_bytes([zentry("a.txt", lines("a"))])),
    opts(),
    "A zip that ends the zip64 way (zip64 end record, locator, and a zip32 end record whose "
    "fields defer to it), with a zip64 end record that claims 2^40 entries. A correct result "
    "refuses it before reading the central directory: an error document with `limit-exceeded`.",
)
case(
    "023-folder-vs-plain-file",
    {"a.txt": lines("one")},
    ("right.txt", lines("one")),
    opts(),
    "A folder against a plain text file. A correct result is an error document with "
    "`invalid-archive` for the right side; the left side keeps its folder meta.",
)
case(
    "024-invalid-exclude",
    {"a.txt": lines("one")},
    {"a.txt": lines("two")},
    opts(exclude=["[z-a]"]),
    "A `tree.exclude` pattern with an invalid range. A correct result is an error document with "
    "`invalid-glob`; nothing is listed.",
)
case(
    "025-targz-bomb",
    (
        "left.tar.gz",
        gzip_member(
            tar_bytes(
                [
                    ("file", "first.txt", lines("kept")),
                    ("file", "zeros.bin", bytes(4 << 20)),
                    ("file", "never.txt", lines("never reached")),
                ]
            )
        ),
    ),
    ("right.tar.gz", gzip_member(tar_bytes([("file", "first.txt", lines("kept"))]))),
    opts(),
    "A gzip-compressed tar holding 4 MiB of zeros in about 26 KB. A correct result stops reading "
    "the left stream once it passes the ratio limit: only `first.txt` is listed (`unchanged`), the "
    "left side has `tree-limit`, and the status is `partial`.",
)
case(
    "026-tar-non-utf8-names",
    ("left.tar", tar_bytes([("file", b"caf\xe9.txt", lines("latin-1 name")), ("file", "plain.txt", lines("x"))])),
    ("right.tar", tar_bytes([("file", "café.txt", lines("latin-1 name")), ("file", "plain.txt", lines("x"))])),
    opts(),
    "A tar entry named in Latin-1. A correct result decodes it as `caf\\ufffd.txt` with a "
    "`non-utf8-path` warning for the left side; its content equals `café.txt` on the right, so "
    "the pair is one `moved` entry.",
)
_shared = lines("one stored stream", "listed under three names")
case(
    "027-zip-overlapping-entries",
    ("left.zip", zip_shared(["a.txt", "b.txt", "c.txt"], _shared)),
    ("right.zip", zip_bytes([zentry(n, _shared) for n in ["a.txt", "b.txt", "c.txt"]])),
    opts(),
    "The left zip lists three names whose entries all point at one local entry, as an "
    "overlapping zip bomb does to inflate one compressed stream many times. The right zip holds "
    "the same content three times, normally. A correct result reads the left stream once: "
    "`a.txt` (the first entry by offset) has its `sha256` and is `unchanged`; `b.txt` and "
    "`c.txt` overlap it, so on the left they have `sha256: null` and their declared size, they "
    "are `modified` without a child document, the left side has `tree-limit`, and the status is "
    "`partial` (spec 19.6).",
)
_record = zip64_end_record(1 << 40, 0, 0)
case(
    "028-zip-end-record-in-content",
    ("left.zip", zip_bytes([zentry("record.bin", _record), zentry("notes.txt", lines("v1"))])),
    ("right.zip", zip_bytes([zentry("record.bin", _record), zentry("notes.txt", lines("v2"))])),
    opts(),
    "Both zips store, as the content of `record.bin`, the bytes of a zip64 end record that claims "
    "2^40 entries. No locator at the end of the archive names it, so it is data, not an end "
    "record (spec 19.3). A correct result lists both entries: `record.bin` is `unchanged` and "
    "`notes.txt` is `modified` with a text child.",
)
_zst_content = [("file", "a.txt", lines("alpha")), ("file", "dir/b.txt", lines("beta"))]
_zst_tar = tar_bytes(_zst_content)
case(
    "029-tarzst-skippable-frames",
    (
        "left.tar.zst",
        zstd_skippable(0, b"metadata written before the first frame")
        + zstd_raw(_zst_tar[:1024])
        + zstd_skippable(0xF, b"")
        + zstd_raw(_zst_tar[1024:])
        + zstd_skippable(7, b"trailing metadata"),
    ),
    ("right.tar", _zst_tar),
    opts(),
    "A zstd-compressed tar that starts with a skippable frame and has more of them between and "
    "after its two frames, against the same tar uncompressed. A correct result recognizes the "
    "left input as a tar.zst from what follows the skippable frames (spec 19.1) and is "
    "`identical`, entry for entry.",
)


# --- writing ---------------------------------------------------------------


def write_side(directory: Path, stem: str, side) -> None:
    for old in directory.iterdir():
        if old.name == stem or old.name.startswith(stem + "."):
            if old.is_dir():
                shutil.rmtree(old)
            else:
                old.unlink()
    if isinstance(side, dict):
        root = directory / stem
        for rel, data in side.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    else:
        name, data = side
        if not name.startswith(stem + "."):
            raise SystemExit(f"{directory.name}: {name} must start with {stem}.")
        (directory / name).write_bytes(data)


def write_case(entry: dict) -> None:
    directory = ROOT / entry["slug"]
    directory.mkdir(parents=True, exist_ok=True)
    write_side(directory, "left", entry["left"])
    write_side(directory, "right", entry["right"])
    (directory / "options.json").write_text(json.dumps(entry["options"], indent=2) + "\n", encoding="utf-8")
    (directory / "case.md").write_text(f"# {entry['slug']}\n\n{entry['about']}\n", encoding="utf-8")


def self_check() -> None:
    """The hand-written encoders must agree with zlib and the tar rules."""
    for data in [b"", b"abc", bytes(1000), b"\x00\x00\xff" * 300, noise(1, 5000), bytes(2 << 20)]:
        if zlib.decompress(deflate(data), -15) != data:
            raise SystemExit("deflate writer is broken")
    sample = tar_bytes(TAR_CONTENT)
    if zlib.decompress(gzip_member(sample), 16 + 15) != sample:
        raise SystemExit("gzip writer is broken")
    if sample[257:262] != b"ustar" or len(sample) % 512:
        raise SystemExit("tar writer is broken")
    if len(zip_bytes([zentry("zeros.bin", bytes(2 << 20), deflated=True)])) > 16 << 10:
        raise SystemExit("the zip bomb is not tiny")
    for entry in CASES:
        for side in (entry["left"], entry["right"]):
            if isinstance(side, dict):
                for rel in side:
                    if any(part in {"node_modules", "target", "dist", "__pycache__"} for part in rel.split("/")):
                        raise SystemExit(f"{entry['slug']}: {rel} would be ignored by git")
                    if rel.split("/")[-1] == ".gitignore":
                        raise SystemExit(f"{entry['slug']}: use .ignore, not .gitignore")


def main() -> int:
    self_check()
    ROOT.mkdir(parents=True, exist_ok=True)
    slugs = {entry["slug"] for entry in CASES}
    for existing in ROOT.iterdir():
        if existing.is_dir() and existing.name not in slugs:
            print(f"note: {existing.name} is not generated by this script", file=sys.stderr)
    for entry in CASES:
        write_case(entry)
    print(f"wrote {len(CASES)} tree cases under tests/corpus/tree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
