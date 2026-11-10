#!/usr/bin/env python3
"""Build the documentation site (an mdBook) from the repository's Markdown.

The chapters are the repository's own files (README.md, CONTRIBUTING.md and
everything under docs/). Nothing is copied into the repository: the script
assembles the book in a build folder and checks the result.

1. Reads book/SUMMARY.md, the table of contents. Its links are paths from the
   repository root, and every Markdown file git knows under docs/ must be one.
2. Writes each chapter to <out>/src at the same path, so links between
   chapters stay as they are. A relative link to any other file of the
   repository becomes a link to that file on GitHub; a link to a file that
   does not exist (or that git does not know) is an error, and so is a
   fragment that names no heading of a Markdown file outside the book.
3. Writes the OpenAPI reference page from docs/api/openapi.json.
4. Runs `mdbook build` into <out>/html.
5. Checks the HTML: every local link and fragment resolves, and no page,
   style sheet or script loads anything from another origin.

Usage (from anywhere; Python 3.11+, standard library only):

    python3 book/build.py [--mdbook PATH] [--out DIR] [--no-build]

The default output folder is target/book; the site is target/book/html.
"""

from __future__ import annotations

import argparse
import json
import os
import posixpath
import re
import shutil
import subprocess
import sys
from collections.abc import Callable, Iterable
from dataclasses import dataclass, field
from html.parser import HTMLParser
from pathlib import Path
from typing import Any
from urllib.parse import quote, unquote, urlsplit

Json = dict[str, Any]

REPO = Path(__file__).resolve().parent.parent
REPO_URL = "https://github.com/reddy-sh/diffyman"
BRANCH = "main"
SITE_PATH = "/diffyman/"

SUMMARY = "book/SUMMARY.md"
BOOK_TOML = "book/book.toml"
# Files served with the book as they are (relative links to them stay local).
ASSETS = ("docs/api/openapi.json",)
# Every Markdown file git knows under these folders must be a chapter.
COVERED_DIRS = ("docs",)
OPENAPI_PAGE = "book/openapi.md"
OPENAPI_SOURCE = "docs/api/openapi.json"
OPENAPI_MARKER = "<!-- openapi-reference -->"

# Hosts a page, style sheet or script must never mention as a resource:
# analytics, tag managers, font services and script CDNs.
FORBIDDEN_HOSTS = (
    "google-analytics.com",
    "googletagmanager.com",
    "fonts.googleapis.com",
    "fonts.gstatic.com",
    "cdnjs.cloudflare.com",
    "cdn.jsdelivr.net",
    "unpkg.com",
    "plausible.io",
    "mathjax",
)


class BookError(Exception):
    """A problem in the sources or the built site."""


# ---------------------------------------------------------------------------
# Markdown links


FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
CODE_SPAN = re.compile(r"(`+)(?:.+?)(?<!`)\1(?!`)")
INLINE_LINK = re.compile(r'(\]\()(<[^<>\n]*>|[^()\s]+)((?:\s+"[^"\n]*")?\))')
REFERENCE_DEF = re.compile(r'^( {0,3}\[[^\]]+\]:[ \t]*)(<[^<>\n]*>|\S+)(.*)$')
HTML_LINK_ATTR = re.compile(r'(\b(?:href|src)=")([^"]*)(")')
SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")
HEADING = re.compile(r"^ {0,3}(#{1,6})[ \t]+(.*?)(?:[ \t]+#+)?[ \t]*$")


def map_outside_code(text: str, fn: Callable[[str, bool], str]) -> str:
    """Apply fn to every line of text outside fenced code blocks and code spans.

    fn gets the piece and whether it is the start of its line (for reference
    definitions); code is passed through unchanged.
    """
    out: list[str] = []
    fence: str | None = None
    for line in text.splitlines(keepends=True):
        match = FENCE.match(line)
        if fence is not None:
            if match and match.group(1)[0] == fence[0] and len(match.group(1)) >= len(fence):
                fence = None
            out.append(line)
            continue
        if match:
            fence = match.group(1)
            out.append(line)
            continue
        body = line.rstrip("\r\n")
        pos = 0
        pieces: list[str] = []
        for span in CODE_SPAN.finditer(body):
            pieces.append(fn(body[pos : span.start()], pos == 0))
            pieces.append(span.group(0))
            pos = span.end()
        pieces.append(fn(body[pos:], pos == 0))
        pieces.append(line[len(body) :])
        out.append("".join(pieces))
    return "".join(out)


