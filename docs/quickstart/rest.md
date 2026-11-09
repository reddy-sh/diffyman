# Quickstart: REST API

`diffyman-server` is a local REST service over the same engine as the CLI and the bindings. It
returns the same [Diff Document](../spec/diff-document.md), keeps saved results for later, and
runs large diffs as background jobs. This page starts it, makes an API key and diffs two files
with `curl`; the [API reference](../api/README.md) has every endpoint.

## What you need

- git, and the Rust toolchain pinned in [`rust-toolchain.toml`](../../rust-toolchain.toml)
  ([rustup](https://rustup.rs) installs it on the first build).
- Docker with Compose, for PostgreSQL 18.
- `curl` 7.84 or newer, and [jq](https://jqlang.org) 1.7 or newer.

## Build from source

Nothing is on a package registry yet. Build the server and the CLI from a clone:

<!-- check: skip clone -->
```sh
git clone https://github.com/reddy-sh/diffyman
cd diffyman
```

<!-- check: build -->
```sh
cargo build --release --locked -p diffyman-server -p diffyman-cli
```

Run the commands of the next three sections from the root of the clone: the server and its
`keys` command read the database password from `.env.local` there.

<!-- check: skip the check has both binaries on PATH -->
```sh
export PATH="$PWD/target/release:$PATH"
```

## Start PostgreSQL and the server

`scripts/local-up.sh` starts PostgreSQL 18 in Docker on `127.0.0.1:5433`. On its first run it
writes a random database password to `.env.local` (mode 0600, ignored by git) and creates
`~/diffyman-data` for blobs, temporary files and backups.

<!-- check: skip the check starts its own database and server -->
```sh
scripts/local-up.sh
```

Then start the server, and leave it running in this terminal:

<!-- check: skip the check starts its own database and server -->
```sh
diffyman-server serve
```

It creates the database tables, writes the admin token file once and says where it is (never
what it holds), and listens on `127.0.0.1` only:

<!-- check: skip what serve prints -->
```text
diffyman-server: admin token written to /home/you/.diffyman/admin-token (keep it private; it creates API keys)
diffyman-server: listening on http://127.0.0.1:8080
```

Ctrl-C stops it gracefully. Every later start keeps the same token file (`kept in`).

## Make an API key

The admin token file is readable by you only, and whoever can read it can make API keys:

<!-- check: run -->
```sh
ls -l ~/.diffyman/admin-token | cut -c1-10
```

<!-- check: output -->
```text
-rw-------
```

In a second terminal, from the root of the clone, create a key. It is printed once, on standard
output; the database keeps only a prefix and a hash of it:

<!-- check: env -->
```sh
export KEY=$(diffyman-server keys create --name quickstart)
```

<!-- check: run -->
```sh
echo "$KEY" | cut -c1-4
echo "$KEY" | tr -d '\n' | wc -c | tr -d ' '
```

<!-- check: output -->
```text
dmk_
68
```

`dmk_` and 64 hex digits: 256 random bits. `diffyman-server keys list` shows each key's ID,
prefix, scopes and last use, never the key itself.

The key has every scope (`read`, `write`, `delete`); `--scopes read` makes a read-only one, and
`diffyman-server keys revoke <id>` revokes it. Then set the server's address:

<!-- check: skip the check sets API to its own server -->
```sh
export API=http://127.0.0.1:8080
```

<!-- check: run -->
```sh
curl -s "$API/healthz"
```

<!-- check: output -->
```json
{"status":"ok","checks":{"database":"ok","storage":"ok"}}
```

## Diff two files

The rest works from any folder. Make two small files to compare:

<!-- check: run -->
```sh
printf 'apple\nbanana\ncherry\n' > old.txt
printf 'apple\nblueberry\ncherry\ndate\n' > new.txt
```

Send them as a multipart form, with options as JSON, and save the result: `?save=true` answers
`201` with the saved diff in `Location`.

<!-- check: env -->
```sh
export DIFF=$(curl -s -o result.json -w '%header{location}' "$API/v1/diff?save=true" \
  -H "Authorization: Bearer $KEY" \
  -F left=@old.txt -F right=@new.txt -F 'options={"granularity": "word"}')
```

The body is the Diff Document. It holds line ranges, not the text, so this prints the verdict, the
stats and each change op:

<!-- check: run -->
```sh
jq -r '"\(.status), \(.stats.changeCount) changes, similarity \(.stats.similarity)",
       (.hunks[].ops[] | select(.op != "equal")
        | "\(.op) left \(.leftRange.start)+\(.leftRange.count) right \(.rightRange.start)+\(.rightRange.count)")' result.json
```

<!-- check: output -->
```text
different, 2 changes, similarity 0.5714
replace left 2+1 right 2+1
insert left 4+0 right 4+1
```

Get the saved document back later, as the same bytes:

<!-- check: run -->
```sh
curl -s "$API$DIFF" -H "Authorization: Bearer $KEY" -o saved.json
cmp saved.json result.json && echo "saved at /v1/diffs/<id>, unchanged"
```

<!-- check: output -->
```text
saved at /v1/diffs/<id>, unchanged
```

## An HTML report with the CLI

The saved document is the one `diffyman diff --json` prints for the same files and options, apart
from `provenance` (which says `"binding": "server"` and holds the timing):

<!-- check: run -->
```sh
jq -c 'del(.provenance)' saved.json > server.json
diffyman diff old.txt new.txt --granularity word --json | jq -c 'del(.provenance)' > cli.json
cmp server.json cli.json && echo "same document"
```

<!-- check: output -->
```text
same document
```

A report shows the lines themselves, which the document leaves out, so the CLI renders it from
the two files with the same options:

<!-- check: run exit=1 -->
```sh
diffyman diff old.txt new.txt --granularity word -f html -o report.html
```

<!-- check: run -->
```sh
grep -q 'blueberry' report.html && echo "report.html is ready"
```

<!-- check: output -->
```text
report.html is ready
```

Open `report.html` in a browser for the side-by-side and inline views. The exit code 1 means the
files differ, as with `diff`.

## Larger files

`POST /v1/diff` takes inputs of up to 8 MiB each. For larger ones, upload each file as a blob
(its ID is its SHA-256) and create a job, which runs in the background:

<!-- check: env -->
```sh
seq 1 200000 > big-old.txt
seq 1 200000 | sed 's/^150000$/150000 changed/' > big-new.txt
export LEFT=$(curl -s "$API/v1/blobs" -H "Authorization: Bearer $KEY" --data-binary @big-old.txt | jq -r .blobId)
export RIGHT=$(curl -s "$API/v1/blobs" -H "Authorization: Bearer $KEY" --data-binary @big-new.txt | jq -r .blobId)
export JOB=$(curl -s -o /dev/null -w '%header{location}' "$API/v1/jobs" \
  -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  -d "{\"left\": {\"blob\": \"$LEFT\"}, \"right\": {\"blob\": \"$RIGHT\"}}")
```

Poll the job until it is done; then `result` is the saved document:

<!-- check: run -->
```sh
until curl -s "$API$JOB" -H "Authorization: Bearer $KEY" > job.json \
  && jq -e '.status != "queued" and .status != "running"' job.json > /dev/null; do sleep 0.2; done
jq -r .status job.json
curl -s "$API$(jq -r .result job.json)" -H "Authorization: Bearer $KEY" \
  | jq -c '{status, linesLeft: .stats.linesLeft, changed: [.hunks[].ops[] | select(.op != "equal") | .leftRange.start]}'
```

<!-- check: output -->
```text
succeeded
{"status":"different","linesLeft":200000,"changed":[150000]}
```

## Clean up

Delete the saved diff; the server also removes the stored inputs that nothing else uses.
Saved diffs, jobs and blobs expire after 30 days anyway.

<!-- check: run -->
```sh
curl -s -o /dev/null -w '%{http_code}\n' -X DELETE "$API$DIFF" -H "Authorization: Bearer $KEY"
curl -s "$API$DIFF" -H "Authorization: Bearer $KEY" | jq -r .error.code
```

<!-- check: output -->
```text
204
diff-not-found
```

Errors are JSON with a stable `code`, as above; the [API reference](../api/README.md#errors) lists
them all. Stop the server with Ctrl-C, and PostgreSQL with
`docker compose -f deploy/local/compose.yaml --env-file .env.local stop` (its data stays in a
Docker volume).

## Next

- [API reference](../api/README.md): authentication, every endpoint, error codes and limits;
  the [OpenAPI 3.1 document](../api/openapi.json).
- [Server guide](../server.md): configuration, retention and tests; [operations](../operations.md):
  backups and restore drills.
- [Diff Document specification](../spec/diff-document.md), and the same document from the
  [CLI](cli.md), [Python](python.md), [Node.js](node.md), [Java](java.md),
  [the browser](browser-react.md) and [C](c.md).
