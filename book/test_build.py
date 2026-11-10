"""Tests of book/build.py: python3 -m unittest discover -s book -p "test_*.py"."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import build
from build import BookError, Repo

GITHUB = "https://github.com/reddy-sh/diffyman"


def fake_repo(root: Path, files: dict[str, str]) -> None:
    for rel, text in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")


class RewriteLinks(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        fake_repo(
            self.root,
            {
                "README.md": "# Top\n",
                "LICENSE-MIT": "MIT\n",
                "docs/cli.md": "# CLI\n\n## Exit codes\n",
                "docs/spec/README.md": "# Specs\n",
                "docs/spec/doc.md": "# Doc\n",
                "docs/api/openapi.json": "{}\n",
                "crates/ffi/README.md": "# FFI\n\n## Ownership rules\n\n```md\n# Not a heading\n```\n",
                "examples/app/index.js": "\n",
            },
        )
        known = {
            "README.md",
            "LICENSE-MIT",
            "docs/cli.md",
            "docs/spec/README.md",
            "docs/spec/doc.md",
            "docs/api/openapi.json",
            "crates/ffi/README.md",
            "examples/app/index.js",
        }
        self.repo = Repo(
            root=self.root,
            known=known,
            chapters={"README.md", "docs/cli.md", "docs/spec/README.md", "docs/spec/doc.md"},
            assets={"docs/api/openapi.json"},
        )

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def rewrite(self, source: str, text: str) -> str:
        return build.rewrite_links(source, text, self.repo)

    def test_links_between_chapters_and_assets_stay(self) -> None:
        text = "See [cli](../cli.md#exit-codes), [doc](doc.md) and [api](../api/openapi.json).\n"
        self.assertEqual(self.rewrite("docs/spec/README.md", text), text)

    def test_other_files_link_to_github(self) -> None:
        text = "[l](../LICENSE-MIT) [f](../crates/ffi/README.md#ownership-rules) [e](../examples/app)\n"
        self.assertEqual(
            self.rewrite("docs/cli.md", text),
            f"[l]({GITHUB}/blob/main/LICENSE-MIT) "
            f"[f]({GITHUB}/blob/main/crates/ffi/README.md#ownership-rules) "
            f"[e]({GITHUB}/tree/main/examples/app)\n",
        )

    def test_folder_with_a_readme_chapter_links_to_it(self) -> None:
        self.assertEqual(
            self.rewrite("README.md", "[specs](docs/spec/)\n"), "[specs](docs/spec/README.md)\n"
        )

    def test_reference_definitions_and_html(self) -> None:
        text = '[x]: LICENSE-MIT\n<a href="LICENSE-MIT">l</a>\n'
        self.assertEqual(
            self.rewrite("README.md", text),
            f'[x]: {GITHUB}/blob/main/LICENSE-MIT\n<a href="{GITHUB}/blob/main/LICENSE-MIT">l</a>\n',
        )

    def test_code_is_left_alone(self) -> None:
        text = "```md\n[gone](missing.md)\n```\n`[gone](missing.md)` and [ok](https://example.org)\n"
        self.assertEqual(self.rewrite("README.md", text), text)

    def test_broken_links_fail(self) -> None:
        for target in ("missing.md", "../outside.md", "/abs.md", "docs/cli.md?x=1"):
            with self.subTest(target=target), self.assertRaises(BookError):
                self.rewrite("README.md", f"[x]({target})\n")

    def test_unknown_fragment_outside_the_book_fails(self) -> None:
        with self.assertRaises(BookError):
            self.rewrite("README.md", "[x](crates/ffi/README.md#not-a-heading)\n")

    def test_untracked_file_fails(self) -> None:
        fake_repo(self.root, {"docs/private.md": "# Private\n"})
        with self.assertRaises(BookError):
            self.rewrite("README.md", "[x](docs/private.md)\n")


class Slugs(unittest.TestCase):
    def test_github_slugs(self) -> None:
        self.assertEqual(build.github_slug("Phase 3 — local platform"), "phase-3--local-platform")
        self.assertEqual(
            build.github_slug("6.8 Tree (`tree`, since 0.3.0)"), "68-tree-tree-since-030"
        )

    def test_repeated_headings_are_numbered(self) -> None:
        anchors = build.heading_anchors("# A\n\n## Notes\n\n## Notes\n\n```\n# Code\n```\n")
        self.assertEqual(anchors, {"a", "notes", "notes-1"})


class Summary(unittest.TestCase):
    def test_chapters_in_order(self) -> None:
        text = "# Summary\n\n[Intro](README.md)\n\n- [A](docs/a.md)\n  - [B](docs/b.md)\n"
        self.assertEqual(build.summary_chapters(text), ["README.md", "docs/a.md", "docs/b.md"])

    def test_drafts_duplicates_and_other_files_fail(self) -> None:
        for text in ("- [Draft]()\n", "- [A](a.md)\n- [A again](a.md)\n", "- [J](a.json)\n"):
            with self.subTest(text=text), self.assertRaises(BookError):
                build.summary_chapters(text)


class OpenApi(unittest.TestCase):
    DOC = {
        "openapi": "3.1.0",
        "info": {"version": "1.2.3", "description": "Send `Bearer <key>` or <b>."},
        "tags": [{"name": "things", "description": "Things"}],
        "paths": {
            "/v1/things/{id}": {
                "get": {
                    "tags": ["things"],
                    "summary": "Gets a | thing.",
                    "operationId": "get_thing",
                    "parameters": [
                        {"name": "id", "in": "path", "required": True, "schema": {"type": "string"}}
                    ],
                    "responses": {
                        "200": {
                            "description": "The thing",
                            "content": {
                                "application/json": {
                                    "schema": {"$ref": "#/components/schemas/Thing"}
                                }
                            },
                        }
                    },
                    "security": [{"bearer": []}],
                }
            }
        },
        "components": {
            "securitySchemes": {"bearer": {"type": "http", "scheme": "bearer"}},
            "schemas": {
                "Thing": {
                    "type": "object",
                    "required": ["id"],
                    "properties": {
                        "id": {"type": "string", "format": "uuid", "description": "ID."},
                        "tags": {"type": ["array", "null"], "items": {"type": "string"}},
                        "state": {"type": "string", "enum": ["a", "b"]},
                    },
                }
            },
        },
    }

    def test_render(self) -> None:
        page = build.render_openapi(self.DOC)
        self.assertIn("| GET | [`/v1/things/{id}`](#op-get-v1-things-id) | Gets a \\| thing. |", page)
        self.assertIn('<a id="op-get-v1-things-id"></a>', page)
        self.assertIn("### GET `/v1/things/{id}`", page)
        self.assertIn("`application/json`: [`Thing`](#schema-thing)", page)
        self.assertIn("| `id` | string (uuid) | yes | ID. |", page)
        self.assertIn("| `tags` | array of string or null | no |  |", page)
        self.assertIn('one of `"a"`, `"b"`', page)
        # Raw HTML is escaped outside code; code keeps its text.
        self.assertIn("Send `Bearer <key>` or &lt;b>.", page)

    def test_unknown_reference_fails(self) -> None:
        with self.assertRaises(BookError):
            build.schema_type({"$ref": "other.json#/Thing"})


class CheckSite(unittest.TestCase):
    def check(self, files: dict[str, str]) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            fake_repo(Path(tmp), files)
            problems, _ = build.check_site(Path(tmp))
            return problems

    def test_good_site(self) -> None:
        problems = self.check(
            {
                "index.html": '<a href="docs/a.html#x">a</a><a href="https://example.org">e</a>'
                '<link href="/diffyman/style.css"><a href="docs/">d</a>',
                "docs/a.html": '<h2 id="x">X</h2><a href="../index.html">top</a><a href="#x">x</a>',
                "docs/index.html": "<p>docs</p>",
                "style.css": "body { background: url(img.png); }",
                "app.js": "const page = 'docs/a.html';",
                "searchindex-1.js": "the page text may mention cdnjs.cloudflare.com",
            }
        )
        self.assertEqual(problems, [])

    def test_problems_are_found(self) -> None:
        problems = self.check(
            {
                "index.html": '<a href="missing.html">m</a><a href="docs/a.html#nope">n</a>'
                '<script src="https://cdn.example.org/x.js"></script>'
                '<link href="//fonts.example.org/f.css"><a href="/elsewhere/">o</a>'
                '<div style="background: url(https://t.example.org/p.gif)"></div>',
                "docs/a.html": '<h2 id="x">X</h2>',
                "style.css": "@import 'https://fonts.googleapis.com/css';",
                "app.js": "load('https://www.googletagmanager.com/gtag/js');",
            }
        )
        text = "\n".join(problems)
        for expected in (
            "missing.html does not exist",
            "docs/a.html has no id 'nope'",
            "<script src> points outside the site",
            "<link href> points outside the site",
            "is outside /diffyman/",
            "<div style> loads from another origin",
            "style.css: style sheet loads https://fonts.googleapis.com/css",
            "style.css: mentions fonts.googleapis.com",
            "app.js: mentions googletagmanager.com",
        ):
            self.assertIn(expected, text)
        self.assertEqual(len(problems), 9, text)


class Assemble(unittest.TestCase):
    def test_assemble_a_small_book(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            fake_repo(
                root,
                {
                    "README.md": "# Top\n\n[cli](docs/cli.md) [license](LICENSE-MIT)\n",
                    "LICENSE-MIT": "MIT\n",
                    "docs/cli.md": "# CLI\n\n[api](api/openapi.json) [top](../README.md)\n",
                    "docs/api/openapi.json": '{"openapi": "3.1.0", "paths": {}}\n',
                    "book/openapi.md": "# OpenAPI\n\n<!-- openapi-reference -->\n",
                    "book/book.toml": "[book]\n",
                    "book/SUMMARY.md": "# Summary\n\n[Top](README.md)\n\n- [CLI](docs/cli.md)\n"
                    "- [OpenAPI](book/openapi.md)\n",
                },
            )
            out = Path(tmp) / "out"
            chapters = build.assemble(out, root)
            self.assertEqual(chapters, ["README.md", "docs/cli.md", "book/openapi.md"])
            self.assertEqual(
                (out / "README.md").read_text(encoding="utf-8"),
                f"# Top\n\n[cli](docs/cli.md) [license]({GITHUB}/blob/main/LICENSE-MIT)\n",
            )
            self.assertTrue((out / "docs/api/openapi.json").is_file())
            self.assertTrue((out / "SUMMARY.md").is_file())
            self.assertTrue((out / "book.toml").is_file())
            self.assertIn("OpenAPI 3.1.0", (out / "book/openapi.md").read_text(encoding="utf-8"))

    def test_missing_chapter_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake_repo(root, {"book/SUMMARY.md": "- [Gone](docs/gone.md)\n", "book/book.toml": ""})
            with self.assertRaises(BookError):
                build.assemble(root / "out", root)


if __name__ == "__main__":
    unittest.main()
