# Specifications

The normative specs for diffyman. Spec text comes first: the Rust types in `diffyman-model` and the generated JSON Schema are checked against these files, and every schema change needs an ADR plus a spec change.

| File | Status | Covers |
|---|---|---|
| [diff-document.md](diff-document.md) | Draft, schema `0.2.0` | The Diff Document JSON: fields (including the `moves[]` shape), canonical form (RFC 8785) and `id`, positions, line handling, options, the warning table, algorithm IDs, conformance, worked example |
| [text-granularity-and-moves.md](text-granularity-and-moves.md) | Draft | How inline spans (`word`, `char`, `smart`) and moved blocks are computed, their constants, and the labeled moved-block corpus with its metrics |

`diff-document.md` is the single source for every output shape, option and warning code. `text-granularity-and-moves.md` refers to it for those and only defines the algorithms. Where the two disagree, `diff-document.md` wins.

Planned:

| File | Covers |
|---|---|
| `diff-document.schema.json` | JSON Schema 2020-12, generated from `diffyman-model` and published with each release |

Conformance cases live in `tests/corpus/<group>/<case>/` (see diff-document.md §3 and `tests/corpus/README.md`).
