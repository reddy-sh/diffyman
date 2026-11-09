"""The surfaces run_bindings.py drives: how each one gets a Diff Document.

A surface is the CLI, one binding or a running diffyman-server. Each runs in
one or two modes:

  paths  the surface reads the inputs itself (files, folders, archives)
  bytes  the runner's helper reads two files and hands the surface their
         bytes and final path components as names

The server's two modes are both bytes forms of POST /v1/diff: `json` and
`multipart`.

A surface that cannot run a case says why (`skip_reason`), and a build that
must refuse a case with an `unsupported` error says so (`refuses`); nothing
is skipped silently. Every surface but the CLI runs its whole batch in one
process. Missing builds stop the run with the command that builds them.

Run as a script, this file is the Python binding's batch worker
(`--python-worker`), started in the interpreter that has `diffyman`.
"""

from __future__ import annotations

import base64
import ctypes
import http.client
import json
import os
import pathlib
import shutil
import subprocess
import sys
import urllib.parse
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Any

REPO = pathlib.Path(__file__).resolve().parents[2]
HERE = pathlib.Path(__file__).resolve().parent
TARGET = REPO / "target"
WINDOWS = os.name == "nt"

PATHS = "paths"
BYTES = "bytes"


@dataclass
class Case:
    name: str
    left: pathlib.Path
    right: pathlib.Path
    options: pathlib.Path
    # None for the extra cross-surface-only cases, which have no golden file.
    expected: pathlib.Path | None

    @property
    def folder(self) -> bool:
        """A side is a folder: only surfaces that read paths can run it."""
        return self.left.is_dir() or self.right.is_dir()

    def needs_archives(self) -> bool:
        """Spec section 19.7: what a build without archive support refuses."""
        options = json.loads(self.options.read_text(encoding="utf-8"))
        adapter = options.get("adapter", "auto")
        if adapter == "tree" or options.get("tree", {}).get("exclude"):
            return True
        return (
            adapter == "auto"
            and starts_like_archive(self.left.read_bytes())
            and starts_like_archive(self.right.read_bytes())
        )


def starts_like_archive(data: bytes) -> bool:
    """`diffyman::starts_like_archive`: zip, tar, gzip or zstd signatures."""
    return (
        data.startswith((b"PK\x03\x04", b"PK\x05\x06", b"\x1f\x8b", b"\x28\xb5\x2f\xfd"))
        or (len(data) >= 512 and data[257:262] == b"ustar")
        or (len(data) >= 4 and data[0] & 0xF0 == 0x50 and data[1:4] == b"\x2a\x4d\x18")
    )


@dataclass
class Output:
    raw: str | None = None
    error: str | None = None
    exit_code: int | None = None
    # The binding's error code, such as "unsupported" or "io".
    code: str | None = None


Job = tuple[Case, str]


def job_dicts(jobs: Sequence[Job]) -> list[dict[str, str]]:
    return [
        {"mode": mode, "left": str(c.left), "right": str(c.right), "options": str(c.options)}
        for c, mode in jobs
    ]


def batch_outputs(what: str, stdout: bytes, count: int) -> list[Output]:
    results = json.loads(stdout.decode("utf-8"))
    if len(results) != count:
        raise SystemExit(f"{what}: batch returned {len(results)} results for {count} jobs")
    return [
        Output(raw=r["json"]) if r["ok"] else Output(error=r["error"], code=r.get("code"))
        for r in results
    ]


def run_batch(what: str, command: list[str], payload: bytes, count: int) -> list[Output]:
    if count == 0:
        return []
    proc = subprocess.run(command, input=payload, capture_output=True, check=False)
    if proc.returncode != 0:
        raise SystemExit(f"{what} failed (exit {proc.returncode}):\n{proc.stderr.decode(errors='replace')}")
    return batch_outputs(what, proc.stdout, count)


def newest(candidates: Sequence[pathlib.Path]) -> pathlib.Path | None:
    existing = [p for p in candidates if p.is_file()]
    return max(existing, key=lambda p: p.stat().st_mtime) if existing else None


