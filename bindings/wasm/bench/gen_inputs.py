"""Generates deterministic text pairs for the Spike C speed runs.

Each pair is source-like text (lines of 40 to 100 characters, some indented,
a few non-ASCII words). The right side changes about 1% of lines: in-line word
edits (which exercise word refinement), small inserted blocks and small deleted
blocks.

    uv run --no-project --python 3.12 python bindings/wasm/bench/gen_inputs.py \
        [--out ~/diffyman-data/tmp/spikec] [--sizes 1,10]

Writes {N}mb-left.txt and {N}mb-right.txt for each size N in MB (10^6 bytes).
"""

from __future__ import annotations

import argparse
import random
from pathlib import Path

WORDS = (
    "alpha beta gamma delta value index count result buffer stream token parse emit "
    "render diff left right line hunk range offset length return match case else "
    "struct impl trait self config option error status width height café naïve "
    "日本 데이터 данные"
).split()


def make_line(rng: random.Random) -> str:
    indent = "    " * rng.choice((0, 0, 1, 1, 2, 3))
    target = rng.randint(40, 100)
    words = []
    length = len(indent)
    while length < target:
        word = rng.choice(WORDS)
        words.append(word)
        length += len(word) + 1
    return indent + " ".join(words) + rng.choice((";", ",", " {", "", ""))


def make_pair(size: int, seed: int) -> tuple[str, str]:
    rng = random.Random(seed)
    left: list[str] = []
    total = 0
    while total < size:
        line = make_line(rng)
        left.append(line)
        total += len(line.encode()) + 1
    right: list[str] = []
    i = 0
    while i < len(left):
        roll = rng.random()
        if roll < 0.008:  # edit a word or two in place
            words = left[i].split(" ")
            for _ in range(rng.randint(1, 2)):
                words[rng.randrange(len(words))] = rng.choice(WORDS)
            right.append(" ".join(words))
        elif roll < 0.009:  # insert a small block
            right.append(left[i])
            right.extend(make_line(rng) for _ in range(rng.randint(1, 5)))
        elif roll < 0.010:  # delete a small block
            i += rng.randint(1, 5)
            continue
        else:
            right.append(left[i])
        i += 1
    return "\n".join(left) + "\n", "\n".join(right) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=Path.home() / "diffyman-data" / "tmp" / "spikec")
    parser.add_argument("--sizes", default="1,10", help="comma-separated sizes in MB")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    for mb in (int(s) for s in args.sizes.split(",")):
        left, right = make_pair(mb * 1_000_000, seed=mb)
        for side, text in (("left", left), ("right", right)):
            path = args.out / f"{mb}mb-{side}.txt"
            path.write_text(text, encoding="utf-8")
            print(f"{path}: {path.stat().st_size} bytes, {text.count(chr(10))} lines")


if __name__ == "__main__":
    main()
