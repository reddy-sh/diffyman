"""Starts a real `diffyman-server serve` for tests, and talks HTTP to it.

The server runs as its own process, as a user would start it, with:

  - a dedicated database (default `diffyman_e2e`) on the PostgreSQL server
    the Rust tests use: `DIFFYMAN_TEST_DB_URL` (a URL to any database on it,
    usually `postgres`), or else `127.0.0.1:5433` with the password
    `DIFFYMAN_PG_PASSWORD` from the environment or the repository's
    `.env.local`. The database is dropped and created again for every run, and
    dropped at the end unless asked to keep it; the main `diffyman` database is
    never used. `psql` must be on PATH.
  - a fresh working folder holding the blob store (`file://`, unless another
    store URL is given), the temporary folder, the admin token file and the
    server log;
  - a random free port on 127.0.0.1, the rate limit off, and no inherited
    `DIFFYMAN_*` variable.

Secrets (the database password, the admin token, API keys) go only into child
process environments and pipes, never onto a command line or into output.

Run as a script, it starts a server, creates an API key and runs a command
with DIFFYMAN_SERVER_URL and DIFFYMAN_SERVER_KEY set, then stops the server:

  python tests/server/harness.py --server target/release/diffyman-server -- \\
      python tests/conformance/run_bindings.py --surfaces cli,server

A command named `python` runs with this interpreter. When no database is reachable it prints why and exits 0 without running the
command, unless DIFFYMAN_TEST_DB_REQUIRED=1, which makes that exit 1.
"""

from __future__ import annotations

import argparse
import http.client
import json
import os
import pathlib
import re
import select
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.parse
from dataclasses import dataclass, field
from typing import Any

REPO = pathlib.Path(__file__).resolve().parents[2]
WINDOWS = os.name == "nt"
KEY_ID = re.compile(r"created key ([0-9a-f-]{36})")


class Skip(Exception):
    """No database to run against."""


def db_required() -> bool:
    return os.environ.get("DIFFYMAN_TEST_DB_REQUIRED") == "1"


# --- PostgreSQL ----------------------------------------------------------------


def env_file_value(key: str) -> str | None:
    path = REPO / ".env.local"
    try:
        text = path.read_text(encoding="utf-8")
    except OSError:
        return None
    for line in text.splitlines():
        line = line.strip().removeprefix("export ")
        name, sep, value = line.partition("=")
        if sep and name.strip() == key:
            return value.strip().strip("'\"")
    return None