def tool_output(command: list[str]) -> str:
    try:
        proc = subprocess.run(command, capture_output=True, check=True, text=True)
    except (OSError, subprocess.CalledProcessError):
        return "unknown"
    return (proc.stdout.strip() or proc.stderr.strip()).splitlines()[0]


class Surface:
    """One way of getting documents: the CLI or a binding."""

    name: str
    binding: str
    modes: tuple[str, ...] = (PATHS, BYTES)

    def prepare(self) -> None:
        """Checks the build is there (SystemExit with a build hint if not)."""

    def describe(self) -> str:
        raise NotImplementedError

    def skip_reason(self, case: Case, mode: str) -> str | None:
        if mode == BYTES and case.folder:
            return "folder input: the bytes API takes two files"
        return None

    def refuses(self, case: Case) -> bool:
        """This build must refuse the case with an `unsupported` error."""
        return False

    def run(self, jobs: Sequence[Job]) -> list[Output]:
        raise NotImplementedError


# --- CLI ---------------------------------------------------------------------


class Cli(Surface):
    name = binding = "cli"
    modes = (PATHS,)

    def __init__(self, explicit: str | None) -> None:
        self.explicit = explicit
        self.path = pathlib.Path()

    def prepare(self) -> None:
        exe = "diffyman.exe" if WINDOWS else "diffyman"
        if self.explicit:
            path = pathlib.Path(self.explicit).resolve()
            if WINDOWS and not path.is_file() and path.suffix != ".exe":
                path = path.with_name(path.name + ".exe")
            found: pathlib.Path | None = path if path.is_file() else None
        else:
            found = newest([TARGET / profile / exe for profile in ("release", "debug")])
        if found is None:
            raise SystemExit(
                "cli: no CLI binary found; build it with `cargo build --release -p diffyman-cli` "
                "or pass --cli PATH"
            )
        self.path = found

    def describe(self) -> str:
        return str(self.path)

    def run(self, jobs: Sequence[Job]) -> list[Output]:
        outputs = []
        for case, _ in jobs:
            proc = subprocess.run(
                [str(self.path), "diff", "--json", "--options", str(case.options), str(case.left), str(case.right)],
                capture_output=True,
                check=False,
            )
            out = Output(exit_code=proc.returncode)
            stdout = proc.stdout.decode("utf-8", errors="strict")
            if stdout.endswith("\n") and not stdout.endswith("\n\n"):
                out.raw = stdout[:-1]
            else:
                out.error = f"exit {proc.returncode}: {proc.stderr.decode(errors='replace').strip() or 'no output'}"
            outputs.append(out)
        return outputs


# --- Python ------------------------------------------------------------------


def python_worker() -> None:
    """Batch mode run inside the interpreter that has the binding installed."""
    import diffyman  # type: ignore[import-not-found]  # noqa: PLC0415

    jobs = json.load(sys.stdin)
    results = []
    for job in jobs:
        try:
            text = pathlib.Path(job["options"]).read_text(encoding="utf-8")
            left, right = pathlib.Path(job["left"]), pathlib.Path(job["right"])
            if job["mode"] == PATHS:
                raw = diffyman.diff(left, right, text)
            else:
                raw = diffyman.diff(
                    left.read_bytes(), right.read_bytes(), text, left_name=left.name, right_name=right.name
                )
            results.append({"ok": True, "json": raw})
        except Exception as err:  # noqa: BLE001 - reported per case
            results.append({"ok": False, "error": f"{type(err).__name__}: {err}", "code": getattr(err, "code", None)})
    sys.stdout.write(json.dumps(results))


def venv_python(venv: pathlib.Path) -> pathlib.Path:
    windows = venv / "Scripts" / "python.exe"
    return windows if windows.is_file() else venv / "bin" / "python"