def github_slug(text: str) -> str:
    """The anchor GitHub gives a heading."""
    text = re.sub(r"<[^>]+>", "", text)
    text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = text.replace("`", "").replace("*", "").strip().lower()
    text = re.sub(r"[^\w\- ]", "", text)
    return text.replace(" ", "-")


def heading_anchors(markdown: str) -> set[str]:
    """GitHub's anchors for the headings of a Markdown file (code skipped)."""
    anchors: set[str] = set()
    seen: dict[str, int] = {}
    fence: str | None = None
    for line in markdown.splitlines():
        match = FENCE.match(line)
        if fence is not None:
            if match and match.group(1)[0] == fence[0] and len(match.group(1)) >= len(fence):
                fence = None
            continue
        if match:
            fence = match.group(1)
            continue
        heading = HEADING.match(line)
        if not heading:
            continue
        slug = github_slug(heading.group(2))
        count = seen.get(slug, 0)
        seen[slug] = count + 1
        anchors.add(slug if count == 0 else f"{slug}-{count}")
    return anchors


@dataclass
class Repo:
    """What the link rewriter knows about the repository."""

    root: Path
    known: set[str] | None  # git's files (tracked and untracked, not ignored); None: no git
    chapters: set[str]
    assets: set[str] = field(default_factory=set)

    def is_file(self, rel: str) -> bool:
        if self.known is not None:
            return rel in self.known
        return (self.root / rel).is_file()

    def is_dir(self, rel: str) -> bool:
        if self.known is not None:
            prefix = rel.rstrip("/") + "/"
            return any(path.startswith(prefix) for path in self.known)
        return (self.root / rel).is_dir()


def resolve_target(source: str, target: str, repo: Repo) -> str:
    """The link to write for target, found in the chapter at source.

    Raises BookError for a link that leads nowhere.
    """
    angle = target.startswith("<") and target.endswith(">")
    raw = target[1:-1] if angle else target
    if not raw or raw.startswith("#") or SCHEME.match(raw) or raw.startswith("//"):
        return target
    if raw.startswith("/"):
        raise BookError(f"{source}: link {target!r} is an absolute path; use a relative one")
    path, _, fragment = raw.partition("#")
    if "?" in path:
        raise BookError(f"{source}: link {target!r} has a query string")
    base = posixpath.dirname(source)
    resolved = posixpath.normpath(posixpath.join(base, unquote(path)))
    if resolved == ".." or resolved.startswith("../"):
        raise BookError(f"{source}: link {target!r} leaves the repository")
    suffix = f"#{fragment}" if fragment else ""

    if resolved in repo.chapters or resolved in repo.assets:
        return target
    if repo.is_dir(resolved):
        index = posixpath.join(resolved, "README.md")
        if index in repo.chapters:
            return posixpath.relpath(index, base or ".") + suffix
        return f"{REPO_URL}/tree/{BRANCH}/{quote(resolved)}{suffix}"
    if repo.is_file(resolved):
        if fragment and resolved.endswith(".md"):
            anchors = heading_anchors((repo.root / resolved).read_text(encoding="utf-8"))
            if fragment not in anchors:
                raise BookError(f"{source}: link {target!r}: {resolved} has no heading #{fragment}")
        return f"{REPO_URL}/blob/{BRANCH}/{quote(resolved)}{suffix}"
    raise BookError(f"{source}: link {target!r}: {resolved} is not a file of the repository")


def rewrite_links(source: str, text: str, repo: Repo) -> str:
    """Rewrite every relative link of a chapter for the book (see resolve_target)."""

    def piece(part: str, line_start: bool) -> str:
        if line_start:
            definition = REFERENCE_DEF.match(part)
            if definition:
                link = resolve_target(source, definition.group(2), repo)
                return definition.group(1) + link + definition.group(3)
        part = INLINE_LINK.sub(
            lambda m: m.group(1) + resolve_target(source, m.group(2), repo) + m.group(3), part
        )
        return HTML_LINK_ATTR.sub(
            lambda m: m.group(1) + resolve_target(source, m.group(2), repo) + m.group(3), part
        )

    return map_outside_code(text, piece)


