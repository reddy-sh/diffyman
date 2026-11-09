"""End-to-end test of a real `diffyman-server serve` process.

Starts the server (tests/server/harness.py: a dedicated database, a fresh
working folder, a random port on 127.0.0.1), creates API keys with the
`keys` command, and checks over HTTP:

  corpus     every corpus case whose two inputs are files: POST /v1/diff as
             JSON with base64, as JSON with text (when both inputs are UTF-8)
             and as multipart. Each response must be RFC 8785 canonical, name
             `server` in provenance.binding, and equal expected.json byte for
             byte once the top-level "provenance" key is removed, like
             tests/conformance/run_bindings.py.
  jobs       a spread sample of those cases (30 by default) through
             POST /v1/blobs, POST /v1/jobs, GET /v1/jobs/{id} and
             GET /v1/diffs/{id}, compared the same way.
  saved      POST /v1/diff?save=true, GET /v1/diffs/{id}, a job whose inputs
             are the sides of the saved diff, DELETE of both diffs, and that
             the objects nothing references any more are gone from the store
             while a shared one stays until its last reference goes.
  security   API keys (missing, wrong, read-only, revoked, another key's
             jobs and diffs), Host and Origin checks, body and input limits,
             typed errors with stable codes, cancel of a finished job.
  large      two generated inputs (200 MB each by default) uploaded as blobs
             and diffed by a job: wall time and the server's peak resident
             memory, and the result equals `diffyman diff --json` when --cli
             is given. The inputs are deleted afterwards.
  bench      small synchronous diffs from one client for 10 s over one
             keep-alive connection: requests per second, p50 and p99.
  logs       the server log holds no input content, input name, corpus path
             or secret.

It also reports the server's idle resident memory after start. `--report`
writes every measurement as JSON. With `--keep` the database and the working
folder stay for a restore drill (scripts/restore-drill.sh --database ...).

Without a database it prints why and exits 0 (1 with
DIFFYMAN_TEST_DB_REQUIRED=1). Run from the repository root:

  uv run --no-project --python 3.12 python tests/server/run_e2e.py \\
      --server target/release/diffyman-server --cli target/release/diffyman
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import pathlib
import secrets
import shutil
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from collections import Counter
from dataclasses import dataclass, field
from typing import Any

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "conformance"))
sys.path.insert(0, str(HERE.parents[1] / "scripts" / "lib"))
import harness as hx  # noqa: E402
import jcs  # noqa: E402
import run_bindings  # noqa: E402

REPO = hx.REPO
CORPUS = REPO / "tests" / "corpus"
OPENAPI = REPO / "docs" / "api" / "openapi.json"
TABLES = ("api_keys", "blobs", "jobs", "diffs", "usage", "audit")
MIB = 1 << 20


# --- Results -------------------------------------------------------------------


@dataclass
class Results:
    checks: Counter[str] = field(default_factory=Counter)
    failures: list[str] = field(default_factory=list)
    measurements: dict[str, Any] = field(default_factory=dict)

    def check(self, group: str, ok: bool, what: str) -> bool:
        self.checks[group] += 1
        if not ok:
            self.failures.append(f"{group}: {what}")
            print(f"  FAIL {group}: {what}", flush=True)
        return ok

    def expect_error(self, group: str, what: str, reply: hx.Reply, status: int, code: str) -> None:
        self.check(
            group,
            reply.status == status and reply.code() == code,
            f"{what}: expected {status} {code}, got {reply.status} {reply.code() or reply.body[:120]!r}",
        )


def judge_document(raw: bytes, expected: bytes | None) -> str | None:
    """Why a response body is not the expected document, or None."""
    try:
        document = json.loads(raw)
    except ValueError:
        return f"not JSON: {raw[:120]!r}"
    if jcs.canonicalize(document) != raw:
        return "not RFC 8785 canonical"
    binding = document.get("provenance", {}).get("binding") if isinstance(document, dict) else None
    if binding != "server":
        return f"provenance.binding is {binding!r}"
    if expected is not None:
        stripped = run_bindings.conformance_bytes(document)
        if stripped != expected:
            return f"differs from expected.json ({run_bindings.first_difference(expected, stripped)})"
    return None


def expected_bytes(case: Any) -> bytes:
    return run_bindings.conformance_bytes(json.loads(case.expected.read_text(encoding="utf-8")))


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


# --- Corpus through POST /v1/diff ----------------------------------------------


def diff_bodies(case: Any) -> list[tuple[str, bytes, str]]:
    """(mode, body, content type) for every request form of one case."""
    left, right = case.left.read_bytes(), case.right.read_bytes()
    options_text = case.options.read_text(encoding="utf-8")
    options = json.loads(options_text)
    forms = []

    def side(data: bytes, name: str, text: bool) -> dict[str, str]:
        if text:
            return {"name": name, "text": data.decode("utf-8")}
        return {"name": name, "base64": base64.b64encode(data).decode("ascii")}

    for text in (False, True):
        if text:
            try:
                left.decode("utf-8")
                right.decode("utf-8")
            except UnicodeDecodeError:
                continue
        request = {"left": side(left, case.left.name, text), "right": side(right, case.right.name, text), "options": options}
        forms.append(("json-text" if text else "json-base64", json.dumps(request).encode("utf-8"), "application/json"))
    form, content_type = hx.multipart(
        [("left", case.left.name, left), ("right", case.right.name, right), ("options", None, options_text.encode())]
    )
    forms.append(("multipart", form, content_type))
    return forms


def corpus_cases(limit: int | None) -> list[Any]:
    cases = [c for c in run_bindings.list_cases(CORPUS) if not c.folder]
    if limit is not None and limit < len(cases):
        cases = spread(cases, limit)
    return cases


def spread(items: list[Any], count: int) -> list[Any]:
    """`count` items evenly spread over the list (deterministic)."""
    if count >= len(items):
        return list(items)
    return [items[(i * len(items)) // count] for i in range(count)]


def test_corpus(client: hx.Client, cases: list[Any], results: Results) -> None:
    modes: Counter[str] = Counter()
    statuses: Counter[str] = Counter()
    for case in cases:
        expected = expected_bytes(case)
        for mode, body, content_type in diff_bodies(case):
            reply = client.request("POST", "/v1/diff", body, {"Content-Type": content_type})
            problem = (
                f"HTTP {reply.status} {reply.code()}"
                if reply.status != 200 or reply.headers.get("content-type") != "application/json"
                else judge_document(reply.body, expected)
            )
            results.check("corpus", problem is None, f"{case.name} [{mode}]: {problem}")
            modes[mode] += 1
            if mode == "multipart" and reply.status == 200:
                statuses[json.loads(reply.body).get("status", "?")] += 1
    results.measurements["corpus"] = {
        "cases": len(cases),
        "requests": dict(modes),
        "statuses": dict(statuses),
    }
    print(f"corpus: {len(cases)} file cases, {sum(modes.values())} requests ({dict(modes)})", flush=True)


# --- Jobs ----------------------------------------------------------------------


def upload(client: hx.Client, data: bytes | pathlib.Path, results: Results, group: str) -> str | None:
    if isinstance(data, pathlib.Path):
        digest = file_sha256(data)
        size = data.stat().st_size
        with data.open("rb") as stream:
            reply = client.request(
                "POST",
                f"/v1/blobs?sha256={digest}",
                stream,
                {"Content-Type": "application/octet-stream", "Content-Length": str(size)},
            )
    else:
        digest = sha256(data)
        reply = client.request(
            "POST", f"/v1/blobs?sha256={digest}", data, {"Content-Type": "application/octet-stream"}
        )
    ok = reply.status in (200, 201) and reply.json().get("blobId") == digest
    results.check(group, ok, f"upload: HTTP {reply.status} {reply.code() or reply.body[:200]!r}")
    return digest if ok else None


def file_sha256(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(8 * MIB):
            h.update(chunk)
    return h.hexdigest()


def wait_job(client: hx.Client, job_id: str, timeout: float, interval: float = 0.05) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    while True:
        reply = client.request("GET", f"/v1/jobs/{job_id}")
        job = reply.json()
        if reply.status != 200 or job.get("status") in ("succeeded", "failed", "cancelled"):
            return job
        if time.monotonic() > deadline:
            return {**job, "status": "timeout"}
        time.sleep(interval)


def job_input(blob: str, name: str) -> dict[str, str]:
    return {"blob": blob, "name": name}


def test_jobs(client: hx.Client, cases: list[Any], results: Results) -> None:
    started = time.monotonic()
    queued = []
    for case in cases:
        left = upload(client, case.left.read_bytes(), results, "jobs")
        right = upload(client, case.right.read_bytes(), results, "jobs")
        if left is None or right is None:
            continue
        body = {
            "left": job_input(left, case.left.name),
            "right": job_input(right, case.right.name),
            "options": json.loads(case.options.read_text(encoding="utf-8")),
        }
        reply = client.post_json("/v1/jobs", body)
        ok = reply.status == 202 and reply.headers.get("location") == f"/v1/jobs/{reply.json().get('id')}"
        if results.check("jobs", ok, f"{case.name}: create: HTTP {reply.status} {reply.code()}"):
            queued.append((case, reply.json()["id"]))
    for case, job_id in queued:
        job = wait_job(client, job_id, timeout=120)
        if not results.check("jobs", job.get("status") == "succeeded", f"{case.name}: job {job.get('status')} {job.get('error')}"):
            continue
        reply = client.request("GET", job["result"])
        problem = f"HTTP {reply.status}" if reply.status != 200 else judge_document(reply.body, expected_bytes(case))
        results.check("jobs", problem is None, f"{case.name}: result: {problem}")
    elapsed = time.monotonic() - started
    results.measurements["jobs"] = {"cases": len(cases), "succeeded": len(queued), "seconds": round(elapsed, 2)}
    print(f"jobs: {len(queued)} of {len(cases)} sampled cases through blobs and jobs in {elapsed:.1f} s", flush=True)


# --- Saved diffs and deletion ----------------------------------------------------


class ObjectStore:
    """Whether an object exists in the server's blob store."""

    def __init__(self, server: hx.Server) -> None:
        self.server = server

    def exists(self, blob: str) -> bool | None:
        key = f"blobs/sha256/{blob[:2]}/{blob}"
        url = self.server.store_url
        if url.startswith("file://"):
            return (self.server.store_dir / key).is_file()
        if url.startswith("s3://"):
            import s3_bucket  # noqa: PLC0415 - scripts/lib, only for s3:// stores

            endpoint = os.environ.get("AWS_ENDPOINT_URL_S3", "").rstrip("/")
            if not endpoint or "AWS_ACCESS_KEY_ID" not in os.environ:
                return None
            bucket_and_prefix = url.removeprefix("s3://").strip("/")
            status = s3_bucket.request("HEAD", f"{endpoint}/{bucket_and_prefix}/{key}")
            return {200: True, 404: False}.get(status)
        return None


