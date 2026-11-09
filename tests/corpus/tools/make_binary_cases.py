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


# --- 018-029: file formats by magic number, extension fallback, canonical ---
# --- positions, empty inputs, the chunked path and the control-byte rule. ---


def flip(data: bytes, start: int, length: int) -> bytes:
    """`data` with `length` bytes from `start` inverted (every byte differs)."""
    out = bytearray(data)
    for i in range(start, start + length):
        out[i] ^= 0xFF
    return bytes(out)


# 018 -------------------------------------------------------------------------
_gif = b"GIF89a" + struct.pack("<HHBBB", 32, 32, 0xF7, 0, 0) + noise(18, 768) + b"\x2c" + noise(180, 600) + b"\x3b"
case(
    "018-gif-palette-edit",
    ("gif", _gif),
    ("gif", flip(_gif, 13 + 3 * 5, 3)),
    opts(),
    "A GIF (`GIF89a` magic, logical screen descriptor, a 256-colour palette and image bytes) "
    "whose palette entry 5 changed colour: three bytes at offset 28. A correct result is "
    "`mediaType: \"image/gif\"` on both sides (by magic) and exactly one `replace` range "
    "`{leftOffset: 28, leftLength: 3, rightOffset: 28, rightLength: 3}`.",
)

# 019 -------------------------------------------------------------------------
_pdf = (
    b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n"
    + b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n"
    + b"2 0 obj << /Type /Pages /Kids [] /Count 0 >> endobj\n"
    + b"3 0 obj << /Length 512 >> stream\n" + noise(19, 512) + b"\nendstream endobj\n"
    + b"trailer << /Root 1 0 R >>\n%%EOF\n"
)
_pdf_update = (
    b"4 0 obj << /Producer (diffyman corpus) >> endobj\n"
    + b"trailer << /Root 1 0 R /Info 4 0 R /Prev 0 >>\n%%EOF\n"
)
case(
    "019-pdf-incremental-update",
    ("pdf", _pdf),
    ("pdf", _pdf + _pdf_update),
    opts(),
    "A PDF and the same file after an incremental update, which appends a new object and "
    "trailer and never rewrites earlier bytes. The binary comment on line 2 and the "
    "stream make it binary. A correct result is `mediaType: \"application/pdf\"` on both "
    f"sides and one `insert` range at the end: `{{leftOffset: {len(_pdf)}, leftLength: 0, "
    f"rightOffset: {len(_pdf)}, rightLength: {len(_pdf_update)}}}`.",
)

# 020 -------------------------------------------------------------------------
_wasm_head = b"\x00asm\x01\x00\x00\x00"
_wasm_types = b"\x01\x05\x01\x60\x00\x01\x7f"
# Section sizes are unsigned LEB128: 300 is 0xAC 0x02.
_wasm_code = b"\x0a\xac\x02" + noise(20, 300)
# A custom section (id 0, 13 bytes): the name `name`, then the function-names
# subsection (id 1, 6 bytes) naming function 0 `run`.
_wasm_custom = b"\x00\x0d\x04name" + b"\x01\x06\x01\x00\x03run"
assert len(_wasm_custom) == 15
case(
    "020-wasm-section-inserted",
    ("wasm", _wasm_head + _wasm_types + _wasm_code),
    ("wasm", _wasm_head + _wasm_types + _wasm_custom + _wasm_code),
    opts(),
    "A WebAssembly module (`\\0asm` magic) that gained a 15-byte custom `name` section "
    "between its type and code sections. A correct result is `mediaType: "
    "\"application/wasm\"` on both sides and one `insert` range of 15 bytes at left offset "
    "15 (after the 8-byte header and the 7-byte type section), with "
    "`stats.bytes.added: 15`.",
)

# 021 -------------------------------------------------------------------------
_sqlite = b"SQLite format 3\x00" + struct.pack(">H", 4096) + noise(21, 4 * 4096 - 18)
case(
    "021-sqlite-page-overwrite",
    ("db", _sqlite),
    ("db", flip(_sqlite, 2 * 4096 + 100, 100)),
    opts(),
    "A 16 KiB SQLite database file (`SQLite format 3` magic, 4 KiB pages) in which 100 bytes "
    "of page 3 were rewritten in place. The extension `.db` is not in the table, so the "
    "media type comes from the magic: `application/vnd.sqlite3`. A correct result is one "
    "`replace` range `{leftOffset: 8292, leftLength: 100, rightOffset: 8292, rightLength: 100}`.",
)

