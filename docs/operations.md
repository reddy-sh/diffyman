# Operating the local service

How to run `diffyman-server` on one machine, keep its data safe, and check that the same setup works with S3-compatible storage and in a container. The design is in [ADR 0012](adr/0012-local-service-architecture.md). The engine, the CLI and the bindings need none of this.

Everything listens on `127.0.0.1` only.

| Port | What |
|---|---|
| 5433 | PostgreSQL 18 (compose service `postgres`) |
| 8080 | `diffyman-server`, run natively |
| 8081 | `diffyman-server` in a container (compose profile `server`) |
| 9000 / 9001 | RustFS S3 API and console (compose profile `s3`) |
| 4317 / 4318 / 3001 | OpenTelemetry collector and Grafana (compose profile `obs`) |

| Folder | What |
|---|---|
| `~/diffyman-data/blobs` | blob store (`file://`), objects at `blobs/sha256/<2 hex>/<64 hex>` |
| `~/diffyman-data/tmp` | temporary files |
| `~/diffyman-data/backups` | database dumps and `backup.log` |
| `~/.diffyman/admin-token` | admin token (mode 0600) |
| `.env.local` (repository root) | local secrets, mode 0600, never committed |

## Start and stop

```sh
scripts/local-up.sh                 # Postgres; creates .env.local and ~/diffyman-data on first run
scripts/local-up.sh s3              # also RustFS, and creates the bucket diffyman-test
scripts/with-rustup cargo run --release -p diffyman-server -- serve
```

`serve` applies the database migrations, writes the admin token file if it is missing, and listens on `127.0.0.1:8080`. The local environment (`DIFFYMAN_ENV=local`, the default) reads `.env.local` and builds the database URL from `DIFFYMAN_PG_PASSWORD`. Every setting is listed in the module documentation of `crates/diffyman-server/src/config.rs`.

Stop the server with Ctrl-C. Stop the containers, keeping their data:

```sh
docker compose -f deploy/local/compose.yaml --env-file .env.local --profile s3 --profile server stop
```

`down` also removes the containers, but keeps the named volumes. Volumes hold the database: remove one only on purpose, by name (for example `docker volume rm diffyman_serverdata`), and never with `down -v` or a prune.

## Keys

- **Admin token.** Written once, on the first `serve`, to `~/.diffyman/admin-token` (`DIFFYMAN_ADMIN_TOKEN_FILE`). The file is created exclusively with mode 0600 in a folder with mode 0700, and the server refuses it later if group or others can read it. The token is never logged or stored in the database. To rotate it, stop the server, delete the file and start again; existing API keys stay valid.
- **API keys.** Created and revoked with the admin token file:

  ```sh
  scripts/with-rustup cargo run -p diffyman-server -- keys create --name laptop   # prints dmk_… once
  scripts/with-rustup cargo run -p diffyman-server -- keys list
  scripts/with-rustup cargo run -p diffyman-server -- keys revoke <id>
  ```

  Clients send the key as `Authorization: Bearer dmk_…`. The database keeps a 12-character prefix and a keyed BLAKE3 hash, never the key. A lost key cannot be recovered: revoke it and create a new one.
- **Database and S3 test secrets.** `DIFFYMAN_PG_PASSWORD`, `DIFFYMAN_S3_TEST_KEY` and `DIFFYMAN_S3_TEST_SECRET` live in `.env.local`, which `scripts/local-up.sh` generates with random values. The scripts below read it themselves and never print it.

## Backups

```sh
scripts/backup-db.sh                # one dump now
scripts/install-backup-job.sh       # show the nightly job and what installing it would do
scripts/install-backup-job.sh --install          # install it (macOS launchd, 03:15 by default)
scripts/install-backup-job.sh --uninstall
```

`backup-db.sh` runs `pg_dump --format=custom` inside the `postgres` container, over the container's local socket, so the password never leaves it. The dump is written under a temporary name and kept only when `pg_restore --list` reads it back; then it becomes `~/diffyman-data/backups/diffyman-<UTC time>.dump` (mode 0600, folder 0700) with a `.sha256` file next to it. Dumps older than 7 days are removed after each successful backup (`--keep-days N`), and the newest dump is never removed. With `--host-pg HOST:PORT` it uses the `pg_dump` on `PATH` against a Postgres that is not in the container; that `pg_dump` must be at least the server's version.

The launchd job (`sh.reddy.diffyman.backup`) runs the same script every night and appends to `~/diffyman-data/backups/backup.log`. A run missed while the Mac sleeps starts at the next wake. Run it once on demand with `launchctl kickstart gui/$(id -u)/sh.reddy.diffyman.backup`. On Linux, call `scripts/backup-db.sh` from a cron entry or a systemd timer instead.

