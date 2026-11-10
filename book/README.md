# Documentation site

The site at <https://reddy-sh.github.io/diffyman> (not published yet) is an [mdBook](https://rust-lang.github.io/mdBook/) built from the repository's own Markdown: the README, CONTRIBUTING.md and everything under `docs/`. No page is copied into this folder, so a page is edited in one place and reads the same on GitHub and on the site.

| File | What it is |
|---|---|
| `SUMMARY.md` | The table of contents. Its links are paths from the repository root. Every Markdown file under `docs/` must be listed. |
| `book.toml` | The mdBook configuration: no preprocessors, no analytics, nothing loaded from another origin, edit links to GitHub. |
| `openapi.md` | The OpenAPI reference page. The build fills in its operations and schemas from `docs/api/openapi.json`. |
| `build.py` | Assembles, builds and checks the site (Python 3.11+, standard library only). |
| `test_build.py` | Tests of `build.py`. |

## Build it

```sh
cargo install --locked mdbook --version 0.5.4 --no-default-features --features search
python3 book/build.py                  # writes target/book/html
python3 -m unittest discover -s book -p "test_*.py"
```

Open `target/book/html/index.html`, or serve the folder (`python3 -m http.server --directory target/book/html`). `--mdbook PATH` picks another mdbook binary. `--no-build` only assembles and checks the sources.

`build.py` writes each chapter to `target/book/src` at its repository path, so links between chapters stay as they are. A relative link to any other file of the repository becomes a link to that file on GitHub (`blob/main`), and a link to a folder whose `README.md` is a chapter goes to that chapter. The build fails when:

- a link names a file that does not exist or that git does not know;
- a fragment names no heading of a Markdown file outside the book;
- a Markdown file under `docs/` is missing from `SUMMARY.md`;
- in the built HTML, a local link or fragment does not resolve;
- a page, style sheet or script loads anything from another origin, or names an analytics, font or script CDN host.

Links to other sites are counted but not fetched.

## Publishing

`.github/workflows/docs.yml` builds and checks the site on every push and pull request, and uploads it as the `docs-site` artifact. It deploys to GitHub Pages only when the workflow is run by hand on `main` with `deploy: true`, and only after Pages is set to "GitHub Actions" in the repository settings.

The site serves mdBook's theme files: its scripts and styles (MPL-2.0), highlight.js (BSD-3-Clause), elasticlunr, mark.js and clipboard.js (MIT), and the Open Sans (Apache-2.0) and Source Code Pro (OFL-1.1) fonts with their license files. None of them is changed.