# 022 -------------------------------------------------------------------------
_elf = b"\x7fELF\x02\x01\x01\x00" + bytes(8) + b"\x03\x00\x3e\x00" + noise(22, 20000)
case(
    "022-elf-two-patches",
    ("so", _elf),
    ("so", flip(flip(_elf, 1000, 4), 15000, 4)),
    opts(),
    "A shared library (`\\x7fELF` magic) with two 4-byte patches far apart, as a binary "
    "patch tool writes them. A correct result has `mediaType: \"application/x-executable\"` "
    "from the magic on both sides (the `.so` extension is not in the table) and exactly "
    "two `replace` ranges, 4 bytes at offset 1000 and 4 bytes at offset 15000, the same "
    "offsets on both sides.",
)

# 023 -------------------------------------------------------------------------
_body = noise(23, 6000)
_header = b"HDR1" + noise(230, 60)
case(
    "023-prepended-header",
    ("bin", _body),
    ("bin", _header + _body),
    opts(),
    "6000 random bytes, and the same bytes with a 64-byte header in front. Nothing is "
    "common at the start, so the common suffix carries the match. A correct result is one "
    "`insert` range `{leftOffset: 0, leftLength: 0, rightOffset: 0, rightLength: 64}`.",
)

# 024 -------------------------------------------------------------------------
case(
    "024-both-empty-forced-binary",
    ("bin", b""),
    ("bin", b""),
    opts(adapter="binary"),
    "Two empty inputs with `adapter: \"binary\"`. A correct result is `kind: \"binary\"`, "
    "`status: \"identical\"`, `bytes: []`, `stats.bytes` all zero and `similarity: 1` "
    "(L + R = 0); both sides report `size: 0` and the media type of the `.bin` extension.",
)

# 025 -------------------------------------------------------------------------
_large = noise(25, 192 * 1024)
case(
    "025-large-delete-chunked",
    ("bin", _large),
    ("bin", _large[:50_000] + _large[120_000:]),
    opts(),
    "192 KiB of random bytes with 70000 bytes cut out at offset 50000. The inputs are over "
    "the byte-alignment limit, so the chunked path finds the cut. A correct result is "
    "exactly one `delete` range of 70000 bytes near offset 50000, `stats.bytes.removed: "
    "70000`, and no `bytes-coarse-limit`.",
)

# 026 -------------------------------------------------------------------------
case(
    "026-periodic-delete-canonical",
    ("bin", b"\x00\x01" * 2048),
    ("bin", b"\x00\x01" * 2047),
    opts(),
    "A periodic input (the two bytes 00 01, 2048 times) and the same with one period "
    "removed. Any period could be the one removed; the common prefix is set aside first "
    "(spec 17), so the answer is canonical: a correct result is one `delete` range at the "
    "end, `{leftOffset: 4094, leftLength: 2, rightOffset: 4094, rightLength: 0}`.",
)

# 027 -------------------------------------------------------------------------
case(
    "027-binary-vs-empty",
    ("bin", _blob[:300]),
    ("bin", b""),
    opts(),
    "300 random bytes against an empty file. The empty side is compatible with both "
    "adapters, so the document is binary because of the left side. A correct result is one "
    "`delete` range `{leftOffset: 0, leftLength: 300, rightOffset: 0, rightLength: 0}`, "
    "`stats.similarity: 0`, and the empty side reports `encoding: \"utf-8\"`.",
)

# 028 -------------------------------------------------------------------------
_ctrl_line = "\x01key\x02value\x03\n"  # 3 control bytes in 12 bytes: over 10 %
assert len(_ctrl_line) == 12
case(
    "028-control-bytes-over-limit",
    ("txt", (_ctrl_line * 40).encode("utf-8")),
    ("txt", (_ctrl_line * 20 + _ctrl_line.replace("value", "VALUE") + _ctrl_line * 19).encode("utf-8")),
    opts(),
    "Valid UTF-8 without NUL bytes, named `.txt`: 40 records of 12 bytes, each with three "
    "control bytes (U+0001-U+0003), so 25 % of the sniff window (the whole 480-byte input) "
    "is control bytes, more than the 10 % a text input may have (spec 16.1). Both sides are "
    "binary, so there is no `binary-fallback`, and the `txt` extension applies to text only: "
    "a correct result has `mediaType: \"application/octet-stream\"` and `encoding: null` on "
    "both sides, and one `replace` range of 5 bytes at offset 245 (`value` -> `VALUE` in "
    "record 21).",
)

# 029 -------------------------------------------------------------------------
_nomagic = b"\x00" + noise(29, 2999)
case(
    "029-extension-fallback-media",
    ("jpg", _nomagic),
    ("jpg", flip(_nomagic, 2000, 1)),
    opts(),
    "Binary data named `.jpg` that does not start with a JPEG signature (it starts with a "
    "NUL byte). No magic number matches, so the binary entry of the extension table gives "
    "the media type: a correct result has `mediaType: \"image/jpeg\"` on both sides and one "
    "`replace` range of 1 byte at offset 2000.",
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