def test_saved(client: hx.Client, other: hx.Client, store: ObjectStore, marker: str, results: Results) -> None:
    group = "saved"
    left = f"{marker} first line\nshared line\n{secrets.token_hex(16)}\n".encode()
    right = f"{marker} first line\nshared line changed\n{secrets.token_hex(16)}\n".encode()
    name_left, name_right = f"{marker}-old.txt", f"{marker}-new.txt"
    body = {
        "left": {"name": name_left, "text": left.decode()},
        "right": {"name": name_right, "text": right.decode()},
        "options": {"granularity": "word"},
    }
    reply = client.post_json("/v1/diff?save=true", body)
    location = reply.headers.get("location", "")
    if not results.check(group, reply.status == 201 and location.startswith("/v1/diffs/"), f"save: HTTP {reply.status} {reply.code()}"):
        return
    saved = reply.body
    results.check(group, judge_document(saved, None) is None, f"saved document: {judge_document(saved, None)}")
    diff_a = location.removeprefix("/v1/diffs/")
    blobs = {"left": sha256(left), "right": sha256(right), "document": sha256(saved)}

    got = client.request("GET", location)
    results.check(group, got.status == 200 and got.body == saved, f"GET saved diff: HTTP {got.status}, same bytes {got.body == saved}")
    for what, blob in blobs.items():
        results.check(group, store.exists(blob) is True, f"{what} object stored: {store.exists(blob)}")

    # Another key sees nothing of it.
    results.expect_error(group, "other key GET diff", other.request("GET", location), 404, "diff-not-found")
    results.expect_error(group, "other key DELETE diff", other.request("DELETE", location), 404, "diff-not-found")
    by_side = {
        "left": {"diff": diff_a, "side": "left", "name": name_left},
        "right": {"diff": diff_a, "side": "right", "name": name_right},
        "options": {"granularity": "word"},
    }
    results.expect_error(group, "other key job from diff sides", other.post_json("/v1/jobs", by_side), 404, "diff-not-found")
    # Nor can it use the stored inputs by their SHA-256: blobs belong to the key that uploaded them.
    by_blob = {"left": job_input(blobs["left"], "a"), "right": job_input(blobs["right"], "b")}
    results.expect_error(group, "other key job over the stored inputs", other.post_json("/v1/jobs", by_blob), 404, "blob-not-found")

    # A job over the saved diff's two sides gives the same document, so its
    # result shares the document object with the first diff.
    reply = client.post_json("/v1/jobs", by_side)
    if not results.check(group, reply.status == 202, f"job from diff sides: HTTP {reply.status} {reply.code()}"):
        return
    job_id = reply.json()["id"]
    results.expect_error(group, "other key GET job", other.request("GET", f"/v1/jobs/{job_id}"), 404, "job-not-found")
    job = wait_job(client, job_id, timeout=60)
    if not results.check(group, job.get("status") == "succeeded", f"job from diff sides: {job.get('status')}"):
        return
    second = client.request("GET", job["result"])
    results.check(
        group,
        second.status == 200 and run_bindings.conformance_bytes(json.loads(second.body)) == run_bindings.conformance_bytes(json.loads(saved)),
        "job over the saved sides gives the same document",
    )
    results.expect_error(group, "cancel a finished job", client.request("POST", f"/v1/jobs/{job_id}/cancel"), 409, "job-finished")

    # Deleting the job's diff keeps every object: the first diff still uses them.
    reply = client.request("DELETE", job["result"])
    results.check(group, reply.status == 204, f"DELETE job diff: HTTP {reply.status} {reply.code()}")
    results.expect_error(group, "GET job after its diff was deleted", client.request("GET", f"/v1/jobs/{job_id}"), 404, "job-not-found")
    for what, blob in blobs.items():
        results.check(group, store.exists(blob) is True, f"{what} object kept while referenced: {store.exists(blob)}")
    results.check(group, client.request("GET", location).body == saved, "first diff still readable")

    # Deleting the last reference removes the row and every object.
    reply = client.request("DELETE", location)
    results.check(group, reply.status == 204, f"DELETE saved diff: HTTP {reply.status} {reply.code()}")
    results.expect_error(group, "GET deleted diff", client.request("GET", location), 404, "diff-not-found")
    results.expect_error(group, "DELETE deleted diff", client.request("DELETE", location), 404, "diff-not-found")
    for what, blob in blobs.items():
        results.check(group, store.exists(blob) is False, f"{what} object removed: exists={store.exists(blob)}")
    reply = client.post_json(
        "/v1/jobs", {"left": job_input(blobs["left"], "a"), "right": job_input(blobs["right"], "b")}
    )
    results.expect_error(group, "job over a deleted blob", reply, 404, "blob-not-found")
    print("saved: save, get, job over saved sides, shared objects, delete and object removal", flush=True)


