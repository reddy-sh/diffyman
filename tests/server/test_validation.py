"""Acceptance tests for the request-validation rules of docs/api/README.md.

Runs against a real `diffyman-server serve` started by harness.py (its own
database, `diffyman_e2e_validation` by default, a fresh blob folder and a
random port on 127.0.0.1) and checks, over HTTP:

  auth       a missing, malformed, unknown or revoked key gets 401
             `unauthorized`, a key without the route's scope 403 `forbidden`,
             and another key's jobs, diffs and blobs answer exactly like IDs
             that do not exist;
  host       a `Host` or `Origin` that is not allow-listed gets 403, for JSON,
             text/plain and multipart bodies and for preflight requests, and
             requests without a key are refused before any work;
  ids        malformed job, diff and blob IDs get their documented 400 or 404;
  strings    quotes, backslashes, SQL keywords and control characters in
             names, options and query parameters are kept as plain data or
             refused with a typed 400, and refused requests change nothing;
  secrets    the admin token file has mode 0600, and no issued key or admin
             token is in an error body, /openapi.json or the server log.

No reply may be a 500, and every error is the documented typed body with the
status its code has in the reference. Without a database it prints why and
exits 0 (1 with DIFFYMAN_TEST_DB_REQUIRED=1). From the repository root:

  uv run --no-project --python 3.12 python tests/server/test_validation.py \\
      --server target/release/diffyman-server
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import pathlib
import re
import secrets
import socket
import stat
import sys
import tempfile
import time
import traceback
import uuid
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import harness as hx  # noqa: E402

API_DOC = hx.REPO / "docs" / "api" / "README.md"
TABLES = ("api_keys", "blobs", "blob_grants", "jobs", "diffs")
MARK = f"vmark{secrets.token_hex(6)}"  # in every hostile string; never in an error or the log
INJECTIONS = (
    f"it's \"{MARK}\".txt",
    f"'; DROP TABLE jobs; -- {MARK}.txt",
    f"Robert'); DELETE FROM diffs WHERE ('1'='1 {MARK}",
    f"{MARK} UNION SELECT hash FROM api_keys --",
    f"back\\slash\\{MARK}.txt",
    f"ctrl\x01\x07\x1b[31m\x7f{MARK}.txt",
    f"tab\tnew\nline\r{MARK}.txt",
    f"‮{MARK}txt.exe",
    f"$(touch {MARK}) `id` <script>{{{{7*7}}}}</script>",
    f"%00%2e%2e%2f{MARK}",
)


def documented_codes() -> dict[str, int]:
    """Error code -> HTTP status, from the reference's Errors table."""
    text = API_DOC.read_text(encoding="utf-8")
    return {code: int(status) for status, code in re.findall(r"^\| (\d{3}) \| `([a-z-]+)` \|", text, re.M)}


def final_component(name: str) -> str | None:
    last = re.split(r"[/\\]", name)[-1]
    return last or None


# --- Context ---------------------------------------------------------------------


@dataclass
class Ctx:
    server: hx.Server
    pg: hx.PgServer
    keys: dict[str, str] = field(default_factory=dict)
    key_ids: dict[str, str] = field(default_factory=dict)
    replies: list[tuple[str, str, hx.Reply]] = field(default_factory=list)
    failures: list[str] = field(default_factory=list)
    clients: dict[str, hx.Client] = field(default_factory=dict)

    def client(self, name: str = "main") -> hx.Client:
        if name not in self.clients:
            self.clients[name] = Recording(self, self.server.port, self.keys.get(name))
        return self.clients[name]

    def ok(self, condition: bool, what: str) -> bool:
        if not condition:
            self.failures.append(what)
        return condition

    def expect(self, what: str, reply: hx.Reply, status: int, *codes: str) -> bool:
        return self.ok(
            reply.status == status and reply.code() in codes,
            f"{what}: expected {status} {'/'.join(codes)}, got {reply.status} {reply.code() or reply.body[:160]!r}",
        )

    def rows(self) -> dict[str, int]:
        counts = self.pg.counts(self.server.database, TABLES)
        store = self.server.store_dir
        counts["objects"] = sum(1 for p in store.rglob("*") if p.is_file()) if store.exists() else 0
        return counts

    def unchanged(self, what: str, before: dict[str, int]) -> None:
        after = self.rows()
        self.ok(after == before, f"{what}: the database or the store changed: {before} -> {after}")


class Recording(hx.Client):
    """A client that keeps every reply, for the checks run after the suite."""

    def __init__(self, ctx: Ctx, port: int, key: str | None) -> None:
        super().__init__(port, key, timeout=60)
        self.ctx = ctx

    def request(self, method: str, path: str, body: Any = None, headers: dict[str, str] | None = None, key: Any = True) -> hx.Reply:
        reply = super().request(method, path, body, headers, key)
        self.ctx.replies.append((method, path[:120], reply))
        return reply

    def headers_only(self, method: str, path: str, headers: dict[str, str]) -> hx.Reply:
        reply = super().headers_only(method, path, headers)
        self.ctx.replies.append((method, path[:120], reply))
        return reply


def raw_request(port: int, data: bytes, timeout: float = 5) -> tuple[int | None, bytes]:
    """Sends bytes as they are (for what http.client refuses to send); the
    status of the answer, or None when the server closed without one."""
    chunks = []
    with socket.create_connection(("127.0.0.1", port), timeout=timeout) as sock:
        sock.sendall(data)
        try:
            while chunk := sock.recv(65536):
                chunks.append(chunk)
        except (TimeoutError, ConnectionResetError):
            pass
    reply = b"".join(chunks)
    found = re.match(rb"HTTP/1\.[01] (\d{3})", reply)
    return (int(found.group(1)) if found else None), reply


# --- Helpers ---------------------------------------------------------------------

JSON = {"Content-Type": "application/json"}
SMALL = {"left": {"name": "a.txt", "text": "a\n"}, "right": {"name": "b.txt", "text": "b\n"}}


def body(value: Any) -> bytes:
    return json.dumps(value).encode()


def upload(client: hx.Client, data: bytes, content_type: str = "application/octet-stream") -> str:
    reply = client.request("POST", "/v1/blobs", data, {"Content-Type": content_type})
    assert reply.status in (200, 201), f"upload: {reply.status} {reply.body[:200]!r}"
    return reply.json()["blobId"]


