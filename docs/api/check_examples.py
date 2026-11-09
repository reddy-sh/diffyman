#!/usr/bin/env python3
"""Runs the curl examples of the REST API docs against a real test server.

It checks docs/api/README.md and docs/quickstart/rest.md with the directives
of docs/quickstart/check_examples.py (`run`, `env`, `output`, `file`, `skip`;
a block without one is an error). Before the first block it starts
`diffyman-server serve` with tests/server/harness.py:

  - on its own database (`diffyman_docs` by default; dropped and created
    again, and dropped at the end) on the PostgreSQL server the Rust tests
    use: `DIFFYMAN_TEST_DB_URL`, or else 127.0.0.1:5433 with
    `DIFFYMAN_PG_PASSWORD` from the environment or `.env.local`;
  - with a fresh blob store, temporary folder and log in a new working
    folder, and the admin token at `~/.diffyman/admin-token` of a home folder
    made for the run;
  - on a random free port of 127.0.0.1.

Every block runs with /bin/sh in an empty folder per page, with `API` set to
the server's base URL (the pages use http://127.0.0.1:8080), HOME set to that
home folder, the server's `DIFFYMAN_*` settings (so `diffyman-server keys`
reaches the same database), and target/release first on PATH. The API
reference also gets `KEY`, an API key with every scope; the quickstart makes
its own. At the end the server is stopped with SIGTERM, must exit with 0, and
its log must not hold any API key the run used.

Needs target/release/diffyman-server and target/release/diffyman
(`cargo build --release -p diffyman-server -p diffyman-cli`), curl, jq and
psql. Without a reachable database it prints why and exits 0, or 1 with
DIFFYMAN_TEST_DB_REQUIRED=1.

Usage: python3 docs/api/check_examples.py [--only api|rest]... [--keep]
       [--database NAME] [--workdir-root DIR]
"""

from __future__ import annotations

import argparse
import importlib.util
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from types import ModuleType
from typing import Any

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
PAGES = {"api": HERE / "README.md", "rest": REPO / "docs" / "quickstart" / "rest.md"}
SECRET = re.compile(r"dmk_[0-9a-f]{64}")


def load(name: str, path: Path) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


examples = load("quickstart_check_examples", REPO / "docs" / "quickstart" / "check_examples.py")
harness = load("server_harness", REPO / "tests" / "server" / "harness.py")


def missing_prerequisites() -> list[str]:
    missing = [f"`{tool}` on PATH" for tool in ("curl", "jq", "psql") if shutil.which(tool) is None]
    for name in ("diffyman-server", "diffyman"):
        exe = examples.RELEASE / (name + (".exe" if os.name == "nt" else ""))
        if not exe.is_file():
            missing.append(str(exe.relative_to(REPO) if exe.is_relative_to(REPO) else exe))
    return missing


def page_env(server: Any, home: Path, key: str | None) -> dict[str, str]:
    """The environment of a page's blocks (`server` is a `harness.Server`)."""
    env = {k: v for k, v in examples.base_env().items() if not k.startswith("DIFFYMAN_")}
    env.update({k: v for k, v in server.env().items() if k.startswith("DIFFYMAN_")})
    env["API"] = server.base_url
    env["HOME"] = str(home)
    env.pop("KEY", None)
    if key is not None:
        env["KEY"] = key
    return env


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--only", action="append", default=[], choices=sorted(PAGES))
    parser.add_argument("--keep", action="store_true", help="keep the database and the folders")
    parser.add_argument("--database", default="diffyman_docs")
    parser.add_argument("--workdir-root", type=Path, default=harness.default_workdir_root())
    args = parser.parse_args()
    pages = [PAGES[name] for name in (args.only or PAGES)]

    try:
        pg = harness.PgServer.find()
        pg.check()
    except harness.Skip as why:
        print(f"skipping: {why}", file=sys.stderr)
        return 1 if harness.db_required() else 0
    missing = missing_prerequisites()
    if missing:
        print("missing: " + ", ".join(missing), file=sys.stderr)
        return 1

    workdir = Path(tempfile.mkdtemp(prefix="docs-", dir=args.workdir_root))
    home = workdir / "home"
    token = home / ".diffyman" / "admin-token"
    server = harness.Server(
        examples.RELEASE / "diffyman-server",
        workdir / "server",
        pg,
        args.database,
        extra_env={"DIFFYMAN_ADMIN_TOKEN_FILE": str(token)},
    )
    failures = 0
    try:
        server.start()
        _, key = server.create_key("docs")
        for page in pages:
            env = page_env(server, home, key if page == PAGES["api"] else None)
            try:
                checked, skipped = examples.check_doc(page, workdir / "pages", False, env)
            except (examples.CheckError, subprocess.TimeoutExpired) as err:
                failures += 1
                print(f"FAIL {page.relative_to(REPO)}: {err}")
            else:
                note = f", {skipped} reference blocks not run" if skipped else ""
                print(f"ok   {page.relative_to(REPO)}: {checked} blocks{note}")
    finally:
        server.stop()
        log = server.log_text()
        if server.exit_code != 0:
            failures += 1
            print(f"FAIL the server exited with {server.exit_code}:\n{server.log_tail()}")
        if SECRET.search(log):
            failures += 1
            print("FAIL the server log holds an API key")
        if args.keep:
            print(f"kept {workdir} and the database {args.database}")
        else:
            server.cleanup(keep=False)
            shutil.rmtree(workdir, ignore_errors=True)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