# --- Security and errors -----------------------------------------------------------


def test_security(server: hx.Server, client: hx.Client, read_only: str, results: Results) -> None:
    group = "security"
    small = {"left": {"name": "a.txt", "text": "a\n"}, "right": {"name": "b.txt", "text": "b\n"}}
    small_body = json.dumps(small).encode()
    json_type = {"Content-Type": "application/json"}

    health = client.request("GET", "/healthz", key=None)
    results.check(group, health.status == 200 and health.json().get("status") == "ok", f"healthz: {health.status} {health.body[:200]!r}")
    spec = client.request("GET", "/openapi.json", key=None)
    results.check(group, spec.status == 200 and spec.body == OPENAPI.read_bytes(), "GET /openapi.json equals docs/api/openapi.json")
    try:
        version = json.loads(spec.body).get("openapi", "")
    except ValueError:
        version = ""
    results.check(group, str(version).startswith("3.1"), f"OpenAPI version {version!r}")
    formats = client.request("GET", "/v1/formats")
    results.check(group, formats.status == 200 and isinstance(formats.json(), dict), f"GET /v1/formats: {formats.status}")

    # Authentication and scopes.
    results.expect_error(group, "no key", client.request("GET", "/v1/formats", key=None), 401, "unauthorized")
    results.expect_error(group, "unknown key", client.request("GET", "/v1/formats", key="dmk_" + "0" * 64), 401, "unauthorized")
    results.expect_error(
        group, "malformed key", client.request("GET", "/v1/formats", headers={"Authorization": "Basic abc"}, key=None), 401, "unauthorized"
    )
    results.expect_error(group, "no key POST", client.request("POST", "/v1/diff", small_body, json_type, key=None), 401, "unauthorized")
    results.check(group, client.request("GET", "/v1/formats", key=read_only).status == 200, "read-only key can GET")
    results.expect_error(group, "read-only key POST", client.request("POST", "/v1/diff", small_body, json_type, key=read_only), 403, "forbidden")
    results.expect_error(
        group, "read-only key DELETE", client.request("DELETE", f"/v1/diffs/{'0' * 8}-0000-7000-8000-{'0' * 12}", key=read_only), 403, "forbidden"
    )
    key_id, revoked = server.create_key("e2e-revoked")
    results.check(group, client.request("GET", "/v1/formats", key=revoked).status == 200, "new key works")
    listed = server.keys("list")
    results.check(group, listed.returncode == 0 and key_id in listed.stdout and revoked not in listed.stdout, "keys list shows the ID, never the secret")
    results.check(group, server.keys("revoke", key_id).returncode == 0, "keys revoke")
    results.expect_error(group, "revoked key", client.request("GET", "/v1/formats", key=revoked), 401, "unauthorized")

    # Host allowlist (DNS rebinding) and Origin refusal.
    for host in ("evil.example", f"evil.example:{server.port}", f"127.0.0.1:{server.port + 1}", f"localhost.evil:{server.port}"):
        results.expect_error(group, f"Host {host} on /healthz", client.request("GET", "/healthz", headers={"Host": host}, key=None), 403, "host-not-allowed")
        results.expect_error(
            group, f"Host {host} on /v1/diff", client.request("POST", "/v1/diff", small_body, {**json_type, "Host": host}), 403, "host-not-allowed"
        )
    results.check(group, client.request("GET", "/healthz", headers={"Host": f"localhost:{server.port}"}, key=None).status == 200, "Host localhost:<port> is allowed")
    for origin in ("https://evil.example", "null", f"http://127.0.0.1:{server.port + 1}"):
        results.expect_error(
            group, f"Origin {origin}", client.request("POST", "/v1/diff", small_body, {**json_type, "Origin": origin}), 403, "origin-not-allowed"
        )

    # Limits and typed errors.
    limit = 8 * MIB
    big_text = {"left": {"name": "a.txt", "text": "x" * (limit + 1)}, "right": {"name": "b.txt", "text": ""}}
    results.expect_error(group, "input over 8 MiB", client.post_json("/v1/diff", big_text), 413, "input-too-large")
    at_limit = {"left": {"name": "a.txt", "text": "x" * (limit - 1) + "\n"}, "right": {"name": "b.txt", "text": "y\n"}}
    reply = client.post_json("/v1/diff", at_limit)
    results.check(group, reply.status == 200 and judge_document(reply.body, None) is None, f"input of exactly 8 MiB: HTTP {reply.status} {reply.code()}")
    declared = client.headers_only("POST", "/v1/diff", {"Content-Type": "application/json", "Content-Length": str(25 * MIB)})
    results.expect_error(group, "body declared over 24 MiB", declared, 413, "payload-too-large")
    part, content_type = hx.multipart([("left", "a.txt", b"x" * (limit + 1)), ("right", "b.txt", b"")])
    results.expect_error(group, "multipart input over 8 MiB", client.request("POST", "/v1/diff", part, {"Content-Type": content_type}), 413, "input-too-large")
    results.expect_error(group, "text/plain body", client.request("POST", "/v1/diff", b"a", {"Content-Type": "text/plain"}), 415, "unsupported-media-type")
    results.expect_error(group, "invalid JSON", client.request("POST", "/v1/diff", b"{", json_type), 400, "invalid-request")
    results.expect_error(group, "unknown field", client.post_json("/v1/diff", {**small, "extra": 1}), 400, "invalid-request")
    results.expect_error(group, "both text and base64", client.post_json("/v1/diff", {**small, "left": {"text": "a", "base64": "YQ=="}}), 400, "invalid-request")
    results.expect_error(group, "invalid option", client.post_json("/v1/diff", {**small, "options": {"granularity": "nope"}}), 400, "invalid-options")
    results.expect_error(group, "save=maybe", client.request("POST", "/v1/diff?save=maybe", small_body, json_type), 400, "invalid-request")
    results.expect_error(
        group, "blob hash mismatch", client.request("POST", f"/v1/blobs?sha256={'0' * 64}", b"abc", {"Content-Type": "application/octet-stream"}), 400, "hash-mismatch"
    )
    results.expect_error(group, "invalid blob ID", client.post_json("/v1/jobs", {"left": job_input("xyz", "a"), "right": job_input("xyz", "b")}), 400, "invalid-blob-id")
    missing = "f" * 64
    results.expect_error(group, "unknown blob", client.post_json("/v1/jobs", {"left": job_input(missing, "a"), "right": job_input(missing, "b")}), 404, "blob-not-found")
    results.expect_error(group, "job ID not a UUID", client.request("GET", "/v1/jobs/not-a-uuid"), 404, "job-not-found")
    results.expect_error(group, "unknown job", client.request("GET", f"/v1/jobs/{'0' * 8}-0000-7000-8000-{'0' * 12}"), 404, "job-not-found")
    results.expect_error(group, "unknown route", client.request("GET", "/v1/nothing"), 404, "not-found")
    results.expect_error(group, "wrong method", client.request("PUT", "/v1/diff", small_body, json_type), 405, "method-not-allowed")
    print("security: keys, scopes, revocation, Host, Origin, limits and typed errors", flush=True)