What a dump holds: every table, including job file names, key prefixes and hashes, but no file content. Treat dumps as private. **Blobs are not in the dump.** The blob folder is content-addressed and append-mostly, so copy it with any file backup (Time Machine, `rsync -a`). When restoring both, restore the blobs first.

## Restore drill

```sh
scripts/restore-drill.sh            # the newest dump in ~/diffyman-data/backups
scripts/restore-drill.sh --dump ~/diffyman-data/backups/diffyman-20261109T031500Z.dump
```

The drill checks the dump's SHA-256 and restores it into a temporary database (`diffyman_drill_<time>`) on the same server. It then compares row counts with the live database, checks that every diff and job in the dump points at a blob row that is also in the dump, and samples 20 blob rows (`--sample N`) against the blob store: the object must exist and have the recorded size, and objects up to 64 MiB are hashed. The temporary database is dropped at the end, also when a check fails, and the live database is only read. A sampled blob that is missing from the store fails the drill only while the live database still has its row: blobs deleted after the backup are expected to be gone. With an `s3://` store, pass `--skip-blobs`.

Run it after setting up backups, and then monthly. A dump of a database the server has not migrated yet restores fine and reports that it has no diffyman tables.

To rehearse on real data without touching the main database, run the server end-to-end test with `--keep` ([`tests/server`](../tests/server/README.md)), which leaves its database `diffyman_e2e` and its blob folder in place, then back up and drill that database into a folder of its own (`--database` names the database to dump, and the live one the drill compares with):

```sh
scripts/backup-db.sh --database diffyman_e2e --dir ~/diffyman-data/tmp/e2e-backups
scripts/restore-drill.sh --database diffyman_e2e --dir ~/diffyman-data/tmp/e2e-backups \
  --store-dir ~/diffyman-data/tmp/e2e-<id>/blobs --sample 100
```

**A real restore** replaces the live database. Run the drill on the dump first, stop the server, then:

```sh
c=(docker compose -f deploy/local/compose.yaml --env-file .env.local exec -T postgres)
"${c[@]}" psql -U diffyman -d postgres -c 'DROP DATABASE diffyman WITH (FORCE)'
"${c[@]}" psql -U diffyman -d postgres -c 'CREATE DATABASE diffyman'
"${c[@]}" pg_restore -U diffyman -d diffyman --no-owner --exit-on-error < ~/diffyman-data/backups/diffyman-<time>.dump
```

## S3-compatible storage

The compose profile `s3` runs RustFS 1.0.4 on `127.0.0.1:9000` with the test credentials from `.env.local`.

```sh
scripts/local-up.sh s3                     # starts RustFS and creates the bucket diffyman-test
scripts/s3-bucket.sh other-bucket          # another bucket
scripts/with-local-s3 scripts/with-rustup cargo run --release -p diffyman-server --features s3 -- serve
```

`s3://` stores need a build with the `s3` feature. `scripts/s3-bucket.sh` uses the aws CLI when it is installed, otherwise a standard-library Python client (`scripts/lib/s3_bucket.py`, run with `uv`). `scripts/with-local-s3` runs a command with these variables, which the server's S3 client reads directly:

| Variable | Value |
|---|---|
| `DIFFYMAN_STORE_URL` | `s3://diffyman-test` (kept if already set; `s3://bucket/prefix` also works) |
| `AWS_ENDPOINT_URL_S3` | `http://127.0.0.1:9000` |
| `AWS_ALLOW_HTTP` | `true` (plain HTTP to a local endpoint; never in the cloud) |
| `AWS_REGION` | `us-east-1` |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | `DIFFYMAN_S3_TEST_KEY`, `DIFFYMAN_S3_TEST_SECRET` from `.env.local` |
| `AWS_CONFIG_FILE`, `AWS_SHARED_CREDENTIALS_FILE` | `/dev/null`, so a real account's profile is never used |

Requests are path-style (`http://127.0.0.1:9000/diffyman-test/blobs/...`). To run against the bucket by hand, export the same variables instead of using the wrapper.

## Docker parity

The image is a static musl build of `diffyman-server` on `gcr.io/distroless/static-debian13:nonroot` (no shell, user 65532), defined in `deploy/docker/Dockerfile`. Both base images are pinned by digest.

```sh
scripts/build-server-image.sh              # diffyman-server:local; --features s3 for s3:// stores
docker compose -f deploy/local/compose.yaml --env-file .env.local --profile server up -d diffyman-server
docker compose -f deploy/local/compose.yaml --env-file .env.local --profile server \
  exec diffyman-server /usr/local/bin/diffyman-server keys create --name parity
```