def wait_job(client: hx.Client, job_id: str, timeout: float = 60) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    while True:
        job = client.request("GET", f"/v1/jobs/{job_id}").json()
        if job.get("status") in ("succeeded", "failed", "cancelled") or time.monotonic() > deadline:
            return job
        time.sleep(0.05)


def stored_job(ctx: Ctx, job_id: str) -> dict[str, Any]:
    """The job's row as JSON, straight from the database."""
    assert re.fullmatch(r"[0-9a-f-]{36}", job_id), job_id
    text = ctx.pg.psql(
        f"SELECT json_build_object('left', left_name, 'right', right_name, 'options', options) FROM jobs WHERE id = '{job_id}';",
        ctx.server.database,
    )
    return json.loads(text)


def multipart_raw(parts: list[tuple[bytes, bytes]]) -> tuple[bytes, str]:
    """A multipart body of (Content-Disposition value, data), header bytes as given."""
    boundary = b"validation-6c1e9f"
    out = bytearray()
    for disposition, data in parts:
        out += b"--" + boundary + b"\r\nContent-Disposition: " + disposition + b"\r\n\r\n" + data + b"\r\n"
    out += b"--" + boundary + b"--\r\n"
    return bytes(out), f"multipart/form-data; boundary={boundary.decode()}"


def random_uuid() -> str:
    return str(uuid.uuid4())


def same_answer(a: hx.Reply, b: hx.Reply) -> bool:
    return a.status == b.status and a.body == b.body and sorted(a.headers) == sorted(b.headers)


# --- Authentication and scopes -------------------------------------------------------


def v1_routes() -> list[tuple[str, str, bytes | None, dict[str, str]]]:
    job, diff = random_uuid(), random_uuid()
    return [
        ("GET", "/v1/formats", None, {}),
        ("POST", "/v1/diff", body(SMALL), JSON),
        ("POST", "/v1/diff?save=true", body(SMALL), JSON),
        ("POST", "/v1/blobs", b"bytes", {"Content-Type": "text/plain"}),
        ("POST", "/v1/jobs", body({"left": {"blob": "0" * 64}, "right": {"blob": "0" * 64}}), JSON),
        ("GET", f"/v1/jobs/{job}", None, {}),
        ("POST", f"/v1/jobs/{job}/cancel", None, {}),
        ("GET", f"/v1/diffs/{diff}", None, {}),
        ("DELETE", f"/v1/diffs/{diff}", None, {}),
    ]


def test_missing_key_is_rejected_with_401(ctx: Ctx) -> None:
    client = ctx.client()
    before = ctx.rows()
    for method, path, data, headers in v1_routes():
        reply = client.request(method, path, data, headers, key=None)
        if ctx.expect(f"{method} {path} without a key", reply, 401, "unauthorized"):
            ctx.ok(reply.headers.get("www-authenticate") == "Bearer", f"{method} {path}: WWW-Authenticate is {reply.headers.get('www-authenticate')!r}")
    ctx.unchanged("requests without a key", before)


def test_malformed_authorization_header_is_rejected_with_401(ctx: Ctx) -> None:
    client = ctx.client()
    key = ctx.keys["main"]
    values = [
        "Bearer",
        "Bearer ",
        "Bearer  " + key,
        f"Basic {base64.b64encode(f'user:{key}'.encode()).decode()}",
        f"Token {key}",
        key,
        f"Bearer {key} {key}",
        f"Bearer {key}0",
        f"Bearer {key[:-1]}",
        f"Bearer {key.upper()}",
        f"Bearer dma_{key[4:]}",
        f"Bearer {key[4:]}",
        "Bearer dmk_not-a-key",
        f"Bearer dmk_{'0' * 64}'; DROP TABLE api_keys; --",
        "Bearer " + "dmk_" + "a" * 10_000,
    ]
    for value in values:
        reply = client.request("GET", "/v1/formats", headers={"Authorization": value}, key=None)
        ctx.expect(f"Authorization {value[:24]!r}...", reply, 401, "unauthorized")
    reply = client.request("GET", "/v1/formats", headers={"Authorization": f"Bearer {key}"}, key=None)
    ctx.ok(reply.status == 200, f"the well-formed header works: {reply.status}")


def test_unknown_key_is_rejected_with_401(ctx: Ctx) -> None:
    client = ctx.client()
    key = ctx.keys["main"]
    same_prefix = key[:12] + ("0" if key[12] != "0" else "1") + key[13:]  # finds the row, fails the hash
    for what, value in (("a random key", "dmk_" + secrets.token_hex(32)), ("a key that shares a real prefix", same_prefix)):
        reply = client.request("GET", "/v1/formats", key=value)
        ctx.expect(what, reply, 401, "unauthorized")
        reply = client.request("POST", "/v1/diff", body(SMALL), JSON, key=value)
        ctx.expect(f"{what} on POST", reply, 401, "unauthorized")


def test_revoked_key_is_rejected_with_401(ctx: Ctx) -> None:
    client = ctx.client()
    key_id, secret = ctx.server.create_key("validation-revoked")
    ctx.keys["revoked"] = secret
    ctx.ok(client.request("GET", "/v1/formats", key=secret).status == 200, "a new key works before it is revoked")
    ctx.ok(ctx.server.keys("revoke", key_id).returncode == 0, "keys revoke")
    revoked = client.request("GET", "/v1/formats", key=secret)
    ctx.expect("revoked key", revoked, 401, "unauthorized")
    ctx.expect("revoked key on POST", client.request("POST", "/v1/diff", body(SMALL), JSON, key=secret), 401, "unauthorized")
    unknown = client.request("GET", "/v1/formats", key="dmk_" + secrets.token_hex(32))
    ctx.ok(revoked.body == unknown.body, "a revoked key gets the same body as an unknown one")