# --- Large inputs through a job -------------------------------------------------------


def write_large_pair(folder: pathlib.Path, size: int) -> tuple[pathlib.Path, pathlib.Path]:
    """Two log-like files of about `size` bytes: the right one has one line in
    1000 changed and one in 50000 inserted."""
    folder.mkdir(parents=True, exist_ok=True)
    left_path, right_path = folder / "left.log", folder / "right.log"
    with left_path.open("wb") as left, right_path.open("wb") as right:
        written, i = 0, 0
        while written < size:
            lines_l, lines_r = [], []
            for _ in range(10_000):
                line = f"{i:09d} 2026-11-08T12:{i // 60 % 60:02d}:{i % 60:02d}Z INFO req={i * 2654435761 % 2**32:08x} path=/v1/items/{i % 997} status=200 ms={i % 89}\n"
                lines_l.append(line)
                if i % 1000 == 999:
                    lines_r.append(line.replace("status=200", "status=503"))
                else:
                    lines_r.append(line)
                if i % 50_000 == 49_999:
                    lines_r.append(f"{i:09d} inserted line for the large test\n")
                i += 1
            chunk_l, chunk_r = "".join(lines_l).encode(), "".join(lines_r).encode()
            left.write(chunk_l)
            right.write(chunk_r)
            written += len(chunk_l)
    return left_path, right_path