class Python(Surface):
    name = binding = "python"

    def __init__(self, venv: pathlib.Path) -> None:
        self.venv = venv
        self.interpreter = pathlib.Path()

    def prepare(self) -> None:
        try:
            import diffyman  # noqa: F401, PLC0415

            self.interpreter = pathlib.Path(sys.executable)
        except ImportError:
            self.interpreter = venv_python(self.venv)
            if not self.interpreter.is_file():
                raise SystemExit(
                    f"python: the diffyman Python binding is not importable and {self.interpreter} does not "
                    "exist; build it with `maturin develop` (bindings/python/README.md) or pass --venv"
                ) from None

    def describe(self) -> str:
        return str(self.interpreter)

    def run(self, jobs: Sequence[Job]) -> list[Output]:
        return run_batch(
            "python worker",
            [str(self.interpreter), str(pathlib.Path(__file__).resolve()), "--python-worker"],
            json.dumps(job_dicts(jobs)).encode("utf-8"),
            len(jobs),
        )


# --- WASM --------------------------------------------------------------------


WASM_SCRIPT = HERE / "wasm_diff.mjs"


class Wasm(Surface):
    """A wasm-pack `--target nodejs` package, with or without `archives`."""

    binding = "wasm"
    modes = (BYTES,)

    def __init__(self, name: str, node: str, pkg: pathlib.Path, build_hint: str) -> None:
        self.name = name
        self.node = node
        self.pkg = pkg
        self.build_hint = build_hint
        self.features: list[str] = []

    def prepare(self) -> None:
        if not (self.pkg / "diffyman_wasm.js").is_file():
            raise SystemExit(f"{self.name}: no WASM package at {self.pkg}; build it with `{self.build_hint}`")
        proc = subprocess.run(
            [self.node, str(WASM_SCRIPT), "--pkg", str(self.pkg), "--features"], capture_output=True, check=False
        )
        if proc.returncode != 0:
            raise SystemExit(f"{self.name}: wasm_diff.mjs --features failed:\n{proc.stderr.decode(errors='replace')}")
        self.features = json.loads(proc.stdout.decode("utf-8"))

    def describe(self) -> str:
        return f"{self.pkg} (node {tool_output([self.node, '--version'])}, features {self.features})"

    def skip_reason(self, case: Case, mode: str) -> str | None:
        return "folder input: the WASM binding takes bytes, not paths" if case.folder else None

    def refuses(self, case: Case) -> bool:
        return "archives" not in self.features and not case.folder and case.needs_archives()

    def run(self, jobs: Sequence[Job]) -> list[Output]:
        return run_batch(
            f"{self.name} (wasm_diff.mjs)",
            [self.node, str(WASM_SCRIPT), "--pkg", str(self.pkg), "--batch"],
            json.dumps(job_dicts(jobs)).encode("utf-8"),
            len(jobs),
        )


# --- Node.js -----------------------------------------------------------------


NODE_SCRIPT = HERE / "node_diff.mjs"


class Node(Surface):
    name = binding = "node"

    def __init__(self, node: str, pkg: pathlib.Path) -> None:
        self.node = node
        self.pkg = pkg
        self.info: dict[str, Any] = {}

    def prepare(self) -> None:
        proc = subprocess.run(
            [self.node, str(NODE_SCRIPT), "--pkg", str(self.pkg), "--info"], capture_output=True, check=False
        )
        if proc.returncode != 0:
            raise SystemExit(
                f"node: cannot load the Node.js binding from {self.pkg}; build it with "
                "`npm ci --prefix bindings/node` and `npm run build --prefix bindings/node`:\n"
                f"{proc.stderr.decode(errors='replace')}"
            )
        self.info = json.loads(proc.stdout.decode("utf-8"))

    def describe(self) -> str:
        return f"{self.pkg} (node {self.info.get('node')}, features {self.info.get('features')})"

    def run(self, jobs: Sequence[Job]) -> list[Output]:
        return run_batch(
            "node (node_diff.mjs)",
            [self.node, str(NODE_SCRIPT), "--pkg", str(self.pkg), "--batch"],
            json.dumps(job_dicts(jobs)).encode("utf-8"),
            len(jobs),
        )


# --- Java --------------------------------------------------------------------


JAVA_SCRIPT = HERE / "JavaDiff.java"