def test_key_without_the_route_scope_gets_403(ctx: Ctx) -> None:
    client = ctx.client()
    allowed = {"read": {"GET"}, "write": {"POST"}, "delete": {"DELETE"}}
    before = ctx.rows()
    for scope, methods in allowed.items():
        for method, path, data, headers in v1_routes():
            if method in methods:
                continue
            reply = client.request(method, path, data, headers, key=ctx.keys[scope])
            ctx.expect(f"{scope}-only key on {method} {path}", reply, 403, "forbidden")
    # The scope is checked before the body is read: no 413 for a body never sent.
    reader = hx.Client(ctx.server.port, ctx.keys["read"], timeout=10)
    reply = reader.headers_only("POST", "/v1/blobs", {"Content-Type": "application/octet-stream", "Content-Length": str(2 << 30)})
    ctx.replies.append(("POST", "/v1/blobs", reply))
    ctx.expect("read-only key declaring a 2 GiB upload", reply, 403, "forbidden")
    ctx.unchanged("requests without the scope", before)


def test_another_keys_ids_look_like_ids_that_do_not_exist(ctx: Ctx) -> None:
    owner, other = ctx.client("main"), ctx.client("other")
    left = upload(owner, f"owner left {MARK}\n".encode())
    right = upload(owner, f"owner right {MARK}\n".encode())
    created = owner.post_json("/v1/jobs", {"left": {"blob": left}, "right": {"blob": right}})
    assert created.status == 202, created.body
    job = wait_job(owner, created.json()["id"])
    saved = owner.post_json("/v1/diff?save=true", SMALL)
    assert saved.status == 201, saved.body
    diff = saved.headers["location"].removeprefix("/v1/diffs/")
    missing_job, missing_diff, missing_blob = random_uuid(), random_uuid(), secrets.token_hex(32)
    pairs = [
        ("GET job", ("GET", f"/v1/jobs/{job['id']}"), ("GET", f"/v1/jobs/{missing_job}")),
        ("cancel job", ("POST", f"/v1/jobs/{job['id']}/cancel"), ("POST", f"/v1/jobs/{missing_job}/cancel")),
        ("GET job result", ("GET", f"/v1/diffs/{job['diffId']}"), ("GET", f"/v1/diffs/{missing_diff}")),
        ("GET saved diff", ("GET", f"/v1/diffs/{diff}"), ("GET", f"/v1/diffs/{missing_diff}")),
        ("DELETE saved diff", ("DELETE", f"/v1/diffs/{diff}"), ("DELETE", f"/v1/diffs/{missing_diff}")),
    ]
    for what, (method, path), (method2, path2) in pairs:
        theirs, nothing = other.request(method, path), other.request(method2, path2)
        ctx.ok(theirs.status == 404 and same_answer(theirs, nothing), f"{what}: another key's ID got {theirs.status} {theirs.body[:120]!r}, an unknown ID {nothing.status} {nothing.body[:120]!r}")
    jobs = [
        ("job over another key's blob", {"left": {"blob": left}, "right": {"blob": right}}, {"left": {"blob": missing_blob}, "right": {"blob": missing_blob}}),
        ("job over another key's diff", {"left": {"diff": diff, "side": "left"}, "right": {"diff": diff, "side": "right"}}, {"left": {"diff": missing_diff, "side": "left"}, "right": {"diff": missing_diff, "side": "right"}}),
    ]
    for what, theirs_body, nothing_body in jobs:
        theirs, nothing = other.post_json("/v1/jobs", theirs_body), other.post_json("/v1/jobs", nothing_body)
        ctx.ok(theirs.status == 404 and same_answer(theirs, nothing), f"{what}: got {theirs.status} {theirs.body[:120]!r}, unknown {nothing.status} {nothing.body[:120]!r}")
    ctx.ok(owner.request("GET", f"/v1/diffs/{diff}").status == 200, "the owner's diff survives another key's DELETE")
    ctx.ok(owner.request("GET", f"/v1/jobs/{job['id']}").status == 200, "the owner's job survives another key's cancel")
    again = other.request("POST", "/v1/blobs", f"owner left {MARK}\n".encode(), {"Content-Type": "application/octet-stream"})
    ctx.ok(again.status == 201, f"another key uploading the same bytes gets 201 (new for it), got {again.status}")


# --- Host and Origin -----------------------------------------------------------------


def test_host_not_on_the_allowlist_is_rejected(ctx: Ctx) -> None:
    client, port = ctx.client(), ctx.server.port
    hosts = ["evil.example", f"evil.example:{port}", "127.0.0.1", f"127.0.0.1:{port + 1}", f"localhost.evil:{port}",
             f"127.0.0.1.nip.io:{port}", f"[::ffff:127.0.0.1]:{port}", f"0.0.0.0:{port}", f"127.0.0.1:{port}.evil.example"]
    routes = [("GET", "/healthz", None, {}), ("GET", "/openapi.json", None, {}), ("GET", "/v1/formats", None, {}),
              ("POST", "/v1/diff", body(SMALL), JSON)]
    before = ctx.rows()
    for host in hosts:
        for method, path, data, headers in routes:
            for key in (True, None):  # with a valid key, and with none: the Host check comes first
                reply = client.request(method, path, data, {**headers, "Host": host}, key=key)
                ctx.expect(f"Host {host} on {method} {path} (key: {key is True})", reply, 403, "host-not-allowed")
    for host in (f"127.0.0.1:{port}", f"localhost:{port}", f"LOCALHOST:{port}", f"[::1]:{port}"):
        ctx.ok(client.request("GET", "/healthz", headers={"Host": host}, key=None).status == 200, f"Host {host} is allowed")
    raw = [
        ("an absolute target naming another host", f"GET http://evil.example/healthz HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n"),
        ("an absolute target and Host both foreign", "GET http://evil.example/healthz HTTP/1.1\r\nHost: evil.example\r\n"),
        ("no Host header", "GET /healthz HTTP/1.1\r\n"),
        ("no Host header over HTTP/1.0", "GET /healthz HTTP/1.0\r\n"),
        ("two Host headers, the second foreign", f"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nHost: evil.example\r\n"),
    ]
    for what, head in raw:
        status, reply = raw_request(port, (head + "Connection: close\r\n\r\n").encode())
        ctx.ok(status in (400, 403), f"{what}: expected 400 or 403, got {status} {reply[:160]!r}")
    ctx.unchanged("requests with a foreign Host", before)


