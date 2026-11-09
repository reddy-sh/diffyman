#!/usr/bin/env python3
"""Runs the code examples of the quickstarts in docs/quickstart against local builds.

Every fenced code block in docs/quickstart/*.md has a directive in an HTML
comment on the line before it (GitHub does not show it):

    <!-- check: file NAME -->     save the block as NAME in the example folder
    <!-- check: run [exit=N] -->  run the block with /bin/sh in the example folder;
                                  it must exit with N (default 0)
    <!-- check: env -->           run the block and keep the variables it exports
                                  for the blocks after it
    <!-- check: output -->        what the previous `run` or `render` block printed
                                  on standard output, exactly (trailing newlines aside)
    <!-- check: render -->        a JSX element: rendered with React in happy-dom,
                                  inside a DiffProvider whose client runs the real
                                  WASM engine; its text content is the output
    <!-- check: build -->         a build-from-source step; run from the repository
                                  root with --build only
    <!-- check: skip REASON -->   not run (a fragment shown for reference)

A block without a directive is an error, so a new example cannot go unchecked.
The REST quickstart (rest.md) needs a running server and database, so
docs/api/check_examples.py checks it, with the same directives.

Each quickstart runs in its own empty folder. DIFFYMAN_REPO is the repository,
NO_COLOR is set, and PATH starts with target/release (the CLI), the Python
binding's virtual environment and the folder of `node`. Without --build the
bindings must already be built:

    cli            target/release/diffyman, jq
    python         bindings/python/.venv with the binding installed (maturin develop)
    node           bindings/node/diffyman.<platform>.node (npm run build), npm
    java           bindings/java/target/diffyman-0.0.1.jar and Jackson in ~/.m2
                   (mvn verify), java
    c              target/release/libdiffyman_ffi (cargo build --release -p diffyman-ffi), cc, jq
    browser-react  bindings/wasm/pkg-web (wasm-pack), bindings/wasm/react (npm ci and
                   npm run build), examples/amplify-react/node_modules (npm ci)

The browser example does not install anything from npm: its folder gets
node_modules links to those packages, as `npm install` of `file:` paths would.

Usage: python3 docs/quickstart/check_examples.py [--only NAME]... [--build] [--keep]
       [--workdir DIR]
Standard library only; POSIX shells (Linux, macOS). Exit status 0 when every example ran and printed what
its page says.
"""

from __future__ import annotations

import argparse
import dataclasses
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", REPO / "target"))
RELEASE = TARGET / "release"
PY_VENV = REPO / "bindings" / "python" / ".venv"
REACT = REPO / "bindings" / "wasm" / "react"
PKG_WEB = REPO / "bindings" / "wasm" / "pkg-web"
AMPLIFY = REPO / "examples" / "amplify-react"
TIMEOUT = 600

DIRECTIVE = re.compile(r"^<!--\s*check:\s*(?P<body>.*?)\s*-->\s*$")
FENCE = re.compile(r"^(?P<fence>```+|~~~+)\s*(?P<lang>[^\s`]*)")


@dataclasses.dataclass
class Block:
    doc: Path
    line: int
    action: str
    args: list[str]
    lang: str
    text: str

    def where(self) -> str:
        return f"{self.doc.relative_to(REPO)}:{self.line}"


class CheckError(Exception):
    pass


def parse(doc: Path) -> list[Block]:
    """The fenced blocks of `doc`, each with the directive just before it."""
    lines = doc.read_text(encoding="utf-8").splitlines()
    blocks: list[Block] = []
    pending: tuple[str, list[str]] | None = None
    i = 0
    while i < len(lines):
        line = lines[i]
        directive = DIRECTIVE.match(line.strip())
        fence = FENCE.match(line)
        if directive:
            words = directive.group("body").split()
            if not words:
                raise CheckError(f"{doc.relative_to(REPO)}:{i + 1}: empty check directive")
            pending = (words[0], words[1:])
        elif fence:
            marker = fence.group("fence")
            start = i
            body: list[str] = []
            i += 1
            while i < len(lines) and not lines[i].startswith(marker):
                body.append(lines[i])
                i += 1
            if i == len(lines):
                raise CheckError(f"{doc.relative_to(REPO)}:{start + 1}: unclosed code block")
            if pending is None:
                raise CheckError(
                    f"{doc.relative_to(REPO)}:{start + 1}: code block without a check directive"
                )
            action, args = pending
            text = "\n".join(body) + "\n"
            blocks.append(Block(doc, start + 1, action, args, fence.group("lang"), text))
            pending = None
        elif line.strip():
            pending = None
        i += 1
    return blocks


