# 0010. Folders and archives as trees, read in memory with hard limits

- **Status:** Accepted
- **Date:** 2026-11-08

## Context

"Diff any file" includes two folders, two zip files, and a folder against the archive it was packed into. Each needs a list of entries with a status, renames that are not reported as delete-plus-add, and the diff of every changed file. Archives come from strangers, so the reader faces zip bombs, forged entry counts, `../` and absolute names, duplicate names, symbolic links, encrypted entries and nested archives. Every result must stay deterministic and reproducible in every binding, including WASM, which has no file system.

Options considered: `zip` 9.4.1 (MIT), `tar` 0.4.46 (MIT OR Apache-2.0), `flate2` 1.1.11 with its pure-Rust `miniz_oxide` backend, `ruzstd` 0.9.1 (MIT), `ignore` 0.4.34 (Unlicense OR MIT) for `.gitignore` rules, and extracting to a temporary folder (rejected below). All build for `wasm32-unknown-unknown`.

## Decision

- **Kind `tree`** (spec §6.8, §19): one entry per path with `added`, `removed`, `modified`, `unchanged`, `moved` or `typeChange`; both sides' type, size, SHA-256 and link target; and for each modified file pair an embedded child document, produced by the same engine (text, binary, or a nested tree) without its `provenance`, so the parent `id` stays deterministic. Renames are pairs of removed and added files with equal SHA-256 (same last component first, then path order); empty files never move.
- **Selection** (spec §16.2): under `adapter: "auto"`, two archives, recognized from their first bytes only, give kind `tree`; `adapter: "tree"` forces it. A folder is always a tree. New `tree` options: `followSymlinks`, `ignoreFiles`, `exclude`, `detectMoves`, `maxDepth`, `childBudget`.
- **Reading** (new crate `diffyman-adapter-tree`): archives are read from memory and never extracted; paths are normalized as strings and never resolved, and unsafe ones are listed with `unsafe-path`. zip uses the `zip` crate (stored and deflate only, CRC and declared size checked); every zip end record is checked against the entry limit before the central directory is parsed. tar is parsed by the adapter itself, because the `tar` crate reads pax and GNU long-name headers without a size limit; ours caps them at 64 KiB and streams file data into SHA-256. gzip and zstd layers are pure Rust. Folders are walked with `ignore` with every implicit filter off, in byte order, reading only names, types, contents and link targets.
- **Limits** (spec §19.6), all deterministic: entries and path text per side end in `limit-exceeded`; compression ratio (over 100× above 1 MiB), zip entries whose bytes overlap an earlier entry's (so one compressed stream is never inflated twice), the per-side content budget (4 GiB across all nesting levels, child contents included) and path length end in `tree-limit`; unreadable and encrypted entries are compared by their stored bytes. Any of those makes the status `partial`, the first use of that status value. Child documents are bounded by input size and by `tree.childBudget` on their canonical JSON; a nested tree sets its own size aside before its children use the budget, and no child is computed once the budget is smaller than the smallest possible child, so the work stays proportional to the budget.
- **WASM**: the `fs` feature (folder walking) is off in the WASM binding; archives from bytes work there unchanged.

Rejected: extracting to a temporary folder (zip-slip and symlink races, disk use, no WASM); the `tar` crate (unbounded extension headers); decrypting entries (needs passwords and crypto dependencies, and is not a diff concern); flattening nested archives into `outer.zip!/inner` paths (child documents keep each archive's own entries and limits); rename detection by similarity (expensive and heuristic; equal content is exact).

## Consequences

- Schema 0.3.0 grows the `tree` block, `stats.tree`, the `tree` options and ten warning codes (spec §18). Two archives that 0.3.0 drafts compared as bytes are now trees; `adapter: "binary"` keeps the byte ranges.
- `tree.entries@1` covers the rules, the limits and the pinned `zip`, `flate2`, `ruzstd` and `ignore` versions (spec §12).
- The tar reader is ours to maintain and fuzz. bzip2, xz and 7z are not read yet; such inputs stay binary.
- Revisit when a service stores child documents apart (`childRef`), when Office or package formats get their own adapters (they are zip files and are trees today), or when the limits prove too tight for real archives.