def test_origin_not_on_the_allowlist_is_rejected_for_every_body_type(ctx: Ctx) -> None:
    client, port = ctx.client(), ctx.server.port
    form, form_type = hx.multipart([("left", "a.txt", b"a\n"), ("right", "b.txt", b"b\n")])
    requests = [
        ("JSON diff", "POST", "/v1/diff?save=true", body(SMALL), JSON),
        ("multipart diff", "POST", "/v1/diff?save=true", form, {"Content-Type": form_type}),
        ("text/plain diff", "POST", "/v1/diff?save=true", b"a", {"Content-Type": "text/plain"}),
        ("text/plain blob", "POST", "/v1/blobs", f"origin {MARK}".encode(), {"Content-Type": "text/plain"}),
        ("form-encoded blob", "POST", "/v1/blobs", b"a=1", {"Content-Type": "application/x-www-form-urlencoded"}),
        ("JSON job", "POST", "/v1/jobs", body({"left": {"blob": "0" * 64}, "right": {"blob": "0" * 64}}), JSON),
        ("GET formats", "GET", "/v1/formats", None, {}),
        ("GET healthz", "GET", "/healthz", None, {}),
    ]
    origins = ["https://evil.example", "null", "", f"http://127.0.0.1:{port}", f"http://localhost:{port}", "HTTPS://EVIL.EXAMPLE"]
    before = ctx.rows()
    for what, method, path, data, headers in requests:
        for origin in origins:
            for key in (True, None):
                reply = client.request(method, path, data, {**headers, "Origin": origin}, key=key)
                if ctx.expect(f"{what} from Origin {origin!r} (key: {key is True})", reply, 403, "origin-not-allowed"):
                    ctx.ok("access-control-allow-origin" not in reply.headers, f"{what}: a refused origin got CORS headers")
    ctx.unchanged("requests from a foreign Origin", before)


def test_preflight_from_a_foreign_origin_is_rejected(ctx: Ctx) -> None:
    client = ctx.client()
    for path in ("/v1/diff", "/v1/blobs", "/v1/jobs", f"/v1/diffs/{random_uuid()}", "/healthz"):
        for method in ("POST", "DELETE", "PUT"):
            headers = {"Origin": "https://evil.example", "Access-Control-Request-Method": method,
                       "Access-Control-Request-Headers": "authorization, content-type"}
            reply = client.request("OPTIONS", path, None, headers, key=None)
            if ctx.expect(f"preflight {method} {path}", reply, 403, "origin-not-allowed"):
                cors = [h for h in reply.headers if h.startswith("access-control-")]
                ctx.ok(not cors, f"preflight {method} {path}: refused but answered with {cors}")


def test_requests_without_a_key_are_rejected_before_any_work(ctx: Ctx) -> None:
    client = ctx.client()
    before = ctx.rows()
    reply = client.request("POST", "/v1/blobs", f"{MARK}\n".encode() * 3000, {"Content-Type": "text/plain"}, key=None)
    ctx.expect("upload without a key", reply, 401, "unauthorized")
    form, form_type = hx.multipart([("left", "a.txt", b"a\n"), ("right", "b.txt", b"b\n")])
    ctx.expect("saved multipart diff without a key", client.request("POST", "/v1/diff?save=true", form, {"Content-Type": form_type}, key=None), 401, "unauthorized")
    anonymous = hx.Client(ctx.server.port, None, timeout=10)
    for path, length in (("/v1/blobs", 2 << 30), ("/v1/diff", 1 << 30), ("/v1/jobs", 1 << 30)):
        started = time.monotonic()
        reply = anonymous.headers_only("POST", path, {"Content-Type": "application/json", "Content-Length": str(length)})
        ctx.replies.append(("POST", path, reply))
        ctx.expect(f"POST {path} declaring {length} bytes without a key", reply, 401, "unauthorized")
        ctx.ok(time.monotonic() - started < 5, f"POST {path}: the refusal waited for the body")
    ctx.unchanged("requests without a key", before)
    tmp = ctx.server.workdir / "tmp"
    leftovers = [p.name for p in tmp.rglob("*") if p.is_file()] if tmp.exists() else []
    ctx.ok(not leftovers, f"files left in the temporary folder: {leftovers[:5]}")
    anonymous_usage = ctx.pg.psql("SELECT count(*) FROM usage WHERE api_key_id IS NULL;", ctx.server.database)
    ctx.ok(anonymous_usage == "0", f"{anonymous_usage} usage records without a key")


# --- ID formats ----------------------------------------------------------------------

BAD_IDS = [
    "..", "%2e%2e", "%2E%2E%2F%2E%2E%2Fetc%2Fpasswd", "..%2F..%2F..%2Fetc%2Fpasswd", "..%5C..%5Cwindows",
    "%2Fetc%2Fpasswd", "C:%5Cwindows", "%00", "a%00b", f"{uuid.uuid4()}%00", f"{uuid.uuid4()}%00.json",
    "x" * 8192, "0" * 65536, "not-a-uuid", "0", "-1", str(uuid.uuid4())[:-1], f"{uuid.uuid4()}0",
    str(uuid.uuid4()).replace("-", "_"), "%27%20OR%20%271%27%3D%271", "1%3B%20DROP%20TABLE%20jobs",
    "%E2%80%AE", "%C0%AE%C0%AE", "%FF%FE", "~", "*", "%20", f"urn:uuid:{uuid.uuid4()}",
]


def test_malformed_job_and_diff_ids_get_400_or_404(ctx: Ctx) -> None:
    client = ctx.client()
    codes = {"/v1/jobs/": ("invalid-request", "job-not-found"), "/v1/diffs/": ("invalid-request", "diff-not-found")}
    before = ctx.rows()
    for bad in BAD_IDS:
        for method, prefix, suffix in (("GET", "/v1/jobs/", ""), ("POST", "/v1/jobs/", "/cancel"),
                                       ("GET", "/v1/diffs/", ""), ("DELETE", "/v1/diffs/", "")):
            reply = client.request(method, prefix + bad + suffix)
            expected = 400 if reply.code() == "invalid-request" else 404
            ctx.expect(f"{method} {prefix}{bad[:40]}{suffix}", reply, expected, *codes[prefix])
    # Unencoded traversal never reaches another route or a file.
    for path in ("/v1/jobs/../formats", "/v1/jobs/../../etc/passwd", "/v1/diffs//etc/passwd", "/v1/diffs/./x",
                 "/v1/jobs/..%2Fformats", "//v1/formats", "/v1/formats/..", "/v1//formats"):
        reply = client.request("GET", path)
        ctx.ok(reply.status in (400, 404) and reply.code() is not None, f"GET {path}: expected a typed 400 or 404, got {reply.status} {reply.body[:100]!r}")
    ctx.unchanged("malformed IDs", before)