def node_platform() -> str:
    system = {"Darwin": "darwin", "Linux": "linux", "Windows": "win32"}[platform.system()]
    machine = platform.machine().lower()
    arch = {"arm64": "arm64", "aarch64": "arm64", "x86_64": "x64", "amd64": "x64"}[machine]
    if system == "linux":
        return f"linux-{arch}-gnu"
    if system == "win32":
        return f"win32-{arch}-msvc"
    return f"{system}-{arch}"


def shared_library(name: str) -> Path:
    if sys.platform == "darwin":
        return RELEASE / f"lib{name}.dylib"
    if sys.platform == "win32":
        return RELEASE / f"{name}.dll"
    return RELEASE / f"lib{name}.so"


def missing_prerequisites(name: str, build: bool) -> list[str]:
    """What the examples of quickstart `name` need and is not there."""
    tools = {
        "cli": ["jq"],
        "python": [],
        "node": ["node", "npm"],
        "java": ["java"],
        "c": ["cc", "jq"],
        "browser-react": ["node"],
    }.get(name, [])
    missing = [f"`{tool}` on PATH" for tool in tools if shutil.which(tool) is None]
    if build:
        return missing
    files = {
        "cli": [RELEASE / "diffyman"],
        "python": [PY_VENV / "bin" / "python"],
        "node": [REPO / "bindings" / "node" / f"diffyman.{node_platform()}.node"],
        "java": [REPO / "bindings" / "java" / "target" / "diffyman-0.0.1.jar"],
        "c": [shared_library("diffyman_ffi")],
        "browser-react": [
            PKG_WEB / "diffyman_wasm_bg.wasm",
            REACT / "dist" / "index.js",
            REACT / "node_modules" / "vitest",
            AMPLIFY / "node_modules" / "@vitejs" / "plugin-react",
        ],
    }.get(name, [])
    missing += [str(path.relative_to(REPO)) for path in files if not path.exists()]
    if name == "python" and not missing:
        probe = subprocess.run(
            [str(PY_VENV / "bin" / "python"), "-c", "import diffyman"],
            capture_output=True,
            check=False,
        )
        if probe.returncode != 0:
            missing.append("the diffyman package in bindings/python/.venv")
    return missing


def base_env() -> dict[str, str]:
    env = dict(os.environ)
    path = [str(RELEASE), str(PY_VENV / "bin")]
    node = shutil.which("node")
    if node:
        path.append(str(Path(node).parent))
    env["PATH"] = os.pathsep.join([*path, env.get("PATH", "")])
    env["DIFFYMAN_REPO"] = str(REPO)
    env["NO_COLOR"] = "1"
    env.pop("VIRTUAL_ENV", None)
    return env