def summary_chapters(summary: str) -> list[str]:
    """The chapter paths of SUMMARY.md, in order."""
    chapters: list[str] = []
    for match in INLINE_LINK.finditer(summary):
        target = match.group(2)
        if target.startswith("<") and target.endswith(">"):
            target = target[1:-1]
        if not target.endswith(".md"):
            raise BookError(f"{SUMMARY}: chapter {target!r} is not a Markdown file")
        if target in chapters:
            raise BookError(f"{SUMMARY}: chapter {target!r} is listed twice")
        chapters.append(posixpath.normpath(target))
    if re.search(r"\]\(\s*\)", summary):
        raise BookError(f"{SUMMARY}: a chapter without a file (a draft) is not allowed")
    return chapters


# ---------------------------------------------------------------------------
# OpenAPI page


def md_text(text: str, *, table: bool = False) -> str:
    """Markdown for a description: one line, no raw HTML, pipes escaped in tables."""
    text = " ".join(str(text).split())
    pieces: list[str] = []
    pos = 0
    for span in CODE_SPAN.finditer(text):
        pieces.append(text[pos : span.start()].replace("&", "&amp;").replace("<", "&lt;"))
        pieces.append(span.group(0))
        pos = span.end()
    pieces.append(text[pos:].replace("&", "&amp;").replace("<", "&lt;"))
    out = "".join(pieces)
    return out.replace("|", "\\|") if table else out


def schema_anchor(name: str) -> str:
    return "schema-" + re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")


def ref_name(ref: str) -> str:
    prefix = "#/components/schemas/"
    if not ref.startswith(prefix):
        raise BookError(f"{OPENAPI_SOURCE}: unsupported reference {ref!r}")
    return ref[len(prefix) :]


def schema_type(schema: Json) -> str:
    """A short Markdown description of a schema's type."""
    if "$ref" in schema:
        name = ref_name(schema["$ref"])
        return f"[`{name}`](#{schema_anchor(name)})"
    for key in ("oneOf", "anyOf"):
        if key in schema:
            return " or ".join(schema_type(part) for part in schema[key])
    if "allOf" in schema:
        return " and ".join(schema_type(part) for part in schema["allOf"])
    if "enum" in schema:
        return "one of " + ", ".join(f"`{json.dumps(value)}`" for value in schema["enum"])
    if "const" in schema:
        return f"`{json.dumps(schema['const'])}`"
    kinds = schema.get("type", "any")
    kinds = kinds if isinstance(kinds, list) else [kinds]
    names = []
    for kind in kinds:
        if kind == "array":
            items = schema.get("items")
            names.append("array of " + schema_type(items) if items else "array")
        elif kind == "object" and isinstance(schema.get("additionalProperties"), dict):
            names.append("map of " + schema_type(schema["additionalProperties"]))
        else:
            names.append(str(kind))
    text = " or ".join(names)
    if "format" in schema:
        text += f" ({schema['format']})"
    return text


def property_description(schema: Json) -> str:
    if schema.get("description"):
        return str(schema["description"])
    for key in ("oneOf", "anyOf", "allOf"):
        for part in schema.get(key, []):
            if part.get("description"):
                return str(part["description"])
    return ""


def operation_anchor(method: str, path: str) -> str:
    return "op-" + re.sub(r"[^a-z0-9]+", "-", f"{method} {path}".lower()).strip("-")


