# 034-folder-exclude-globstar

`tree.exclude` with `.gitignore` globstar patterns: `**/*.tmp` at any depth, `docs/**` for everything inside `docs`, and a negation that brings `docs/keep.md` back (allowed because `docs` itself is not excluded). A correct result leaves out both `.tmp` files and `docs/guide.md`, lists `docs` and `src/deep` as `unchanged` folders, `docs/keep.md` as `modified` with a text child, and `src/main.rs` as `unchanged`.