def test_raw_control_bytes_in_the_request_target_are_refused(ctx: Ctx) -> None:
    port = ctx.server.port
    for target in (b"/v1/jobs/\x00", b"/v1/diffs/a\x00b", b"/v1/jobs/\x1b[31m", b"/v1/jobs/\xff\xfe"):
        request = b"GET " + target + f" HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {ctx.keys['main']}\r\nConnection: close\r\n\r\n".encode()
        status, reply = raw_request(port, request)
        ctx.ok(status in (None, 400, 404), f"GET {target!r}: expected 400, 404 or a closed connection, got {status} {reply[:120]!r}")


BAD_BLOB_IDS = [
    "", "0" * 63, "0" * 65, "g" * 64, "../" * 21 + "x", "/etc/passwd", "..%2F..%2Fetc%2Fpasswd", "a" * 63 + "\u0000",
    "\u0000" * 64, " " + "a" * 63, "a" * 63 + "\n", "0x" + "a" * 62, "a" * 100_000, "é" * 32, "' OR '1'='1" + "a" * 53,
]


def test_malformed_blob_ids_get_400_invalid_blob_id(ctx: Ctx) -> None:
    client = ctx.client()
    real = upload(client, f"blob ids {MARK}\n".encode())
    before = ctx.rows()
    for bad in [real.upper(), real[:32].upper() + real[32:], *BAD_BLOB_IDS]:
        reply = client.post_json("/v1/jobs", {"left": {"blob": bad}, "right": {"blob": real}})
        ctx.expect(f"job blob {bad[:40]!r}", reply, 400, "invalid-blob-id")
        query = "/v1/blobs?sha256=" + re.sub(r"[^0-9A-Za-z._~-]", lambda m: "%" + "%".join(f"{b:02X}" for b in m.group().encode()), bad)
        reply = client.request("POST", query, f"blob ids {MARK}\n".encode(), {"Content-Type": "application/octet-stream"})
        ctx.expect(f"?sha256={bad[:40]!r}", reply, 400, "invalid-blob-id")
    for value in (1, None, [real], {"id": real}, True):
        reply = client.post_json("/v1/jobs", {"left": {"blob": value}, "right": {"blob": real}})
        ok = (reply.status, reply.code()) in ((400, "invalid-request"), (400, "invalid-blob-id"))
        ctx.ok(ok, f"job blob {value!r}: expected a 400, got {reply.status} {reply.code()}")
    ctx.unchanged("malformed blob IDs", before)
    escaped = [p for p in ctx.server.workdir.rglob("*") if p.name in ("passwd", "x") and p.is_file()]
    ctx.ok(not escaped, f"files created from blob IDs: {escaped}")


def test_malformed_diff_references_in_a_job_get_400(ctx: Ctx) -> None:
    client = ctx.client()
    diff = random_uuid()
    before = ctx.rows()
    for side in ({"diff": "../../x", "side": "left"}, {"diff": diff + "\u0000", "side": "left"}, {"diff": diff, "side": "LEFT"},
                 {"diff": diff, "side": "middle"}, {"diff": diff}, {"side": "left"}, {"diff": 7, "side": "left"},
                 {"blob": "0" * 64, "diff": diff, "side": "left"}, {}):
        reply = client.post_json("/v1/jobs", {"left": side, "right": {"blob": "0" * 64}})
        ctx.expect(f"job input {side}", reply, 400, "invalid-request")
    ctx.unchanged("malformed diff references", before)


# --- String fields ------------------------------------------------------------------


def test_names_with_quotes_sql_and_control_characters_are_plain_data(ctx: Ctx) -> None:
    client = ctx.client()
    left_blob = upload(client, b"name test left\n")
    right_blob = upload(client, b"name test right\n")
    for name in INJECTIONS:
        expected = final_component(name)
        reply = client.post_json("/v1/diff", {"left": {"name": name, "text": "a\n"}, "right": {"name": name, "base64": "Yg=="}})
        if reply.status == 200:
            doc = reply.json()
            ctx.ok(doc["left"]["name"] == expected and doc["right"]["name"] == expected, f"diff name {name!r}: recorded {doc['left']['name']!r}, expected {expected!r}")
        else:
            ctx.expect(f"diff name {name!r}", reply, 400, "invalid-request")
        reply = client.post_json("/v1/jobs", {"left": {"blob": left_blob, "name": name}, "right": {"blob": right_blob, "name": name}})
        if reply.status != 202:
            ctx.expect(f"job name {name!r}", reply, 400, "invalid-request")
            continue
        job = reply.json()
        ctx.ok(job["left"]["name"] == expected, f"job name {name!r}: echoed {job['left']['name']!r}")
        ctx.ok(stored_job(ctx, job["id"])["left"] == expected, f"job name {name!r}: stored differently")
        done = wait_job(client, job["id"])
        if ctx.ok(done.get("status") == "succeeded", f"job name {name!r}: {done.get('status')} {done.get('error')}"):
            doc = client.request("GET", done["result"]).json()
            ctx.ok(doc["left"]["name"] == expected, f"job name {name!r}: result records {doc['left']['name']!r}")
    for bad in ("\ud800", "a\udfffb"):  # lone surrogates are not text
        raw = b'{"left": {"name": "' + json.dumps(bad)[1:-1].encode() + b'", "text": "a"}, "right": {"text": "b"}}'
        ctx.expect(f"name {bad!r}", client.request("POST", "/v1/diff", raw, JSON), 400, "invalid-request")