class Java(Surface):
    name = binding = "java"

    def __init__(self, java: str, jar: pathlib.Path | None, library: pathlib.Path | None) -> None:
        self.java = java
        self.jar = jar
        self.library = library
        self.command: list[str] = []
        self.info: dict[str, Any] = {}

    def prepare(self) -> None:
        if self.jar is None:
            target = REPO / "bindings" / "java" / "target"
            jars = [
                p
                for p in (target.glob("diffyman-*.jar") if target.is_dir() else [])
                if not p.name.endswith(("-sources.jar", "-javadoc.jar"))
            ]
            self.jar = newest(jars)
        if self.jar is None or not self.jar.is_file():
            raise SystemExit(
                "java: no binding jar; build it with `cargo build --release -p diffyman-java` and "
                "`mvn -f bindings/java/pom.xml verify`, or pass --java-jar"
            )
        command = [self.java]
        # JDK 24 and later warn about JNI use unless native access is enabled;
        # every supported JDK (17+) takes the flag.
        probe = subprocess.run([self.java, "--enable-native-access=ALL-UNNAMED", "-version"], capture_output=True)
        if probe.returncode == 0:
            command.append("--enable-native-access=ALL-UNNAMED")
        if self.library is not None:
            command.append(f"-Ddiffyman.library.path={self.library}")
        command += ["-cp", str(self.jar), str(JAVA_SCRIPT)]
        self.command = command
        proc = subprocess.run([*command, "--info"], capture_output=True, check=False)
        if proc.returncode != 0:
            raise SystemExit(f"java: JavaDiff.java --info failed:\n{proc.stderr.decode(errors='replace')}")
        self.info = json.loads(proc.stdout.decode("utf-8"))

    def describe(self) -> str:
        library = f", library {self.library}" if self.library else ", library from the jar"
        return f"{self.jar} (java {self.info.get('java')}{library})"

    def run(self, jobs: Sequence[Job]) -> list[Output]:
        lines = []
        for case, mode in jobs:
            fields = [mode, str(case.left), str(case.right), str(case.options)]
            if any(ch in field for field in fields for ch in "\t\r\n"):
                raise SystemExit(f"java: a path of {case.name} holds a tab or line break")
            lines.append("\t".join(fields) + "\n")
        return run_batch("java (JavaDiff.java)", [*self.command, "--batch"], "".join(lines).encode("utf-8"), len(jobs))


# --- C ABI -------------------------------------------------------------------


C_STATUS = {1: "invalid-options", 2: "io", 3: "other"}


def c_program_default() -> pathlib.Path | None:
    exe = "test_ffi.exe" if WINDOWS else "test_ffi"
    return newest([TARGET / profile / "ffi-test" / exe for profile in ("release", "debug")])


def c_library_default() -> pathlib.Path | None:
    if WINDOWS:
        name = "diffyman_ffi.dll"
    elif sys.platform == "darwin":
        name = "libdiffyman_ffi.dylib"
    else:
        name = "libdiffyman_ffi.so"
    return newest([TARGET / profile / name for profile in ("release", "debug")])