class RssSampler(threading.Thread):
    def __init__(self, server: hx.Server, interval: float = 0.02) -> None:
        super().__init__(daemon=True)
        self.server = server
        self.interval = interval
        self.peak = 0
        self.done = threading.Event()

    def run(self) -> None:
        while not self.done.is_set():
            rss = self.server.rss()
            if rss:
                self.peak = max(self.peak, rss)
            time.sleep(self.interval)

    def stop(self) -> int:
        self.done.set()
        self.join()
        return self.peak


def test_large(server: hx.Server, client: hx.Client, cli: pathlib.Path | None, size_mb: int, results: Results) -> None:
    group = "large"
    folder = server.workdir / "large-inputs"
    try:
        started = time.monotonic()
        left, right = write_large_pair(folder, size_mb * 1000 * 1000)
        sizes = (left.stat().st_size, right.stat().st_size)
        print(f"large: generated {sizes[0]:,} and {sizes[1]:,} bytes in {time.monotonic() - started:.1f} s", flush=True)
        uploaded = time.monotonic()
        left_blob = upload(client, left, results, group)
        right_blob = upload(client, right, results, group)
        upload_secs = time.monotonic() - uploaded
        if left_blob is None or right_blob is None:
            return
        rss_before = server.rss()
        sampler = RssSampler(server)
        sampler.start()
        queued = time.monotonic()
        reply = client.post_json(
            "/v1/jobs", {"left": job_input(left_blob, left.name), "right": job_input(right_blob, right.name)}
        )
        if not results.check(group, reply.status == 202, f"create: HTTP {reply.status} {reply.code()} {reply.body[:200]!r}"):
            sampler.stop()
            return
        job = wait_job(client, reply.json()["id"], timeout=900, interval=0.02)
        job_secs = time.monotonic() - queued
        peak = sampler.stop()
        if not results.check(group, job.get("status") == "succeeded", f"job {job.get('status')} {job.get('error')}"):
            return
        fetched = time.monotonic()
        document = client.request("GET", job["result"])
        fetch_secs = time.monotonic() - fetched
        problem = f"HTTP {document.status}" if document.status != 200 else judge_document(document.body, None)
        results.check(group, problem is None, f"result: {problem}")
        summary = json.loads(document.body) if document.status == 200 else {}
        measurement = {
            "input_bytes": list(sizes),
            "upload_seconds": round(upload_secs, 2),
            "job_seconds": round(job_secs, 2),
            "result_bytes": len(document.body),
            "result_fetch_seconds": round(fetch_secs, 2),
            "status": summary.get("status"),
            "stats": {k: summary.get("stats", {}).get(k) for k in ("linesLeft", "linesAdded", "linesRemoved", "hunkCount")},
            "server_rss_before_bytes": rss_before,
            "server_peak_rss_during_job_bytes": peak,
        }
        if cli is not None:
            cli_started = time.monotonic()
            proc = subprocess.run([str(cli), "diff", "--json", str(left), str(right)], capture_output=True, check=False)
            measurement["cli_seconds"] = round(time.monotonic() - cli_started, 2)
            same = (
                proc.returncode in (0, 1)
                and document.status == 200
                and run_bindings.conformance_bytes(json.loads(proc.stdout)) == run_bindings.conformance_bytes(summary)
            )
            results.check(group, same, f"result equals diffyman diff --json (exit {proc.returncode})")
            measurement["equals_cli"] = same
        results.measurements["large"] = measurement
        print(
            f"large: job {job_secs:.2f} s, peak RSS during the job {peak / MIB:.0f} MiB "
            f"(before {(rss_before or 0) / MIB:.0f} MiB), result {len(document.body):,} bytes",
            flush=True,
        )
    finally:
        shutil.rmtree(folder, ignore_errors=True)


