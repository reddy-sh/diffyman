#!/usr/bin/env python3
"""Generate the binary conformance cases under tests/corpus/binary/.

Every input is written byte for byte from the definitions below. Random
data comes from a fixed xorshift generator, so re-running the script always
writes the same bytes. Re-running rewrites the inputs, options.json and
case.md of every case; it never touches expected.json.

Usage (from the repository root):

    uv run --python 3.12 python tests/corpus/tools/make_binary_cases.py

All content in this file is original and written for the diffyman corpus.
"""

from __future__ import annotations

import gzip
import io
import json
import struct
import sys
import zipfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "binary"
MAX_FILE_BYTES = 1024 * 1024 + 4096


def opts(adapter: str = "auto") -> dict:
    """A fully spelled-out options object (diff-document.md section 9)."""
    return {
        "algorithm": "histogram",
        "granularity": "line",
        "ignore": {"whitespace": "none", "case": False, "blankLines": False, "regex": []},
        "context": 3,
        "positionEncoding": "utf-8",
        "detectMoves": False,
        "adapter": adapter,
        # The tree options at their defaults; they do not change a binary result.
        "tree": {
            "followSymlinks": False,
            "ignoreFiles": False,
            "exclude": [],
            "detectMoves": True,
            "maxDepth": 3,
            "childBudget": 16777216,
        },
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


def png(width: int, height: int, seed: int) -> bytes:
    """A valid RGBA PNG with pseudo-random pixels (stored, not compressed,
    so a pixel byte appears as itself in the file)."""

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    pixels = noise(seed, width * height * 4)
    raw = b"".join(b"\x00" + pixels[y * width * 4 : (y + 1) * width * 4] for y in range(height))
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 0))
        + chunk(b"IEND", b"")
    )


def gzip_bytes(data: bytes) -> bytes:
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0) as f:
        f.write(data)
    return buf.getvalue()


def zip_bytes(name: str, data: bytes) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", compression=zipfile.ZIP_STORED) as z:
        info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
        z.writestr(info, data)
    return buf.getvalue()


CASES: list[dict] = []


def case(slug: str, left: tuple[str, bytes], right: tuple[str, bytes], options: dict, about: str) -> None:
    CASES.append({"slug": slug, "left": left, "right": right, "options": options, "about": about})


# 001 -------------------------------------------------------------------------
_png = png(16, 16, 1)
_png_edit = bytearray(_png)
_png_edit[200] ^= 0x01
case(
    "001-png-one-byte",
    ("png", _png),
    ("png", bytes(_png_edit)),
    opts(),
    "A 16x16 RGBA PNG and the same file with one pixel byte (offset 200) changed by one bit; "
    "the CRC is not updated, as in a corrupted copy. Both sides are binary (`image/png` by "
    "magic), so a correct result is `kind: \"binary\"` with exactly one `replace` range "
    "`{leftOffset: 200, leftLength: 1, rightOffset: 200, rightLength: 1}`, no warnings, "
    "and `stats.bytes.unchanged` one less than the size.",
)

# 002 -------------------------------------------------------------------------
_blob = noise(2, 4096)
case(
    "002-inserted-bytes",
    ("bin", _blob),
    ("bin", _blob[:2048] + b"\x00INSERTED-16B\x00\xfe\xff" + _blob[2048:]),
    opts(),
    "4096 random bytes, and the same bytes with 16 bytes inserted at offset 2048. Byte "
    "alignment must find the insertion rather than report everything after it as changed: "
    "a correct result has one `insert` range with `leftLength: 0` and `rightLength: 16`, "
    "and `stats.bytes` with `added: 16`, `removed: 0`, `unchanged: 4096`.",
)

# 003 -------------------------------------------------------------------------
case(
    "003-truncated",
    ("bin", _blob),
    ("bin", _blob[:3000]),
    opts(),
    "The 4096 random bytes of case 002 against their first 3000 bytes, as from an "
    "interrupted download. A correct result is one `delete` range "
    "`{leftOffset: 3000, leftLength: 1096, rightOffset: 3000, rightLength: 0}`.",
)

# 004 -------------------------------------------------------------------------
case(
    "004-identical",
    ("png", _png),
    ("png", _png),
    opts(),
    "The same PNG on both sides. A correct result is `kind: \"binary\"`, "
    "`status: \"identical\"`, `bytes: []`, `stats.similarity: 1` and equal `sha256`.",
)

# 005 -------------------------------------------------------------------------
_text = "Packing list\n- tent\n- stove\n- map\n".encode("utf-8")
case(
    "005-text-vs-binary",
    ("txt", _text),
    ("png", _png),
    opts(),
    "A UTF-8 text file against a PNG. The PNG is not UTF-8, so under `adapter: \"auto\"` the "
    "pair is compared as bytes and gets the `binary-fallback` warning. A correct result has "
    "`left.mediaType: \"text/plain\"` with `encoding: \"utf-8\"`, `right.mediaType: "
    "\"image/png\"` with `encoding: null`, no line fields, and ranges that rebuild the PNG "
    "from the text.",
)

