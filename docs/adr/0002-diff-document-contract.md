# 0002. One canonical Diff Document as the contract

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

diffyman promises the same answer in every language. The CLI, the Rust crate, the Python, Node, WASM and Java bindings and the REST API all run the same engine (ADR 0001). If each surface had its own result shape, or serialized it differently, they would drift: a renamed field, `0.75` here and `0.7500` there, keys in hash-map order. Callers could not compare, cache or store results across surfaces, and CI could not prove the promise. We also want results that can be rendered later without re-running the diff, with an identifier that is the same wherever the diff ran.

## Decision

1. **One result format.** Every surface returns the **Diff Document**, defined in `docs/spec/diff-document.md`. Renderers (unified patch, HTML, side-by-side) read a Diff Document.
2. **Spec first.** The spec text is normative. The Rust types in `diffyman-model` are checked against it, and the JSON Schema (2020-12) generated from them is published with each release. TypeScript, Python, Java and C# types are generated from that schema.
3. **Canonical form.** RFC 8785 JSON Canonicalization Scheme (JCS): sorted keys, fixed escaping, ECMAScript number formatting, no insignificant whitespace.
4. **Content id.** `id` is `"sha256:"` plus the lowercase hex SHA-256 of the JCS bytes with the top-level `id` and `provenance` keys removed. The same inputs, input names and options give the same `id` everywhere.
5. **Provenance is outside the contract.** Engine version, binding, platform and timing live only in `provenance`, which is excluded from `id` and from conformance.
6. **Byte-identical conformance.** CI runs every case in `tests/corpus/<case>/` through every surface on macOS, Linux and Windows. Output whose JCS bytes, with `provenance` removed, differ from `expected.json` fails the build.
7. **Determinism.** Stable array order, no locale-dependent output, integers only except `stats.similarity`, and versioned algorithm IDs such as `text.histogram@1`. Any change in an algorithm's output, including a library upgrade, bumps its ID and regenerates the corpus in the same change.
8. **Governed evolution.** `schemaVersion` is semver. Readers ignore unknown fields, enums are open, and field names are never reused. Every schema change needs an ADR and a spec change.

Alternatives rejected:

- **Per-surface native result objects.** Easy to write, impossible to keep identical.
- **Comparing parsed JSON trees.** Hides number and escaping differences that break caching and hashing.
- **A binary format.** Harder to read and debug, and the JSON Schema already gives typed bindings.

## Consequences

- Diff logic and result shaping are written once, in Rust. Bindings stay thin: `diff(left, right, options_json) -> json`, plus typed wrappers.
- Each binding must return the engine's canonical bytes unchanged or use a JCS-correct serializer. Number formatting and escaping need dedicated tests.
- The golden corpus becomes a core asset. It is hand-checked and never contains a result flagged `approximate`.
- Input names are part of the `id`, so the conformance harness passes the corpus file names.
- Streaming (NDJSON hunks) and paging must preserve the same canonical content.
- Upgrading a diff library is a visible, versioned event, not a silent change.