def test_name_length_and_nul_rules_hold_for_every_input_form(ctx: Ctx) -> None:
    client = ctx.client()
    blob = upload(client, b"length rules\n")
    for name, status, code in (("n" * 1024, 200, None), ("é" * 1024, 200, None), ("😀" * 1024, 200, None),
                               ("n" * 1025, 400, "invalid-request"), ("é" * 1025, 400, "invalid-request"),
                               ("", 400, "invalid-request"), (f"a\u0000{MARK}.txt", 400, "nul-character")):
        what = f"name of {len(name)} {name[:1]!r} characters"
        diff = client.post_json("/v1/diff", {"left": {"name": name, "text": "a"}, "right": {"text": "b"}})
        job = client.post_json("/v1/jobs", {"left": {"blob": blob, "name": name}, "right": {"blob": blob}})
        form, form_type = multipart_raw([(b'form-data; name="left"; filename="' + name.encode() + b'"', b"a"),
                                         (b'form-data; name="right"; filename="b.txt"', b"b")])
        multipart = client.request("POST", "/v1/diff", form, {"Content-Type": form_type})
        if status == 200:
            ctx.ok(diff.status == 200, f"JSON diff, {what}: {diff.status} {diff.code()}")
            ctx.ok(multipart.status == 200, f"multipart diff, {what}: {multipart.status} {multipart.code()}")
            if ctx.ok(job.status == 202, f"job, {what}: {job.status} {job.code()}"):
                wait_job(client, job.json()["id"])
        else:
            ctx.expect(f"JSON diff, {what}", diff, status, code)
            ctx.expect(f"job, {what}", job, status, code)
            # An empty multipart file name is the form's way of sending no name.
            ok = multipart.status == 400 and multipart.code() in (code, "invalid-request") or (name == "" and multipart.status == 200)
            ctx.ok(ok, f"multipart diff, {what}: expected 400, got {multipart.status} {multipart.code()}")


def test_multipart_file_names_with_quotes_and_control_characters(ctx: Ctx) -> None:
    client = ctx.client()
    dispositions = [
        b'form-data; name="left"; filename="it\\"s.txt"',
        b'form-data; name="left"; filename="\'; DROP TABLE diffs; --.txt"',
        b'form-data; name="left"; filename="a\\\\b.txt"',
        b"form-data; name=\"left\"; filename*=UTF-8''%27%3B%20DROP%20TABLE%20jobs%3B%20--.txt",
        b'form-data; name="left"; filename="ctrl\x01\x1b.txt"',
        b'form-data; name="left"; filename="..%2F..%2Fetc%2Fpasswd"',
        b'form-data; name="left"; filename="\xff\xfe.txt"',
        b'form-data; name="left"; filename="a\x00b.txt"',
        b'form-data; name="left\'; DROP TABLE x;--"; filename="a.txt"',
        b'form-data; name="options"',
    ]
    before = ctx.rows()
    for disposition in dispositions:
        form, form_type = multipart_raw([(disposition, b"a\n"), (b'form-data; name="right"; filename="b.txt"', b"b\n")])
        reply = client.request("POST", "/v1/diff", form, {"Content-Type": form_type})
        ok = reply.status == 200 or (reply.status == 400 and reply.code() in ("invalid-request", "nul-character", "invalid-options"))
        ctx.ok(ok, f"multipart {disposition[:60]!r}: expected 200 or a typed 400, got {reply.status} {reply.body[:120]!r}")
    ctx.unchanged("multipart diffs that are not saved", before)


def test_options_with_quotes_sql_and_control_characters_are_plain_data(ctx: Ctx) -> None:
    client = ctx.client()
    patterns = [f"'; DROP TABLE jobs; -- {MARK}", f'"{MARK}"', f"back\\\\slash \\d+ {MARK}", f"\x01\x1f‮{MARK}",
                f"$1 OR 1=1 {MARK}", f"(?i){MARK}' UNION SELECT 1 --"]
    excludes = [f"'; DROP TABLE diffs; -- {MARK}", f"\\\\{MARK}\\*.log", f"\x07{MARK}/**"]
    options = {"ignore": {"regex": patterns}, "tree": {"exclude": excludes}}
    reply = client.post_json("/v1/diff", {**SMALL, "options": options})
    if ctx.ok(reply.status == 200, f"diff with hostile option strings: {reply.status} {reply.code()}"):
        echoed = reply.json()["options"]
        ctx.ok(echoed["ignore"]["regex"] == patterns and echoed["tree"]["exclude"] == excludes, "diff options echoed differently")
    left, right = upload(client, b"options left\n"), upload(client, b"options right\n")
    reply = client.post_json("/v1/jobs", {"left": {"blob": left}, "right": {"blob": right}, "options": options})
    if ctx.ok(reply.status == 202, f"job with hostile option strings: {reply.status} {reply.code()}"):
        job_id = reply.json()["id"]
        ctx.ok(stored_job(ctx, job_id)["options"] == options, "job options stored differently")
        done = wait_job(client, job_id)
        if ctx.ok(done.get("status") == "succeeded", f"job with hostile options: {done.get('status')} {done.get('error')}"):
            echoed = client.request("GET", done["result"]).json()["options"]
            ctx.ok(echoed["ignore"]["regex"] == patterns and echoed["tree"]["exclude"] == excludes, "job result echoes other options")
    ctx.pg.counts(ctx.server.database, TABLES)  # every table is still there


