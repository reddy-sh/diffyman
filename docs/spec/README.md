# Specifications

The normative specs for diffyman. Spec text comes first: the Rust types in `diffyman-model` and the generated JSON Schema are checked against these files, and every schema change needs an ADR plus a spec change.

| File | Status | Covers |
|---|---|---|
| [diff-document.md](diff-document.md) | Draft, schema `0.1.0` | The Diff Document JSON: fields, canonical form (RFC 8785) and `id`, positions, line handling, options, warnings, algorithm IDs, conformance, worked example |

Planned for Phase 0:

| File | Covers |
|---|---|
| `smart-granularity.md` | When "smart" granularity picks word or character highlights, with test cases |
| `moved-blocks.md` | Moved-block detection thresholds and the `moves[]` block, with at least 30 labeled cases |
| `diff-document.schema.json` | JSON Schema 2020-12, generated from `diffyman-model` and published with each release |

Conformance cases live in `tests/corpus/<case>/` (see diff-document.md §3).
