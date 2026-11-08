"""Writes the seed corpora of the fuzz targets from the golden corpus.

Usage (from the repository root):

    uv run --python 3.12 fuzz/seed_corpus.py

Reads tests/corpus/{text,moved,binary}/*/ and writes fuzz/corpus/<target>/seed-*,
replacing earlier seeds. Inputs are framed as the targets expect
(diffyman_fuzz::chunks: every chunk but the last is a little-endian u16
length followed by the bytes). Seeds over MAX_SEED bytes are skipped so the
fuzzers keep a small default input size.
"""

from __future__ import annotations

import json
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "tests" / "corpus"
OUT = ROOT / "fuzz" / "corpus"
MAX_SEED = 8 * 1024
MAX_JSON_SEEDS = 24

WHITESPACE = ["none", "trailing", "change", "all"]
ALGORITHMS = ["histogram", "myers", "patience"]
GRANULARITIES = ["line", "word", "char", "smart"]
ENCODINGS = ["utf-8", "utf-16", "codepoint"]

# Options texts that are not in the golden corpus.
EXTRA_OPTIONS = [
    b"",
    b"{}",
    b'{"granularity":"smart","positionEncoding":"utf-16","detectMoves":true}',
    b'{"ignore":{"regex":["\\\\d+","[a-f]+",""],"blankLines":true,"case":true}}',
    b'{"ignore":{"regex":["(?i)x{2,9}y","\\\\w{32}"]}}',
    b'{"context":2147483647,"algorithm":"myers"}',
    b'{"context":0,"ignore":{"whitespace":"change"}}',
    b'{"adapter":"binary"}',
    b'{"adapter":"text"}',
    b'{"ignore":{"regex":[1]}}',
    b'{"contxt":1}',
]


def frame(*chunks: bytes) -> bytes:
    """All chunks but the last get a u16 length prefix (longer ones are cut)."""
    out = bytearray()
    for chunk in chunks[:-1]:
        chunk = chunk[:0xFFFF]
        out += struct.pack("<H", len(chunk)) + chunk
    out += chunks[-1]
    return bytes(out)


def find(case: Path, stem: str) -> Path | None:
    for path in sorted(case.iterdir()):
        if path.stem == stem and path.is_file():
            return path
    return None


def cases() -> list[tuple[str, bytes, bytes, bytes]]:
    found = []
    for group in ("text", "moved", "binary"):
        for case in sorted((CORPUS / group).iterdir()):
            if not case.is_dir():
                continue
            left, right = find(case, "left"), find(case, "right")
            options = case / "options.json"
            if left is None or right is None or not options.is_file():
                continue
            name = f"{group}-{case.name}"
            found.append((name, options.read_bytes(), left.read_bytes(), right.read_bytes()))
    return found


def option_bits(options: bytes) -> dict:
    try:
        value = json.loads(options)
    except ValueError:
        return {}
    return value if isinstance(value, dict) else {}


def index(items: list[str], value: object) -> int:
    return items.index(value) if value in items else 0


def refine_selector(options: dict) -> int:
    ignore = options.get("ignore") if isinstance(options.get("ignore"), dict) else {}
    return index(WHITESPACE, ignore.get("whitespace")) | (4 if ignore.get("case") is True else 0)


def moves_bytes(options: dict) -> tuple[int, int]:
    ignore = options.get("ignore") if isinstance(options.get("ignore"), dict) else {}
    a = index(ALGORITHMS, options.get("algorithm"))
    a |= index(GRANULARITIES, options.get("granularity")) << 2
    a |= index(WHITESPACE, ignore.get("whitespace")) << 4
    a |= 0x40 if ignore.get("case") is True else 0
    a |= 0x80 if ignore.get("blankLines") is True else 0
    context = options.get("context", 3)
    context = context if isinstance(context, int) and 0 <= context <= 7 else 3
    b = (context << 1) | (index(ENCODINGS, options.get("positionEncoding")) << 4)
    return a, b


def main() -> int:
    seeds: dict[str, dict[str, bytes]] = {
        t: {} for t in ("diff", "options", "jcs_roundtrip", "refine", "moves")
    }
    json_seeds = []
    for name, options, left, right in cases():
        bits = option_bits(options)
        seeds["diff"][name] = frame(options, left, right)
        seeds["options"][name] = options
        seeds["refine"][name] = bytes([refine_selector(bits)]) + frame(left, right)
        a, b = moves_bytes(bits)
        seeds["moves"][name] = bytes([a, b]) + frame(left, right, b"")
        if name.startswith("moved-"):
            # Right side built from the left: two block moves and no extra line.
            seeds["moves"][name + "-rearranged"] = bytes([a, b | 1]) + frame(
                left, b"", bytes([3, 2, 40, 17, 5, 2])
            )
        expected = CORPUS / name.split("-", 1)[0] / name.split("-", 1)[1] / "expected.json"
        if expected.is_file():
            json_seeds.append((len(expected.read_bytes()), name, expected.read_bytes()))
    for i, options in enumerate(EXTRA_OPTIONS):
        seeds["options"][f"extra-{i:02}"] = options
        seeds["diff"][f"extra-{i:02}"] = frame(options, b"a\nb\nc\n", b"a\nB\nc\nd")
    for _, name, text in sorted(json_seeds)[:MAX_JSON_SEEDS]:
        seeds["jcs_roundtrip"][name] = text
    seeds["jcs_roundtrip"]["numbers"] = b"[0,-0,1e19,1e21,9007199254740991,0.1,5e-324,-1.5e300]"

    total = 0
    for target, items in seeds.items():
        out = OUT / target
        out.mkdir(parents=True, exist_ok=True)
        for old in out.glob("seed-*"):
            old.unlink()
        # Options texts repeat across cases; keep one seed per distinct input.
        seen = set()
        for name, data in sorted(items.items()):
            if len(data) > MAX_SEED or data in seen:
                continue
            seen.add(data)
            (out / f"seed-{name}").write_bytes(data)
            total += len(data)
        print(f"{target}: {len(seen)} seeds")
    print(f"total: {total} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
