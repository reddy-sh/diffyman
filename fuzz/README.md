# Fuzzing

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets for the diffyman engine. This directory is its own Cargo workspace (the root workspace excludes it), because libFuzzer needs a nightly toolchain and sanitizer flags.

## Run

From the repository root, with cargo-fuzz 0.13 and a nightly toolchain installed:

```sh
cargo +nightly fuzz list --fuzz-dir fuzz
cargo +nightly fuzz run diff --fuzz-dir fuzz -- -max_total_time=90
cargo +nightly fuzz run options --fuzz-dir fuzz -- -max_total_time=90 -dict=fuzz/options.dict
```

Everything after `--` goes to libFuzzer. A crash is saved under `fuzz/artifacts/<target>/`. To replay one:

```sh
cargo +nightly fuzz run diff --fuzz-dir fuzz fuzz/artifacts/diff/crash-<hash>
```

The shared checks have unit tests that run on the stable toolchain:

```sh
cargo test --manifest-path fuzz/Cargo.toml --lib
```

## Targets

| Target | Input | What it asserts |
|---|---|---|
| `diff` | options JSON, left bytes, right bytes | The engine never panics; rejected options are rejected by the options parser too; every document passes the document checks below; a second run gives the same bytes without `provenance`; the typed API gives the same document as the JSON API. |
| `options` | options JSON text | Parsing fails only with `InvalidOptions`; accepted options validate, serialize canonically and parse back to the same value, from text and from a JSON value; a small diff with them passes the document checks and echoes them. This also drives `ignore.regex` compilation and its limits. |
| `jcs_roundtrip` | JSON text, and the same bytes as the source of an arbitrary `DiffDocument` | The tree writer and the direct canonical writer agree (both fail or give the same text); canonical text is a fixed point; a document reads back and writes the same bytes, with the same `id` and conformance form. |
| `refine` | selector byte, left text, right text | `split_lines` gives exactly the spec §7.2 lines; for `word`, `char` and `smart` with every position encoding, refinement is deterministic, spans are non-empty, sorted, non-overlapping and on character boundaries, UTF-16 and code point spans are the UTF-8 spans converted, and for `word` and `char` the text outside the spans is the same on both sides. |
| `moves` | two option bytes, left text, extra lines, move steps | With `detectMoves: true` (the right side is often the left side with blocks moved), every document passes the document checks; a second run is identical; moves never change hunks, ops, stats or status (spec §6.5). |

The document checks (`diffyman_fuzz::check_document`) cover:

- The output is canonical JSON: re-canonicalizing gives the same bytes, and reading it as a `DiffDocument` and writing it again gives the same bytes.
- `id` recomputes from the document (spec §5).
- File meta matches the inputs (size, SHA-256, BOM, line count, terminator style, final newline); warnings are sorted, unique and carry their fixed severity and message; `status` is `error` exactly when an error is present.
- Hunks and their ops tile both sides inside the line counts, in order; the lines of `equal` ops and the lines between hunks have equal comparison keys; op shapes, adjacency and context rules follow spec §6.4, §7.3 and §7.4.
- Inline spans are only on `replace` ops, inside their op's lines, on character boundaries in the document's position encoding, sorted and never on a moved line.
- Moves cover only deleted and inserted lines, one move per line, with the right kind and similarity and the exact `fromOps` and `toOps` (spec §6.5).
- `stats` adds up (spec §6.3), equal texts are `identical`, and under exact comparison an `identical` document has equal texts.
- For a binary document (spec §6.7, §17): no hunks, moves or line meta; the byte ranges are sorted, at least 8 bytes apart, with an `op` that matches their lengths; the bytes between them are equal on both sides; copying the gaps from the left and the ranges from the right rebuilds the right input; and the byte stats add up.

## Input layout

The targets split their input with `diffyman_fuzz::chunks`: every chunk but the last is a little-endian `u16` length followed by that many bytes, and the last chunk is the rest. A short or truncated input still splits; the missing chunks are empty.

## Seeds

`corpus/<target>/seed-*` are built from the golden corpus (`tests/corpus/text`, `tests/corpus/moved` and `tests/corpus/binary`) by:

```sh
uv run --python 3.12 fuzz/seed_corpus.py
```

The script skips seeds over 8 KiB, so the whole seed set stays well under 2 MB. Only the seeds are committed: the inputs libFuzzer adds to `corpus/<target>/`, and `artifacts/`, `coverage/` and `target/`, are ignored.

## Licenses

The fuzz workspace adds `libfuzzer-sys` (MIT OR Apache-2.0, plus NCSA for the bundled libFuzzer sources) and `arbitrary` (MIT OR Apache-2.0). Neither is linked into any shipped crate. `fuzz/deny.toml` is the root policy plus NCSA, and cargo-deny picks it up next to the manifest:

```sh
cargo deny --manifest-path fuzz/Cargo.toml check
```