@dataclass(frozen=True)
class PgServer:
    host: str
    port: int
    user: str
    password: str = field(repr=False)

    @staticmethod
    def find() -> PgServer:
        url = os.environ.get("DIFFYMAN_TEST_DB_URL")
        if url:
            parts = urllib.parse.urlsplit(url)
            if parts.scheme not in ("postgres", "postgresql") or not parts.hostname:
                raise SystemExit("DIFFYMAN_TEST_DB_URL is not a postgres:// URL")
            return PgServer(
                parts.hostname,
                parts.port or 5432,
                urllib.parse.unquote(parts.username or "postgres"),
                urllib.parse.unquote(parts.password or ""),
            )
        password = os.environ.get("DIFFYMAN_PG_PASSWORD") or env_file_value("DIFFYMAN_PG_PASSWORD")
        if not password:
            raise Skip("no test database configured (set DIFFYMAN_TEST_DB_URL or run scripts/local-up.sh)")
        return PgServer("127.0.0.1", 5433, "diffyman", password)

    def url(self, database: str) -> str:
        user = urllib.parse.quote(self.user, safe="")
        password = urllib.parse.quote(self.password, safe="")
        host = f"[{self.host}]" if ":" in self.host else self.host
        return f"postgres://{user}:{password}@{host}:{self.port}/{database}"

    def psql(self, sql: str, database: str = "postgres") -> str:
        psql = shutil.which("psql")
        if psql is None:
            raise SystemExit("psql is not on PATH (it creates and drops the test database)")
        env = {
            **os.environ,
            "PGHOST": self.host,
            "PGPORT": str(self.port),
            "PGUSER": self.user,
            "PGPASSWORD": self.password,
            "PGDATABASE": database,
            "PGCONNECT_TIMEOUT": "3",
        }
        proc = subprocess.run(
            [psql, "-X", "-q", "-At", "-v", "ON_ERROR_STOP=1", "-f", "-"],
            input=sql,
            capture_output=True,
            text=True,
            env=env,
            check=False,
        )
        if proc.returncode != 0:
            raise RuntimeError(f"psql failed: {proc.stderr.strip()}")
        return proc.stdout.strip()

    def check(self) -> None:
        try:
            self.psql("SELECT 1")
        except RuntimeError as error:
            raise Skip(f"database not reachable on {self.host}:{self.port} ({error})") from None

    def recreate(self, database: str) -> None:
        check_name(database)
        self.psql(f"DROP DATABASE IF EXISTS {database} WITH (FORCE);")
        self.psql(f"CREATE DATABASE {database};")

    def drop(self, database: str) -> None:
        check_name(database)
        self.psql(f"DROP DATABASE IF EXISTS {database} WITH (FORCE);")

    def counts(self, database: str, tables: tuple[str, ...]) -> dict[str, int]:
        sql = " UNION ALL ".join(f"SELECT '{t}', count(*) FROM {t}" for t in tables)
        rows = self.psql(sql + ";", database)
        return {name: int(n) for name, n in (line.split("|") for line in rows.splitlines())}


def check_name(database: str) -> None:
    if not re.fullmatch(r"diffyman_[a-z0-9_]{1,40}", database) or database == "diffyman":
        raise SystemExit(f"refusing database name {database!r}: test databases are diffyman_<name>")


# --- HTTP ----------------------------------------------------------------------


@dataclass
class Reply:
    status: int
    headers: dict[str, str]
    body: bytes

    def json(self) -> Any:
        return json.loads(self.body)

    def code(self) -> str | None:
        """The error code of a typed error body, or None."""
        try:
            value = json.loads(self.body)
        except ValueError:
            return None
        error = value.get("error") if isinstance(value, dict) else None
        if isinstance(error, dict) and isinstance(error.get("code"), str) and isinstance(error.get("message"), str):
            return error["code"]
        return None


