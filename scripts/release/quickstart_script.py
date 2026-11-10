#!/usr/bin/env python3
"""Writes the examples of a quickstart page as one POSIX shell script.

    python3 scripts/release/quickstart_script.py docs/quickstart/python.md \
        [--skip-run TEXT]... [--include-skip TEXT]... [--replace OLD=NEW]... \
        [--save-files DIR] [-o FILE]

The script does what docs/quickstart/check_examples.py does for the page,
with nothing but /bin/sh, cat, mkdir and diff, so it runs in a bare container
(scripts/release/install-check.sh): it saves every `file` block, runs every
`run` block with /bin/sh and checks its exit status, sources every `env`
block, and compares every `output` block with what the run before it printed
(trailing newlines aside). `build` and `skip` blocks are left out, and so are
`render` blocks (they need the React test harness) and their output.

--skip-run TEXT leaves out each `run` block that contains TEXT, with the
output block after it (for a step the container does another way, such as
installing a package from a release file instead of a folder of the
repository, or for a tool the container does not have). --include-skip TEXT runs each `skip`
block that contains TEXT as a `run` block (the page's install command for
a package file). --replace OLD=NEW replaces text in the `run` blocks before
they are written. --save-files DIR also writes the page's `file` blocks into
DIR (to compile an example before the container starts).

The script runs in the current folder, keeps its own files in .qs/, sets
NO_COLOR=1 and prints one line per block. Its exit status is 0 when every
block ran and printed what the page says, 1 at the first one that did not.

Python 3.11 or later, standard library only.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.dont_write_bytecode = True  # no __pycache__ next to the quickstarts
sys.path.insert(0, str(ROOT / "docs" / "quickstart"))

from check_examples import Block, CheckError, parse  # noqa: E402

PROLOGUE = """\
#!/bin/sh
# Generated from {doc} by scripts/release/quickstart_script.py.
set -u
export NO_COLOR=1
mkdir -p .qs
fail() {{ echo "FAILED: $1" >&2; exit 1; }}
"""


def heredoc(path: str, text: str, tag: str) -> str:
    if f"\n{tag}\n" in f"\n{text}":
        raise CheckError(f"the text of {path} contains the line {tag}")
    if not text.endswith("\n"):
        text += "\n"
    return f"cat > '{path}' <<'{tag}'\n{text}{tag}\n"


def render_script(
    doc: Path,
    blocks: list[Block],
    skip_runs: list[str],
    include_skips: list[str],
    replace: list[tuple[str, str]],
) -> str:
    rel = doc.relative_to(ROOT).as_posix() if doc.is_relative_to(ROOT) else doc.name
    out = [PROLOGUE.format(doc=rel)]
    last: str | None = None  # the output file of the last run, "" when it was skipped
    for n, block in enumerate(blocks):
        where = f"{doc.name}:{block.line}"
        tag = f"QS_EOF_{n}"
        if block.action == "skip" and any(text in block.text for text in include_skips):
            block = Block(block.doc, block.line, "run", [], block.lang, block.text)
        if block.action == "file":
            if len(block.args) != 1 or block.args[0].startswith(("/", "-")) or ".." in block.args[0].split("/"):
                raise CheckError(f"{block.where()}: `file` takes one relative name")
            name = block.args[0]
            out.append(f"\n# {where} file {name}\n")
            if "/" in name:
                out.append(f"mkdir -p '{name.rsplit('/', 1)[0]}'\n")
            out.append(heredoc(name, block.text, tag))
            out.append(f"echo 'ok   {where} file {name}'\n")
        elif block.action in ("run", "env"):
            want = 0
            for arg in block.args:
                if not arg.startswith("exit="):
                    raise CheckError(f"{block.where()}: unknown argument {arg!r}")
                want = int(arg.removeprefix("exit="))
            if block.action == "run" and any(text in block.text for text in skip_runs):
                out.append(f"\n# {where} run: left out\necho 'skip {where} run'\n")
                last = ""
                continue
            text = block.text
            for old, new in replace:
                text = text.replace(old, new)
            step = f".qs/step-{n}.sh"
            out.append(f"\n# {where} {block.action}\n")
            out.append(heredoc(step, text, tag))
            if block.action == "env":
                out.append(f". ./{step}\n")
                out.append(f"echo 'ok   {where} env'\n")
                last = None
                continue
            result = f".qs/out-{n}"
            out.append(f"sh {step} > {result} 2> .qs/err-{n}; rc=$?\n")
            out.append(
                f'[ "$rc" -eq {want} ] || {{ cat {result} .qs/err-{n} >&2; '
                f'fail "{where}: exit status $rc, expected {want}"; }}\n'
            )
            out.append(f"echo 'ok   {where} run'\n")
            last = result
        elif block.action == "output":
            if last is None:
                raise CheckError(f"{block.where()}: `output` without a `run` before it")
            if last == "":
                out.append(f"\n# {where} output of a run that was left out\necho 'skip {where} output'\n")
                last = None
                continue
            want_file = f".qs/want-{n}"
            out.append(f"\n# {where} output\n")
            out.append(heredoc(want_file, block.text, tag))
            out.append(f'printf \'%s\\n\' "$(cat {want_file})" > {want_file}.n\n')
            out.append(f'printf \'%s\\n\' "$(cat {last})" > {last}.n\n')
            out.append(f'diff -u {want_file}.n {last}.n >&2 || fail "{where}: output differs from the page"\n')
            out.append(f"echo 'ok   {where} output'\n")
            last = None
        elif block.action == "render":
            out.append(f"\n# {where} render: needs the React test harness\necho 'skip {where} render'\n")
            last = ""
        elif block.action in ("build", "skip"):
            continue
        else:
            raise CheckError(f"{block.where()}: unknown directive {block.action!r}")
    out.append('\necho "every example of the page ran"\n')
    return "".join(out)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("doc", type=Path)
    parser.add_argument("--skip-run", action="append", default=[], metavar="TEXT")
    parser.add_argument("--include-skip", action="append", default=[], metavar="TEXT")
    parser.add_argument("--replace", action="append", default=[], metavar="OLD=NEW")
    parser.add_argument("--save-files", type=Path, metavar="DIR")
    parser.add_argument("-o", "--output", type=Path)
    args = parser.parse_args()
    replace = []
    for pair in args.replace:
        old, sep, new = pair.partition("=")
        if not sep or not old:
            parser.error(f"--replace takes OLD=NEW, not {pair!r}")
        replace.append((old, new))
    doc = args.doc.resolve()
    try:
        blocks = parse(doc)
        script = render_script(doc, blocks, args.skip_run, args.include_skip, replace)
    except CheckError as err:
        print(f"quickstart_script.py: {err}", file=sys.stderr)
        return 1
    if args.save_files:
        for block in blocks:
            if block.action == "file":
                path = args.save_files / block.args[0]
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(block.text, encoding="utf-8")
    if args.output:
        args.output.write_text(script, encoding="utf-8")
    else:
        sys.stdout.write(script)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