class C(Surface):
    """The C ABI (crates/diffyman-ffi), through one of two drivers.

    `program` runs the C test program that tests/ffi/run.sh compiles with the
    platform's C compiler (one process per job). `ctypes` loads the shared
    library into this process and calls the same exported functions; it needs
    no compiler, so it is what Windows uses (run.sh is a POSIX script).
    """

    name = binding = "c"

    def __init__(self, driver: str, program: pathlib.Path | None, library: pathlib.Path | None) -> None:
        self.driver = driver
        self.program = program
        self.library = library
        self.lib: Any = None

    def prepare(self) -> None:
        if self.driver == "auto":
            if self.program is not None:
                self.driver = "program"
            elif self.library is not None or WINDOWS or c_program_default() is None:
                self.driver = "ctypes"
            else:
                self.driver = "program"
        if self.driver == "program":
            self.program = self.program or c_program_default()
            if self.program is None or not self.program.is_file():
                raise SystemExit("c: no C test program; build it with `bash tests/ffi/run.sh` or pass --c-program")
        else:
            self.library = self.library or c_library_default()
            if self.library is None or not self.library.is_file():
                raise SystemExit("c: no C ABI library; build it with `cargo build --release -p diffyman-ffi`")
            self.lib = load_c_library(self.library)

    def describe(self) -> str:
        if self.driver == "program":
            return f"{self.program} (C test program)"
        version = self.lib.diffyman_engine_version().decode("utf-8")
        return f"{self.library} (ctypes, engine {version})"

    def run(self, jobs: Sequence[Job]) -> list[Output]:
        if self.driver == "program":
            return [self.run_program(case, mode) for case, mode in jobs]
        return [self.run_ctypes(case, mode) for case, mode in jobs]

    def run_program(self, case: Case, mode: str) -> Output:
        assert self.program is not None
        command = "paths" if mode == PATHS else "diff"
        proc = subprocess.run(
            [str(self.program), command, str(case.left), str(case.right), str(case.options)],
            capture_output=True,
            check=False,
        )
        if proc.returncode == 0:
            return Output(raw=proc.stdout.decode("utf-8", errors="strict"))
        message = proc.stderr.decode(errors="replace").strip()
        return Output(error=f"exit {proc.returncode}: {message}", code=C_STATUS.get(proc.returncode - 10))

    def run_ctypes(self, case: Case, mode: str) -> Output:
        lib = self.lib
        options = case.options.read_bytes()
        out_json = ctypes.c_void_p()
        out_error = ctypes.c_void_p()
        if mode == PATHS:
            status = lib.diffyman_diff_paths(
                str(case.left).encode("utf-8"),
                str(case.right).encode("utf-8"),
                options,
                0,
                ctypes.byref(out_json),
                ctypes.byref(out_error),
            )
        else:
            left, right = case.left.read_bytes(), case.right.read_bytes()
            status = lib.diffyman_diff(
                left,
                len(left),
                right,
                len(right),
                options,
                case.left.name.encode("utf-8"),
                case.right.name.encode("utf-8"),
                ctypes.byref(out_json),
                ctypes.byref(out_error),
            )
        try:
            if status == 0 and out_json.value and not out_error.value:
                return Output(raw=ctypes.string_at(out_json.value).decode("utf-8", errors="strict"))
            message = ctypes.string_at(out_error.value).decode(errors="replace") if out_error.value else "(no message)"
            return Output(error=f"status {status}: {message}", code=C_STATUS.get(status))
        finally:
            lib.diffyman_free_string(out_json)
            lib.diffyman_free_string(out_error)


def load_c_library(path: pathlib.Path) -> Any:
    lib = ctypes.CDLL(str(path))
    out = ctypes.POINTER(ctypes.c_void_p)
    lib.diffyman_diff.argtypes = [
        ctypes.c_char_p,
        ctypes.c_size_t,
        ctypes.c_char_p,
        ctypes.c_size_t,
        ctypes.c_char_p,
        ctypes.c_char_p,
        ctypes.c_char_p,
        out,
        out,
    ]
    lib.diffyman_diff.restype = ctypes.c_int
    lib.diffyman_diff_paths.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint64, out, out]
    lib.diffyman_diff_paths.restype = ctypes.c_int
    lib.diffyman_free_string.argtypes = [ctypes.c_void_p]
    lib.diffyman_free_string.restype = None
    lib.diffyman_engine_version.argtypes = []
    lib.diffyman_engine_version.restype = ctypes.c_char_p
    return lib


# --- diffyman-server -----------------------------------------------------------


SERVER_JSON = "json"
SERVER_MULTIPART = "multipart"