class Client:
    """One keep-alive connection to the server. With `log` (the server's
    log file), a request that fails on a connection error prints the log's
    tail."""

    def __init__(self, port: int, key: str | None = None, timeout: float = 120, log: pathlib.Path | None = None) -> None:
        self.port = port
        self.key = key
        self.timeout = timeout
        self.log = log
        self.conn: http.client.HTTPConnection | None = None

    def close(self) -> None:
        if self.conn is not None:
            self.conn.close()
            self.conn = None

    def dropped(self) -> bool:
        """Whether the server closed the kept-alive connection (it is
        readable while no request is outstanding: end of stream or a reset).
        A server may close one after answering a request without reading its
        body, without saying so in the answer."""
        sock = self.conn.sock if self.conn is not None else None
        if sock is None:
            return False
        try:
            readable, _, _ = select.select([sock], [], [], 0)
        except (OSError, ValueError):
            return True
        return bool(readable)

    def early_reply(self, method: str) -> Reply | None:
        """An answer the server sent before the request was sent in full
        (typically a refusal, after which it closed), if one is readable."""
        sock = self.conn.sock if self.conn is not None else None
        if sock is None:
            return None
        try:
            sock.settimeout(2)
            response = http.client.HTTPResponse(sock, method=method)
            response.begin()
            try:
                data = response.read()
            except (OSError, http.client.HTTPException):
                data = b""
        except (OSError, http.client.HTTPException):
            return None
        return Reply(response.status, {k.lower(): v for k, v in response.getheaders()}, data)

    def report(self, method: str, path: str, error: BaseException) -> None:
        """With a log: the failed request and the log's tail, on stderr."""
        if self.log is None:
            return
        try:
            lines = self.log.read_text(encoding="utf-8", errors="replace").splitlines()[-40:]
        except OSError as why:
            lines = [f"(cannot read {self.log.name}: {why})"]
        print(
            f"harness: {method} {path.split('?')[0]} failed with {type(error).__name__}: {error}; "
            f"the server sent no HTTP answer. The last {len(lines)} lines of its log:",
            file=sys.stderr,
        )
        print("\n".join(lines), file=sys.stderr, flush=True)

    def request(
        self,
        method: str,
        path: str,
        body: bytes | Any = None,
        headers: dict[str, str] | None = None,
        key: str | None | bool = True,
    ) -> Reply:
        """`key=True` uses the client's key, a string another key, None/False none.
        A body may be bytes or a binary file object (streamed; set
        Content-Length): a seekable one is sent again when a retry needs it."""
        all_headers = dict(headers or {})
        token = self.key if key is True else key or None
        if token:
            all_headers["Authorization"] = f"Bearer {token}"
        start = body.tell() if hasattr(body, "seekable") and body.seekable() else None
        retryable = body is None or isinstance(body, bytes) or start is not None
        for attempt in (0, 1):
            if self.conn is not None and self.dropped():
                self.close()
            if self.conn is None:
                self.conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=self.timeout)
            try:
                self.conn.request(method, path, body=body, headers=all_headers)
                response = self.conn.getresponse()
                data = response.read()
            except (ConnectionError, http.client.RemoteDisconnected, http.client.CannotSendRequest) as error:
                # The server may have answered before the body was sent in
                # full, then closed: that answer is the reply.
                early = self.early_reply(method)
                self.close()
                if early is not None:
                    return early
                # Otherwise a kept-alive connection the server closed: retry
                # once on a new one, unless the body was a stream that is now
                # consumed and cannot be rewound.
                if attempt == 1 or not retryable:
                    self.report(method, path, error)
                    raise
                if start is not None:
                    body.seek(start)
                continue
            reply = Reply(response.status, {k.lower(): v for k, v in response.getheaders()}, data)
            if reply.headers.get("connection", "").lower() == "close":
                self.close()
            return reply
        raise AssertionError("unreachable")

    def headers_only(self, method: str, path: str, headers: dict[str, str]) -> Reply:
        """Sends the request line and headers but no body (to test limits
        the server applies to the declared Content-Length)."""
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=self.timeout)
        try:
            conn.putrequest(method, path)
            for name, value in headers.items():
                conn.putheader(name, value)
            if self.key:
                conn.putheader("Authorization", f"Bearer {self.key}")
            conn.endheaders()
            response = conn.getresponse()
            return Reply(response.status, {k.lower(): v for k, v in response.getheaders()}, response.read())
        finally:
            conn.close()

    def post_json(self, path: str, value: Any, key: str | None | bool = True) -> Reply:
        return self.request(
            "POST", path, json.dumps(value).encode("utf-8"), {"Content-Type": "application/json"}, key
        )


BOUNDARY = "diffyman-e2e-7d1f0b9a3c"


def multipart(parts: list[tuple[str, str | None, bytes]]) -> tuple[bytes, str]:
    """A multipart/form-data body of (field, file name or None, bytes)."""
    out = bytearray()
    for name, filename, data in parts:
        out += f"--{BOUNDARY}\r\n".encode()
        if filename is None:
            out += f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode()
        else:
            out += (
                f'Content-Disposition: form-data; name="{name}"; filename="{filename}"\r\n'
                "Content-Type: application/octet-stream\r\n\r\n"
            ).encode()
        out += data + b"\r\n"
    out += f"--{BOUNDARY}--\r\n".encode()
    return bytes(out), f"multipart/form-data; boundary={BOUNDARY}"


