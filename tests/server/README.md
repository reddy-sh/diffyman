# Server end to end

`run_e2e.py` tests a built `diffyman-server` binary the way a user runs it: as its own process, over HTTP, with a real PostgreSQL and a real blob store. The in-process HTTP tests in `crates/diffyman-server/tests` cover the router; this covers the binary, its start-up and shutdown, the `keys` command, the worker and the store together.

`harness.py` holds the parts both scripts here use: it starts `diffyman-server serve` on a dedicated database, a fresh working folder (blob store, temporary folder, admin token file, log) and a random free port on `127.0.0.1`, creates API keys with `diffyman-server keys create`, and stops the server with SIGTERM. Run as a script, it starts a server for one command (see [the conformance runner](../conformance/README.md)).

## What it checks

| Group | Checks |
|---|---|
| corpus | every corpus case whose two inputs are files (199 of 216; folders cannot be sent over HTTP) through `POST /v1/diff` as JSON with base64, JSON with text (when both inputs are UTF-8) and multipart; each response is RFC 8785 canonical, has `provenance.binding` `server`, and equals `expected.json` byte for byte once `provenance` is removed |
| jobs | 30 of those cases, evenly spread, through `POST /v1/blobs` (with `?sha256=`), `POST /v1/jobs`, `GET /v1/jobs/{id}` and `GET /v1/diffs/{id}`, compared the same way |
| saved | `?save=true` (201 and `Location`), `GET /v1/diffs/{id}` returns the same bytes, a job whose inputs are the saved diff's sides gives the same document, deleting that job's diff keeps the shared objects, deleting the last diff removes its row, its job and every object (checked in the store: the folder, or a signed `HEAD` for `s3://`), a job over a deleted blob gets `blob-not-found`, cancelling a finished job gets `job-finished` |
| security | `/healthz` and `/openapi.json` (equal to `docs/api/openapi.json`) without a key; missing, unknown, malformed and revoked keys (401); a read-only key on POST and DELETE (403); another key's jobs and diffs (404); `keys list` never shows a secret; foreign `Host` values (403 `host-not-allowed`, DNS-rebinding defence) and browser `Origin` values (403 `origin-not-allowed`); an input over 8 MiB as JSON and multipart and a body declared over 24 MiB (413), exactly 8 MiB (200); wrong media type, bad JSON, unknown fields, bad options, a hash mismatch, bad and unknown IDs, unknown routes and methods, each with its stable error code |
| large | two generated log files of 200 MB each uploaded as blobs and diffed by a job: upload and job time, the server's peak resident memory during the job, and the result equals `diffyman diff --json` of the same files (with `--cli`); the files are deleted afterwards |
| bench | one client, one keep-alive connection, small synchronous word diffs (two 100-line texts) for 10 s: requests per second, p50, p99 |
| logs | the server log has none of: the content marker of the saved test, the corpus path, the large inputs' path, the database password, any API key |
| shutdown | SIGTERM stops the server with exit code 0 |

It also reports the server's resident memory when idle after start, after the run, and its lifetime peak.

## Running it

