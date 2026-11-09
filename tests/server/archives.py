"""Archives built in memory for tests: zips written entry by entry (so
declared sizes can differ from the content), gzip-compressed tars of zeros,
zips nested in zips, and a reader of the warning codes of a Diff Document."""

from __future__ import annotations

import pathlib
import struct
import zlib
from collections.abc import Iterable, Iterator
from typing import Any

MIB = 1 << 20


def deflate(pieces: Iterable[bytes]) -> tuple[bytes, int, int]:
    """Raw deflate of the pieces: (compressed, CRC-32, size)."""
    packer = zlib.compressobj(6, zlib.DEFLATED, -15)
    out, crc, size = bytearray(), 0, 0
    for piece in pieces:
        out += packer.compress(piece)
        crc, size = zlib.crc32(piece, crc), size + len(piece)
    out += packer.flush()
    return bytes(out), crc, size


def zeros(total: int, piece: int = 4 * MIB) -> Iterator[bytes]:
    block = bytes(piece)
    for offset in range(0, total, piece):
        yield block[: min(piece, total - offset)]


def zip_archive(entries: list[tuple[str, int, bytes, int, int]]) -> bytes:
    """A zip of (name, method, stored bytes, CRC-32, declared size) entries."""
    out, directory = bytearray(), bytearray()
    for name, method, data, crc, size in entries:
        raw = name.encode()
        directory += (
            struct.pack(
                "<IHHHHHHIIIHHHHHII",
                0x02014B50,
                20,
                20,
                0,
                method,
                0,
                0x21,
                crc,
                len(data),
                size,
                len(raw),
                0,
                0,
                0,
                0,
                0,
                len(out),
            )
            + raw
        )
        out += struct.pack("<IHHHHHIIIHH", 0x04034B50, 20, 0, method, 0, 0x21, crc, len(data), size, len(raw), 0) + raw + data
    count = len(entries)
    return bytes(out + directory + struct.pack("<IHHHHIIH", 0x06054B50, 0, 0, count, count, len(directory), len(out), 0))


def deflated_entry(name: str, pieces: Iterable[bytes], declared: int | None = None) -> tuple[str, int, bytes, int, int]:
    data, crc, size = deflate(pieces)
    return name, 8, data, crc, size if declared is None else declared


def stored_entry(name: str, data: bytes) -> tuple[str, int, bytes, int, int]:
    return name, 0, data, zlib.crc32(data), len(data)


def tar_gz(path: pathlib.Path, name: str, size: int) -> None:
    """A gzip-compressed tar holding one file of `size` zero bytes."""
    header = bytearray(512)
    fields = [
        (0, name.encode(), 100),
        (100, b"0000644", 8),
        (108, b"0000000", 8),
        (116, b"0000000", 8),
        (124, b"%011o" % size, 12),
        (136, b"00000000000", 12),
    ]
    for offset, value, width in fields:
        header[offset : offset + len(value[:width])] = value[:width]
    header[156:157], header[257:263], header[263:265] = b"0", b"ustar\x00", b"00"
    header[148:156] = b" " * 8
    header[148:156] = b"%06o\x00 " % sum(header)
    packer = zlib.compressobj(6, zlib.DEFLATED, 31)
    with path.open("wb") as out:
        out.write(packer.compress(bytes(header)))
        for piece in zeros(size):
            out.write(packer.compress(piece))
        out.write(packer.compress(bytes(-size % 512 + 1024)))
        out.write(packer.flush())


def fanout_zip(width: int, levels: int, leaf: bytes) -> bytes:
    """Zips of zips, `width` entries at each of `levels` levels, over leaves of
    `leaf`: every entry stays under 1 MiB, so the ratio limit never applies,
    while the whole expands to width**levels leaves."""
    entry = deflated_entry("leaf.bin", [leaf])
    for level in range(levels):
        inner = zip_archive([(f"{level}-{i}{'.zip' if level else '.bin'}", *entry[1:]) for i in range(width)])
        entry = deflated_entry("inner.zip", [inner])
    return inner


def chain_zip(depth: int, text: bytes) -> bytes:
    data = zip_archive([stored_entry("note.txt", text)])
    for _ in range(depth - 1):
        data = zip_archive([stored_entry("inner.zip", data)])
    return data


def warning_codes(value: Any) -> set[str]:
    found: set[str] = set()
    if isinstance(value, dict):
        for key, item in value.items():
            if key == "warnings" and isinstance(item, list):
                found |= {w["code"] for w in item if isinstance(w, dict) and isinstance(w.get("code"), str)}
            else:
                found |= warning_codes(item)
    elif isinstance(value, list):
        for item in value:
            found |= warning_codes(item)
    return found