# 006 -------------------------------------------------------------------------
_nul = b"name=alpha\x00size=12\x00flags=rw\x00\n"
case(
    "006-nul-bytes",
    ("txt", _nul),
    ("txt", _nul.replace(b"size=12", b"size=13")),
    opts(),
    "Valid UTF-8 with NUL bytes, as in a NUL-separated record file, named `.txt`. A NUL in "
    "the first 8000 bytes makes an input binary (spec 16.1), so a correct result is "
    "`kind: \"binary\"` without `binary-fallback` (neither side is text), "
    "`mediaType: \"application/octet-stream\"` on both sides (the `txt` extension applies "
    "to text only) and one `replace` range of length 1 at offset 17.",
)

# 007 -------------------------------------------------------------------------
_big = noise(7, 1024 * 1024)
case(
    "007-random-1mb-insertion",
    ("bin", _big),
    ("bin", _big[:400_000] + noise(70, 100) + _big[400_000:]),
    opts(),
    "1 MiB of random bytes, and the same bytes with 100 random bytes inserted at offset "
    "400000. A correct result is exactly one `insert` range with `rightLength: 100` near "
    "offset 400000 (common prefix and suffix are set aside first), `stats.bytes.unchanged: "
    "1048576` and no `bytes-coarse-limit`.",
)

# 008 -------------------------------------------------------------------------
case(
    "008-empty-vs-binary",
    ("bin", b""),
    ("bin", _blob[:64]),
    opts(),
    "An empty file against 64 random bytes. An empty input is compatible with both "
    "adapters, so the document is binary because of the right side, and there is no "
    "`binary-fallback` (the empty side is not text). A correct result is one `insert` range "
    "`{leftOffset: 0, leftLength: 0, rightOffset: 0, rightLength: 64}` and "
    "`stats.similarity: 0`.",
)

# 009 -------------------------------------------------------------------------
_invoice = b"Invoice 2026-118\nCustomer: Harbour Bakery\nTotal: 412.50 EUR"
case(
    "009-latin1-vs-utf8",
    ("txt", _invoice),
    ("txt", b"Invoice 2026-118\nCustomer: Caf\xe9 Lumi\xe8re\nTotal: 412.50 EUR\n"),
    opts(),
    "The inputs of `text/021-invalid-utf8` under the default `adapter: \"auto\"`: the right "
    "file holds Latin-1 bytes, so it is binary, and the pair is compared as bytes with the "
    "`binary-fallback` warning instead of giving an `invalid-utf8` error. The left side "
    "keeps `encoding: \"utf-8\"`, the right side has `encoding: null` and "
    "`mediaType: \"application/octet-stream\"`.",
)

# 010 -------------------------------------------------------------------------
_list = "Shopping list\nmilk\neggs\nrye bread\n"
case(
    "010-utf16-vs-utf8",
    ("txt", _list.encode("utf-8")),
    ("txt", b"\xff\xfe" + _list.encode("utf-16-le")),
    opts(),
    "The inputs of `text/022-utf16-bom` under `adapter: \"auto\"`. The right file is valid "
    "UTF-16LE text, which the text adapter does not read, so both are compared as bytes "
    "with `binary-fallback`. A correct result reports the right side as "
    "`encoding: \"utf-16le\"` and `mediaType: \"text/plain\"`.",
)

# 011 -------------------------------------------------------------------------
case(
    "011-forced-text-on-binary",
    ("png", _png),
    ("png", bytes(_png_edit)),
    opts(adapter="text"),
    "The PNGs of case 001 with `adapter: \"text\"`. Neither is UTF-8, so a correct result "
    "is a `kind: \"text\"` error document with `invalid-utf8` for both sides, "
    "`stats: null`, `hunks: []` and `bytes: []`; `mediaType` still says `image/png`.",
)

# 012 -------------------------------------------------------------------------
case(
    "012-forced-binary-on-text",
    ("txt", b"alpha\nbeta\ngamma delta\n"),
    ("txt", b"alpha\nBETA\ngamma delta\r\n"),
    opts(adapter="binary"),
    "Two UTF-8 text files with `adapter: \"binary\"`. A correct result is `kind: \"binary\"` "
    "without `binary-fallback` (the adapter was asked for) and without text warnings, both "
    "sides `encoding: \"utf-8\"` with null line fields, and two ranges: `beta` -> `BETA` "
    "(4 bytes at offset 6) and the CR inserted at left offset 22, before the last LF.",
)

# 013 -------------------------------------------------------------------------
_chunked = noise(13, 256 * 1024)
case(
    "013-chunked-two-edits",
    ("bin", _chunked),
    (
        "bin",
        _chunked[:60_000]
        + b"<<new block of 32 bytes here!!>>"
        + _chunked[60_000:200_000]
        + bytes(b ^ 0x5A for b in _chunked[200_000:200_040])
        + _chunked[200_040:],
    ),
    opts(),
    "256 KiB of random bytes with 32 bytes inserted at offset 60000 and 40 bytes changed at "
    "offset 200000. What lies between the edits is larger than the byte-alignment limit, so "
    "the inputs are chunked (spec 17 step 3) and only the changed chunks are aligned byte by "
    "byte. A correct result is exactly two ranges, an `insert` of 32 bytes and a `replace` "
    "of 40 bytes at left offset 200000 (right offset 200032), with no "
    "`bytes-coarse-limit`.",
)