It needs a PostgreSQL 18 server, `psql` on `PATH` (to drop and create the test database), [uv](https://docs.astral.sh/uv/) and about 1 GB of free disk for the large test (it refuses to start with less than 15 GB free unless `--min-free-gb` says otherwise).

```sh
scripts/local-up.sh                                    # PostgreSQL on 127.0.0.1:5433
cargo build --locked --release -p diffyman-server -p diffyman-cli
uv run --no-project --python 3.12 python tests/server/run_e2e.py \
  --server target/release/diffyman-server --cli target/release/diffyman
```

The database server is found like the Rust tests find it: `DIFFYMAN_TEST_DB_URL` (a URL to any database on it, usually `postgres`), or else `127.0.0.1:5433` with `DIFFYMAN_PG_PASSWORD` from the environment or `.env.local`. The test database (`diffyman_e2e`, `--database` to change it; the name must start with `diffyman_` and cannot be `diffyman`) is dropped and created at the start and dropped at the end; the main `diffyman` database is never used. Without a reachable server the script prints why and exits 0, or 1 with `DIFFYMAN_TEST_DB_REQUIRED=1` (as in CI). The working folder goes under `~/diffyman-data/tmp` when it exists, else the system temporary folder (`--workdir-root`).

Options: `--cases N` and `--jobs N` (subsets, evenly spread), `--large-mb N` (0 skips it), `--bench-secs N` (0 skips it), `--store-url URL`, `--keep` (keep the database and the working folder, for example for a [restore drill](../../docs/operations.md#restore-drill)), `--report FILE` (every measurement and result as JSON), `--server-log FILE` (a copy of the server log, pass or fail; CI uploads it when the job fails, with the `server.log` of any working folder a script left behind).

The client in `harness.py` reopens a kept-alive connection the server has closed before sending on it, and resends a request once on a new connection after a connection error (a file body is rewound). When the server answered before the body was sent in full, that answer is the reply. Any other connection error is raised after printing the last 40 lines of the server log (for a client given the log, as in `run_e2e.py`); on an error, `run_e2e.py` also says whether the server was still running, exited or was killed by a signal, and keeps the working folder.

Against the S3-compatible server of the `s3` profile, with a server built with `--features s3`:

```sh
scripts/local-up.sh s3
cargo build --locked --release -p diffyman-server --features s3
scripts/with-local-s3 uv run --no-project --python 3.12 python tests/server/run_e2e.py \
  --database diffyman_e2e_s3 --store-url s3://diffyman-test/e2e --cases 40 --jobs 10 --large-mb 50 --bench-secs 3
docker compose -f deploy/local/compose.yaml --env-file .env.local --profile s3 stop rustfs
```

## Request validation

`test_validation.py` checks the request-validation rules of the [API reference](../../docs/api/README.md) against a real server on its own database (`diffyman_e2e_validation`), each test named after the rule it checks:

| Area | Checks |
|---|---|
| keys | a missing, malformed, unknown or revoked key gets `401 unauthorized` with `WWW-Authenticate: Bearer` on every `/v1` route; a key without the route's scope gets `403 forbidden`, before its body is read; another key's jobs, diffs and blobs get the same status, headers and body as IDs that do not exist |
| Host and Origin | foreign `Host` values (with and without a key, absolute-form targets, a missing or repeated `Host`) and foreign `Origin` values for JSON, `text/plain`, form-encoded and multipart bodies and for preflight requests get `403` with no CORS headers; requests without a key are refused before their body is read, and leave no row, object, temporary file or usage record |
| IDs | traversal sequences, percent-encoded dots, absolute paths, NUL, control bytes and very long IDs in job and diff paths get `400 invalid-request` or `404`; uppercase, short, long and non-hex blob IDs in `?sha256=` and in jobs get `400 invalid-blob-id` |
| strings | quotes, backslashes, SQL keywords and control characters in input names (JSON, multipart, jobs), options, query parameters, media types and key names are stored and echoed as plain data (checked in the database and the result document) or refused with a typed `400`; the name length and NUL rules hold for every input form; refused requests change no table or object |
| secrets | the admin token file has mode 0600 and is not an API key; no issued key or admin token is in any error body, `/openapi.json` or the server log, and no error message or log line quotes the request |

Every reply is checked too: never a `500`, and every error is `{"error": {"code", "message"}}` with the status its code has in the reference's table. Run it like the end-to-end test (`-k NAME` runs only the tests whose name contains `NAME`):

```sh
uv run --no-project --python 3.12 python tests/server/test_validation.py --server target/release/diffyman-server
```

## Limits

`test_limits.py` checks the size, time and concurrency limits of the [API reference](../../docs/api/README.md#limits). Each group starts its own server (database `diffyman_limits`, dropped at the end) with the limits it checks set low, reads them back from `GET /v1/formats`, and names each test after the rule it checks:

| Group | Checks |
|---|---|
| requests | a body over `DIFFYMAN_MAX_BODY_BYTES` (413 `payload-too-large`) and one of exactly the limit (200); a `Content-Length` that understates the body (only the declared bytes are read: a truncated body gets `400 invalid-request`, an upload stores the declared bytes or fails its `?sha256=`, and the bytes after them never get a `2xx`); a chunked body of unknown length reserves the body limit, even with a `Content-Length` beside it; chunked bodies that grow past the body, blob and job-request limits (413 with their codes); multipart bodies with thousands of parts (typed `4xx`, the server stays healthy); bodies sent one byte a second end with `503 timeout` at the request time limit while other clients are served; slow bodies on every diff slot leave other routes served and other diffs get `503 busy` after half the limit; headers sent one byte a second close the connection at the header limit |
| sync | 40 parallel synchronous diffs of two 8 MiB inputs (2 MiB with `--scale ci`): each gets `200` or `503 busy` with `Retry-After`, `/healthz` stays ok, and the peak resident memory stays under the idle size plus `DIFFYMAN_SYNC_MEMORY_BYTES` plus 64 MiB |
| jobs | archives built from zeros, run as jobs: a zip entry over the ratio limit and a gzip-compressed tar of 2 GiB of zeros each (256 MiB with `--scale ci`) end `partial` with `tree-limit`; a zip entry that inflates past its declared size ends `partial` with `entry-unreadable`; zips nested 16 wide and 4 deep over 1 MiB leaves (64 GiB per side) end within the content and child budgets; zips nested 64 deep get `depth-limit` at the default and the largest `tree.maxDepth`; inputs over a quarter of `DIFFYMAN_JOB_MEMORY_BYTES` get `413 job-too-large`; each job's peak resident memory stays under the idle size plus `DIFFYMAN_JOB_MEMORY_BYTES`; a job cancelled right after it starts ends `cancelled` with no result, and the next job runs |

Every group also checks that the server is still running at the end and stops with exit code 0. Generated inputs go into the group's working folder (under `~/diffyman-data/tmp/limits` when `~/diffyman-data/tmp` exists) and are deleted after each test. `--only requests,sync,jobs` picks groups, `--report FILE` writes every result and measurement as JSON, and CI runs it with `--scale ci`:

```sh
uv run --no-project --python 3.12 python tests/server/test_limits.py --server target/release/diffyman-server
```

## Measurements

One run on macOS arm64 (release build, PostgreSQL 18.6 in Docker, `file://` store), 2026-11-08:

| What | Result |
|---|---|
| Checks | 788 of 788 passed (551 corpus requests over 199 cases, 30 jobs) |
| Small synchronous diffs, one client, 10 s | 1,142 requests/s; p50 0.84 ms, p99 1.36 ms |
| 200 MB pair through a job | 1.38 s from `POST /v1/jobs` to `succeeded` (upload of both blobs 0.61 s; the CLI takes 0.76 s on the same files); result 983 KB, equal to the CLI's |
| Peak resident memory | 659 MiB during the large job; 679 MiB lifetime peak |
| Idle resident memory | 10.8 MiB after start |

With the `s3://` store on RustFS 1.0.4 (40 cases, 10 jobs, a 50 MB pair): 248 of 248 checks passed, 1,130 requests/s, the 50 MB job took 0.46 s with a 222 MiB peak, and idle memory was 13.2 MiB.

`test_limits.py` on the same machine (full scale), 2026-11-08: 19 of 20 tests passed. Bodies past their limits got their 413 within 8 ms, slow bodies ended with `503 timeout` at 6.0 s and slow headers were closed at 3.0 s, while other clients were answered within 8 ms. The 40 parallel diffs of 16 MiB bodies all succeeded in 0.42 s, but the peak resident memory was 628 MiB against a bound of 331 MiB (11 MiB idle, a 256 MiB budget, 64 MiB slack): one such diff peaks at about 35 MiB, but memory freed after each diff is not returned, and the process stays near 570 MiB once idle. Since the server maps blocks of 1 MiB or more itself and unmaps them when freed, all 20 tests pass: the same 40 diffs succeed in 0.40 s with a 121 MiB peak. The archive jobs took 0.04 to 0.06 s for the 2 GiB entries and 22 s for the 64 GiB fan-out (stopped by the 4 GiB content budget), with at most 31 MiB resident. A cancel ended its job in 2.0 s, but the job after it waited 20 s, because the cancelled job's engine thread holds its slot until it returns.