def render_openapi(doc: Json) -> str:
    """The generated part of the OpenAPI page."""
    lines: list[str] = []
    info = doc.get("info", {})
    lines += [
        f"OpenAPI {doc.get('openapi', '?')}, API version `{info.get('version', '?')}`.",
        "",
        md_text(info.get("description", "")),
        "",
    ]
    operations: list[tuple[str, str, Json]] = []
    for path, methods in doc.get("paths", {}).items():
        for method, operation in methods.items():
            if method.lower() in {"get", "put", "post", "delete", "patch", "head", "options"}:
                operations.append((method.upper(), path, operation))

    lines += ["## Operations", "", "| Method | Path | Summary |", "|---|---|---|"]
    for method, path, operation in operations:
        summary = md_text(operation.get("summary", ""), table=True)
        lines.append(f"| {method} | [`{path}`](#{operation_anchor(method, path)}) | {summary} |")
    lines.append("")

    tags = doc.get("tags") or []
    tag_names = [tag["name"] for tag in tags]
    for method, path, operation in operations:
        for tag in operation.get("tags", []):
            if tag not in tag_names:
                tag_names.append(tag)
                tags.append({"name": tag})
    untagged = [op for op in operations if not op[2].get("tags")]
    groups = [
        (tag, [op for op in operations if tag["name"] in op[2].get("tags", [])]) for tag in tags
    ]
    if untagged:
        groups.append(({"name": "other"}, untagged))

    schemes = doc.get("components", {}).get("securitySchemes", {})
    for tag, members in groups:
        if not members:
            continue
        lines += [f"## {md_text(tag['name']).capitalize()}", ""]
        if tag.get("description"):
            lines += [md_text(tag["description"]) + ".", ""]
        for method, path, operation in members:
            lines += [
                f'<a id="{operation_anchor(method, path)}"></a>',
                "",
                f"### {method} `{path}`",
                "",
                md_text(operation.get("summary", "")),
                "",
            ]
            if operation.get("description"):
                lines += [md_text(operation["description"]), ""]
            security: list[Json] = operation.get("security", doc.get("security")) or []
            names = sorted({name for requirement in security for name in requirement})
            if names:
                described = []
                for name in names:
                    scheme = schemes.get(name, {})
                    text = f"`{name}`"
                    if scheme.get("scheme") == "bearer":
                        text += " (`Authorization: Bearer <API key>`"
                        if scheme.get("bearerFormat"):
                            text += f", {md_text(scheme['bearerFormat'])}"
                        text += ")"
                    described.append(text)
                lines += [f"- **Authentication:** {', '.join(described)}"]
            else:
                lines += ["- **Authentication:** none"]
            lines += [f"- **Operation ID:** `{operation.get('operationId', '')}`", ""]

            parameters = operation.get("parameters", [])
            if parameters:
                lines += [
                    "**Parameters**",
                    "",
                    "| Name | In | Type | Required | Description |",
                    "|---|---|---|---|---|",
                ]
                for parameter in parameters:
                    lines.append(
                        f"| `{parameter['name']}` | {parameter.get('in', '')} "
                        f"| {schema_type(parameter.get('schema', {}))} "
                        f"| {'yes' if parameter.get('required') else 'no'} "
                        f"| {md_text(parameter.get('description', ''), table=True)} |"
                    )
                lines.append("")

            body = operation.get("requestBody")
            if body:
                need = "required" if body.get("required") else "optional"
                lines += [f"**Request body** ({need})", ""]
                if body.get("description"):
                    lines += [md_text(body["description"]), ""]
                lines += ["| Media type | Schema |", "|---|---|"]
                for media, content in body.get("content", {}).items():
                    lines.append(f"| `{media}` | {schema_type(content.get('schema', {}))} |")
                lines.append("")

            lines += ["**Responses**", "", "| Status | Description | Body |", "|---|---|---|"]
            for status, response in operation.get("responses", {}).items():
                contents = response.get("content", {})
                body_text = "<br>".join(
                    f"`{media}`: {schema_type(content.get('schema', {}))}"
                    for media, content in contents.items()
                )
                description = md_text(response.get("description", ""), table=True)
                headers = response.get("headers", {})
                if headers:
                    description += " (headers: " + ", ".join(f"`{h}`" for h in headers) + ")"
                lines.append(f"| {status} | {description} | {body_text} |")
            lines.append("")

    schemas = doc.get("components", {}).get("schemas", {})
    if schemas:
        lines += ["## Schemas", ""]
    for name, schema in schemas.items():
        lines += [f'<a id="{schema_anchor(name)}"></a>', "", f"### `{name}`", ""]
        if schema.get("description"):
            lines += [md_text(schema["description"]), ""]
        properties = schema.get("properties")
        if properties:
            required = set(schema.get("required", []))
            lines += ["| Property | Type | Required | Description |", "|---|---|---|---|"]
            for prop, prop_schema in properties.items():
                lines.append(
                    f"| `{prop}` | {schema_type(prop_schema)} "
                    f"| {'yes' if prop in required else 'no'} "
                    f"| {md_text(property_description(prop_schema), table=True)} |"
                )
            lines.append("")
        else:
            lines += [f"Type: {schema_type(schema)}.", ""]
    return "\n".join(lines).rstrip() + "\n"


