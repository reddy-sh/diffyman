# 0011. Inline spans inside edited moves

- **Status:** Accepted
- **Date:** 2026-11-08

## Context

An edited move (`moves[]` with `kind: "edited"`) says that a block was
deleted in one place and inserted, with small edits, in another: a function
that moved and had a variable renamed, a paragraph that moved and had a typo
fixed. Up to schema 0.3.0, moved lines carry no inline spans, because the
spans of an op compare its own left and right lines, and a moved line's
counterpart is in another op. So a reader sees that the block moved but not
what changed inside it, which is the part that needs review (corpus case
`moved/026`: `acc` renamed to `hash` on every line, with no highlighting).

Options considered:

1. Spans on the referenced ops (`fromOps`, `toOps`): an op's `inline` would
   then mix spans of two unrelated comparisons, and a `delete` or `insert` op
   would carry spans, which breaks the rule that only `replace` ops do.
2. A per-line pairing, `{fromLine, toLine, spans[]}` for each paired line:
   it exposes the move search's internal alignment, which is not part of the
   output today, and lines without a partner (an added line, a dropped
   comment) need a second shape.
3. One `inline: Span[]` per move, with the `Span` shape and order of an op's
   `inline`, computed by refining the two ranges as one `replace` op.

## Decision

Option 3, schema 0.4.0 (spec §6.5, moves spec §4.7). Each move has a required
`inline` array. For an edited move it holds the spans of `text.refine@1` over
the move's left range against its right range, with the document's
`granularity`, ignore rules, regex masks, limits and `smart` noise cutoff, and
one difference: each line's indentation (its leading whitespace, up to the
first regex match) is left out, as the move key leaves it out, so a block
that moved into a nested scope is not highlighted for its new indentation.
Pure moves, granularity `line`, a move over the refinement limits
(`refine-skipped-limit`) and the noise cutoff give `[]`. Ops still never
carry spans for moved lines, so every changed line has its spans in exactly
one place.

The refinement IDs stay at `@1`: neither `text.refine@1` nor `text.moves@1`
has been released.

## Consequences

- Renderers reuse their span code: for a moved line they read the spans of
  its move instead of those of its op, keyed by side and absolute line.
- Limits and determinism are those of refinement already: at most 100 lines,
  16 KiB and 4096 tokens per side, the same Myers token diff, no clock.
- A token diff over a whole block can align a token with a similar one on a
  neighbouring line (an added list item may leave the `- ` of the next line
  highlighted), as it can in a multi-line `replace` op. A per-line pairing
  would avoid that; a later minor version may add one as a new field.
- Every document with a move changes `id`, and the moved corpus was
  re-blessed; the edited cases now run with `granularity: "smart"`.
