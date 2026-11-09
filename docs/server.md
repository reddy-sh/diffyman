# diffyman-server

`diffyman-server` is a local REST service over the diffyman engine. It returns the same canonical Diff Document as `diffyman diff --json` (apart from `provenance`, whose `binding` is `server`), keeps uploads and saved results in a content-addressed blob store, and runs larger diffs as background jobs. Its design is in [ADR 0012](adr/0012-local-service-architecture.md). The full API reference is the OpenAPI 3.1 document in [`api/openapi.json`](api/openapi.json), which the server also serves at `/openapi.json`; the [API reference](api/README.md) explains every endpoint, error code and limit with `curl` examples, and the [REST quickstart](quickstart/rest.md) goes from nothing to a first diff.

It is safe by default: it listens on `127.0.0.1` only, every `/v1` route needs an API key, requests must name an allow-listed `Host` and must not come from a browser origin that is not allow-listed, every size and time has a limit, logs hold IDs, sizes and timings only, and stored content expires after 30 days.

## Running it

The server needs PostgreSQL 18 and a folder for blobs. Locally, `scripts/local-up.sh` starts PostgreSQL in Docker on `127.0.0.1:5433` and writes its password to `.env.local` (mode 0600), which the server reads in the `local` environment.

```sh
cargo run --release -p diffyman-server -- serve
```

On start it runs the database migrations, creates the blob folder (`~/diffyman-data/blobs`) and the temporary folder, writes the admin token file `~/.diffyman/admin-token` (mode 0600) if it is missing and prints where it is (never what it holds), and listens on `127.0.0.1:8080`. Ctrl-C or SIGTERM stops it gracefully: requests in flight finish, running jobs get a short grace period and are then handed back to the queue.

## API keys

Keys are managed on the server's machine with the server's configuration. The commands need the admin token file, so run `serve` once first.

```sh
diffyman-server keys create --name laptop            # prints the key once: dmk_ and 64 hex digits
diffyman-server keys create --name ci --scopes read  # read-only
diffyman-server keys list                            # IDs, prefixes, scopes, last use; never the secrets
diffyman-server keys revoke <id>
```

Scopes: `read` (GET routes), `write` (POST routes), `delete` (DELETE routes); a new key gets all three unless `--scopes` says otherwise. The database stores only a 12-character prefix and a keyed BLAKE3 hash of each key. Jobs and saved diffs belong to the key that created them; other keys see `404`.

For quick experiments, `DIFFYMAN_AUTH_MODE=none` turns authentication off. It is accepted only in the `local` environment.

## Using the API

```sh
KEY=dmk_...   # from keys create
H="Authorization: Bearer $KEY"

# A synchronous diff of two small inputs (JSON: text or base64 per side).
curl -s -H "$H" -H 'Content-Type: application/json' http://127.0.0.1:8080/v1/diff \
  -d '{"left": {"name": "a.txt", "text": "one\ntwo\n"},
       "right": {"name": "b.txt", "text": "one\nthree\n"},
       "options": {"granularity": "word"}}'

# The same as multipart; the file names are the input names.
curl -s -H "$H" -F left=@old.txt -F right=@new.txt -F 'options={"context": 1}' \
  http://127.0.0.1:8080/v1/diff

# Save the result: 201 with Location: /v1/diffs/{id}.
curl -si -H "$H" -F left=@old.txt -F right=@new.txt 'http://127.0.0.1:8080/v1/diff?save=true'

# Larger inputs: upload blobs (the blob ID is the SHA-256), then create a job.
curl -s -H "$H" --data-binary @old.bin http://127.0.0.1:8080/v1/blobs   # {"blobId": "...", ...}
curl -s -H "$H" --data-binary @new.bin http://127.0.0.1:8080/v1/blobs
curl -si -H "$H" -H 'Content-Type: application/json' http://127.0.0.1:8080/v1/jobs \
  -d '{"left": {"blob": "<id>", "name": "old.bin"}, "right": {"blob": "<id>", "name": "new.bin"}}'
curl -s -H "$H" http://127.0.0.1:8080/v1/jobs/<job id>      # status; "result": "/v1/diffs/<id>" once done
curl -s -H "$H" http://127.0.0.1:8080/v1/diffs/<diff id>    # the Diff Document
curl -s -H "$H" -X POST http://127.0.0.1:8080/v1/jobs/<job id>/cancel
curl -s -H "$H" -X DELETE http://127.0.0.1:8080/v1/diffs/<diff id>  # also removes blobs nothing else uses
```

A job input is either `{"blob": "<sha256>"}` (a blob the same key uploaded: blobs are stored once by content, but each key can use only its own uploads) or one side of a saved diff, `{"diff": "<id>", "side": "left" | "right"}`, so a new version can be compared with the previous one without uploading it again. Only the last path component of a `name` is kept. A cancel accepted while the job runs wins over a result that arrives after it: the job ends `cancelled` with no saved diff. `GET /v1/formats` lists what the server compares and its limits; `GET /healthz` reports whether the database and the blob store answer.

The body of a diff response is the engine's canonical JSON (RFC 8785), byte for byte; the CLI prints the same bytes followed by a newline.

## Errors

Every error is JSON with a stable code:

```json
{"error": {"code": "input-too-large", "message": "an input of a synchronous diff is larger than 8388608 bytes; ..."}}
```

