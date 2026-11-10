# REST API reference

`diffyman-server` is a local REST service over the diffyman engine. It answers with the same
[Diff Document](../spec/diff-document.md) as `diffyman diff --json`, keeps uploads and saved results
in a content-addressed blob store, and runs larger diffs as background jobs. This page covers
authentication, every endpoint with a `curl` example, the error codes and the limits. The
machine-readable reference is the OpenAPI 3.1 document [`openapi.json`](openapi.json), which the
server also serves at `/openapi.json`. How to run and configure the server is in the
[server guide](../server.md); the [REST quickstart](../quickstart/rest.md) goes from nothing to a
first diff in five minutes.

[`check_examples.py`](check_examples.py) runs every example on this page and in the quickstart
against a real server on its own test database and checks what each one prints (see
[Checking the examples](#checking-the-examples)).

## Conventions

- The base URL is `http://127.0.0.1:8080` unless `DIFFYMAN_BIND` says otherwise. The server
  listens on loopback addresses only in the `local` environment, and speaks plain HTTP/1.1.
- Every route under `/v1` needs an API key. `/healthz` and `/openapi.json` do not.
- Requests are JSON (`Content-Type: application/json`), except the two inputs of `POST /v1/diff`,
  which may also come as `multipart/form-data`, and `POST /v1/blobs`, whose body is the raw bytes.
  JSON objects with unknown fields are refused (`invalid-request`).
- Responses are JSON. A diff result is the engine's canonical JSON (RFC 8785) byte for byte: the
  bytes `diffyman diff --json` prints for the same inputs, names and options, without the final
  newline, apart from `provenance` (its `binding` is `server`, and it holds the timing).
- Problems with the inputs themselves (an invalid regular expression, an input over an engine
  limit) are a document with `status: "error"` and the reason in `warnings`, with status `200`, as
  in every binding ([spec §10](../spec/diff-document.md#10-warnings-and-errors)). HTTP errors are
  for requests the server refuses.
- Blob IDs are the lowercase hex SHA-256 of the bytes. Job and diff IDs are UUIDs (version 7).
  Times are RFC 3339 in UTC.
- Every response carries `X-Request-Id`, the ID of its line in the server log.

## Authentication

Send an API key as a bearer token:

<!-- check: skip header format -->
```http
Authorization: Bearer dmk_<64 hex digits>
```

Keys are made on the server's machine, never over HTTP: `diffyman-server keys create` prints a new
key once on standard output, and needs the admin token file that the first `serve` writes
(`~/.diffyman/admin-token`, mode 0600, or the path in `DIFFYMAN_ADMIN_TOKEN_FILE`). Whoever can
read that file can manage keys; the token itself never travels over the network and is never
logged.

<!-- check: skip needs the server's configuration; the quickstart runs these -->
```sh
diffyman-server keys create --name laptop            # every scope
diffyman-server keys create --name ci --scopes read  # read-only
diffyman-server keys list                            # IDs, prefixes, scopes, last use; never a secret
diffyman-server keys revoke <id>
```

| Scope | Allows |
|---|---|
| `read` | `GET` routes |
| `write` | `POST` routes |
| `delete` | `DELETE` routes |

- A key is 256 random bits from the operating system (`dmk_` and 64 hex digits). The database
  keeps only its first 12 characters, to find the row, and a keyed BLAKE3 hash, compared in
  constant time. A slow password hash would add nothing for a random 256-bit value and would cost
  every request tens of milliseconds.
- Jobs and saved diffs belong to the key that created them. Another key gets `404` for them, as if
  they did not exist.
- A missing, malformed, unknown or revoked key gets `401 unauthorized` (with
  `WWW-Authenticate: Bearer`); a key without the route's scope gets `403 forbidden`.
- Each key may send 600 requests a minute (`DIFFYMAN_RATE_LIMIT_PER_MINUTE`); over that, `429
  rate-limited` with `Retry-After`.
- `DIFFYMAN_AUTH_MODE=none` turns authentication off for quick experiments. Only the `local`
  environment accepts it.

## Host and Origin checks

Two checks run before authentication, on every route:

- **`Host`.** The request must name an allow-listed host: by default `127.0.0.1`, `localhost` and
  `[::1]` with the bound port (`DIFFYMAN_ALLOWED_HOSTS`). Anything else gets `403
  host-not-allowed`. A web page on another site that points its own name at `127.0.0.1` (DNS
  rebinding) sends its own name, so it is refused.
- **`Origin`.** Browsers send `Origin` with cross-site requests. A request that has one gets `403
  origin-not-allowed` unless the origin is in `DIFFYMAN_ALLOWED_ORIGINS` (none by default), so no
  web page can use the server unless you allow its origin. Allowed origins get CORS headers:
  methods `GET`, `POST`, `DELETE`, request headers `authorization` and `content-type`, readable
  response headers `location`, `retry-after` and `x-request-id`, and preflight answers cached for
  10 minutes.

A request with two or more `Host` headers gets `400 invalid-request`, whatever they say: it names
two hosts, and checking only one of them would let the other through.

Command-line tools and programs do not send `Origin`, so these checks never get in their way.

## Endpoints

| Method and path | Scope | What it does |
|---|---|---|
| [`GET /healthz`](#get-healthz) | none | Whether the database and the blob store answer |
| [`GET /openapi.json`](#get-openapijson) | none | The OpenAPI 3.1 document |
| [`GET /v1/formats`](#get-v1formats) | `read` | What the server compares, and its limits |
| [`POST /v1/diff`](#post-v1diff) | `write` | Diff two small inputs sent in the request; `?save=true` keeps the result |
| [`GET /v1/diffs/{id}`](#get-v1diffsid) | `read` | A saved Diff Document |
| [`DELETE /v1/diffs/{id}`](#delete-v1diffsid) | `delete` | Delete a saved diff and the blobs nothing else uses |
| [`POST /v1/blobs`](#post-v1blobs) | `write` | Upload an input for jobs; its ID is its SHA-256 |
| [`POST /v1/jobs`](#post-v1jobs) | `write` | Diff two stored inputs in the background |
| [`GET /v1/jobs/{id}`](#get-v1jobsid) | `read` | A job's status and, once done, its result |
| [`POST /v1/jobs/{id}/cancel`](#post-v1jobsidcancel) | `write` | Cancel a queued or running job |

The examples use two shell variables, the base URL and a key:

<!-- check: skip the check script sets API to its own server and KEY to a new key -->
```sh
API=http://127.0.0.1:8080
KEY=$(diffyman-server keys create --name docs)   # run on the server's machine, in its folder
```

and two small files:

<!-- check: run -->
```sh
printf 'apple\nbanana\ncherry\n' > old.txt
printf 'apple\nblueberry\ncherry\ndate\n' > new.txt
```

### `GET /healthz`

<!-- check: run -->
```sh
curl -s "$API/healthz"
```

<!-- check: output -->
```json
{"status":"ok","checks":{"database":"ok","storage":"ok"}}
```

`200` when both answer within 2 seconds, otherwise `503` with `"unavailable"` for the one that did
not. It needs no key, so a supervisor or a container health check can call it.

### `GET /openapi.json`

<!-- check: run -->
```sh
curl -s "$API/openapi.json" | jq -r '.openapi, .info.title, (.paths | keys | join(" "))'
```

<!-- check: output -->
```text
3.1.0
diffyman-server
/healthz /openapi.json /v1/blobs /v1/diff /v1/diffs/{id} /v1/formats /v1/jobs /v1/jobs/{id} /v1/jobs/{id}/cancel
```

The same bytes as [`openapi.json`](openapi.json) in this folder; a test regenerates that file when
the API changes.

### `GET /v1/formats`

The kinds, adapters, algorithm IDs and input formats of the engine (as `diffyman formats --json`
lists them, without folders, which the server never reads), and the server's limits:

<!-- check: run -->
```sh
curl -s "$API/v1/formats" -H "Authorization: Bearer $KEY" \
  | jq -c '{schemaVersion, kinds: [.kinds[] | {kind, algorithms}], limits}'
```

<!-- check: output -->
```json
{"schemaVersion":"0.4.0","kinds":[{"kind":"text","algorithms":["text.histogram@1","text.myers@1","text.patience@1"]},{"kind":"binary","algorithms":["binary.bytes@1"]},{"kind":"tree","algorithms":["tree.entries@1"]}],"limits":{"jobTimeoutSecs":600,"maxBlobBytes":1073741824,"maxBodyBytes":25165824,"maxJobInputBytes":536870912,"maxSyncInputBytes":8388608,"requestTimeoutSecs":30}}
```

### `POST /v1/diff`

Diffs two inputs sent in the request and returns the Diff Document at once. Each input may be at
most 8 MiB and the whole body 24 MiB; larger inputs go through [blobs and a job](#post-v1blobs).
The inputs come as JSON or as a multipart form; `options` is an options object
([spec §9](../spec/diff-document.md#9-options)), `{}` when left out. Options are read with the
engine's own parser, as `diffyman diff --options` reads them: an object that gives a key twice, at
any depth, is `400 invalid-options`, never its last value.

**JSON.** Each side has exactly one of `text` (UTF-8) or `base64` (standard, padded), and an
optional `name`, used for type detection by extension and recorded in the document (only its
last path component is kept, split on `/` and `\`):

<!-- check: run -->
```sh
curl -s "$API/v1/diff" -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  -d '{"left":  {"name": "old.txt", "text": "apple\nbanana\ncherry\n"},
       "right": {"name": "new.txt", "base64": "YXBwbGUKYmx1ZWJlcnJ5CmNoZXJyeQpkYXRlCg=="},
       "options": {"granularity": "word"}}' \
  | jq -c '{status, kind, similarity: .stats.similarity, binding: .provenance.binding}'
```

<!-- check: output -->
```json
{"status":"different","kind":"text","similarity":0.5714,"binding":"server"}
```

**Multipart.** Fields `left` and `right` (their file names are the input names) and an optional
`options` field with JSON text:

<!-- check: run -->
```sh
curl -s "$API/v1/diff" -H "Authorization: Bearer $KEY" \
  -F left=@old.txt -F right=@new.txt -F 'options={"granularity": "word"}' \
  | jq -c '.hunks[].ops[] | {op, leftRange, rightRange, inline: [.inline[] | {side, start, end}]}'
```

<!-- check: output -->
```json
{"op":"equal","leftRange":{"count":1,"start":1},"rightRange":{"count":1,"start":1},"inline":[]}
{"op":"replace","leftRange":{"count":1,"start":2},"rightRange":{"count":1,"start":2},"inline":[{"side":"left","start":0,"end":6},{"side":"right","start":0,"end":9}]}
{"op":"equal","leftRange":{"count":1,"start":3},"rightRange":{"count":1,"start":3},"inline":[]}
{"op":"insert","leftRange":{"count":0,"start":4},"rightRange":{"count":1,"start":4},"inline":[]}
```

The document is the CLI's, apart from `provenance`:

<!-- check: run -->
```sh
curl -s "$API/v1/diff" -H "Authorization: Bearer $KEY" -F left=@old.txt -F right=@new.txt \
  | jq -c 'del(.provenance)' > server.json
diffyman diff old.txt new.txt --json | jq -c 'del(.provenance)' > cli.json
cmp server.json cli.json && echo "the server's document is the CLI's"
```

<!-- check: output -->
```text
the server's document is the CLI's
```

**Saving.** With `?save=true` the server also keeps the document and both inputs, answers `201`,
and `Location` is the saved diff. Saved diffs expire after 30 days; their inputs can be the inputs
of a later [job](#post-v1jobs).

<!-- check: env -->
```sh
export DIFF=$(curl -s -o saved.json -w '%header{location}' "$API/v1/diff?save=true" \
  -H "Authorization: Bearer $KEY" -F left=@old.txt -F right=@new.txt)
echo "$DIFF"    # /v1/diffs/<id>
```

Errors: `400 invalid-request` (bad JSON, unknown fields, both or neither of `text` and `base64`),
`400 invalid-options` (also a multipart `options` field that is not JSON), `400 nul-character` (a name with a NUL character), `413 payload-too-large` or `input-too-large`, `415
unsupported-media-type`, `422 unsupported`, and `503 busy` or `timeout` when every diff slot stays
taken or the diff runs past the request time limit (30 s). A diff is admitted before its body is
read: at most `DIFFYMAN_MAX_PARALLEL_DIFFS` at once (one per CPU core), together within
`DIFFYMAN_SYNC_MEMORY_BYTES` (1 GiB, four bytes per byte of `Content-Length`), and a request that
waits half its time limit without admission gets `503 busy`. The budget bounds the server's
resident memory: blocks of 1 MiB or more go back to the operating system as soon as they are
freed, so the memory of finished diffs is not kept.

### `GET /v1/diffs/{id}`

The saved document, byte for byte as it was returned when it was made:

<!-- check: run -->
```sh
curl -s "$API$DIFF" -H "Authorization: Bearer $KEY" | cmp - saved.json && echo "the saved bytes"
```

<!-- check: output -->
```text
the saved bytes
```

`404 diff-not-found` for an unknown ID, another key's diff, or one that has expired.

### `POST /v1/blobs`

Uploads one input for jobs. The body is the raw bytes, streamed to the store while it is hashed;
`Content-Type` is recorded as the blob's media type. With `?sha256=<hex>` the upload is refused
unless the bytes hash to it. A blob belongs to the key that uploaded it: only that key can start
jobs on it, and the answer never shows whether another key stored the same bytes. `201` when the
bytes are new for this key, `200` when this key had already uploaded them (the expiry is extended):

<!-- check: run -->
```sh
printf 'start\nload config\nserve\n' > v1.log
printf 'start\nload config v2\nserve\nstop\n' > v2.log
curl -s -o blob.json -w '%{http_code}\n' "$API/v1/blobs" -H "Authorization: Bearer $KEY" \
  -H 'Content-Type: text/plain' --data-binary @v1.log
jq -c '{blobId, size, mediaType}' blob.json
curl -s -o /dev/null -w '%{http_code}\n' "$API/v1/blobs" -H "Authorization: Bearer $KEY" \
  --data-binary @v1.log
```

<!-- check: output -->
```text
201
{"blobId":"07d419baf9701031fc574207f37272559e1f10c1e9f0655d80415a4f877aa78a","size":24,"mediaType":"text/plain"}
200
```

The response also has `createdAt` and `expiresAt` (this key's first upload, and 30 days after its
last upload of the same bytes; a job or a saved diff that uses the blob keeps it longer). Keep the
IDs for the job:

<!-- check: env -->
```sh
export LEFT=$(curl -s "$API/v1/blobs" -H "Authorization: Bearer $KEY" --data-binary @v1.log | jq -r .blobId)
export RIGHT=$(curl -s "$API/v1/blobs" -H "Authorization: Bearer $KEY" --data-binary @v2.log | jq -r .blobId)
```

A hash that does not match:

<!-- check: run -->
```sh
curl -s -w ' %{http_code}\n' "$API/v1/blobs?sha256=$RIGHT" -H "Authorization: Bearer $KEY" \
  --data-binary @v1.log
```

<!-- check: output -->
```text
{"error":{"code":"hash-mismatch","message":"content does not match the expected SHA-256"}} 400
```

Errors: `400 invalid-blob-id`, `hash-mismatch` or `input-error` (the body broke off), `413
blob-too-large` (over 1 GiB, refused from `Content-Length` before reading when it is given), and
`503 timeout` when the upload takes longer than the request time limit (30 s; raise
`DIFFYMAN_REQUEST_TIMEOUT_SECS` for very large blobs on slow links).

### `POST /v1/jobs`

Creates a background diff of two stored inputs and answers `202` at once, with `Location` set to
the job. Each input is an uploaded blob, `{"blob": "<sha256>"}`, or one side of a saved diff,
`{"diff": "<id>", "side": "left"}` (or `"right"`), so a new version can be compared with the
previous one without sending it again; both take an optional `name`, of which only the last path
component is kept and stored (split on `/` and `\`, as the engine does), so `/home/me/v1.log` is
recorded and shown as `v1.log`. `options` is the same object as for `POST /v1/diff`.

<!-- check: env -->
```sh
jq -n --arg left "$LEFT" --arg right "$RIGHT" \
  '{left: {blob: $left, name: "v1.log"}, right: {blob: $right, name: "v2.log"}, options: {context: 1}}' \
  > job-request.json
export JOB=$(curl -s -o job.json -w '%header{location}' "$API/v1/jobs" \
  -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' --data-binary @job-request.json)
```

<!-- check: run -->
```sh
jq -c '{status, left: .left.name, right: .right.name, attempts, maxAttempts}' job.json
```

<!-- check: output -->
```json
{"status":"queued","left":"v1.log","right":"v2.log","attempts":0,"maxAttempts":3}
```

The inputs of one job may total a quarter of the job memory budget (512 MiB of the default 2
GiB), and at most `DIFFYMAN_MAX_PARALLEL_JOBS` jobs run at once; the rest wait in the queue.
Errors: `400 invalid-request`, `invalid-blob-id`, `invalid-options` or `nul-character` (a name or
the options hold a NUL character, U+0000, which the database cannot store; such options still work
with `POST /v1/diff`), `404 blob-not-found` (also for a blob only another key uploaded) or
`diff-not-found`, `413 job-too-large`.

### `GET /v1/jobs/{id}`

The job's `status` is `queued`, `running`, `succeeded`, `failed` or `cancelled`. Once it has
succeeded, `result` is the path of the saved Diff Document (and `diffId` its ID); once it has
failed, `error` holds a code and a message. Poll until it is done:

<!-- check: env -->
```sh
while :; do
  curl -s "$API$JOB" -H "Authorization: Bearer $KEY" > job.json
  case $(jq -r .status job.json) in queued | running) sleep 0.2 ;; *) break ;; esac
done
export RESULT=$(jq -r .result job.json)
```

<!-- check: run -->
```sh
jq -c '{status, error}' job.json
curl -s "$API$RESULT" -H "Authorization: Bearer $KEY" \
  | jq -c '{status, left: .left.name, right: .right.name, added: .stats.linesAdded, removed: .stats.linesRemoved}'
```

<!-- check: output -->
```json
{"status":"succeeded","error":null}
{"status":"different","left":"v1.log","right":"v2.log","added":2,"removed":1}
```

A job's result is a saved diff like any other: `GET` and `DELETE /v1/diffs/{id}` work on it. Job
records expire after 30 days. `404 job-not-found` for an unknown ID or another key's job.

### `POST /v1/jobs/{id}/cancel`

Cancels a queued job at once (`200`, status `cancelled`) or a running one at its worker's next
heartbeat (`202`; poll the job). A cancel accepted with `202` always wins: if the engine finishes
before that heartbeat, its result is discarded (no saved diff is kept, `result` and `diffId` stay
`null`) and the job ends `cancelled`. A finished job cannot be cancelled:

<!-- check: run -->
```sh
curl -s -w ' %{http_code}\n' -X POST "$API$JOB/cancel" -H "Authorization: Bearer $KEY"
```

<!-- check: output -->
```text
{"error":{"code":"job-finished","message":"the job has already finished"}} 409
```

### `DELETE /v1/diffs/{id}`

Deletes a saved diff at once, with the job that produced it and every blob (its inputs and the
document) that nothing else holds: no other job or saved diff uses it, and no key's upload of the
same bytes (another key's included) is still live. `204` with no body:

<!-- check: run -->
```sh
curl -s -o /dev/null -w '%{http_code}\n' -X DELETE "$API$RESULT" -H "Authorization: Bearer $KEY"
curl -s -w ' %{http_code}\n' "$API$RESULT" -H "Authorization: Bearer $KEY"
curl -s -w ' %{http_code}\n' "$API$JOB" -H "Authorization: Bearer $KEY"
```

<!-- check: output -->
```text
204
{"error":{"code":"diff-not-found","message":"no such diff"}} 404
{"error":{"code":"job-not-found","message":"no such job"}} 404
```

The records go first, then the stored objects. If an object cannot be deleted from the store, the
diff is still deleted (`204`) and the sweep removes the object later; `503` means nothing was
deleted.

## Errors

Every error is a JSON object with a stable code and a message for people:

<!-- check: run -->
```sh
curl -s -w ' %{http_code}\n' "$API/v1/formats"
curl -s -w ' %{http_code}\n' "$API/v1/formats" -H 'Authorization: Bearer dmk_not-a-key'
```

<!-- check: output -->
```text
{"error":{"code":"unauthorized","message":"send Authorization: Bearer <API key>"}} 401
{"error":{"code":"unauthorized","message":"the API key is not valid or has been revoked"}} 401
```

Branch on `code`, never on `message`. New codes may be added; treat an unknown one by its HTTP
status. Messages never hold file content, file names, paths, secrets or any other part of the
request: a body that is not valid JSON for the route gets a fixed message with the position and
the kind of error (`invalid JSON body: data error at line 1, column 23: a field is unknown,
missing or of the wrong type`; the kinds are `syntax`, `eof`, `data` and `io`), never the text it
choked on.

`invalid-options` is the engine's own code for options it refuses, the code `diffyman diff
--options` reports for the same options. Its message is one of three fixed texts, and names no
field: an unknown or repeated key is itself request content, and so is a value of the wrong type.

| Message | When |
|---|---|
| `invalid options: an object in the options gives a key twice` | A key is given twice in one object, at any depth |
| `invalid options: the options are not valid JSON` | A multipart `options` field that does not parse |
| `invalid options: an option is unknown, of the wrong type or out of range` | Anything else spec §9 refuses |

To find the field, run the same options through `diffyman diff --options`, whose message names it.

A request that hyper, the server's HTTP library, refuses before routing gets the same typed body,
with `X-Request-Id` and `Connection: close`: `400 invalid-request` for a request line or header that
is not valid HTTP/1.1 (including the bytes after a body whose `Content-Length` understated it, read
as the next request) and for a request target over 65,534 bytes (`400 invalid-blob-id` when what
makes it that long is the `?sha256=` value of `POST /v1/blobs`, as for any other value that is not
a blob ID), and `431 headers-too-large` for a request head over about 400 KiB or with more than 100
headers. Three refusals carry no answer at
all, because no request was received to answer: a connection closed by the header-read or idle
limit ([Limits](#limits)), one whose client went away, and an HTTP/2 connection preface (the server
speaks HTTP/1.1 only), which is closed at once.

| Status | Code | Meaning |
|---|---|---|
| 400 | `invalid-request` | The request is malformed: bad JSON, unknown or missing fields, a bad query parameter or path ID, two `Host` headers, a request line or header that is not HTTP/1.1, a target over 65,534 bytes |
| 400 | `invalid-options` | The options object is not valid ([spec §9](../spec/diff-document.md#9-options)) |
| 400 | `invalid-blob-id` | A blob ID is not 64 lowercase hex digits |
| 400 | `hash-mismatch` | The uploaded bytes do not hash to `?sha256=` |
| 400 | `input-error` | Reading the request body failed |
| 400 | `nul-character` | An input name, or a job's options, holds a NUL character (U+0000) |
| 401 | `unauthorized` | No key, or a key that is malformed, unknown or revoked |
| 403 | `forbidden` | The key lacks the route's scope |
| 403 | `host-not-allowed` | `Host` is not allow-listed |
| 403 | `origin-not-allowed` | `Origin` is not allow-listed |
| 404 | `not-found` | No such route |
| 404 | `blob-not-found` | No such blob, or a saved diff that did not keep its inputs |
| 404 | `job-not-found` | No such job for this key |
| 404 | `diff-not-found` | No such saved diff for this key, or it has expired |
| 405 | `method-not-allowed` | The route does not take this method |
| 409 | `job-finished` | The job has already finished |
| 413 | `payload-too-large` | The request body is over its limit |
| 413 | `input-too-large` | An input of a synchronous diff is over 8 MiB |
| 413 | `blob-too-large` | An upload is over the blob limit |
| 413 | `job-too-large` | A job's inputs are over its memory budget |
| 415 | `unsupported-media-type` | The body is not a media type the route takes |
| 422 | `unsupported` | This build cannot compare these inputs |
| 429 | `rate-limited` | The key sent too many requests; see `Retry-After` |
| 431 | `headers-too-large` | The request head is over about 400 KiB or has more than 100 headers |
| 500 | `internal` | A bug; the server log has the request ID |
| 503 | `db-unavailable` | The database did not answer; see `Retry-After` |
| 503 | `storage-unavailable` | The blob store did not answer; see `Retry-After` |
| 503 | `storage-conflict` | A concurrent write to the same blob; try again |
| 503 | `busy` | Every synchronous diff slot, or the memory they share, stayed taken; see `Retry-After` |
| 503 | `timeout` | The request ran past its time limit; see `Retry-After` |

A failed job reports its error in the same shape in `error`. Besides the codes above, a job can
fail with `job-timeout` (over the job time budget, 600 s), `lease-expired` (its worker stopped
renewing the lease after the last attempt) or `interrupted` (the server stopped during the job
and no attempt was left). Only infrastructure errors (the `503` codes) are retried, up to 3
attempts; input and option errors fail the job at once.

More examples, each with its stable code:

<!-- check: run -->
```sh
curl -s -w ' %{http_code}\n' "$API/healthz" -H 'Host: attacker.example'
curl -s -w ' %{http_code}\n' "$API/healthz" -H 'Origin: https://attacker.example'
curl -s -w ' %{http_code}\n' "$API/v1/diff" -H "Authorization: Bearer $KEY" \
  -H 'Content-Type: text/plain' --data-binary @old.txt
curl -s -w ' %{http_code}\n' "$API/v1/diff" -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  -d '{"left": {"text": "a"}, "right": {"text": "b"}, "options": {"granularity": "sentence"}}'
head -c 8388609 /dev/zero | tr '\0' x | jq -Rs '{left: {text: .}, right: {text: ""}}' > big.json
curl -s -w ' %{http_code}\n' "$API/v1/diff" -H "Authorization: Bearer $KEY" \
  -H 'Content-Type: application/json' --data-binary @big.json
```

<!-- check: output -->
```text
{"error":{"code":"host-not-allowed","message":"the Host header is not one this server answers to"}} 403
{"error":{"code":"origin-not-allowed","message":"requests from this origin are not allowed"}} 403
{"error":{"code":"unsupported-media-type","message":"send application/json or multipart/form-data"}} 415
{"error":{"code":"invalid-options","message":"invalid options: an option is unknown, of the wrong type or out of range"}} 400
{"error":{"code":"input-too-large","message":"an input of a synchronous diff is larger than 8388608 bytes; upload blobs and create a job instead"}} 413
```

## Limits

Every size and time has a limit. The defaults, the setting that changes each one and the error
when it is passed:

| Limit | Default | Setting | Error |
|---|---|---|---|
| One input of `POST /v1/diff` | 8 MiB | `DIFFYMAN_MAX_SYNC_INPUT_BYTES` | `413 input-too-large` |
| Body of `POST /v1/diff` | 24 MiB | `DIFFYMAN_MAX_BODY_BYTES` | `413 payload-too-large` |
| `options` part of a multipart body, body of `POST /v1/jobs` | 1 MiB | fixed | `413 payload-too-large` |
| One blob | 1 GiB | `DIFFYMAN_MAX_BLOB_BYTES` | `413 blob-too-large` |
| Input names | 1024 characters, no NUL character | fixed | `400 invalid-request`, `400 nul-character` |
| Any request, uploads included | 30 s | `DIFFYMAN_REQUEST_TIMEOUT_SECS` | `503 timeout` |
| Sending a request's headers: from the connection's start, or from the first byte of a later request on a kept-alive connection | 10 s | `DIFFYMAN_HEADER_READ_TIMEOUT_SECS` | the connection is closed |
| A kept-alive connection with no request | 60 s | `DIFFYMAN_IDLE_TIMEOUT_SECS` | the connection is closed |
| Synchronous diffs at once | one per CPU core | `DIFFYMAN_MAX_PARALLEL_DIFFS` | `503 busy` after half the request time limit |
| Memory of synchronous diffs, together | 1 GiB, four bytes per body byte | `DIFFYMAN_SYNC_MEMORY_BYTES` | `503 busy` after half the request time limit |
| Inputs of one job, in total | a quarter of the job memory budget (512 MiB) | `DIFFYMAN_JOB_MEMORY_BYTES` (2 GiB) | `413 job-too-large` |
| One job's run time | 600 s | `DIFFYMAN_JOB_TIMEOUT_SECS` | job fails with `job-timeout` |
| Jobs running at once | half the CPU cores, 1 to 4 | `DIFFYMAN_MAX_PARALLEL_JOBS` | jobs wait in the queue |
| Attempts per job | 3 | `DIFFYMAN_JOB_MAX_ATTEMPTS` | job fails with its last error |
| Requests per key | 600 a minute | `DIFFYMAN_RATE_LIMIT_PER_MINUTE` (`0` turns it off) | `429 rate-limited` |

The engine's own limits apply on top ([spec §7, §17, §19.6](../spec/diff-document.md)): an input
over one of them gives a document with `status: "error"` and a `limit-exceeded` warning, or a
`tree-limit` warning, never a crash. `GET /v1/formats` reports the server's current limits.

A body with a `Content-Length` of at most 64 KiB is read in full before the request is handled,
so the connection stays open for the next request even when the answer is an error. A refused
request with a larger or chunked body is answered without waiting for the body, and the
server may then close the connection; a client that streams a large body should send it on a
new connection, or one it has checked is still open.

## Retention

Saved diffs, jobs and blobs expire after 30 days (`DIFFYMAN_DIFF_TTL_DAYS`, `DIFFYMAN_JOB_TTL_DAYS`,
`DIFFYMAN_BLOB_TTL_DAYS`). A sweep runs at start and every hour: it deletes expired diffs with their
jobs, expired finished jobs, expired blobs that nothing holds (the row, then the object), then
stored objects that have no row and unfinished uploads older than a day. Uploading the same bytes
again extends the uploading key's expiry. `DELETE /v1/diffs/{id}` removes a saved
diff at once.

## Checking the examples

`check_examples.py` runs every example on this page and in the [REST quickstart](../quickstart/rest.md)
and compares what each prints with the page. It starts `diffyman-server serve` on its own
database (`diffyman_docs`, dropped at the end; the main `diffyman` database is never used), a
fresh blob folder and a random port, and stops it at the end:

<!-- check: skip runs this check -->
```sh
scripts/local-up.sh                                       # PostgreSQL on 127.0.0.1:5433
cargo build --locked --release -p diffyman-server -p diffyman-cli
uv run --no-project --python 3.12 python docs/api/check_examples.py
```

It needs `curl`, `jq` and `psql`, and finds the database server like the Rust tests do
(`DIFFYMAN_TEST_DB_URL`, or `127.0.0.1:5433` with the password from `.env.local`). Without one it
says why and exits 0, or 1 with `DIFFYMAN_TEST_DB_REQUIRED=1`.
