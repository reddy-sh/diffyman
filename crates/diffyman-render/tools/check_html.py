#!/usr/bin/env python3
"""Parses every HTML report diffyman generates and checks its safety rules.

The reports checked are:

- the renderer snapshots (crates/diffyman-render/tests/snapshots/*.html) and
  the CLI golden files (crates/diffyman-cli/tests/golden/*.html);
- a fresh report for every corpus case (tests/corpus/{text,moved,binary,tree}),
  made by the CLI with `diffyman diff <left> <right> -f html --options ...`.

Each report is parsed with Python's html.parser and must be well formed and
safe:

- every element is closed explicitly and in order (void elements aside), and
  the document ends with every element closed;
- only allowlisted tags and attributes appear: no script, no event handler
  (`on...`), no `src`, `style` or `srcdoc` attribute, exactly one `<style>`
  element, and no `url(`, `@import` or `expression(` in it, so nothing is
  loaded from anywhere;
- every link is an in-page `#id` that exists, and ids are unique;
- no text or attribute value holds a bidi-control character, a C0 or C1
  control (tab and line ends aside) or another invisible character the
  renderer labels, so names and content cannot reorder or hide parts of the
  page.

Usage (from the repository root; builds the CLI first when it is missing):

    uv run --python 3.12 crates/diffyman-render/tools/check_html.py [--cli PATH]

It prints a summary and exits with status 1 when a report breaks a rule.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CORPUS = ROOT / "tests" / "corpus"
GROUPS = ("text", "moved", "binary", "tree")
STATIC = (
    ROOT / "crates" / "diffyman-render" / "tests" / "snapshots",
    ROOT / "crates" / "diffyman-cli" / "tests" / "golden",
)

TAGS = {
    "html", "head", "meta", "title", "style", "body", "header", "h1", "h2", "p",
    "span", "strong", "ul", "ol", "li", "details", "summary", "table", "thead",
    "tbody", "tr", "th", "td", "code", "main", "input", "label", "section",
    "colgroup", "col", "del", "ins", "a", "footer",
}
VOID = {"meta", "input", "col"}
ATTRS = {
    "class", "id", "href", "title", "lang", "charset", "name", "content", "type",
    "for", "colspan", "scope", "role", "aria-label", "aria-labelledby",
    "aria-hidden", "checked",
}


def forbidden_char(c: str) -> bool:
    """The characters the renderer always replaces with a visible label."""
    o = ord(c)
    if c in "\t\n\r":
        return False
    return (
        o < 0x20
        or 0x7F <= o <= 0x9F
        or o in (0xAD, 0x34F, 0x61C, 0x180E, 0x200B, 0x200E, 0x200F, 0x2028, 0x2029, 0xFEFF)
        or o in (0x115F, 0x1160, 0x3164, 0xFFA0, 0x17B4, 0x17B5)
        or 0x202A <= o <= 0x202E
        or 0x2060 <= o <= 0x2064
        or 0x2066 <= o <= 0x206F
        or 0xFFF9 <= o <= 0xFFFB
        or 0x1D173 <= o <= 0x1D17A
        or 0xE0000 <= o <= 0xE007F
    )


class Checker(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.stack: list[str] = []
        self.errors: list[str] = []
        self.ids: set[str] = set()
        self.hrefs: list[str] = []
        self.styles = 0
        self.in_style = False
        self.css = ""
        self.doctype = False

    def error(self, message: str) -> None:
        line, col = self.getpos()
        if len(self.errors) < 20:
            self.errors.append(f"line {line}:{col}: {message}")

    def handle_decl(self, decl: str) -> None:
        if decl.lower() != "doctype html":
            self.error(f"unexpected declaration <!{decl}>")
        self.doctype = True

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag not in TAGS:
            self.error(f"tag <{tag}> is not allowed")
        for name, value in attrs:
            if name not in ATTRS or name.startswith("on"):
                self.error(f"attribute {name} on <{tag}> is not allowed")
            value = value or ""
            if any(forbidden_char(c) for c in value):
                self.error(f"attribute {name} holds a control or invisible character")
            if name == "id":
                if value in self.ids:
                    self.error(f"duplicate id {value}")
                self.ids.add(value)
            if name == "href":
                if not value.startswith("#"):
                    self.error(f"link {value!r} leaves the page")
                self.hrefs.append(value)
        if tag == "style":
            self.styles += 1
            self.in_style = True
        if tag not in VOID:
            self.stack.append(tag)

    def handle_startendtag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        self.error(f"self-closing <{tag}/>")
        self.handle_starttag(tag, attrs)

    def handle_endtag(self, tag: str) -> None:
        if tag in VOID:
            self.error(f"end tag for void element </{tag}>")
            return
        if not self.stack or self.stack[-1] != tag:
            top = self.stack[-1] if self.stack else "nothing"
            self.error(f"</{tag}> closes <{top}>")
            if tag in self.stack:
                while self.stack and self.stack.pop() != tag:
                    pass
            return
        self.stack.pop()
        if tag == "style":
            self.in_style = False

    def handle_data(self, data: str) -> None:
        if self.in_style:
            self.css += data
            return
        if any(forbidden_char(c) for c in data):
            bad = next(c for c in data if forbidden_char(c))
            self.error(f"text holds U+{ord(bad):04X}")

    def handle_comment(self, data: str) -> None:
        self.error("comment in the report")

    def handle_pi(self, data: str) -> None:
        self.error("processing instruction in the report")

    def finish(self) -> list[str]:
        self.close()
        if not self.doctype:
            self.error("no <!DOCTYPE html>")
        if self.stack:
            self.error(f"unclosed elements at the end: {self.stack}")
        if self.styles != 1:
            self.error(f"{self.styles} <style> elements")
        for bad in ("url(", "@import", "expression("):
            if bad in self.css.lower():
                self.error(f"the styles use {bad}")
        for href in self.hrefs:
            if href[1:] not in self.ids:
                self.error(f"dangling link {href}")
        return self.errors


def check(text: str) -> list[str]:
    checker = Checker()
    checker.feed(text)
    return checker.finish()


def side(case: Path, stem: str) -> Path:
    if (case / stem).is_dir():
        return case / stem
    found = sorted(p for p in case.iterdir() if p.name.startswith(stem + "."))
    if len(found) != 1:
        raise SystemExit(f"{case}: expected one {stem}.* input, found {len(found)}")
    return found[0]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--cli", type=Path, default=ROOT / "target" / "debug" / "diffyman")
    args = parser.parse_args()
    if not args.cli.exists():
        subprocess.run(
            [str(ROOT / "scripts" / "with-rustup"), "cargo", "build", "--manifest-path",
             str(ROOT / "Cargo.toml"), "-p", "diffyman-cli"],
            check=True,
        )

    reports: list[tuple[str, str]] = []
    for folder in STATIC:
        for path in sorted(folder.glob("*.html")):
            reports.append((str(path.relative_to(ROOT)), path.read_text(encoding="utf-8")))
    static = len(reports)
    for group in GROUPS:
        for case in sorted((CORPUS / group).iterdir()):
            if not (case / "options.json").exists():
                continue
            run = subprocess.run(
                [str(args.cli), "diff", str(side(case, "left")), str(side(case, "right")),
                 "-f", "html", "--options", str(case / "options.json")],
                capture_output=True,
                timeout=120,
            )
            name = f"{group}/{case.name}"
            if run.returncode not in (0, 1, 2) or not run.stdout:
                reports.append((name, ""))
                print(f"FAIL {name}: exit {run.returncode}, {run.stderr.decode(errors='replace')}")
                continue
            reports.append((name, run.stdout.decode("utf-8")))

    failures = 0
    for name, text in reports:
        errors = check(text) if text else ["no output"]
        if errors:
            failures += 1
            print(f"FAIL {name}")
            for e in errors:
                print(f"  {e}")
    print(
        f"{len(reports)} reports parsed ({static} stored, {len(reports) - static} from the corpus), "
        f"{failures} with problems"
    )
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
