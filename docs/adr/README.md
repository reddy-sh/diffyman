# Architecture decision records

This folder records the decisions that shape diffyman: what we chose, why, and what it costs us. An ADR is short, written once, and changed only by a newer ADR.

## Index

| # | Title | Status | Date |
|---|---|---|---|
| [0001](0001-rust-core.md) | Rust for the core engine | Accepted | 2026-11-07 |
| [0002](0002-diff-document-contract.md) | One canonical Diff Document as the contract | Accepted | 2026-11-07 |
| [0003](0003-postgres-over-sqlite.md) | PostgreSQL for the REST API service, not SQLite | Accepted | 2026-11-07 |
| [0004](0004-object-storage.md) | Object storage behind a `BlobStore` trait | Accepted | 2026-11-07 |
| [0005](0005-license.md) | MIT OR Apache-2.0, with a dependency license allowlist | Accepted | 2026-11-07 |
| [0006](0006-line-diff-library.md) | Line diff library and algorithm IDs | Accepted | 2026-11-07 |
| [0007](0007-binding-strategy.md) | Thin JSON-in/JSON-out bindings with typed wrappers | Accepted | 2026-11-07 |
| [0008](0008-wasm-target.md) | WASM target: opt-level 3, name section removed, no wasm-opt | Accepted | 2026-11-07 |
| [0009](0009-type-detection-and-binary-diff.md) | Type detection and a byte-level binary diff | Accepted | 2026-11-08 |
| [0010](0010-folders-and-archives.md) | Folders and archives as trees, read in memory with hard limits | Accepted | 2026-11-08 |
| [0011](0011-inline-spans-in-edited-moves.md) | Inline spans inside edited moves | Accepted | 2026-11-08 |
| [0012](0012-local-service-architecture.md) | Local service architecture | Accepted | 2026-11-08 |

## Numbering rules

- Numbers are four digits, start at `0001` and go up by one. Take the next free number when you open the pull request.
- Numbers `0006` to `0008` are held for the Phase 0 spike results. Do not use them for anything else.
- A number is never reused, even if the ADR is rejected or withdrawn.
- File names are `NNNN-short-kebab-title.md`.
- An accepted ADR is not rewritten. To change a decision, write a new ADR, set the old one's status to `Superseded by NNNN`, and link both ways. Fixing typos or broken links is fine.
- Every change to the Diff Document schema needs an ADR and a matching change to `docs/spec/` (see ADR 0002).

## Status values

`Proposed`, `Accepted`, `Rejected`, `Deprecated`, `Superseded by NNNN`.

## Template

Copy this into a new file and keep it under about 500 words.

```markdown
# NNNN. Title in a few words

- **Status:** Proposed
- **Date:** YYYY-MM-DD

## Context

What problem or force makes a decision necessary now? Facts, limits and
options considered, with versions where they matter.

## Decision

What we will do, stated plainly. Include the alternatives we rejected and
one line on why.

## Consequences

What becomes easier, what becomes harder, what we must now maintain, and
what would make us revisit this decision.
```