# ---------------------------------------------------------------------------
# HTML check


@dataclass
class Page:
    ids: set[str] = field(default_factory=set)
    links: list[tuple[str, str, str]] = field(default_factory=list)  # (tag, attribute, value)


RESOURCE_ATTRS = {
    ("a", "href"),
    ("area", "href"),
    ("link", "href"),
    ("script", "src"),
    ("img", "src"),
    ("iframe", "src"),
    ("source", "src"),
    ("video", "src"),
    ("audio", "src"),
    ("embed", "src"),
    ("object", "data"),
    ("form", "action"),
}


class PageParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.page = Page()

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        for name, value in attrs:
            if value is None:
                continue
            if name == "id" or (tag == "a" and name == "name"):
                self.page.ids.add(value)
            if (tag, name) in RESOURCE_ATTRS:
                self.page.links.append((tag, name, value))
            if name == "srcset":
                for candidate in value.split(","):
                    if candidate.strip():
                        self.page.links.append((tag, name, candidate.split()[0]))
            if name == "style" and re.search(r"url\(\s*['\"]?(?:https?:)?//", value):
                self.page.links.append((tag, "style", value))

    handle_startendtag = handle_starttag


def parse_page(path: Path) -> Page:
    parser = PageParser()
    parser.feed(path.read_text(encoding="utf-8"))
    parser.close()
    return parser.page