def test_invalid_options_are_a_typed_400_and_change_nothing(ctx: Ctx) -> None:
    client = ctx.client()
    blob = upload(client, b"invalid options\n")
    bad = [
        ({"granularity": f"line'; DROP TABLE jobs; -- {MARK}"}, "invalid-options"),
        ({f"'; DROP TABLE jobs; -- {MARK}": 1}, "invalid-options"),
        ({"ignore": {f"\"{MARK}\\": True}}, "invalid-options"),
        ({"ignore": {"regex": f"{MARK}"}}, "invalid-options"),
        ({"ignore": {"regex": [1, None]}}, "invalid-options"),
        ({"context": -1}, "invalid-options"),
        ({"context": 10**30}, "invalid-options"),
        ({"context": "1; DROP TABLE jobs"}, "invalid-options"),
        ("'; DROP TABLE jobs; --", "invalid-options"),
        ([f"{MARK}"], "invalid-options"),
        ({"ignore": {"regex": [f"a\u0000{MARK}"]}}, None),  # fine for a sync diff; a job refuses it with nul-character
    ]
    before = ctx.rows()
    for options, code in bad:
        diff = client.post_json("/v1/diff", {**SMALL, "options": options})
        job = client.post_json("/v1/jobs", {"left": {"blob": blob}, "right": {"blob": blob}, "options": options})
        if code is None:
            ctx.ok(diff.status == 200, f"diff options {options!r}: {diff.status} {diff.code()}")
            ctx.expect(f"job options {options!r}", job, 400, "nul-character")
            continue
        for what, reply in (("diff", diff), ("job", job)):
            ok = reply.status == 400 and reply.code() in (code, "invalid-request")
            ctx.ok(ok, f"{what} options {str(options)[:60]!r}: expected 400 {code}, got {reply.status} {reply.code()}")
    form, form_type = hx.multipart([("left", "a.txt", b"a\n"), ("right", "b.txt", b"b\n"), ("options", None, b'{"context": 1, "context": 2}')])
    reply = client.request("POST", "/v1/diff", form, {"Content-Type": form_type})
    ctx.ok(reply.status == 400 and reply.code() in ("invalid-options", "invalid-request"), f"duplicate option key (multipart): {reply.status} {reply.code()}")
    # Spec §9: options that give a key twice, at any depth, are invalid.
    for raw in (b'{"left": {"text": "a"}, "right": {"text": "b"}, "options": {"context": 1, "context": 2}}',
                b'{"left": {"text": "a"}, "right": {"text": "b"}, "options": {"ignore": {"case": true, "case": false}}}'):
        reply = client.request("POST", "/v1/diff", raw, JSON)
        ctx.ok(reply.status == 400 and reply.code() in ("invalid-options", "invalid-request"), f"duplicate option key (JSON {raw[50:90]!r}): {reply.status} {reply.code()}")
    deep = b'{"left": {"text": "a"}, "right": {"text": "b"}, "options": {"ignore": {"regex": ' + b"[" * 5000 + b"]" * 5000 + b"}}}"
    ctx.expect("options nested 5000 deep", client.request("POST", "/v1/diff", deep, JSON), 400, "invalid-request", "invalid-options")
    ctx.unchanged("invalid options", before)


def test_query_parameters_with_quotes_and_sql_are_rejected_or_ignored(ctx: Ctx) -> None:
    client = ctx.client()
    before = ctx.rows()
    for query in ("save=%27%20OR%20%271%27%3D%271", "save=true%00", "save=true%3B%20DROP%20TABLE%20diffs", "save=1&save=0",
                  "save=TRUE", "save=", "save=%E2%80%AEtrue", f"save={MARK}"):
        reply = client.request("POST", f"/v1/diff?{query}", body(SMALL), JSON)
        ctx.expect(f"/v1/diff?{query}", reply, 400, "invalid-request")
    for query in ("sha256=%27%20OR%201%3D1--", f"sha256={'a' * 64}&sha256={'b' * 64}", "sha256=" + "a" * 64 + "%00"):
        reply = client.request("POST", f"/v1/blobs?{query}", b"x", {"Content-Type": "application/octet-stream"})
        ctx.expect(f"/v1/blobs?{query[:50]}", reply, 400, "invalid-blob-id", "invalid-request")
    ctx.unchanged("refused query parameters", before)
    for path in ("/v1/diff?x=%27%3B%20DROP%20TABLE%20diffs%3B--", "/v1/diff?%27=1", "/v1/diff?save=false&%00=1"):
        reply = client.request("POST", path, body(SMALL), JSON)
        ok = reply.status == 200 or (reply.status == 400 and reply.code() == "invalid-request")
        ctx.ok(ok, f"{path}: an unknown parameter gives 200 or 400, got {reply.status} {reply.code()}")
    ctx.unchanged("unknown query parameters", before)


def test_blob_media_type_is_plain_data(ctx: Ctx) -> None:
    client = ctx.client()
    for index, media in enumerate(("text/plain; name=\"'; DROP TABLE blobs; --\"", "application/x-' OR '1'='1", "text/\\plain")):
        reply = client.request("POST", "/v1/blobs", f"media {index} {MARK}\n".encode(), {"Content-Type": media})
        if ctx.ok(reply.status == 201, f"media type {media!r}: {reply.status} {reply.code()}"):
            ctx.ok(reply.json()["mediaType"] == media, f"media type {media!r}: recorded {reply.json()['mediaType']!r}")
    reply = client.request("POST", "/v1/blobs", f"media long {MARK}\n".encode(), {"Content-Type": "text/" + "x" * 300})
    ctx.ok(reply.status == 201 and reply.json()["mediaType"] is None, f"an over-long media type is not recorded: {reply.status}")


def test_api_key_names_with_quotes_and_sql_are_plain_data(ctx: Ctx) -> None:
    name = f"'; DROP TABLE api_keys; -- \"{MARK}\""
    key_id, secret = ctx.server.create_key(name, "read")
    ctx.keys["sql-name"] = secret
    listed = ctx.server.keys("list")
    # Shown escaped, as Rust's escape_debug writes it, never interpreted.
    shown = name.replace("\\", "\\\\").replace("'", "\\'").replace('"', '\\"')
    row = next((line for line in listed.stdout.splitlines() if line.startswith(key_id)), "")
    ctx.ok(listed.returncode == 0 and row.endswith(shown), f"keys list shows the name {row[-60:]!r}")
    ctx.ok(secret not in listed.stdout + listed.stderr, "keys list shows a secret")
    ctx.ok(ctx.client().request("GET", "/v1/formats", key=secret).status == 200, "the key with that name works")
    long = ctx.server.keys("create", "--name", "n" * 101)
    ctx.ok(long.returncode != 0 and "panicked" not in long.stderr and not re.search(r"dmk_[0-9a-f]{64}", long.stdout), "an over-long key name is refused cleanly")
    ctx.pg.counts(ctx.server.database, TABLES)


# --- Secrets ------------------------------------------------------------------------


def admin_token_path(ctx: Ctx) -> pathlib.Path:
    return ctx.server.workdir / "admin" / "admin-token"


def test_admin_token_file_has_mode_0600(ctx: Ctx) -> None:
    path = admin_token_path(ctx)
    mode = stat.S_IMODE(path.stat().st_mode)
    folder = stat.S_IMODE(path.parent.stat().st_mode)
    ctx.ok(mode == 0o600, f"the admin token file has mode {mode:o}")
    ctx.ok(folder & 0o077 == 0, f"the admin token folder has mode {folder:o}")
    ctx.ok(re.fullmatch(r"dma_[0-9a-f]{64}\n?", path.read_text()) is not None, "the admin token file holds one token")