def link(dest: Path, source: Path) -> None:
    if not source.exists():
        raise CheckError(f"missing {source.relative_to(REPO)}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.symlink_to(source, target_is_directory=source.is_dir())


def prepare_browser(workdir: Path) -> None:
    """node_modules links in place of `npm install` for the browser example."""
    modules = workdir / "node_modules"
    for name in [
        "react",
        "react-dom",
        "vitest",
        "happy-dom",
        "@types/react",
        "@types/react-dom",
        "@types/node",
    ]:
        link(modules / name, REACT / "node_modules" / name)
    link(modules / "@diffyman" / "react", REACT)
    link(modules / "diffyman-wasm", PKG_WEB)
    for name in ["vite", "@vitejs/plugin-react"]:
        link(modules / name, AMPLIFY / "node_modules" / name)
    link(modules / ".bin" / "vite", AMPLIFY / "node_modules" / "vite" / "bin" / "vite.js")


RENDER_TEST = """\
// @vitest-environment happy-dom
// Written by docs/quickstart/check_examples.py.
import {{ act }} from "react";
import {{ createRoot }} from "react-dom/client";
import {{ writeFileSync }} from "node:fs";
import {{ it }} from "vitest";
import {{ DiffClient, DiffProvider }} from "@diffyman/react";
import {{ realEngine, wasmModule }} from {engine};
import {{ workerFactory }} from {worker};
{imports}

(globalThis as {{ IS_REACT_ACT_ENVIRONMENT?: boolean }}).IS_REACT_ACT_ENVIRONMENT = true;

it("renders the example", async () => {{
  const client = new DiffClient({{ createWorker: workerFactory(realEngine).createWorker, wasm: wasmModule }});
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => root.render(<DiffProvider client={{client}}>{{(
{snippet}
  )}}</DiffProvider>));
  // The first render is the loading state; wait for the result to replace it.
  const first = container.textContent;
  for (let i = 0; i < 2000 && container.textContent === first; i++) {{
    await act(async () => {{
      await new Promise((resolve) => setTimeout(resolve, 5));
    }});
  }}
  writeFileSync({out}, container.textContent ?? "");
  act(() => root.unmount());
  client.dispose();
}});
"""

RENDER_CONFIG = """\
// Written by docs/quickstart/check_examples.py.
export default {
  oxc: { jsx: { runtime: "automatic" } },
  resolve: { dedupe: ["react", "react-dom"] },
  test: { include: ["_check/*.test.tsx"], testTimeout: 60000 },
};
"""


def render(block: Block, workdir: Path, env: dict[str, str], written: list[Path]) -> str:
    """Renders the JSX of `block` and returns its text content.

    The element can use every named export of the .ts and .tsx files the page
    saved before it.
    """
    imports = []
    for source in written:
        if source.suffix not in (".ts", ".tsx"):
            continue
        names = re.findall(r"^export (?:function|const) (\w+)", source.read_text(), re.M)
        if names:
            rel = os.path.relpath(source, workdir / "_check")
            imports.append(f"import {{ {', '.join(names)} }} from {json_str(rel)};")
    check = workdir / "_check"
    check.mkdir(exist_ok=True)
    out = check / "rendered.txt"
    out.unlink(missing_ok=True)
    support = REACT / "test" / "support"
    (check / "render.test.tsx").write_text(
        RENDER_TEST.format(
            engine=json_str(str(support / "engine.ts")),
            worker=json_str(str(support / "in-process-worker.ts")),
            imports="\n".join(imports),
            snippet=block.text.rstrip(),
            out=json_str(str(out)),
        )
    )
    (check / "vitest.config.mjs").write_text(RENDER_CONFIG)
    vitest = REACT / "node_modules" / "vitest" / "vitest.mjs"
    result = subprocess.run(
        ["node", str(vitest), "run", "--config", str(check / "vitest.config.mjs")],
        cwd=workdir,
        env=env,
        capture_output=True,
        text=True,
        timeout=TIMEOUT,
        check=False,
    )
    if result.returncode != 0 or not out.exists():
        raise CheckError(f"{block.where()}: render failed\n{result.stdout}{result.stderr}")
    return out.read_text()


def typecheck(workdir: Path, env: dict[str, str], written: list[Path]) -> None:
    """Type-checks the .ts and .tsx files a page saved, strictly, as a Vite app would."""
    sources = [path for path in written if path.suffix in (".ts", ".tsx")]
    if not sources:
        return
    check = workdir / "_check"
    check.mkdir(exist_ok=True)
    files = ", ".join(json_str(os.path.relpath(path, check)) for path in sources)
    (check / "tsconfig.json").write_text(
        "{\n"
        '  "compilerOptions": {\n'
        '    "target": "ES2022", "module": "ESNext", "moduleResolution": "Bundler",\n'
        '    "lib": ["ES2022", "DOM", "DOM.Iterable"], "jsx": "react-jsx", "strict": true,\n'
        '    "noUncheckedIndexedAccess": true, "verbatimModuleSyntax": true,\n'
        '    "isolatedModules": true, "skipLibCheck": true, "noEmit": true,\n'
        '    "types": ["vite/client"]\n'
        "  },\n"
        f'  "files": [{files}]\n'
        "}\n"
    )
    tsc = REACT / "node_modules" / "typescript" / "bin" / "tsc"
    result = subprocess.run(
        ["node", str(tsc), "-p", str(check / "tsconfig.json")],
        cwd=workdir,
        env=env,
        capture_output=True,
        text=True,
        timeout=TIMEOUT,
        check=False,
    )
    if result.returncode != 0:
        raise CheckError(f"type check failed\n{result.stdout}{result.stderr}")


def json_str(text: str) -> str:
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


def run_shell(block: Block, cwd: Path, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["/bin/sh", "-c", block.text],
        cwd=cwd,
        env=env,
        capture_output=True,
        text=True,
        timeout=TIMEOUT,
        check=False,
    )


def check_doc(
    doc: Path, root: Path, build: bool, env: dict[str, str] | None = None
) -> tuple[int, int]:
    """Runs one quickstart; returns (blocks checked, blocks skipped).

    `env` replaces the environment of its blocks (default: `base_env()`).
    """
    blocks = parse(doc)
    name = doc.stem
    missing = missing_prerequisites(name, build)
    if missing:
        raise CheckError(f"{doc.relative_to(REPO)}: missing " + ", ".join(missing))
    workdir = root / name
    workdir.mkdir(parents=True)
    env = dict(env) if env is not None else base_env()
    if name == "browser-react":
        prepare_browser(workdir)
    last: str | None = None
    written: list[Path] = []
    checked = skipped = 0
    for block in blocks:
        if block.action == "file":
            if len(block.args) != 1:
                raise CheckError(f"{block.where()}: `file` takes one name")
            path = workdir / block.args[0]
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(block.text)
            written.append(path)
            checked += 1
        elif block.action in ("run", "env"):
            want = 0
            for arg in block.args:
                if arg.startswith("exit="):
                    want = int(arg.removeprefix("exit="))
                else:
                    raise CheckError(f"{block.where()}: unknown argument {arg!r}")
            if block.action == "env":
                env_file = workdir / "_check.env"
                script = block.text + f'\n/usr/bin/env -0 > "{env_file}"\n'
                result = run_shell(dataclasses.replace(block, text=script), workdir, env)
            else:
                result = run_shell(block, workdir, env)
            if result.returncode != want:
                raise CheckError(
                    f"{block.where()}: exit status {result.returncode}, expected {want}\n"
                    f"--- stdout\n{result.stdout}--- stderr\n{result.stderr}"
                )
            if block.action == "env":
                pairs = env_file.read_bytes().decode().split("\0")
                env = dict(pair.split("=", 1) for pair in pairs if "=" in pair)
                env_file.unlink()
            last = result.stdout
            checked += 1
        elif block.action == "render":
            last = render(block, workdir, env, written)
            checked += 1
        elif block.action == "output":
            if last is None:
                raise CheckError(f"{block.where()}: `output` without a `run` or `render` before it")
            if last.rstrip("\n") != block.text.rstrip("\n"):
                raise CheckError(
                    f"{block.where()}: output differs\n--- page\n{block.text}--- actual\n{last}"
                )
            last = None
            checked += 1
        elif block.action == "build":
            if build:
                result = run_shell(block, REPO, env)
                if result.returncode != 0:
                    raise CheckError(
                        f"{block.where()}: build step failed\n{result.stdout}{result.stderr}"
                    )
                checked += 1
            else:
                skipped += 1
        elif block.action == "skip":
            skipped += 1
        else:
            raise CheckError(f"{block.where()}: unknown directive {block.action!r}")
    if name == "browser-react":
        typecheck(workdir, env, written)
    return checked, skipped


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--only", action="append", default=[], help="quickstart name, such as python"
    )
    parser.add_argument("--build", action="store_true", help="also run the build-from-source steps")
    parser.add_argument("--keep", action="store_true", help="keep the example folders")
    parser.add_argument("--workdir", type=Path, help="where to make the example folders")
    args = parser.parse_args()

    docs = sorted(HERE.glob("*.md"))
    # rest.md needs a running server: docs/api/check_examples.py checks it.
    docs = [doc for doc in docs if doc.stem not in ("README", "rest")]
    if args.only:
        if "rest" in args.only:
            parser.error("rest.md needs a server: run docs/api/check_examples.py --only rest")
        unknown = set(args.only) - {doc.stem for doc in docs}
        if unknown:
            parser.error(f"no quickstart named {', '.join(sorted(unknown))}")
        docs = [doc for doc in docs if doc.stem in args.only]

    if args.workdir:
        args.workdir.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="diffyman-quickstart-", dir=args.workdir))
    failures = 0
    try:
        for doc in docs:
            try:
                checked, skipped = check_doc(doc, root, args.build)
            except (CheckError, subprocess.TimeoutExpired) as err:
                failures += 1
                print(f"FAIL {doc.name}: {err}")
            else:
                note = f", {skipped} build or reference blocks not run" if skipped else ""
                print(f"ok   {doc.name}: {checked} blocks{note}")
    finally:
        if args.keep:
            print(f"example folders kept in {root}")
        else:
            shutil.rmtree(root, ignore_errors=True)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
