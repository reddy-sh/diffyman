"""Acceptance tests for the documented size, time and concurrency limits.

Each group starts its own `diffyman-server serve` (tests/server/harness.py: a
test database, a fresh blob folder, a random port on 127.0.0.1) with the
limits it checks set low enough to reach quickly, and every test is named
after the rule of docs/api/README.md ("Limits", "Errors") it checks:

  requests  bodies over the body, blob and job-request limits (declared and
            chunked), a Content-Length that understates the body, multipart
            bodies with thousands of parts, bodies and headers sent at one
            byte a second, while other clients keep being served.
  sync      40 parallel synchronous diffs of large inputs: each succeeds or
            gets `503 busy`, the peak resident memory stays within
            DIFFYMAN_SYNC_MEMORY_BYTES of the idle server, /healthz stays ok.
  jobs      archives built here from zeros that expand far beyond their size
            (a zip entry over the ratio limit, one that inflates past its
            declared size, a compressed tar, nested zips that fan out) and
            zips nested deeper than tree.maxDepth, run as jobs: each ends with
            the outcome spec §19.6 documents, within the job memory budget;
            a job cancelled right after it starts ends `cancelled`.

rawhttp.py sends requests with the framing under the test's control and
watches the server; archives.py builds the archives. Generated inputs go to
the working folder (under ~/diffyman-data/tmp/limits
when ~/diffyman-data/tmp exists) and are deleted with it. `--scale ci` uses
smaller inputs. Without a database it prints why and exits 0 (1 with
DIFFYMAN_TEST_DB_REQUIRED=1). Run from the repository root:

  uv run --no-project --python 3.12 python tests/server/test_limits.py \\
      --server target/release/diffyman-server
"""

from __future__ import annotations

import argparse
import hashlib
import itertools
import json
import pathlib
import shutil
import sys
import tempfile
import threading
import time
from collections.abc import Callable, Iterator
from dataclasses import dataclass, field
from typing import Any

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import harness as hx  # noqa: E402
from archives import chain_zip, deflated_entry, fanout_zip, tar_gz, warning_codes, zeros, zip_archive  # noqa: E402
from rawhttp import Answer, Monitor, RssSampler, chunked, exchange, head  # noqa: E402

KIB, MIB, GIB = 1 << 10, 1 << 20, 1 << 30
#: Allocator, connection buffers and threads on top of a configured budget.
SLACK = 64 * MIB


@dataclass(frozen=True)
class Scale:
    sync_requests: int
    sync_input_bytes: int
    sync_memory_bytes: int
    bomb_bytes: int
    fanout: tuple[int, int, int]  # width, levels, leaf bytes
    chain_depth: int
    job_timeout_secs: int


SCALES = {
    "full": Scale(40, 8 * MIB - 4 * KIB, 256 * MIB, 2 * GIB, (16, 4, 1 * MIB), 64, 180),
    "ci": Scale(40, 2 * MIB, 64 * MIB, 256 * MIB, (8, 4, 256 * KIB), 24, 120),
}


def expect(ok: bool, what: str) -> None:
    if not ok:
        raise AssertionError(what)


# --- Test context -------------------------------------------------------------------


@dataclass
class Ctx:
    server: hx.Server
    client: hx.Client
    key: str
    env: dict[str, str]
    limits: dict[str, int]
    scale: Scale
    inputs: pathlib.Path
    idle_rss: int
    measure: dict[str, Any] = field(default_factory=dict)

    @property
    def port(self) -> int:
        return self.server.port

    def head(self, method: str, path: str, headers: dict[str, str] | None = None) -> bytes:
        return head(self.port, method, path, {"Authorization": f"Bearer {self.key}", **(headers or {})})

    def healthy(self) -> None:
        expect(self.server.proc is not None and self.server.proc.poll() is None, "the server process exited")
        reply = hx.Client(self.port, timeout=10).request("GET", "/healthz", key=None)
        expect(reply.status == 200 and reply.json().get("status") == "ok", f"/healthz: {reply.status} {reply.body[:120]!r}")


def json_body(text_bytes: int, pad_to: int = 0) -> bytes:
    body = json.dumps({"left": {"name": "a.txt", "text": "x" * text_bytes}, "right": {"name": "b.txt", "text": ""}}).encode()
    return body + b" " * max(0, pad_to - len(body))


def mib(n: float) -> float:
    return round(n / MIB, 1)


# --- Request size --------------------------------------------------------------------


def test_body_over_limit_returns_413(ctx: Ctx) -> None:
    limit = ctx.limits["maxBodyBytes"]
    body = json_body(16, pad_to=limit + 1)
    answer = exchange(
        ctx.port, ctx.head("POST", "/v1/diff", {"Content-Type": "application/json", "Content-Length": str(len(body))}), [body]
    )
    expect((answer.status, answer.code()) == (413, "payload-too-large"), f"body of limit + 1: {answer.show()}")
    ctx.measure["over_limit"] = {"seconds": round(answer.seconds, 3), "body_bytes_sent": answer.sent}