def test_admin_token_is_not_an_api_key(ctx: Ctx) -> None:
    token = admin_token_path(ctx).read_text().strip()
    client = ctx.client()
    ctx.expect("the admin token as a bearer key", client.request("GET", "/v1/formats", key=token), 401, "unauthorized")
    ctx.expect("the admin token in a key's shape", client.request("GET", "/v1/formats", key="dmk_" + token[4:]), 401, "unauthorized")


def secrets_of(ctx: Ctx) -> dict[str, str]:
    found = {"the admin token": admin_token_path(ctx).read_text().strip()}
    for name, key in ctx.keys.items():
        found[f"the {name} key"] = key
        found[f"the {name} key without its prefix"] = key[4:]
    found["the admin token without its prefix"] = found["the admin token"][4:]
    return found


def test_openapi_document_holds_no_secret(ctx: Ctx) -> None:
    text = ctx.client().request("GET", "/openapi.json", key=None).body.decode()
    for what, value in secrets_of(ctx).items():
        ctx.ok(value not in text, f"/openapi.json holds {what}")


def test_every_error_is_typed_documented_and_holds_no_secret(ctx: Ctx) -> None:
    codes = documented_codes()
    hidden = secrets_of(ctx)
    errors = [(m, p, r) for m, p, r in ctx.replies if r.status >= 400]
    ctx.ok(bool(errors), "no error reply was checked")
    for method, path, reply in ctx.replies:
        ctx.ok(reply.status < 500 or reply.code() not in ("internal", None), f"{method} {path}: {reply.status} {reply.body[:160]!r}")
    for method, path, reply in errors:
        try:
            value = json.loads(reply.body)
        except ValueError:
            value = None
        shaped = isinstance(value, dict) and set(value) == {"error"} and isinstance(value["error"], dict) and set(value["error"]) == {"code", "message"}
        if not ctx.ok(shaped and reply.headers.get("content-type") == "application/json", f"{method} {path}: not a typed error: {reply.body[:160]!r}"):
            continue
        code = value["error"]["code"]
        ctx.ok(codes.get(code) == reply.status, f"{method} {path}: {code} with {reply.status}, documented {codes.get(code)}")
        text = reply.body.decode("utf-8", "replace")
        ctx.ok(MARK not in text, f"{method} {path}: the error message quotes the request: {value['error']['message'][:160]!r}")
        for what, secret in hidden.items():
            ctx.ok(secret not in text, f"{method} {path}: the error body holds {what}")


AFTER_STOP = ("test_server_log_holds_no_key_admin_token_or_request_text",)


def test_server_log_holds_no_key_admin_token_or_request_text(ctx: Ctx) -> None:
    text = ctx.server.log_text()
    ctx.ok("listening" in text, "the log has the start-up line")
    ctx.ok(ctx.server.exit_code == 0, f"the server stopped with exit code {ctx.server.exit_code}")
    ctx.ok("panicked" not in text, "the log records a panic")
    for what, secret in secrets_of(ctx).items():
        ctx.ok(secret not in text, f"the log holds {what}")
    ctx.ok(MARK not in text, "the log holds request content (names, options or bodies)")
    ctx.ok(ctx.pg.password not in text, "the log holds the database password")


# --- Main ---------------------------------------------------------------------------


def run(ctx: Ctx, tests: list[Callable[[Ctx], None]]) -> list[str]:
    failed = []
    for test in tests:
        before = len(ctx.failures)
        started = time.monotonic()
        try:
            test(ctx)
        except Exception as error:  # noqa: BLE001 - report the test, keep going
            ctx.failures.append(f"raised {type(error).__name__}: {error}")
            traceback.print_exc(limit=3)
        new = ctx.failures[before:]
        print(f"{'FAIL' if new else 'ok  '} {test.__name__} ({time.monotonic() - started:.1f} s)", flush=True)
        for line in new[:25]:
            print(f"       {line}", flush=True)
        if len(new) > 25:
            print(f"       ... and {len(new) - 25} more", flush=True)
        failed += [f"{test.__name__}: {line}" for line in new]
    return failed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--server", type=pathlib.Path, help="the diffyman-server binary (default: newest build)")
    parser.add_argument("--database", default="diffyman_e2e_validation", help="test database, dropped and created")
    parser.add_argument("--workdir-root", type=pathlib.Path, default=hx.default_workdir_root())
    parser.add_argument("-k", dest="only", help="run only the tests whose name contains this")
    args = parser.parse_args()
    try:
        pg = hx.PgServer.find()
        pg.check()
    except hx.Skip as why:
        print(f"skipping the validation tests: {why}", file=sys.stderr)
        return 1 if hx.db_required() else 0
    tests = [f for name, f in globals().items() if name.startswith("test_") and callable(f)]
    if args.only:
        tests = [t for t in tests if args.only in t.__name__ or t.__name__ in AFTER_STOP]
    during = [t for t in tests if t.__name__ not in AFTER_STOP and t is not test_every_error_is_typed_documented_and_holds_no_secret]
    args.workdir_root.mkdir(parents=True, exist_ok=True)
    workdir = pathlib.Path(tempfile.mkdtemp(prefix="validation-", dir=args.workdir_root))
    server = hx.Server(args.server or hx.default_server(), workdir, pg, args.database)
    ctx = Ctx(server, pg)
    failed: list[str] = []
    try:
        server.start()
        for name, scopes in (("main", None), ("other", None), ("read", "read"), ("write", "write"), ("delete", "delete")):
            ctx.key_ids[name], ctx.keys[name] = server.create_key(f"validation-{name}", scopes)
        failed += run(ctx, during + [test_every_error_is_typed_documented_and_holds_no_secret])
        for client in ctx.clients.values():
            client.close()
        server.stop()
        failed += run(ctx, [t for t in tests if t.__name__ in AFTER_STOP])
    finally:
        server.stop()
        server.cleanup(keep=False)
    print(f"\n{len(tests)} tests, {len(failed)} failed checks")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