| Status | Codes |
|---|---|
| 400 | `invalid-request`, `invalid-options`, `invalid-blob-id`, `hash-mismatch`, `input-error`, `nul-character` |
| 401 | `unauthorized` |
| 403 | `forbidden` (missing scope), `host-not-allowed`, `origin-not-allowed` |
| 404 | `not-found`, `blob-not-found`, `job-not-found`, `diff-not-found` |
| 405 | `method-not-allowed` |
| 409 | `job-finished` |
| 413 | `payload-too-large`, `input-too-large`, `blob-too-large`, `job-too-large` |
| 415 | `unsupported-media-type` |
| 422 | `unsupported` |
| 429 | `rate-limited` (with `Retry-After`) |
| 431 | `headers-too-large` |
| 500 | `internal` |
| 503 | `db-unavailable`, `storage-unavailable`, `storage-conflict`, `busy`, `timeout` (with `Retry-After`) |

A failed job reports its error the same way in `error`. Only `503`-class failures (the database or the blob store was unavailable) are retried, up to `DIFFYMAN_JOB_MAX_ATTEMPTS`; input and option errors fail the job at once. Jobs also fail with `job-timeout` (over the time budget), `lease-expired` (the worker stopped renewing its lease after the last attempt) or, when a server stops mid-job and no attempt is left, `interrupted`.

## Configuration

Settings come from built-in defaults, then `diffyman.toml` (or the file in `DIFFYMAN_CONFIG`; unknown keys are errors), then `DIFFYMAN_*` variables. The full table is in `crates/diffyman-server/src/config.rs`; the ones most often changed:

| Variable | Default |
|---|---|
| `DIFFYMAN_BIND` | `127.0.0.1:8080` (the `local` environment accepts loopback addresses only) |
| `DIFFYMAN_DB_URL` | the local container, from `DIFFYMAN_PG_PASSWORD` |
| `DIFFYMAN_STORE_URL` | `file://~/diffyman-data/blobs` (`s3://bucket/prefix` with the `s3` feature) |
| `DIFFYMAN_ALLOWED_HOSTS` | `127.0.0.1`, `localhost` and `[::1]` with the bound port |
| `DIFFYMAN_ALLOWED_ORIGINS` | none (browser pages are refused) |
| `DIFFYMAN_MAX_SYNC_INPUT_BYTES` | 8 MiB per input of `POST /v1/diff` |
| `DIFFYMAN_MAX_BODY_BYTES` | 24 MiB per `POST /v1/diff` body |
| `DIFFYMAN_MAX_BLOB_BYTES` | 1 GiB per upload |
| `DIFFYMAN_REQUEST_TIMEOUT_SECS` | 30, for every request including uploads (raise it for very large blobs) |
| `DIFFYMAN_HEADER_READ_TIMEOUT_SECS` | 10 to send a request's headers, from the connection's start or from the first byte of a later request |
| `DIFFYMAN_IDLE_TIMEOUT_SECS` | 60 before a kept-alive connection with no request is closed |
| `DIFFYMAN_MAX_PARALLEL_DIFFS` | one per CPU core: synchronous diffs admitted at once, before their body is read |
| `DIFFYMAN_SYNC_MEMORY_BYTES` | 1 GiB shared by synchronous diffs, four bytes per body byte |
| `DIFFYMAN_JOB_TIMEOUT_SECS` | 600 |
| `DIFFYMAN_JOB_MEMORY_BYTES` | 2 GiB; a job's inputs may total a quarter of it |
| `DIFFYMAN_MAX_PARALLEL_JOBS` | half the CPU cores, between 1 and 4 |
| `DIFFYMAN_RATE_LIMIT_PER_MINUTE` | 600 per key (`0` turns it off) |
| `DIFFYMAN_DIFF_TTL_DAYS`, `DIFFYMAN_BLOB_TTL_DAYS`, `DIFFYMAN_JOB_TTL_DAYS` | 30 |
| `DIFFYMAN_LOG` | `info` |

## Retention

Saved diffs, jobs and blobs expire (30 days by default). Every hour, and at start, a sweep deletes expired diffs and their jobs, expired finished jobs, and expired blobs that nothing holds (the row is deleted and committed, then the object), then objects that have no row and unfinished uploads once they are older than a day. Uploading the same bytes again extends the uploading key's expiry. `DELETE /v1/diffs/{id}` removes a saved diff at once, with the job that produced it and every blob nothing else holds: no other job or saved diff uses it and no key's upload of it is still live.

## Tests

`cargo test -p diffyman-server` runs everywhere. Tests that need no database (authentication, `Host` and `Origin` checks, limits, typed errors, the listener and its shutdown, and every corpus case whose inputs are files, which must match the CLI's document) always run. Tests of keys, jobs, cancellation, saved diffs and retention need PostgreSQL: each runs in its own schema of a `diffyman_test` database, found through `DIFFYMAN_TEST_DB_URL` or the local default from `.env.local`, and prints why it skipped when none is reachable. CI sets `DIFFYMAN_TEST_DB_REQUIRED=1` in the job that has a PostgreSQL service, so a skip there is a failure.

When the API changes, `cargo test -p diffyman-server --test openapi` rewrites `docs/api/openapi.json` and fails once; commit the new file.

[`tests/server/run_e2e.py`](../tests/server/README.md) tests the built binary end to end: it starts `diffyman-server serve` on its own database (`diffyman_e2e`), blob folder and port, and checks every file case of the corpus through `POST /v1/diff` (JSON and multipart), jobs, saved diffs and deletion, keys, `Host`, `Origin` and limits, a 200 MB pair through a job, throughput, and that the log holds no content or secrets. `tests/conformance/run_bindings.py` has the server as one more surface. CI runs both against PostgreSQL 18.