def is_external(value: str) -> bool:
    return value.startswith("//") or bool(re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:", value))


def check_site(site: Path) -> tuple[list[str], dict[str, int]]:
    """Problems in the built site, and counts for the report."""
    problems: list[str] = []
    pages = {p.relative_to(site).as_posix(): parse_page(p) for p in sorted(site.rglob("*.html"))}
    counts = {"pages": len(pages), "local links": 0, "external links": 0}

    for rel, page in pages.items():
        for tag, attr, value in page.links:
            if attr == "style":
                problems.append(f"{rel}: <{tag} style> loads from another origin: {value}")
                continue
            if is_external(value):
                scheme = value.split(":", 1)[0].lower()
                if tag in {"a", "area"} and scheme in {"http", "https", "mailto"}:
                    counts["external links"] += 1
                    continue
                problems.append(f"{rel}: <{tag} {attr}> points outside the site: {value}")
                continue
            parts = urlsplit(value)
            target_path = unquote(parts.path)
            if target_path.startswith("/"):
                if not target_path.startswith(SITE_PATH):
                    problems.append(f"{rel}: <{tag} {attr}> {value!r} is outside {SITE_PATH}")
                    continue
                target = target_path[len(SITE_PATH) :]
            elif target_path:
                target = posixpath.normpath(posixpath.join(posixpath.dirname(rel), target_path))
            else:
                target = rel
            if target == "." or target.endswith("/") or (site / target).is_dir():
                target = posixpath.join(target, "index.html") if target != "." else "index.html"
            if target.startswith("../") or not (site / target).is_file():
                problems.append(f"{rel}: <{tag} {attr}> {value!r}: {target} does not exist")
                continue
            counts["local links"] += 1
            fragment = unquote(parts.fragment)
            if fragment and target.endswith(".html"):
                if fragment not in pages[target].ids:
                    problems.append(f"{rel}: link {value!r}: {target} has no id {fragment!r}")

    # Style sheets and scripts (the search index holds page text, not code).
    for path in sorted(site.rglob("*")):
        if path.suffix not in {".css", ".js"} or path.name.startswith("searchindex"):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        rel = path.relative_to(site).as_posix()
        if path.suffix == ".css":
            for match in re.finditer(r"(?:url\(|@import)\s*['\"]?((?:https?:)?//[^'\")\s]+)", text):
                problems.append(f"{rel}: style sheet loads {match.group(1)}")
        lowered = text.lower()
        problems += [f"{rel}: mentions {host}" for host in FORBIDDEN_HOSTS if host in lowered]
    return problems, counts


# ---------------------------------------------------------------------------
# Build


def git_files(root: Path) -> set[str] | None:
    """Files git knows (tracked, and untracked ones not ignored); None without git."""
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            check=True,
            capture_output=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    files = {name for name in result.stdout.decode("utf-8").split("\0") if name}
    # A file deleted in the working tree is still listed by --cached.
    return {name for name in files if (root / name).is_file()}


def assemble(out_src: Path, root: Path = REPO) -> list[str]:
    """Write the book's sources to out_src. Returns the chapter paths."""
    summary = (root / SUMMARY).read_text(encoding="utf-8")
    chapters = summary_chapters(summary)
    known = git_files(root)
    if known is None:
        print(
            f"warning: git does not list the files of {root}; "
            "links are checked against the file system only",
            file=sys.stderr,
        )

    for chapter in chapters:
        if not (root / chapter).is_file():
            raise BookError(f"{SUMMARY}: chapter {chapter} does not exist")
        if known is not None and chapter not in known:
            raise BookError(f"{SUMMARY}: chapter {chapter} is not a file git knows")
    if known is not None:
        missing = sorted(
            path
            for path in known
            if path.endswith(".md") and path.split("/", 1)[0] in COVERED_DIRS
            and path not in chapters
        )
        if missing:
            raise BookError(f"{SUMMARY} does not list: " + ", ".join(missing))

    repo = Repo(root=root, known=known, chapters=set(chapters), assets=set(ASSETS))
    if out_src.exists():
        shutil.rmtree(out_src)
    out_src.mkdir(parents=True)

    for chapter in chapters:
        text = (root / chapter).read_text(encoding="utf-8")
        if chapter == OPENAPI_PAGE:
            if OPENAPI_MARKER not in text:
                raise BookError(f"{OPENAPI_PAGE}: the marker {OPENAPI_MARKER} is missing")
            doc = json.loads((root / OPENAPI_SOURCE).read_text(encoding="utf-8"))
            text = text.replace(OPENAPI_MARKER, render_openapi(doc).rstrip("\n"))
        text = rewrite_links(chapter, text, repo)
        dest = out_src / chapter
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(text, encoding="utf-8")
    for asset in ASSETS:
        dest = out_src / asset
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(root / asset, dest)
    shutil.copyfile(root / SUMMARY, out_src / "SUMMARY.md")
    shutil.copyfile(root / BOOK_TOML, out_src / "book.toml")
    return chapters


def tidy_site(site: Path) -> None:
    """Remove the copied book.toml; drop the `./` that src = "." puts in edit links."""
    (site / "book.toml").unlink(missing_ok=True)
    edit = f"{REPO_URL}/edit/{BRANCH}/"
    for page in site.rglob("*.html"):
        text = page.read_text(encoding="utf-8")
        tidied = text.replace(edit + "./", edit)
        if tidied != text:
            page.write_text(tidied, encoding="utf-8")


def site_size(site: Path) -> tuple[int, int]:
    files = [p for p in site.rglob("*") if p.is_file()]
    return len(files), sum(p.stat().st_size for p in files)


def report(lines: Iterable[str]) -> None:
    text = "\n".join(lines)
    print(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(text + "\n")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--mdbook", default="mdbook", help="the mdbook program (default: on PATH)")
    parser.add_argument("--out", type=Path, default=REPO / "target" / "book", help="output folder")
    parser.add_argument(
        "--no-build", action="store_true", help="only assemble and check the sources"
    )
    args = parser.parse_args(argv)
    out: Path = args.out.resolve()
    src, site = out / "src", out / "html"

    try:
        chapters = assemble(src)
        if args.no_build:
            print(f"{len(chapters)} chapters assembled in {src}")
            return 0
        if site.exists():
            shutil.rmtree(site)
        subprocess.run([args.mdbook, "build", str(src), "--dest-dir", str(site)], check=True)
        tidy_site(site)
        problems, counts = check_site(site)
    except BookError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    for problem in problems:
        print(f"error: {problem}", file=sys.stderr)
    files, size = site_size(site)
    report(
        [
            "### Documentation site",
            "",
            f"- {len(chapters)} chapters, {counts['pages']} HTML pages, {files} files, "
            f"{size / 1_000_000:.2f} MB in all",
            f"- {counts['local links']} local links checked, {counts['external links']} "
            "links to other sites (not fetched)",
            f"- {len(problems)} problems",
        ]
    )
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