def test_body_at_limit_is_accepted(ctx: Ctx) -> None:
    body = json_body(16, pad_to=ctx.limits["maxBodyBytes"])
    reply = ctx.client.request("POST", "/v1/diff", body, {"Content-Type": "application/json"})
    expect(reply.status == 200, f"body of exactly the limit: {reply.status} {reply.code()}")


def test_understated_content_length_reads_only_the_declared_bytes(ctx: Ctx) -> None:
    full = json.dumps({"left": {"text": "a\n"}, "right": {"text": "b\n"}}).encode()
    start = ctx.head("POST", "/v1/diff", {"Content-Type": "application/json", "Content-Length": str(len(full) // 2)})
    answer = exchange(ctx.port, start, [full], timeout=5, all_answers=True)
    expect((answer.status, answer.code()) == (400, "invalid-request"), f"truncated JSON: {answer.show()}")
    expect(all(not 200 <= s < 300 for s, _ in answer.more), f"the rest of the body was answered: {answer.more}")
    ctx.measure["the bytes past Content-Length"] = [f"{s} {b[:60]!r}" for s, b in answer.more] or "connection closed"
    data = bytes(range(256)) * 256
    declared = len(data) // 2
    for query, status, code in ((f"?sha256={hashlib.sha256(data).hexdigest()}", 400, "hash-mismatch"), ("", 201, None)):
        start = ctx.head(
            "POST", f"/v1/blobs{query}", {"Content-Type": "application/octet-stream", "Content-Length": str(declared)}
        )
        answer = exchange(ctx.port, start, [data], timeout=5, all_answers=True)
        expect(answer.status == status and answer.code() == code, f"upload {query or 'without sha256'}: {answer.show()}")
        if status == 201:
            stored = json.loads(answer.body)
            expect(stored.get("size") == declared, f"stored {stored.get('size')} bytes, declared {declared}")
            expect(stored.get("blobId") == hashlib.sha256(data[:declared]).hexdigest(), "stored other bytes than declared")
        expect(all(not 200 <= s < 300 for s, _ in answer.more), f"the rest of the upload was answered: {answer.more}")


def test_body_of_unknown_length_reserves_the_body_limit(ctx: Ctx) -> None:
    """A diff reserves four bytes per body byte before reading it, the body
    limit when the length is not known. Under chunked framing the length is
    not known, whatever a Content-Length beside it says (RFC 9112 §6.3), so a
    held chunked body takes the whole 1 MiB budget and a second diff waits
    half the request time limit, then gets `503 busy`; refusing the
    ambiguous framing outright is fine too."""
    limit = ctx.limits["requestTimeoutSecs"]
    expect(
        4 * ctx.limits["maxBodyBytes"] >= int(ctx.env["DIFFYMAN_SYNC_MEMORY_BYTES"]),
        "the budget must be below the body reservation",
    )
    for label, extra in (("chunked", {}), ("chunked with Content-Length: 1", {"Content-Length": "1"})):
        start = ctx.head("POST", "/v1/diff", {"Content-Type": "application/json", "Transfer-Encoding": "chunked", **extra})
        held: list[Answer] = []
        body = chunked([b"{"] + [b" "] * 20)
        thread = threading.Thread(
            target=lambda s=start, b=body, h=held: h.append(exchange(ctx.port, s, b, pace=1.0, timeout=limit + 30))
        )
        thread.start()
        time.sleep(1)
        started = time.monotonic()
        reply = ctx.client.post_json("/v1/diff", {"left": {"text": "a\n"}, "right": {"text": "b\n"}})
        waited = time.monotonic() - started
        thread.join()
        first = held[0]
        refused = first.status == 400 and first.seconds < 1
        ctx.measure[label] = {"held": first.show(), "second diff": f"{reply.status} {reply.code()} after {waited:.2f} s"}
        expect(
            refused or (reply.status, reply.code()) == (503, "busy"),
            f"{label}: a second diff got {reply.status} {reply.code()} after {waited:.2f} s while the first body was held ({first.show()})",
        )


def grow(first: bytes, total: int, piece: bytes) -> Iterator[bytes]:
    yield first
    for _ in range(total // len(piece)):
        yield piece


def test_chunked_body_past_limit_returns_413(ctx: Ctx) -> None:
    body_limit, blob_limit = ctx.limits["maxBodyBytes"], ctx.limits["maxBlobBytes"]
    pad = b" " * (64 * KIB)
    part = f'--{hx.BOUNDARY}\r\nContent-Disposition: form-data; name="left"; filename="a.bin"\r\n\r\n'.encode()
    content_type = f"multipart/form-data; boundary={hx.BOUNDARY}"
    cases = [
        ("JSON to /v1/diff", "/v1/diff", "application/json", grow(b'{"left": ', 2 * body_limit, pad), {"payload-too-large"}),
        (
            "multipart to /v1/diff",
            "/v1/diff",
            content_type,
            grow(part, 2 * body_limit, bytes(64 * KIB)),
            {"payload-too-large", "input-too-large"},
        ),
        ("job request", "/v1/jobs", "application/json", grow(b"{", 2 * MIB + 1, pad), {"payload-too-large"}),
        ("upload", "/v1/blobs", "application/octet-stream", grow(b"", 2 * blob_limit, bytes(64 * KIB)), {"blob-too-large"}),
    ]
    for what, path, media, pieces, codes in cases:
        start = ctx.head("POST", path, {"Content-Type": media, "Transfer-Encoding": "chunked"})
        answer = exchange(ctx.port, start, chunked(pieces))
        expect(answer.status == 413 and answer.code() in codes, f"chunked {what}: {answer.show()}")
        ctx.measure[what] = {"code": answer.code(), "seconds": round(answer.seconds, 3), "sent_mib": mib(answer.sent)}


def test_multipart_with_thousands_of_parts_is_rejected(ctx: Ctx) -> None:
    variants = {
        "left repeated": [("left", "a.txt", b"0123456789")] * 5000,
        "unknown fields": [(f"part{i}", None, b"x") for i in range(5000)],
        "options repeated": [("options", None, b"{}")] * 5000,
        "after both inputs": [("left", "a.txt", b"a"), ("right", "b.txt", b"b")] + [("left", "c.txt", b"c")] * 20000,
    }
    nameless = b"".join(f"--{hx.BOUNDARY}\r\nContent-Type: text/plain\r\n\r\nx\r\n".encode() for _ in range(5000))
    bodies = {what: hx.multipart(parts)[0] for what, parts in variants.items()}
    bodies["without names"] = nameless + f"--{hx.BOUNDARY}--\r\n".encode()
    for what, body in bodies.items():
        start = ctx.head(
            "POST", "/v1/diff", {"Content-Type": f"multipart/form-data; boundary={hx.BOUNDARY}", "Content-Length": str(len(body))}
        )
        answer = exchange(ctx.port, start, [body], timeout=30)
        expect(answer.status is not None and 400 <= answer.status < 500 and answer.code() is not None, f"{what}: {answer.show()}")
        ctx.measure[what] = {
            "body_kib": len(body) // KIB,
            "answer": f"{answer.status} {answer.code()}",
            "ms": round(answer.seconds * 1000, 1),
        }
    ctx.healthy()


# --- Slow clients ---------------------------------------------------------------------


def one_byte_a_second(ctx: Ctx, path: str, media: str, length: int = 400) -> Answer:
    start = ctx.head("POST", path, {"Content-Type": media, "Content-Length": str(length)})
    pieces = itertools.chain([b"{"], itertools.repeat(b" ", length - 1))
    return exchange(ctx.port, start, pieces, pace=1.0, timeout=ctx.limits["requestTimeoutSecs"] + 30)


def test_slow_body_is_ended_within_the_request_timeout(ctx: Ctx) -> None:
    limit = ctx.limits["requestTimeoutSecs"]
    monitor = Monitor(ctx.port, ctx.key)
    monitor.start()
    answers: dict[str, Answer] = {}
    threads = [
        threading.Thread(target=lambda p=p, m=m: answers.__setitem__(p, one_byte_a_second(ctx, p, m)))
        for p, m in (("/v1/diff", "application/json"), ("/v1/blobs", "application/octet-stream"))
    ]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    others = monitor.check()
    for path, answer in answers.items():
        expect((answer.status, answer.code()) == (503, "timeout"), f"slow body to {path}: {answer.show()}")
        expect(
            limit - 1 <= answer.seconds <= limit + 2, f"slow body to {path} ended after {answer.seconds:.1f} s, limit {limit} s"
        )
        ctx.measure[path] = {"seconds": round(answer.seconds, 2), "body_bytes_sent": answer.sent}
    ctx.measure["other_clients"] = others


def test_slow_bodies_on_every_diff_slot_leave_other_routes_served(ctx: Ctx) -> None:
    slots = int(ctx.env["DIFFYMAN_MAX_PARALLEL_DIFFS"])
    limit = ctx.limits["requestTimeoutSecs"]
    answers: list[Answer] = []
    # Larger than the 64 KiB the server reads before a request is handled,
    # so each slow body reaches the diff and holds a slot.
    length = 64 * KIB + 1
    threads = [
        threading.Thread(target=lambda: answers.append(one_byte_a_second(ctx, "/v1/diff", "application/json", length)))
        for _ in range(slots + 1)
    ]
    for thread in threads:
        thread.start()
    time.sleep(1.5)
    client = hx.Client(ctx.port, ctx.key, timeout=limit + 10)
    checks = {
        "healthz": lambda: client.request("GET", "/healthz", key=None),
        "formats": lambda: client.request("GET", "/v1/formats"),
        "upload": lambda: client.request("POST", "/v1/blobs", b"slot test\n", {"Content-Type": "text/plain"}),
    }
    for what, call in checks.items():
        started = time.monotonic()
        reply = call()
        seconds = time.monotonic() - started
        expect(
            reply.status in (200, 201) and seconds < 2,
            f"{what} while every diff slot is held: {reply.status} after {seconds:.1f} s",
        )
    started = time.monotonic()
    diff = client.post_json("/v1/diff", {"left": {"text": "a\n"}, "right": {"text": "b\n"}})
    waited = time.monotonic() - started
    expect(
        diff.status == 200 or (diff.status, diff.code()) == (503, "busy"),
        f"diff while every slot is held: {diff.status} {diff.code()}",
    )
    if diff.status == 503:
        expect(
            "retry-after" in diff.headers and limit / 2 - 1 <= waited <= limit / 2 + 2,
            f"busy after {waited:.1f} s, without Retry-After or not after half the limit",
        )
    for thread in threads:
        thread.join()
    statuses = sorted(f"{a.status} {a.code()}" for a in answers)
    expect(all(a.status == 503 and a.code() in ("timeout", "busy") for a in answers), f"slow bodies: {statuses}")
    ctx.measure.update(
        {
            "slots": slots,
            "diff_status": f"{diff.status} {diff.code()}",
            "diff_waited_s": round(waited, 2),
            "slow_bodies": statuses,
        }
    )
    client.close()


def test_slow_headers_close_the_connection_within_the_header_timeout(ctx: Ctx) -> None:
    limit = int(ctx.env["DIFFYMAN_HEADER_READ_TIMEOUT_SECS"])
    text = f"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1:{ctx.port}\r\nX-Slow: " + "a" * 60
    answer = exchange(ctx.port, b"", [c.encode() for c in text], pace=1.0, timeout=limit + 30)
    expect(answer.status is None or answer.status >= 400, f"slow headers answered: {answer.show()}")
    expect(answer.seconds <= limit + 2, f"slow headers kept the connection {answer.seconds:.1f} s, limit {limit} s")
    ctx.measure["closed_after_s"] = round(answer.seconds, 2)


# --- Concurrency ---------------------------------------------------------------------------


def log_text(size: int, change_every: int) -> tuple[bytes, bytes]:
    left, right, i = [], [], 0
    total = 0
    while total < size - 200:
        line = (
            f"{i:08d} 2026-11-08T12:00:{i % 60:02d}Z INFO req={i * 2654435761 % 2**32:08x} path=/v1/items/{i % 997} ms={i % 89}\n"
        )
        left.append(line)
        right.append(line.replace("INFO", "WARN") if i % change_every == 0 else line)
        total += len(line)
        i += 1
    return "".join(left).encode(), "".join(right).encode()


def test_parallel_sync_diffs_succeed_or_get_busy_within_the_memory_budget(ctx: Ctx) -> None:
    """Synchronous diffs share DIFFYMAN_SYNC_MEMORY_BYTES (four bytes per body
    byte, reserved before the body is read), so the server's resident memory
    stays within that budget of its idle size, plus SLACK; a request that
    cannot be admitted within half the request time limit gets `503 busy`."""
    left, right = log_text(ctx.scale.sync_input_bytes, 100)
    body, content_type = hx.multipart([("left", "old.log", left), ("right", "new.log", right)])
    start = ctx.head("POST", "/v1/diff", {"Content-Type": content_type, "Content-Length": str(len(body))})
    budget = int(ctx.env["DIFFYMAN_SYNC_MEMORY_BYTES"])
    bound = ctx.idle_rss + budget + SLACK
    answers: list[Answer] = []
    sampler, monitor = RssSampler(ctx.server), Monitor(ctx.port, None)
    sampler.start()
    monitor.start()
    started = time.monotonic()
    threads = [
        threading.Thread(target=lambda: answers.append(exchange(ctx.port, start, [body], timeout=90)))
        for _ in range(ctx.scale.sync_requests)
    ]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    seconds = time.monotonic() - started
    peak = sampler.stop()
    others = monitor.check(max_secs=2.5)
    outcomes: dict[str, int] = {}
    for answer in answers:
        name = "200" if answer.status == 200 else f"{answer.status} {answer.code()}"
        outcomes[name] = outcomes.get(name, 0) + 1
    ctx.measure.update(
        {
            "requests": len(answers),
            "body_mib": mib(len(body)),
            "outcomes": outcomes,
            "seconds": round(seconds, 2),
            "slowest_s": round(max(a.seconds for a in answers), 2),
            "idle_rss_mib": mib(ctx.idle_rss),
            "peak_rss_mib": mib(peak),
            "bound_mib": mib(bound),
            "healthz": others,
        }
    )
    expect(set(outcomes) <= {"200", "503 busy"}, f"outcomes {outcomes}")
    expect(all("retry-after" in a.headers for a in answers if a.status == 503), "a busy answer without Retry-After")
    expect(outcomes.get("200", 0) > 0, "no diff succeeded")
    expect(peak <= bound, f"peak RSS {mib(peak)} MiB over the bound {mib(bound)} MiB")


# --- Archives and jobs ------------------------------------------------------------------------


def upload_file(ctx: Ctx, path: pathlib.Path) -> str:
    with path.open("rb") as stream:
        reply = ctx.client.request(
            "POST", "/v1/blobs", stream, {"Content-Type": "application/octet-stream", "Content-Length": str(path.stat().st_size)}
        )
    expect(reply.status in (200, 201), f"upload {path.name}: {reply.status} {reply.code()}")
    return str(reply.json()["blobId"])


def write_input(ctx: Ctx, name: str, data: bytes | Callable[[pathlib.Path], None]) -> pathlib.Path:
    ctx.inputs.mkdir(parents=True, exist_ok=True)
    path = ctx.inputs / name
    if isinstance(data, bytes):
        path.write_bytes(data)
    else:
        data(path)
    return path


def create_job(ctx: Ctx, left: str, right: str, options: dict[str, Any] | None = None) -> str:
    body: dict[str, Any] = {"left": {"blob": left, "name": "left.bin"}, "right": {"blob": right, "name": "right.bin"}}
    if options:
        body["options"] = options
    reply = ctx.client.post_json("/v1/jobs", body)
    expect(reply.status == 202, f"create job: {reply.status} {reply.code()} {reply.body[:160]!r}")
    return str(reply.json()["id"])


def wait_job(
    ctx: Ctx, job_id: str, timeout: float, until: tuple[str, ...] = ("succeeded", "failed", "cancelled")
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    while True:
        job = ctx.client.request("GET", f"/v1/jobs/{job_id}").json()
        if job.get("status") in until or time.monotonic() > deadline:
            return job
        time.sleep(0.02)


def run_job(
    ctx: Ctx, left: pathlib.Path, right: pathlib.Path, options: dict[str, Any] | None = None
) -> tuple[dict[str, Any], Any]:
    """Uploads both files, runs a job on them and checks its memory; returns the
    job and its document (None unless it succeeded)."""
    left_id, right_id = upload_file(ctx, left), upload_file(ctx, right)
    sampler, monitor = RssSampler(ctx.server), Monitor(ctx.port, None)
    sampler.start()
    monitor.start()
    started = time.monotonic()
    job = wait_job(ctx, create_job(ctx, left_id, right_id, options), ctx.scale.job_timeout_secs + 30)
    seconds = time.monotonic() - started
    peak = sampler.stop()
    others = monitor.check(max_secs=2.5)
    bound = ctx.idle_rss + int(ctx.env["DIFFYMAN_JOB_MEMORY_BYTES"])
    document = None
    if job.get("status") == "succeeded":
        reply = ctx.client.request("GET", job["result"])
        expect(reply.status == 200, f"result: {reply.status}")
        document = reply.json()
    ctx.measure.update(
        {
            "input_bytes": [left.stat().st_size, right.stat().st_size],
            "job": job.get("status"),
            "error": job.get("error"),
            "seconds": round(seconds, 2),
            "peak_rss_mib": mib(peak),
            "bound_mib": mib(bound),
            "healthz_max_ms": others["max_ms"],
            "status": document and document.get("status"),
            "warnings": sorted(warning_codes(document)) if document else None,
        }
    )
    ctx.healthy()
    expect(peak <= bound, f"peak RSS {mib(peak)} MiB over the job memory bound {mib(bound)} MiB")
    return job, document


def expect_partial(job: dict[str, Any], document: Any, codes: set[str]) -> None:
    expect(job.get("status") == "succeeded", f"job {job.get('status')} {job.get('error')}")
    found = warning_codes(document)
    expect(document.get("status") == "partial", f"document status {document.get('status')}, warnings {sorted(found)}")
    expect(bool(found & codes), f"warnings {sorted(found)}, expected one of {sorted(codes)}")


def test_zip_entry_over_the_ratio_limit_is_not_inflated(ctx: Ctx) -> None:
    size = ctx.scale.bomb_bytes
    left = write_input(ctx, "ratio-left.zip", zip_archive([deflated_entry("zeros.bin", zeros(size))]))
    right = write_input(ctx, "ratio-right.zip", zip_archive([deflated_entry("zeros.bin", zeros(size + MIB))]))
    job, document = run_job(ctx, left, right)
    expect_partial(job, document, {"tree-limit"})


def test_zip_entry_inflating_past_its_declared_size_is_not_read_past_it(ctx: Ctx) -> None:
    lie = MIB  # not over 1 MiB, so the ratio limit lets it be inflated
    left = write_input(ctx, "lie-left.zip", zip_archive([deflated_entry("zeros.bin", zeros(ctx.scale.bomb_bytes), declared=lie)]))
    right = write_input(
        ctx, "lie-right.zip", zip_archive([deflated_entry("zeros.bin", zeros(ctx.scale.bomb_bytes + MIB), declared=lie)])
    )
    job, document = run_job(ctx, left, right)
    expect_partial(job, document, {"entry-unreadable", "tree-limit"})


def test_compressed_tar_over_the_ratio_limit_stops_reading(ctx: Ctx) -> None:
    size = ctx.scale.bomb_bytes
    left = write_input(ctx, "left.tar.gz", lambda p: tar_gz(p, "zeros.bin", size))
    right = write_input(ctx, "right.tar.gz", lambda p: tar_gz(p, "zeros.bin", size + MIB))
    job, document = run_job(ctx, left, right)
    expect_partial(job, document, {"tree-limit"})


def test_nested_zip_fanout_stays_within_the_content_and_child_budgets(ctx: Ctx) -> None:
    """Spec §19.5 and §19.6: content read from archives is at most 4 GiB per
    side and child documents at most `tree.childBudget` bytes, so a fan-out
    over the content budget ends with `tree-limit` or `children-omitted` (or
    the job time budget, `job-timeout`), and a small child budget omits
    children at any size."""
    width, levels, leaf = ctx.scale.fanout
    left = write_input(ctx, "fan-left.zip", fanout_zip(width, levels, bytes(leaf)))
    right = write_input(ctx, "fan-right.zip", fanout_zip(width, levels, b"\x01" * leaf))
    expands = width**levels * leaf
    runs = {}
    for label, options, codes in (
        ("default budgets", None, {"children-omitted", "tree-limit"} if expands > 4 * GIB else set()),
        ("childBudget 65536", {"tree": {"childBudget": 65536}}, {"children-omitted"}),
    ):
        job, document = run_job(ctx, left, right, options)
        runs[label] = {k: ctx.measure[k] for k in ("seconds", "peak_rss_mib", "status", "warnings", "error")}
        if job.get("status") == "failed":
            expect((job.get("error") or {}).get("code") == "job-timeout", f"{label}: job failed: {job.get('error')}")
            continue
        expect(job.get("status") == "succeeded", f"{label}: job {job.get('status')} {job.get('error')}")
        found = warning_codes(document)
        expect(not codes or bool(found & codes), f"{label}: warnings {sorted(found)}, expected one of {sorted(codes)}")
    ctx.measure.clear()
    ctx.measure.update(
        {"expands_to_gib_per_side": round(expands / GIB, 1), "input_bytes": [left.stat().st_size, right.stat().st_size], **runs}
    )


def test_zips_nested_deeper_than_max_depth_are_compared_as_bytes(ctx: Ctx) -> None:
    depth = ctx.scale.chain_depth
    left = write_input(ctx, "chain-left.zip", chain_zip(depth, b"left\n"))
    right = write_input(ctx, "chain-right.zip", chain_zip(depth, b"right\n"))
    for max_depth in (None, 16):
        job, document = run_job(ctx, left, right, {"tree": {"maxDepth": max_depth}} if max_depth else None)
        expect(job.get("status") == "succeeded", f"maxDepth {max_depth}: job {job.get('status')} {job.get('error')}")
        expect("depth-limit" in warning_codes(document), f"maxDepth {max_depth}: warnings {sorted(warning_codes(document))}")
        ctx.measure[f"maxDepth {max_depth or 3}"] = {k: ctx.measure[k] for k in ("seconds", "peak_rss_mib", "status")}


def test_job_inputs_over_the_memory_budget_return_413(ctx: Ctx) -> None:
    limit = ctx.limits["maxJobInputBytes"]
    path = write_input(ctx, "half.bin", lambda p: p.write_bytes(bytes(limit // 2 + 1)))
    blob = upload_file(ctx, path)
    reply = ctx.client.post_json("/v1/jobs", {"left": {"blob": blob}, "right": {"blob": blob}})
    expect((reply.status, reply.code()) == (413, "job-too-large"), f"inputs of limit + 2: {reply.status} {reply.code()}")


def test_job_cancelled_right_after_it_starts_ends_cancelled(ctx: Ctx) -> None:
    width, levels, leaf = ctx.scale.fanout
    left = upload_file(ctx, write_input(ctx, "fan-left.zip", fanout_zip(width, levels, bytes(leaf))))
    right = upload_file(ctx, write_input(ctx, "fan-right.zip", fanout_zip(width, levels, b"\x01" * leaf)))
    job_id = create_job(ctx, left, right)
    running = wait_job(ctx, job_id, 30, until=("running", "succeeded", "failed", "cancelled"))
    expect(running.get("status") == "running", f"job never ran: {running.get('status')}")
    started = time.monotonic()
    reply = ctx.client.request("POST", f"/v1/jobs/{job_id}/cancel")
    expect(reply.status == 202, f"cancel of a running job: {reply.status} {reply.code()}")
    job = wait_job(ctx, job_id, ctx.scale.job_timeout_secs)
    ended = time.monotonic() - started
    expect(job.get("status") == "cancelled", f"cancelled job ended {job.get('status')} {job.get('error')}")
    expect(job.get("result") is None and job.get("diffId") is None, f"a cancelled job kept a result: {job.get('result')}")
    lease = int(ctx.env["DIFFYMAN_JOB_LEASE_SECS"])
    expect(ended <= lease / 3 + 2, f"cancel took {ended:.1f} s, heartbeat every {lease / 3:.1f} s")
    small = upload_file(ctx, write_input(ctx, "small.txt", b"a\n"))
    started = time.monotonic()
    after = wait_job(ctx, create_job(ctx, small, small), ctx.scale.job_timeout_secs + 30)
    ctx.measure.update(
        {
            "cancel_to_cancelled_s": round(ended, 2),
            "next_job": after.get("status"),
            "next_job_s": round(time.monotonic() - started, 2),
        }
    )
    expect(after.get("status") == "succeeded", f"the next job ended {after.get('status')} {after.get('error')}")


# --- Groups and the runner ----------------------------------------------------------------------


def groups(scale: Scale) -> dict[str, tuple[dict[str, str], list[Callable[[Ctx], None]]]]:
    return {
        "requests": (
            {
                "DIFFYMAN_MAX_BODY_BYTES": str(4 * MIB),
                "DIFFYMAN_MAX_SYNC_INPUT_BYTES": str(MIB),
                "DIFFYMAN_MAX_BLOB_BYTES": str(8 * MIB),
                "DIFFYMAN_REQUEST_TIMEOUT_SECS": "6",
                "DIFFYMAN_HEADER_READ_TIMEOUT_SECS": "3",
                "DIFFYMAN_MAX_PARALLEL_DIFFS": "4",
                "DIFFYMAN_SYNC_MEMORY_BYTES": str(MIB),
            },
            [
                test_body_over_limit_returns_413,
                test_body_at_limit_is_accepted,
                test_understated_content_length_reads_only_the_declared_bytes,
                test_body_of_unknown_length_reserves_the_body_limit,
                test_chunked_body_past_limit_returns_413,
                test_multipart_with_thousands_of_parts_is_rejected,
                test_slow_body_is_ended_within_the_request_timeout,
                test_slow_bodies_on_every_diff_slot_leave_other_routes_served,
                test_slow_headers_close_the_connection_within_the_header_timeout,
            ],
        ),
        "sync": (
            {"DIFFYMAN_MAX_PARALLEL_DIFFS": "8", "DIFFYMAN_SYNC_MEMORY_BYTES": str(scale.sync_memory_bytes)},
            [test_parallel_sync_diffs_succeed_or_get_busy_within_the_memory_budget],
        ),
        "jobs": (
            {
                "DIFFYMAN_JOB_MEMORY_BYTES": str(GIB),
                "DIFFYMAN_MAX_PARALLEL_JOBS": "1",
                "DIFFYMAN_JOB_TIMEOUT_SECS": str(scale.job_timeout_secs),
                "DIFFYMAN_JOB_LEASE_SECS": "6",
            },
            [
                test_zip_entry_over_the_ratio_limit_is_not_inflated,
                test_zip_entry_inflating_past_its_declared_size_is_not_read_past_it,
                test_compressed_tar_over_the_ratio_limit_stops_reading,
                test_nested_zip_fanout_stays_within_the_content_and_child_budgets,
                test_zips_nested_deeper_than_max_depth_are_compared_as_bytes,
                test_job_inputs_over_the_memory_budget_return_413,
                test_job_cancelled_right_after_it_starts_ends_cancelled,
            ],
        ),
    }


def run_group(
    name: str,
    env: dict[str, str],
    tests: list[Callable[[Ctx], None]],
    args: argparse.Namespace,
    pg: hx.PgServer,
    report: dict[str, Any],
) -> None:
    workdir = pathlib.Path(tempfile.mkdtemp(prefix=f"{name}-", dir=args.workdir_root))
    server = hx.Server(args.server, workdir, pg, args.database, extra_env=env)
    print(f"{name}: server on {server.base_url} with {env}", flush=True)
    alive = False
    try:
        server.start()
        _, key = server.create_key(f"limits-{name}")
        client = hx.Client(server.port, key)
        limits = client.request("GET", "/v1/formats").json()["limits"]
        time.sleep(1)
        ctx = Ctx(server, client, key, env, limits, args.scale, workdir / "inputs", server.rss() or 0)
        report["measurements"][f"{name}.idle_rss_mib"] = mib(ctx.idle_rss)
        for test in tests:
            ctx.measure = {}
            started = time.monotonic()
            try:
                test(ctx)
                outcome = None
            except AssertionError as failure:
                outcome = str(failure)
            except Exception as error:  # noqa: BLE001 - a crash of the test is a failure to report
                outcome = f"{type(error).__name__}: {error}"
            seconds = time.monotonic() - started
            report["tests"].append(
                {"group": name, "test": test.__name__, "ok": outcome is None, "failure": outcome, "seconds": round(seconds, 2)}
            )
            if ctx.measure:
                report["measurements"][f"{name}.{test.__name__}"] = ctx.measure
            print(
                f"  {'PASS' if outcome is None else 'FAIL'} {test.__name__} ({seconds:.1f} s){'' if outcome is None else ': ' + outcome}",
                flush=True,
            )
            if ctx.measure:
                print(f"       {json.dumps(ctx.measure, default=str)}", flush=True)
            shutil.rmtree(ctx.inputs, ignore_errors=True)
        alive = server.proc is not None and server.proc.poll() is None
        client.close()
    finally:
        server.stop()
        server.cleanup(keep=False)
    stopped = alive and server.exit_code == 0
    report["tests"].append(
        {
            "group": name,
            "test": "test_server_survives_and_stops_gracefully",
            "ok": stopped,
            "failure": None if stopped else f"alive {alive}, exit {server.exit_code}",
            "seconds": 0,
        }
    )
    report["measurements"][f"{name}.lifetime_peak_rss_mib"] = mib(server.max_rss or 0)
    verdict = "PASS" if stopped else "FAIL"
    peak = mib(server.max_rss or 0)
    print(f"  {verdict} test_server_survives_and_stops_gracefully (exit {server.exit_code}, lifetime peak RSS {peak} MiB)", flush=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--server", type=pathlib.Path, help="the diffyman-server binary (default: newest build)")
    parser.add_argument(
        "--database", default="diffyman_limits", help="test database, dropped and created (default diffyman_limits)"
    )
    parser.add_argument("--workdir-root", type=pathlib.Path, default=hx.default_workdir_root() / "limits")
    parser.add_argument("--scale", choices=sorted(SCALES), default="full", help="input sizes (default full; ci is smaller)")
    parser.add_argument("--only", help="comma-separated groups: requests, sync, jobs (default all)")
    parser.add_argument("--min-free-gb", type=float, default=15, help="refuse to run with less free disk (default 15)")
    parser.add_argument("--report", type=pathlib.Path, help="write the results and measurements as JSON here")
    args = parser.parse_args()
    args.scale = SCALES[args.scale]
    args.server = (args.server or hx.default_server()).resolve()
    try:
        pg = hx.PgServer.find()
        pg.check()
    except hx.Skip as why:
        print(f"skipping the server limit tests: {why}", file=sys.stderr)
        return 1 if hx.db_required() else 0
    selected = groups(args.scale)
    wanted = args.only.split(",") if args.only else list(selected)
    unknown = sorted(set(wanted) - set(selected))
    if unknown:
        parser.error(f"unknown groups {unknown}; the groups are {', '.join(selected)}")
    created_root = not args.workdir_root.exists()
    args.workdir_root.mkdir(parents=True, exist_ok=True)
    free = shutil.disk_usage(args.workdir_root).free / 1e9
    if free < args.min_free_gb:
        print(f"only {free:.1f} GB free under {args.workdir_root}; need {args.min_free_gb} GB", file=sys.stderr)
        return 1
    report: dict[str, Any] = {"scale": args.scale.__dict__, "tests": [], "measurements": {}}
    for name in wanted:
        env, tests = selected[name]
        run_group(name, env, tests, args, pg, report)
    if created_root:
        try:
            args.workdir_root.rmdir()  # only when this run created it and left it empty
        except OSError:
            pass
    failed = [t for t in report["tests"] if not t["ok"]]
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2, default=str) + "\n", encoding="utf-8")
    print(f"\n{len(report['tests']) - len(failed)} of {len(report['tests'])} limit tests passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