# --- Throughput ---------------------------------------------------------------------


def test_bench(client: hx.Client, seconds: float, results: Results) -> None:
    lines = [f"line {i}: the quick brown fox jumps over the lazy dog\n" for i in range(100)]
    changed = list(lines)
    changed[10] = "line 10: the quick brown cat jumps over the lazy dog\n"
    changed[60:61] = []
    changed.insert(80, "an inserted line\n")
    body = json.dumps(
        {
            "left": {"name": "old.txt", "text": "".join(lines)},
            "right": {"name": "new.txt", "text": "".join(changed)},
            "options": {"granularity": "word"},
        }
    ).encode()
    headers = {"Content-Type": "application/json"}
    for _ in range(20):  # warm-up
        client.request("POST", "/v1/diff", body, headers)
    latencies = []
    errors = 0
    end = time.perf_counter() + seconds
    started = time.perf_counter()
    while (now := time.perf_counter()) < end:
        reply = client.request("POST", "/v1/diff", body, headers)
        latencies.append(time.perf_counter() - now)
        errors += reply.status != 200
    elapsed = time.perf_counter() - started
    latencies.sort()
    q = statistics.quantiles(latencies, n=100, method="inclusive")
    bench = {
        "seconds": round(elapsed, 2),
        "requests": len(latencies),
        "requests_per_second": round(len(latencies) / elapsed, 1),
        "p50_ms": round(q[49] * 1000, 3),
        "p99_ms": round(q[98] * 1000, 3),
        "max_ms": round(latencies[-1] * 1000, 3),
        "errors": errors,
        "request_bytes": len(body),
    }
    results.check("bench", errors == 0, f"{errors} non-200 replies")
    results.measurements["bench"] = bench
    print(
        f"bench: {bench['requests_per_second']} requests/s over {elapsed:.1f} s, p50 {bench['p50_ms']} ms, "
        f"p99 {bench['p99_ms']} ms",
        flush=True,
    )


