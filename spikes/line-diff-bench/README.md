# Spike A: line diff bench

A standalone Cargo project (not part of the root workspace) that compares the
`similar` and `imara-diff` line diff crates on large and pathological inputs.
The write-up is in [`docs/spikes/A-line-diff.md`](../../docs/spikes/A-line-diff.md),
and the decision is in [`docs/adr/0006-line-diff-library.md`](../../docs/adr/0006-line-diff-library.md).

## Binaries

| Binary | What it does |
|---|---|
| `gen` | Deterministic corpus generator (fixed seeds, xorshift RNG). Writes `<case>.left` / `<case>.right` pairs. |
| `bench` | Runs **one** case with **one** algorithm and prints one flat JSON object: timings per phase, hunks, deleted/inserted lines, an output fingerprint, and a validity check of the edit script. |
| `run-all` | Runs every (case, algorithm) pair in its own process under `/usr/bin/time -l`, so peak memory is per case. Appends JSON lines to a results file. `--report FILE` prints the Markdown tables. |

## Real CLI

`cli-bench.sh` runs a release build of the `diffyman` CLI (not this bench) on the same corpus, one
process per run with a warm file cache, and prints wall time and peak footprint for `--json` and the
text diff. `compare-outputs.py` checks that two such runs wrote the same outputs (text byte for byte,
JSON with `provenance` removed), for example before and after an engine change.

```zsh
scripts/with-rustup cargo build --release -p diffyman-cli
spikes/line-diff-bench/cli-bench.sh target/release/diffyman ~/diffyman-data/corpus/spike-a /tmp/after
uv run --python 3.12 spikes/line-diff-bench/compare-outputs.py /tmp/before /tmp/after
```

The recorded numbers are in [ADR 0006, large-file mode](../../docs/adr/0006-line-diff-library.md#large-file-mode).

Pipeline inside `bench`: mmap both files (or `--io read`), strip the common prefix and suffix at line
granularity, intern lines to dense `u32` tokens with one shared interner, run the algorithm on the
token slices, then normalize the result to a canonical hunk list. `imara-histogram-native` instead
uses imara-diff's own `InternedInput` plus `postprocess_lines`.

## Reproduce

Large data stays outside the repo, in `~/diffyman-data/corpus/spike-a` (about 3 GB).

```zsh
scripts/with-rustup cargo build --release --manifest-path spikes/line-diff-bench/Cargo.toml
spikes/line-diff-bench/target/release/gen ~/diffyman-data/corpus/spike-a
spikes/line-diff-bench/target/release/run-all --data ~/diffyman-data/corpus/spike-a \
  --out spikes/line-diff-bench/results/run.jsonl --timeout-s 120
spikes/line-diff-bench/target/release/run-all --report spikes/line-diff-bench/results/run.jsonl
```

`run-all --cases log-1g,source-50m --algos imara-histogram,similar-myers --repeat 3` narrows a run.
Raw results from the recorded run are in `results/2026-11-07-m4max.jsonl`.
