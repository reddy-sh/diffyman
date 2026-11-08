# 0003. PostgreSQL for the REST API service, not SQLite

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

Most of diffyman holds no state. The engine takes inputs and returns a Diff Document, the CLI keeps nothing by default, and the bindings live in the caller's process. The one stateful part is the REST API service (`diffyman-server`). It stores jobs (which also act as the job queue), saved-diff metadata and share links, hashed API keys, append-only usage records and append-only audit records. Blobs go to object storage (ADR 0004).

The service runs locally first, with a 1:1 cloud twin later. Moving to the cloud should mean changing environment variables and adding infrastructure code, not changing application code.

In the service, the API handlers, job workers and usage counters all write at the same time. In the cloud, several containers will share one database over the network.

## Decision

- **PostgreSQL 18 for the REST API service.** Locally it runs in Docker (`postgres:18.6-trixie`, defined in `deploy/local/compose.yaml`), published only on **`127.0.0.1:5433`**. Port 5432 is left alone, because other local projects may use it. The cloud twin is **Amazon RDS for PostgreSQL 18**, with the same SQL and the same migrations.
- **Access through `sqlx`**, with compile-time checked queries. Migrations are plain SQL, forward-only, embedded in the binary. The offline query data (`.sqlx/`) is committed so builds work without a live database.
- **Only extensions that RDS for PostgreSQL 18 also offers** (for example pgcrypto, pg_trgm, citext, pg_stat_statements, pg_cron, pg_partman).
- **The job queue is a Postgres table** polled with `FOR UPDATE SKIP LOCKED`. No separate queue service until scale requires one.
- **SQLite only for optional CLI history** (`~/.diffyman/history.db`), and similar single-user local caches or test fixtures.
- **The engine, CLI and bindings need no database.** Only `diffyman-server`, `diffyman-db` and `diffyman-storage` may depend on `sqlx`.
- **Not PostgreSQL 19 yet.** Revisit once managed cloud support for it is established.

Alternatives rejected:

- **SQLite for the service.** One writer at a time, and SQLite's own guidance recommends a client/server database when several machines share data over a network. It would also make local and cloud behave differently.
- **Homebrew PostgreSQL as the default.** Works, but Docker gives a pinned version, no port clash and a one-command reset. Homebrew `postgresql@18` on port 5433, or Colima with the same compose file, stays the fallback where Docker Desktop is not allowed.

## Consequences

- Local and cloud share SQL features: `SKIP LOCKED`, JSONB, partitioning, `uuidv7()`, `RETURNING OLD/NEW` and SCRAM auth.
- Running the REST API service locally needs Docker (or the fallback) and a password in `.env.local`, which is gitignored. Using the engine, CLI or bindings needs nothing.
- CI needs a PostgreSQL 18 service container to check queries and run integration tests.
- Backups and restore drills are our job locally (`pg_dump`), and RDS snapshots later.
- `docker compose down -v` destroys the local data, so it is documented as "only after a backup".
- Exact RDS minor-version availability must be checked before the cloud move. The migrations must not use features newer than the RDS version we target.
