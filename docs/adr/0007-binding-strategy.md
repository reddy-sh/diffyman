# 0007. Thin JSON-in/JSON-out bindings with typed wrappers

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

diffyman promises the same Diff Document in every language (ADR 0002). diffyman ships rich native
bindings (Python, Node, browser WASM, Java, Swift) and one C ABI for the rest. Each binding could map the
document into native objects in Rust (`#[pyclass]`, `serde-wasm-bindgen`, JNI object builders), or it
could pass the engine's canonical JSON through unchanged. Spike B
([results](../spikes/B-binding-round-trip.md)) built the Python (PyO3 0.30.1, abi3) and WASM
(wasm-bindgen 0.2.130) bindings the second way and checked them against the CLI on all 60 corpus cases.
Byte-identical output held for all 60 cases, and for 40 more line diffs.

## Decision

1. **One native entry point.** Every binding calls `diffyman::diff_inputs_with(left, right,
   options_json, settings)`: two byte inputs with optional names, an options object as JSON text, and the
   binding name for `provenance.binding`. It returns the canonical (RFC 8785) Diff Document as a string.
   The native layer of each binding is only this call plus error mapping.
2. **The canonical string is the primary result.** `diff(...) -> str` in Python and
   `diff(...): string` in JavaScript return the engine's bytes unchanged. No binding re-serializes the
   document, so number formatting, escaping and key order cannot drift.
3. **Typed wrappers live in the host language.** Conveniences such as reading `os.PathLike` inputs,
   accepting a `dict` for options and returning a parsed document (`diff_document(...) -> dict`) are
   written in Python or JavaScript over the string. Typed views (Python dataclasses, TypeScript types,
   Java records) are generated from the JSON Schema that `diffyman schema` prints, never written by hand.
4. **Inputs keep their names.** Names are part of the document `id`, so every binding accepts optional
   left and right names. Python takes them from path inputs; a Python `str` is text, never a path.
5. **One error rule.** Invalid options (unknown key, wrong type, unknown value) raise the host's error
   type and return no document: `DiffymanError` (a `ValueError`) in Python, `Error` in JavaScript.
   Everything else, including invalid UTF-8 and unsupported options, is a document with
   `status: "error"`.
6. **Conformance gates every binding.** `tests/conformance/run_bindings.py` runs every corpus case
   through the CLI and every binding and requires byte-identical documents, with `provenance` removed.
   It runs in CI on Linux and macOS. A new binding joins it before it ships.
7. **Next bindings follow the same shape.** Node (napi-rs), Java (JNI) and the C ABI expose
   `diff(left, right, optionsJson[, leftName, rightName]) -> string` first and add typed wrappers on top.

Alternatives rejected:

- **Native object graphs built in Rust** (`#[pyclass]` per type, `serde-wasm-bindgen`, JNI builders).
  Each binding would re-serialize the document its own way, and every schema change would touch every
  binding.
- **WASM everywhere** (Python and Java through a WASM runtime). Workable, but slower: the WASM build is
  about 1.6 times slower than native in the spike, and it adds a runtime dependency. It stays the
  no-native-code option (ADR 0001).
- **Separate entry points per binding.** They would drift. One function keeps the surfaces equal by
  construction.

## Consequences

- A binding is small: under 100 lines of Rust plus a host-language wrapper. Adding a field to the schema
  changes no binding code, only the generated types.
- Callers that need objects pay one JSON parse. For a 41 KB document this is small next to the diff
  itself, and callers that store or forward the result can skip it.
- The JSON Schema becomes a build input for every typed wrapper, so it must be published with each
  release (ADR 0002 already requires this).
- Inputs cross the boundary as copied bytes. Very large inputs (the large-file mode in Phase 1) may need
  a path-based or streaming entry point. That would be a new function with the same output contract.
- Revisit if profiling shows the JSON boundary dominates for typical documents, or if a host needs
  incremental results before the whole document is ready.
