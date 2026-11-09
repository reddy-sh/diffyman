"""Raw HTTP/1.1 exchanges and watchers of a running server, for tests that
need the framing under their control (bodies sent piece by piece or slowly,
chunked encoding, a Content-Length that does not match) and that check other
clients and the server's memory while they run."""

from __future__ import annotations

import json
import select
import socket
import threading
import time
from collections.abc import Iterable, Iterator
from dataclasses import dataclass, field
from typing import Any

import harness as hx


@dataclass
class Answer:
    status: int | None
    headers: dict[str, str]
    body: bytes
    seconds: float  # until the answer arrived or the connection closed
    sent: int  # body bytes written before that
    more: list[tuple[int, bytes]] = field(default_factory=list)  # later answers

    def code(self) -> str | None:
        return hx.Reply(self.status or 0, self.headers, self.body).code()

    def show(self) -> str:
        return f"{self.status} {self.code() or self.body[:80]!r} after {self.seconds:.2f} s"


def parse_answers(data: bytes) -> list[tuple[int, dict[str, str], bytes]]:
    out = []
    while (end := data.find(b"\r\n\r\n")) >= 0:
        lines = data[:end].decode("latin-1").split("\r\n")
        try:
            status = int(lines[0].split()[1])
        except (IndexError, ValueError):
            break
        headers = {k.strip().lower(): v.strip() for k, _, v in (h.partition(":") for h in lines[1:])}
        length = int(headers.get("content-length", "0") or 0)
        body = data[end + 4 : end + 4 + length]
        if len(body) < length:
            break
        out.append((status, headers, body))
        data = data[end + 4 + length :]
    return out


def head(port: int, method: str, path: str, headers: dict[str, str]) -> bytes:
    lines = [f"{method} {path} HTTP/1.1", f"Host: 127.0.0.1:{port}", *(f"{k}: {v}" for k, v in headers.items())]
    return ("\r\n".join(lines) + "\r\n\r\n").encode("latin-1")


def chunked(pieces: Iterable[bytes]) -> Iterator[bytes]:
    for piece in pieces:
        if piece:  # an empty chunk would end the body
            yield f"{len(piece):x}\r\n".encode() + piece + b"\r\n"
    yield b"0\r\n\r\n"


def exchange(
    port: int, start: bytes, body: Iterable[bytes] = (), pace: float = 0, timeout: float = 60, all_answers: bool = False
) -> Answer:
    """Sends `start` then `body` piece by piece (`pace` seconds apart) while
    reading, so an answer the server sends early is never lost."""
    sock = socket.create_connection(("127.0.0.1", port), timeout=timeout)
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    got = bytearray()
    answered = threading.Event()
    at: list[float] = []

    def read() -> None:
        try:
            while all_answers or not parse_answers(bytes(got)):
                data = sock.recv(1 << 16)
                if not data:
                    break
                got.extend(data)
        except OSError:
            pass
        at.append(time.monotonic())
        answered.set()

    reader = threading.Thread(target=read, daemon=True)
    started = time.monotonic()
    reader.start()
    sent = -len(start)
    try:
        for piece in _prepend(start, body):
            view = memoryview(piece)
            while view and not answered.is_set():
                if select.select([], [sock], [], 0.2)[1]:
                    n = sock.send(view[: 1 << 16])
                    view, sent = view[n:], sent + n
            if answered.is_set() or (pace and answered.wait(pace)):
                break
    except OSError:
        pass
    answered.wait(timeout)
    sock.close()
    reader.join(5)
    answers = parse_answers(bytes(got))
    seconds = (at[0] if at else time.monotonic()) - started
    first = answers[0] if answers else (None, {}, bytes(got))
    return Answer(first[0], first[1], first[2], seconds, max(sent, 0), [(s, b) for s, _, b in answers[1:]])


def _prepend(first: bytes, rest: Iterable[bytes]) -> Iterator[bytes]:
    yield first
    yield from rest


# --- Watching the server ------------------------------------------------------------


class RssSampler(threading.Thread):
    def __init__(self, server: hx.Server) -> None:
        super().__init__(daemon=True)
        self.server, self.peak, self.done = server, 0, threading.Event()

    def run(self) -> None:
        while not self.done.is_set():
            self.peak = max(self.peak, self.server.rss() or 0)
            self.done.wait(0.02)

    def stop(self) -> int:
        self.done.set()
        self.join()
        return self.peak


class Monitor(threading.Thread):
    """Another client: GET /healthz (and a small diff) every quarter second."""

    SMALL = json.dumps({"left": {"text": "a\nb\n"}, "right": {"text": "a\nc\n"}}).encode()

    def __init__(self, port: int, key: str | None) -> None:
        super().__init__(daemon=True)
        self.port, self.key, self.done = port, key, threading.Event()
        self.samples: list[tuple[str, int | str, float]] = []

    def run(self) -> None:
        client = hx.Client(self.port, self.key, timeout=20)
        while not self.done.is_set():
            calls = [("healthz", lambda: client.request("GET", "/healthz", key=None))]
            if self.key:
                calls.append(
                    ("diff", lambda: client.request("POST", "/v1/diff", self.SMALL, {"Content-Type": "application/json"}))
                )
            for name, call in calls:
                started = time.monotonic()
                try:
                    status: int | str = call().status
                except OSError as error:
                    status = type(error).__name__
                    client.close()
                self.samples.append((name, status, time.monotonic() - started))
            self.done.wait(0.25)
        client.close()

    def stop(self) -> dict[str, Any]:
        self.done.set()
        self.join()
        bad = [s for s in self.samples if s[1] != 200]
        return {
            "requests": len(self.samples),
            "not_ok": len(bad),
            "first_not_ok": bad[:3],
            "max_ms": round(max((s[2] for s in self.samples), default=0) * 1000, 1),
        }

    def check(self, max_secs: float = 2.0) -> dict[str, Any]:
        summary = self.stop()
        _expect(summary["requests"] > 0, "the other client sent nothing")
        _expect(summary["not_ok"] == 0, f"other clients were not served: {summary['first_not_ok']}")
        _expect(summary["max_ms"] < max_secs * 1000, f"other clients waited {summary['max_ms']} ms")
        return summary


def _expect(ok: bool, what: str) -> None:
    if not ok:
        raise AssertionError(what)