class Server(Surface):
    """A running diffyman-server, through POST /v1/diff.

    `json` sends both inputs base64-encoded in a JSON body, `multipart` as
    multipart/form-data parts with the options as JSON text; both name the
    inputs by their final path components. The API key comes from the
    environment (DIFFYMAN_SERVER_KEY), never from the command line.
    tests/server/harness.py starts a server and sets both variables.
    """

    name = binding = "server"
    modes = (SERVER_JSON, SERVER_MULTIPART)

    def __init__(self, url: str) -> None:
        parts = urllib.parse.urlsplit(url)
        if parts.scheme != "http" or not parts.hostname or parts.path not in ("", "/"):
            raise SystemExit(f"server: --server-url must be http://host:port, not {url!r}")
        self.url = url.rstrip("/")
        self.host = parts.hostname
        self.port = parts.port or 80
        self.key = os.environ.get("DIFFYMAN_SERVER_KEY", "")
        self.conn: http.client.HTTPConnection | None = None
        self.health: dict[str, Any] = {}

    def request(self, method: str, path: str, body: bytes | None, headers: dict[str, str]) -> tuple[int, bytes]:
        for attempt in (0, 1):
            if self.conn is None:
                self.conn = http.client.HTTPConnection(self.host, self.port, timeout=120)
            try:
                self.conn.request(method, path, body=body, headers=headers)
                response = self.conn.getresponse()
                return response.status, response.read()
            except (ConnectionError, http.client.HTTPException):
                self.conn.close()
                self.conn = None
                if attempt == 1:
                    raise
        raise AssertionError("unreachable")

    def prepare(self) -> None:
        if not self.key:
            raise SystemExit("server: set DIFFYMAN_SERVER_KEY to an API key (tests/server/harness.py does)")
        try:
            status, body = self.request("GET", "/healthz", None, {})
        except OSError as error:
            raise SystemExit(f"server: nothing answers at {self.url} ({error})") from None
        if status != 200:
            raise SystemExit(f"server: GET /healthz answered HTTP {status}: {body[:200]!r}")
        self.health = json.loads(body)

    def describe(self) -> str:
        return f"{self.url} (POST /v1/diff, health {self.health.get('status')})"

    def skip_reason(self, case: Case, mode: str) -> str | None:
        return "folder input: the HTTP API takes two files" if case.folder else None

    def run(self, jobs: Sequence[Job]) -> list[Output]:
        return [self.run_one(case, mode) for case, mode in jobs]

    def run_one(self, case: Case, mode: str) -> Output:
        left, right = case.left.read_bytes(), case.right.read_bytes()
        options = case.options.read_text(encoding="utf-8")
        if mode == SERVER_JSON:
            body = json.dumps(
                {
                    "left": {"name": case.left.name, "base64": base64.b64encode(left).decode("ascii")},
                    "right": {"name": case.right.name, "base64": base64.b64encode(right).decode("ascii")},
                    "options": json.loads(options),
                }
            ).encode("utf-8")
            content_type = "application/json"
        else:
            body, content_type = multipart_body(
                [
                    ("left", case.left.name, left),
                    ("right", case.right.name, right),
                    ("options", None, options.encode("utf-8")),
                ]
            )
        headers = {"Content-Type": content_type, "Authorization": f"Bearer {self.key}"}
        status, raw = self.request("POST", "/v1/diff", body, headers)
        if status == 200:
            return Output(raw=raw.decode("utf-8", errors="strict"))
        try:
            error = json.loads(raw)["error"]
            return Output(error=f"HTTP {status} {error['code']}: {error['message']}", code=error["code"])
        except (ValueError, KeyError, TypeError):
            return Output(error=f"HTTP {status}: {raw[:200]!r}")


MULTIPART_BOUNDARY = "diffyman-conformance-5b2e8c41"


def multipart_body(parts: list[tuple[str, str | None, bytes]]) -> tuple[bytes, str]:
    out = bytearray()
    for name, filename, data in parts:
        disposition = f'form-data; name="{name}"' + (f'; filename="{filename}"' if filename is not None else "")
        out += f"--{MULTIPART_BOUNDARY}\r\nContent-Disposition: {disposition}\r\n\r\n".encode()
        out += data + b"\r\n"
    out += f"--{MULTIPART_BOUNDARY}--\r\n".encode()
    return bytes(out), f"multipart/form-data; boundary={MULTIPART_BOUNDARY}"


def which(name: str) -> str:
    return shutil.which(name) or name


if __name__ == "__main__":
    if sys.argv[1:] == ["--python-worker"]:
        python_worker()
    else:
        sys.exit("surfaces.py is a module of run_bindings.py; run that instead")
