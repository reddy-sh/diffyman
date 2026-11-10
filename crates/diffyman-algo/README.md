# diffyman-algo

The text algorithms of [diffyman](https://github.com/reddy-sh/diffyman): line diff (Histogram,
Myers and Patience) with slider compaction, inline refinement by word, character or grapheme,
ignore rules and moved-block detection.

This is a building block of the [`diffyman`](https://crates.io/crates/diffyman) engine; its API
follows the engine's needs and may change in any release. Depend on `diffyman` instead. The
two are released together with the same version.

- [Documentation](https://reddy-sh.github.io/diffyman)
- [Text granularity and moved blocks](https://github.com/reddy-sh/diffyman/blob/main/docs/spec/text-granularity-and-moves.md)

## License

`(MIT OR Apache-2.0) AND Apache-2.0`: the crate is MIT OR Apache-2.0, at your option, except
the Histogram diff copied from gix-imara-diff (`src/line_diff/histogram.rs` and
`src/line_diff/histogram/`), which is Apache-2.0 only. See `src/line_diff/histogram/NOTICE`.