# 014 -------------------------------------------------------------------------
_records = noise(14, 2000)
_edited = bytearray(_records)
for at in (100, 104, 300, 307, 600, 608, 900, 909):
    _edited[at] ^= 0xFF
case(
    "014-close-and-far-changes",
    ("bin", _records),
    ("bin", bytes(_edited)),
    opts(),
    "2000 random bytes with single bytes flipped at offsets 100 and 104 (3 equal bytes "
    "between), 300 and 307 (6 between), 600 and 608 (7 between), and 900 and 909 (8 "
    "between). Ranges less than 8 equal bytes apart are joined (spec 17 step 4), so a "
    "correct result is five `replace` ranges: 100-104 (5 bytes), 300-307 (8 bytes), "
    "600-608 (9 bytes), and 900 and 909 (1 byte each, kept apart).",
)

# 015 -------------------------------------------------------------------------
case(
    "015-unrelated-coarse",
    ("bin", noise(150, 48 * 1024)),
    ("bin", noise(152, 48 * 1024)),
    opts(),
    "Two unrelated 48 KiB random files. Together they exceed the byte-alignment limit and "
    "share no chunk, so the changed region is reported as one `replace` range over both "
    "whole files with the `bytes-coarse-limit` warning (spec 17 step 3).",
)

# 016 -------------------------------------------------------------------------
_payload = b"diffyman corpus payload\n" * 8
case(
    "016-gzip-vs-zip",
    ("dat", gzip_bytes(_payload)),
    ("dat", zip_bytes("payload.txt", _payload)),
    opts(),
    "The same payload as a gzip stream and as a stored ZIP archive, both named `.dat`. "
    "Media types come from the magic bytes (spec 16.1), not the name: a correct result has "
    "`left.mediaType: \"application/gzip\"` and `right.mediaType: \"application/zip\"`, and "
    "ranges that rebuild the ZIP from the gzip file.",
)


# 017 -------------------------------------------------------------------------
_dense = noise(17, 128 * 1024)
_dense_edit = bytearray(_dense)
for k in range(1, 31):
    for t in range(8):
        _dense_edit[k * 4228 + t] ^= 0xA5
case(
    "017-dense-overwrites",
    ("bin", _dense),
    ("bin", bytes(_dense_edit)),
    opts(),
    "128 KiB of random bytes with 30 runs of 8 bytes overwritten, one every 4228 bytes. The "
    "edits are closer together than the average first-level chunk (8 KiB), so every first-level "
    "chunk changes and the changed region is far over the byte-alignment limit; the smaller "
    "chunk levels (spec 17 step 3) still align it. A correct result is exactly 30 `replace` "
    "ranges of 8 bytes at left offsets 4228, 8456, ... 126840 (equal right offsets), "
    "`stats.bytes.removed: 240`, and no `bytes-coarse-limit`.",
)


def write_case(entry: dict) -> None:
    directory = ROOT / entry["slug"]
    directory.mkdir(parents=True, exist_ok=True)
    for stale in directory.glob("left.*"):
        stale.unlink()
    for stale in directory.glob("right.*"):
        stale.unlink()
    for side in ("left", "right"):
        ext, data = entry[side]
        if len(data) > MAX_FILE_BYTES:
            raise SystemExit(f"{entry['slug']}: {side} is {len(data)} bytes, over the limit")
        (directory / f"{side}.{ext}").write_bytes(data)
    options_text = json.dumps(entry["options"], indent=2) + "\n"
    (directory / "options.json").write_text(options_text, encoding="utf-8")
    (directory / "case.md").write_text(f"# {entry['slug']}\n\n{entry['about']}\n", encoding="utf-8")


def self_check() -> None:
    slugs = [entry["slug"] for entry in CASES]
    if len(set(slugs)) != len(slugs):
        raise SystemExit("duplicate case slugs")
    for number, slug in enumerate(slugs, start=1):
        if not slug.startswith(f"{number:03d}-"):
            raise SystemExit(f"case {slug} is out of order")
    by_slug = {entry["slug"]: entry for entry in CASES}
    assert by_slug["001-png-one-byte"]["left"][1].startswith(b"\x89PNG\r\n\x1a\n")
    assert len(by_slug["007-random-1mb-insertion"]["left"][1]) == 1024 * 1024
    assert by_slug["008-empty-vs-binary"]["left"][1] == b""
    assert b"\x00" in by_slug["006-nul-bytes"]["left"][1]
    unrelated = by_slug["015-unrelated-coarse"]
    assert unrelated["left"][1] != unrelated["right"][1]
    by_slug["006-nul-bytes"]["left"][1].decode("utf-8")
    assert by_slug["010-utf16-vs-utf8"]["right"][1].startswith(b"\xff\xfe")


def main() -> int:
    self_check()
    for entry in CASES:
        write_case(entry)
    print(f"wrote {len(CASES)} binary cases under tests/corpus/binary")
    return 0


if __name__ == "__main__":
    sys.exit(main())
