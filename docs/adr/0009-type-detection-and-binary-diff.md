# 0009. Type detection and a byte-level binary diff

- **Status:** Accepted
- **Date:** 2026-11-08

## Context

Up to schema 0.2.0 every input went to the text adapter, and an input that is not UTF-8 gave an `invalid-utf8` error document. diffyman promises an answer for any pair of files, so a pair with a PNG, a gzip stream or Latin-1 text needs a result, not an error. That needs two things: a deterministic rule that says text or binary (and which media type), and a diff of plain bytes that finds insertions instead of reporting everything after the first change as different.

Options considered for detection: `infer` 0.22.0 (MIT, magic numbers, `no_std`), `file-format` 0.29.0, and `tree_magic_mini`, whose database feature is GPL-2.0-or-later and therefore excluded (ADR 0005). Machine-learned detectors (Magika) need ONNX Runtime and do not build for `wasm32-unknown-unknown`. For the byte diff: Myers over byte values (already pinned through `gix-imara-diff`), and content-defined chunking with `fastcdc` 5.0.0 (MIT) plus `blake3` 1.8.7 (CC0-1.0 OR Apache-2.0) for large inputs.

## Decision

- **Detection** (`diffyman-detect`, `detect.sniff@1`, spec §16.1) decides the class from the content only: valid UTF-8 (optional BOM) or valid UTF-16 after its BOM, with no NUL and few control bytes in the first 8000 bytes (git's window), is text; anything else is binary. The name never changes the class. The media type comes from `infer` for binary inputs and from a small extension table of registered types for text. `infer` is built without default features, so its Compound File parser never runs on untrusted input.
- **Adapter selection** (spec §16.2): a new option `adapter` (`auto`, `text`, `binary`). Under `auto`, two UTF-8 texts (or empty inputs) give kind `text`, exactly as before; anything else gives kind `binary`, with a `binary-fallback` warning when one side is text. `adapter: "text"` keeps the 0.2.0 behavior, including `invalid-utf8`.
- **Binary diff** (`diffyman-adapter-binary`, `binary.bytes@1`, spec §17): common prefix and suffix first; byte-level Myers for regions up to 64 KiB under a global work budget; FastCDC chunks with BLAKE3 for larger inputs, so equal regions are skipped and only changed chunks are aligned, with two smaller chunk levels for changed regions that are still too large (edits closer together than one chunk) and a position-by-position comparison as the last step for data overwritten in place; ranges closer than 8 bytes are joined. Every limit gives a coarser range and the `bytes-coarse-limit` warning, never a failure. The output is `bytes[]` (changes only, a complete edit script) and `stats.bytes`.
- Both crates build for `wasm32-unknown-unknown`: no file system, no threads, no memory maps, and `blake3` runs its portable code only.

Rejected: treating a text/binary mix as an error (it breaks the "any file" promise); deciding the class from the extension (a renamed file would change how it is compared); emitting equal ranges in `bytes[]` (they are implied by the gaps and would double the output for large files); minimal byte diffs at any cost (a valid edit script with bounded work is enough for a hex view, and a hostile input must not stall the engine).

## Consequences

- Schema 0.3.0 (spec §18). Inputs that are not UTF-8 now give binary documents instead of error documents, and valid UTF-8 with a NUL in its first 8000 bytes is now binary. Golden cases that test the text adapter's error pin `adapter: "text"`.
- Detection and the byte diff have their own algorithm IDs. An `infer`, `fastcdc` or `blake3` upgrade that changes any output bumps them, like any pinned library (spec §12).
- Renderers must handle kind `binary`; the unified renderer prints git's `Binary files ... differ`, and a hex view is still to come.
- Revisit when legacy encodings (for example Latin-1 decoded with an explicit option) or container formats (ZIP, OOXML) get their own adapters: both would move inputs from kind `binary` to richer kinds.
