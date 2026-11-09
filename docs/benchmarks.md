# Benchmarks

`scripts/bench.sh` runs the release `diffyman` CLI next to `git diff --no-index`,
`diff -u` and, when it is installed, difftastic (`difft`), one process per run,
on generated inputs, and prints a markdown table. The nightly workflow runs it
on a shared Linux runner (`bench.md` in the run's artifacts); the table below
was recorded on a developer machine. The criterion benchmarks of the engine
(`cargo bench -p diffyman --bench engine`) and the large-file CLI runs of
`spikes/line-diff-bench/cli-bench.sh` ([ADR 0006](adr/0006-line-diff-library.md))
are separate.

```
scripts/bench.sh [--bin PATH] [--difft PATH] [--data DIR] [--sizes "N ..."] [--runs N]
```

`scripts/bench.sh --help` describes the cases and the options. In short:

- `text-N` and `moved-N`: N generated lines per side with about 2 % of the
  lines edited; `moved-N` also swaps pairs of 6 to 20 line blocks.
- `js-N`: about N lines of generated JavaScript in 8-line functions, named
  `.js`, with about 3 % of the functions changed, deleted or new.
- `binary-10m`: 10 MiB of fixed AES-CTR keystream with 256 bytes overwritten.

Inputs come from a Park-Miller generator in awk, so every machine gets the same
bytes. Wall time is the median of the runs; peak memory is the largest peak
memory footprint (macOS) or maximum resident set size (Linux) of any run.

## Reading the difftastic rows

difftastic is a structural diff: it parses both sides with tree-sitter and
aligns the syntax trees, which is more work than a line diff and answers a
different question, so only its wall time and memory compare with the other
tools. It works structurally only on a language it recognizes and within its
own limits; otherwise it uses its line-oriented text diff:

- `text-*`, `moved-*` and `binary-*` have no file extension, so difftastic
  diffs them as text (for `binary-10m` it only reports that the files differ,
  like `diff` and plain `git diff`).
- An input over its default `--byte-limit` of 1,000,000 bytes also gets the
  text diff. Of the JavaScript cases only `js-10000` (about 0.15 MB per side)
  is structural; `js-100000` (1.4 MiB) and `js-1000000` (13.9 MiB) fall back,
  as difftastic's own header line says.

It runs with `--display inline --color never --syntax-highlight off --width
120 --exit-code`, so its output is plain text and its exit code is 1 when the
files differ, as for the other tools. difftastic is a local tool for this
comparison only, not a dependency: install it with
`cargo install --locked difftastic` (MIT) and put `difft` on `PATH`, or pass
`--difft PATH`. Without it the difftastic rows are left out.

## Results, 2026-11-08

Darwin arm64, diffyman 0.0.1 (release build), git 2.54.0, Apple diff,
difftastic 0.72.0, 3 runs per command, warm file cache.

| Case | Input MB (both sides) | Command | Exit | Wall s | Peak MB | Output MB |
|---|--:|---|--:|--:|--:|--:|
| text-10000 | 0.9 | `diffyman diff --json` | 1 | 0.006 | 3 | 0.1 |
| text-10000 | 0.9 | `diffyman diff` | 1 | 0.005 | 3 | 0.1 |
| text-10000 | 0.9 | `diffyman diff --format unified` | 1 | 0.005 | 3 | 0.1 |
| text-10000 | 0.9 | `git diff --no-index` | 1 | 0.013 | 4 | 0.1 |
| text-10000 | 0.9 | `git diff --no-index --histogram` | 1 | 0.013 | 5 | 0.1 |
| text-10000 | 0.9 | `diff -u` | 1 | 0.006 | 6 | 0.1 |
| text-10000 | 0.9 | `difft --display inline` | 1 | 0.026 | 15 | 0.1 |
| moved-10000 | 0.9 | `diffyman diff --json` | 1 | 0.006 | 3 | 0.1 |
| moved-10000 | 0.9 | `diffyman diff --json --detect-moves` | 1 | 0.009 | 3 | 0.1 |
| moved-10000 | 0.9 | `diffyman diff --format unified` | 1 | 0.006 | 3 | 0.1 |
| moved-10000 | 0.9 | `git diff --no-index` | 1 | 0.014 | 4 | 0.1 |
| moved-10000 | 0.9 | `git diff --no-index --histogram` | 1 | 0.014 | 5 | 0.1 |
| moved-10000 | 0.9 | `git diff --no-index --color=always --color-moved` | 1 | 0.013 | 4 | 0.1 |
| moved-10000 | 0.9 | `diff -u` | 1 | 0.007 | 8 | 0.1 |
| moved-10000 | 0.9 | `difft --display inline` | 1 | 0.026 | 16 | 0.1 |
| js-10000 | 0.3 | `diffyman diff --json` | 1 | 0.006 | 2 | 0.0 |
| js-10000 | 0.3 | `diffyman diff --format unified` | 1 | 0.006 | 2 | 0.0 |
| js-10000 | 0.3 | `git diff --no-index --histogram` | 1 | 0.012 | 4 | 0.0 |
| js-10000 | 0.3 | `diff -u` | 1 | 0.005 | 8 | 0.0 |
| js-10000 | 0.3 | `difft --display inline` (structural) | 1 | 0.128 | 65 | 0.0 |
| text-100000 | 8.8 | `diffyman diff --json` | 1 | 0.019 | 7 | 0.7 |
| text-100000 | 8.8 | `diffyman diff` | 1 | 0.018 | 7 | 0.7 |
| text-100000 | 8.8 | `diffyman diff --format unified` | 1 | 0.016 | 7 | 0.7 |
| text-100000 | 8.8 | `git diff --no-index` | 1 | 0.051 | 17 | 0.7 |
| text-100000 | 8.8 | `git diff --no-index --histogram` | 1 | 0.054 | 21 | 0.7 |
| text-100000 | 8.8 | `diff -u` | 1 | 0.046 | 15 | 0.7 |
| text-100000 | 8.8 | `difft --display inline` | 1 | 0.159 | 147 | 1.1 |
| moved-100000 | 8.8 | `diffyman diff --json` | 1 | 0.018 | 7 | 0.7 |
| moved-100000 | 8.8 | `diffyman diff --json --detect-moves` | 1 | 0.020 | 8 | 0.7 |
| moved-100000 | 8.8 | `diffyman diff --format unified` | 1 | 0.017 | 7 | 0.7 |
| moved-100000 | 8.8 | `git diff --no-index` | 1 | 0.053 | 17 | 0.7 |
| moved-100000 | 8.8 | `git diff --no-index --histogram` | 1 | 0.056 | 21 | 0.7 |
| moved-100000 | 8.8 | `git diff --no-index --color=always --color-moved` | 1 | 0.057 | 19 | 0.8 |
| moved-100000 | 8.8 | `diff -u` | 1 | 0.049 | 15 | 0.7 |
| moved-100000 | 8.8 | `difft --display inline` | 1 | 0.174 | 152 | 1.2 |
| js-100000 | 2.8 | `diffyman diff --json` | 1 | 0.013 | 5 | 0.1 |
| js-100000 | 2.8 | `diffyman diff --format unified` | 1 | 0.013 | 5 | 0.1 |
| js-100000 | 2.8 | `git diff --no-index --histogram` | 1 | 0.026 | 12 | 0.1 |
| js-100000 | 2.8 | `diff -u` | 1 | 0.024 | 20 | 0.1 |
| js-100000 | 2.8 | `difft --display inline` (text, over the byte limit) | 1 | 0.106 | 105 | 0.1 |
| text-1000000 | 88.4 | `diffyman diff --json` | 1 | 0.160 | 67 | 7.2 |
| text-1000000 | 88.4 | `diffyman diff` | 1 | 0.157 | 74 | 6.9 |
| text-1000000 | 88.4 | `diffyman diff --format unified` | 1 | 0.147 | 74 | 6.9 |
| text-1000000 | 88.4 | `git diff --no-index` | 1 | 0.478 | 146 | 6.9 |
| text-1000000 | 88.4 | `git diff --no-index --histogram` | 1 | 0.566 | 184 | 6.9 |
| text-1000000 | 88.4 | `diff -u` | 1 | 0.448 | 100 | 6.9 |
| text-1000000 | 88.4 | `difft --display inline` | 1 | 1.806 | 1204 | 11.4 |
| moved-1000000 | 88.4 | `diffyman diff --json` | 1 | 0.160 | 71 | 7.4 |
| moved-1000000 | 88.4 | `diffyman diff --json --detect-moves` | 1 | 0.197 | 75 | 7.5 |
| moved-1000000 | 88.4 | `diffyman diff --format unified` | 1 | 0.146 | 75 | 7.6 |
| moved-1000000 | 88.4 | `git diff --no-index` | 1 | 0.479 | 146 | 7.6 |
| moved-1000000 | 88.4 | `git diff --no-index --histogram` | 1 | 0.523 | 175 | 7.6 |
| moved-1000000 | 88.4 | `git diff --no-index --color=always --color-moved` | 1 | 0.496 | 161 | 8.5 |
| moved-1000000 | 88.4 | `diff -u` | 1 | 0.503 | 99 | 7.6 |
| moved-1000000 | 88.4 | `difft --display inline` | 1 | 1.786 | 1237 | 12.3 |
| js-1000000 | 27.8 | `diffyman diff --json` | 1 | 0.094 | 34 | 1.4 |
| js-1000000 | 27.8 | `diffyman diff --format unified` | 1 | 0.091 | 38 | 0.7 |
| js-1000000 | 27.8 | `git diff --no-index --histogram` | 1 | 0.200 | 110 | 0.8 |
| js-1000000 | 27.8 | `diff -u` | 1 | 0.243 | 143 | 0.7 |
| js-1000000 | 27.8 | `difft --display inline` (text, over the byte limit) | 1 | 1.248 | 971 | 1.5 |
| binary-10m | 20.0 | `diffyman diff --json` | 1 | 0.035 | 6 | 0.0 |
| binary-10m | 20.0 | `git diff --no-index` | 1 | 0.105 | 2 | 0.0 |
| binary-10m | 20.0 | `git diff --no-index --binary` | 1 | 0.408 | 44 | 0.0 |
| binary-10m | 20.0 | `diff -u` | 1 | 0.036 | 13 | 0.0 |
| binary-10m | 20.0 | `difft --display inline` (reports only that the files differ) | 1 | 0.212 | 70 | 0.0 |

What the difftastic rows show: on the one structural case, `js-10000`, it
takes 0.13 s and 65 MB where the line diffs take 5-12 ms and 2-8 MB; that is
the cost of parsing and aligning syntax trees, which none of the other tools
do. In its text fallback it is 4-13 times slower than `diffyman --json` and
uses 1.2 GB on the 1M-line pairs (`diffyman --json` 67-71 MB, `git diff`
146-184 MB).
Output sizes differ by format and are not comparable across tools.