`build-server-image.sh` needs more than 20 GB of free disk. It builds on a temporary buildx builder and removes that builder, with its whole build cache, when it finishes, so no other build cache or image is touched. It prints the image size.

The `server` profile runs the image the way a deployment would (`DIFFYMAN_ENV=prod`: no `.env.local`, explicit settings only), published on `127.0.0.1:8081`, with a read-only root file system, no capabilities and `no-new-privileges`. It has its own database, `diffyman_parity` (created by the one-shot `parity-db` service), and its own volume `diffyman_serverdata` for blobs, temporary files and its admin token: sharing the native server's database with a different blob store would leave rows pointing at objects the other server cannot see.

A parity check sends the same request to both servers and compares the results. The Diff Documents must be equal except for `provenance`:

```sh
curl -s -H "Authorization: Bearer $KEY_8080" -H 'Content-Type: application/json' \
  --data @request.json http://127.0.0.1:8080/v1/diff | jq -S 'del(.provenance)' > native.json
curl -s -H "Authorization: Bearer $KEY_8081" -H 'Content-Type: application/json' \
  --data @request.json http://127.0.0.1:8081/v1/diff | jq -S 'del(.provenance)' > container.json
diff native.json container.json && echo identical
```

The conformance runner checks the container against the corpus directly: with the key from `keys create` (use `exec -T` to capture it from a script) in `DIFFYMAN_SERVER_KEY`, run `DIFFYMAN_SERVER_URL=http://127.0.0.1:8081 uv run --no-project --python 3.12 python tests/conformance/run_bindings.py --surfaces server`.

Measured on macOS arm64 (Docker 29.6.2, default features), 2026-11-08: the build took 121 s, and the image is 6.1 MB of content (24.6 MB unpacked). The running container's process has UID and GID 65532, no capabilities in any set, `NoNewPrivs` 1, a read-only root, and its port published on `127.0.0.1:8081` only; `/healthz` answered `ok` for the database and the store. The conformance runner's server surface passed every corpus case with file inputs (199 of 216, as JSON and multipart, each byte-identical to `expected.json` without `provenance`) and the 68 line-only variants, 534 requests in all; the log held no API key, and `stop` ended the server with exit code 0.

Clean up with `docker compose ... --profile server rm -sf diffyman-server parity-db`, `docker image rm diffyman-server:local` and, to drop its data, `docker volume rm diffyman_serverdata`.

## From local to cloud

The same binary moves to a managed environment by configuration alone.

| Setting | Local | Cloud (AWS names as an example) |
|---|---|---|
| `DIFFYMAN_ENV` | `local` | `prod` |
| `DIFFYMAN_BIND` | `127.0.0.1:8080` | `0.0.0.0:8080` behind a TLS load balancer |
| `DIFFYMAN_ALLOWED_HOSTS` | `127.0.0.1`, `localhost`, `[::1]` with the port | the public host name, for example `diff.example.com` |
| `DIFFYMAN_ALLOWED_ORIGINS` | none | none, or the origin of a browser UI |
| `DIFFYMAN_DB_URL` | built from `DIFFYMAN_PG_PASSWORD` in `.env.local` | from a secrets manager (Secrets Manager), RDS or Aurora PostgreSQL 18, with `sslmode=verify-full` |
| `DIFFYMAN_STORE_URL` | `file://~/diffyman-data/blobs` | `s3://<bucket>/<prefix>`, bucket encrypted at rest (SSE-S3 or SSE-KMS), public access blocked |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | test keys (`s3` profile only) | unset: a task or instance role (`AWS_CONTAINER_CREDENTIALS_RELATIVE_URI`, `AWS_WEB_IDENTITY_TOKEN_FILE` and `AWS_ROLE_ARN`, or instance metadata) |
| `AWS_ENDPOINT_URL_S3`, `AWS_ALLOW_HTTP` | `http://127.0.0.1:9000`, `true` | unset |
| `AWS_REGION` | `us-east-1` | the bucket's region |
| `DIFFYMAN_ADMIN_TOKEN_FILE` | `~/.diffyman/admin-token` | a secret mounted as a file |
| `DIFFYMAN_TMP_DIR` | `~/diffyman-data/tmp` | container scratch disk |
| `DIFFYMAN_LOG` | `info` | `info`, shipped to the platform's log service |
| Backups | `backup-db.sh`, nightly launchd job, 7 days | automated snapshots with point-in-time recovery |
| Restore drill | `restore-drill.sh` | restore a snapshot to a temporary instance and run the same checks |
| Blob retention | the server's sweeper | the same sweeper; a bucket lifecycle rule may abort incomplete multipart uploads and expire `staging/`, but must never expire `blobs/`, whose objects have rows |
