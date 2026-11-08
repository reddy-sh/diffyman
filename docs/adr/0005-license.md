# 0005. MIT OR Apache-2.0, with a dependency license allowlist

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

diffyman's core (engine, CLI, bindings and the REST API service) is open source and meant to be embedded in other people's products, including commercial ones. Its license must not stop that, and no dependency may quietly add copyleft terms. Many tools in the diff space are GPL, AGPL or LGPL (for example diffoscope, git's xdiff, Mergiraf, GumTree, WinMerge, Meld, MuPDF, dssim, pHash and ImHex), so the risk of pulling one in is real.

## Decision

**Project license.** The core is dual-licensed **MIT OR Apache-2.0**, at the user's choice (`LICENSE-MIT`, `LICENSE-APACHE`). This is the Rust ecosystem norm, matches most of our dependencies, and Apache-2.0 adds a patent grant. Contributions use DCO sign-off (`Signed-off-by`). No CLA is needed.

**Dependency allowlist.** One list, enforced by **cargo-deny** in CI on every change:

- **Allowed:** MIT, Apache-2.0 (including `Apache-2.0 WITH LLVM-exception`), BSD-2-Clause, BSD-3-Clause, ISC, Zlib, CC0-1.0, Unicode-3.0, BSL-1.0.
- **Allowed with care: MPL-2.0.** It is file-level copyleft. A runtime dependency under MPL-2.0 (such as `uniffi`) means we ship its notices and publish any changes to its files. Build-time-only tools (such as `cbindgen`) are fine.
- **Denied in shipped code:** GPL, AGPL, LGPL (all versions), SSPL and BUSL. Copyleft programs (for example FFmpeg or diffoscope) may only run as optional external processes, never bundled or linked.
- **Unknown or unlicensed crates are denied** until reviewed.

**License-risk register.** Known copyleft projects are studied for behavior only, never copied, and each has a permissive replacement:

| Avoid | License | Use instead |
|---|---|---|
| git xdiff | LGPL-2.1+ | `similar` / `imara-diff`, or reimplement from papers |
| diffoscope | GPL-3+ | Our own adapter registry |
| Mergiraf | GPL-3.0-only | Our own structured merge (later) |
| GumTree, Graphtage | LGPL-3.0 | Reimplement from the published algorithms |
| MuPDF / PyMuPDF, poppler | AGPL-3 / GPL | PDFium, hayro, pdf.js |
| dssim, pHash | AGPL-3.0 / GPL-3 | image-compare, ssimulacra2, image_hasher |
| ImHex | GPL-2 | Our own hex diff |
| libheif, rawler | LGPL | Defer HEIC and RAW |
| `tree_magic_mini` feature `with-gpl-data` | GPL-2.0+ | `infer` / `file-format`; never enable that feature |
| epub crate | GPL-3.0+ | zip + quick-xml |
| UnRAR source | Proprietary terms | Optional plugin after legal review |

Grammars, fonts and other data bundled at build time follow the same allowlist.

**Tooling.** `deny.toml` at the repository root holds the allowlist. `cargo-deny` (licenses, bans, advisories, sources) runs in CI. `cargo-about` generates the third-party notices file for each release. Both tools are MIT OR Apache-2.0.

## Consequences

- Anyone can embed diffyman in open or closed products under either license.
- Adding a dependency with a new license is a deliberate change to `deny.toml`, reviewed in its own pull request.
- Some useful libraries are off limits, so we build more ourselves (structural diff, hex diff, pixel diff) or reimplement algorithms from papers.
- Every release must ship complete license notices, including MPL-2.0 files.
- Non-Rust packages (npm, PyPI, Maven artifacts) need an equivalent license check, to be added with each binding.
- A lawyer should confirm the project license and the allowlist before the first public release.