# --- The server process --------------------------------------------------------


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def rss_bytes(pid: int) -> int | None:
    """Resident set size of a process now."""
    status = pathlib.Path(f"/proc/{pid}/status")
    if status.exists():
        for line in status.read_text().splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) * 1024
        return None
    proc = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True, check=False)
    text = proc.stdout.strip()
    return int(text) * 1024 if text.isdigit() else None


def default_server() -> pathlib.Path:
    exe = "diffyman-server.exe" if WINDOWS else "diffyman-server"
    found = [REPO / "target" / p / exe for p in ("release", "debug")]
    existing = [p for p in found if p.is_file()]
    if not existing:
        raise SystemExit("no diffyman-server binary: cargo build --release -p diffyman-server, or pass --server")
    return max(existing, key=lambda p: p.stat().st_mtime)


def default_workdir_root() -> pathlib.Path:
    data = pathlib.Path.home() / "diffyman-data" / "tmp"
    return data if data.is_dir() else pathlib.Path(tempfile.gettempdir())


class Server:
    """A `diffyman-server serve` process on its own database and folder."""

    def __init__(
        self,
        binary: pathlib.Path,
        workdir: pathlib.Path,
        pg: PgServer,
        database: str,
        store_url: str | None = None,
        extra_env: dict[str, str] | None = None,
    ) -> None:
        self.binary = binary.resolve()
        self.workdir = workdir
        self.pg = pg
        self.database = database
        self.store_dir = workdir / "blobs"
        self.store_url = store_url or self.store_dir.as_uri() + "/"
        self.log_path = workdir / "server.log"
        self.port = free_port()
        self.extra_env = extra_env or {}
        self.proc: subprocess.Popen[bytes] | None = None
        self.max_rss: int | None = None
        self.exit_code: int | None = None

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def env(self) -> dict[str, str]:
        env = {k: v for k, v in os.environ.items() if not k.startswith("DIFFYMAN_")}
        env.update(
            {
                "DIFFYMAN_ENV": "local",
                "DIFFYMAN_BIND": f"127.0.0.1:{self.port}",
                "DIFFYMAN_DB_URL": self.pg.url(self.database),
                "DIFFYMAN_STORE_URL": self.store_url,
                "DIFFYMAN_TMP_DIR": str(self.workdir / "tmp"),
                "DIFFYMAN_ADMIN_TOKEN_FILE": str(self.workdir / "admin" / "admin-token"),
                "DIFFYMAN_RATE_LIMIT_PER_MINUTE": "0",
                "DIFFYMAN_LOG": "info",
            }
        )
        env.update(self.extra_env)
        return env

    def start(self, timeout: float = 60) -> None:
        self.workdir.mkdir(parents=True, exist_ok=True)
        self.pg.recreate(self.database)
        log = self.log_path.open("ab")
        self.proc = subprocess.Popen(
            [str(self.binary), "serve"],
            cwd=self.workdir,
            env=self.env(),
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        log.close()
        deadline = time.monotonic() + timeout
        client = Client(self.port, timeout=2)
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise SystemExit(f"diffyman-server exited with {self.proc.returncode} at start:\n{self.log_tail()}")
            try:
                if client.request("GET", "/healthz").status == 200:
                    client.close()
                    return
            except OSError:
                client.close()
            time.sleep(0.1)
        self.stop()
        raise SystemExit(f"diffyman-server did not answer /healthz within {timeout:.0f} s:\n{self.log_tail()}")

    def log_text(self) -> str:
        try:
            return self.log_path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            return ""

    def log_tail(self, lines: int = 30) -> str:
        return "\n".join(self.log_text().splitlines()[-lines:])

    def keys(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(self.binary), "keys", *args],
            cwd=self.workdir,
            env=self.env(),
            capture_output=True,
            text=True,
            check=False,
        )

    def create_key(self, name: str, scopes: str | None = None) -> tuple[str, str]:
        """(key ID, secret). The secret comes over a pipe and is never printed."""
        args = ["create", "--name", name] + (["--scopes", scopes] if scopes else [])
        proc = self.keys(*args)
        found = KEY_ID.search(proc.stderr)
        secret = proc.stdout.strip()
        if proc.returncode != 0 or not found or not re.fullmatch(r"dmk_[0-9a-f]{64}", secret):
            raise SystemExit(f"keys create failed (exit {proc.returncode}): {proc.stderr.strip()}")
        return found.group(1), secret

    def describe_exit(self) -> str:
        """How the process stands: still running, exited with a code, or
        killed by a signal (a crash)."""
        code = self.proc.poll() if self.proc is not None else None
        if self.proc is None:
            return "was not started"
        if code is None:
            return "is still running"
        if code < 0:
            try:
                return f"was killed by {signal.Signals(-code).name}"
            except ValueError:
                return f"was killed by signal {-code}"
        return f"exited with {code}"

    def rss(self) -> int | None:
        return rss_bytes(self.proc.pid) if self.proc and self.proc.poll() is None else None

    def stop(self, timeout: float = 30) -> None:
        """SIGTERM (graceful shutdown), then the lifetime peak RSS."""
        proc = self.proc
        if proc is None or proc.returncode is not None:
            return
        if WINDOWS:
            proc.terminate()
            proc.wait(timeout)
            self.exit_code = proc.returncode
            return
        proc.send_signal(signal.SIGTERM)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            pid, status, usage = os.wait4(proc.pid, os.WNOHANG)
            if pid:
                proc.returncode = os.waitstatus_to_exitcode(status)
                self.exit_code = proc.returncode
                # ru_maxrss is in bytes on macOS and in KiB on Linux.
                self.max_rss = usage.ru_maxrss if sys.platform == "darwin" else usage.ru_maxrss * 1024
                return
            time.sleep(0.05)
        proc.kill()
        proc.wait()
        self.exit_code = proc.returncode

    def cleanup(self, keep: bool) -> None:
        if keep:
            return
        try:
            self.pg.drop(self.database)
        except RuntimeError as error:
            print(f"warning: could not drop {self.database}: {error}", file=sys.stderr)
        shutil.rmtree(self.workdir, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--server", type=pathlib.Path, help="the diffyman-server binary (default: newest build)")
    parser.add_argument("--database", default="diffyman_e2e_conformance")
    parser.add_argument("--store-url", help="blob store URL (default: file:// in the working folder)")
    parser.add_argument("--workdir-root", type=pathlib.Path, default=default_workdir_root())
    parser.add_argument("command", nargs=argparse.REMAINDER, help="-- COMMAND [ARGS...]")
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("give a command after --")
    if command[0] == "python":
        command = [sys.executable, *command[1:]]
    try:
        pg = PgServer.find()
        pg.check()
    except Skip as why:
        print(f"skipping: {why}", file=sys.stderr)
        return 1 if db_required() else 0
    workdir = pathlib.Path(tempfile.mkdtemp(prefix="server-", dir=args.workdir_root))
    server = Server(args.server or default_server(), workdir, pg, args.database, args.store_url)
    try:
        server.start()
        _, key = server.create_key("conformance")
        env = {**os.environ, "DIFFYMAN_SERVER_URL": server.base_url, "DIFFYMAN_SERVER_KEY": key}
        code = subprocess.run(command, env=env, check=False).returncode
        if code != 0:
            print(f"harness: the command exited with {code}; the server {server.describe_exit()}; last lines of its log:", file=sys.stderr)
            print(server.log_tail(40), file=sys.stderr, flush=True)
    finally:
        server.stop()
        server.cleanup(keep=False)
    return code


if __name__ == "__main__":
    sys.exit(main())