# --- Logs -----------------------------------------------------------------------------


def test_logs(server: hx.Server, forbidden: dict[str, str], results: Results) -> None:
    text = server.log_text()
    results.check("logs", "listening" in text, "the log has the start-up line")
    for what, value in forbidden.items():
        results.check("logs", value not in text, f"the log contains {what}")
    results.measurements["log_lines"] = len(text.splitlines())
    print(f"logs: {len(text.splitlines())} lines, none with content, names, paths or secrets", flush=True)


# --- Main -----------------------------------------------------------------------------


def free_gb(path: pathlib.Path) -> float:
    return shutil.disk_usage(path).free / 1e9


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--server", type=pathlib.Path, help="the diffyman-server binary (default: newest build)")
    parser.add_argument("--cli", type=pathlib.Path, help="the diffyman CLI, to compare the large result with")
    parser.add_argument("--database", default="diffyman_e2e", help="test database, dropped and created (default diffyman_e2e)")
    parser.add_argument("--store-url", help="blob store URL (default: file:// in the working folder)")
    parser.add_argument("--workdir-root", type=pathlib.Path, default=hx.default_workdir_root())
    parser.add_argument("--cases", type=int, help="only this many corpus cases, evenly spread (default: all)")
    parser.add_argument("--jobs", type=int, default=30, help="corpus cases also run as jobs (default 30)")
    parser.add_argument("--large-mb", type=int, default=200, help="size of each large input in MB (0: skip; default 200)")
    parser.add_argument("--bench-secs", type=float, default=10, help="length of the throughput run (0: skip; default 10)")
    parser.add_argument("--min-free-gb", type=float, default=15, help="refuse to run with less free disk (default 15)")
    parser.add_argument("--keep", action="store_true", help="keep the database and working folder (for a restore drill)")
    parser.add_argument("--report", type=pathlib.Path, help="write the measurements and results as JSON here")
    args = parser.parse_args()

    try:
        pg = hx.PgServer.find()
        pg.check()
    except hx.Skip as why:
        print(f"skipping the server end-to-end test: {why}", file=sys.stderr)
        return 1 if hx.db_required() else 0
    args.workdir_root.mkdir(parents=True, exist_ok=True)
    free = free_gb(args.workdir_root)
    if free < args.min_free_gb:
        print(f"only {free:.1f} GB free under {args.workdir_root}; need {args.min_free_gb} GB", file=sys.stderr)
        return 1
    binary = args.server or hx.default_server()
    cli = args.cli.resolve() if args.cli else None
    workdir = pathlib.Path(tempfile.mkdtemp(prefix="e2e-", dir=args.workdir_root))
    server = hx.Server(binary, workdir, pg, args.database, args.store_url)
    results = Results()
    marker = f"e2emarker{secrets.token_hex(8)}"
    print(f"server: {server.binary} on {server.base_url}, database {args.database}, store {server.store_url.split(':')[0]}://", flush=True)
    keys: dict[str, str] = {}
    try:
        server.start()
        started_at = time.monotonic()
        _, keys["main"] = server.create_key("e2e")
        _, keys["read"] = server.create_key("e2e-read", "read")
        _, keys["other"] = server.create_key("e2e-other")
        client = hx.Client(server.port, keys["main"])
        other = hx.Client(server.port, keys["other"])
        time.sleep(max(0.0, 2 - (time.monotonic() - started_at)))
        results.measurements["server_idle_rss_bytes"] = server.rss()
        print(f"idle RSS: {(server.rss() or 0) / MIB:.1f} MiB", flush=True)

        cases = corpus_cases(args.cases)
        test_corpus(client, cases, results)
        if args.jobs:
            test_jobs(client, spread(cases, args.jobs), results)
        test_saved(client, other, ObjectStore(server), marker, results)
        test_security(server, client, keys["read"], results)
        if args.large_mb:
            test_large(server, client, cli, args.large_mb, results)
        if args.bench_secs:
            test_bench(client, args.bench_secs, results)
        results.measurements["server_rss_after_bytes"] = server.rss()
        client.close()
        other.close()
    finally:
        server.stop()
    results.check("shutdown", server.exit_code == 0, f"graceful stop exit code {server.exit_code}")
    results.measurements["server_lifetime_peak_rss_bytes"] = server.max_rss
    forbidden = {
        "input content": marker,
        "the corpus path": str(CORPUS),
        "a large input path": str(workdir / "large-inputs"),
        "the database password": pg.password,
        **{f"the {name} API key": secret for name, secret in keys.items()},
    }
    forbidden = {k: v for k, v in forbidden.items() if v}
    test_logs(server, forbidden, results)
    if args.keep:
        try:
            results.measurements["rows"] = pg.counts(args.database, TABLES)
        except RuntimeError as error:
            results.check("rows", False, str(error))
        print(f"kept database {args.database} and {workdir} (blob store {server.store_dir})")
    server.cleanup(keep=args.keep)

    total = sum(results.checks.values())
    print()
    print("checks: " + ", ".join(f"{group} {n}" for group, n in results.checks.items()))
    print(json.dumps(results.measurements, indent=2))
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        report = {"checks": dict(results.checks), "failures": results.failures, "measurements": results.measurements}
        args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if results.failures:
        print(f"{len(results.failures)} of {total} checks FAILED")
        return 1
    print(f"all {total} checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
