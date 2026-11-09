#!/usr/bin/env zsh
# Refresh the offline query data in .sqlx/ (docs/adr/0012-local-service-architecture.md).
# Run after changing a migration or a query in crates/diffyman-db, then commit .sqlx/.
#
# Usage: scripts/db-prepare.sh [--check]
#   --check  only verify that .sqlx/ matches the queries (as CI would)
#
# Needs the local Postgres (scripts/local-up.sh) and sqlx-cli 0.9.0:
#   cargo install --locked sqlx-cli --version 0.9.0 --no-default-features --features postgres,rustls,sqlx-toml
# The queries are checked against a scratch database, diffyman_sqlx, which is
# dropped and migrated from scratch on every run. The main database is not touched.
set -euo pipefail

repo_root=${0:A:h:h}
cd "$repo_root"

[[ -f .env.local ]] || { print -u2 "No .env.local: run scripts/local-up.sh first"; exit 1; }
set -a; source ./.env.local; set +a

export PGHOST=127.0.0.1 PGPORT=5433 PGUSER=diffyman PGPASSWORD="$DIFFYMAN_PG_PASSWORD"
scratch=diffyman_sqlx

psql -q -d postgres -v ON_ERROR_STOP=1 -c "DROP DATABASE IF EXISTS $scratch WITH (FORCE)"
psql -q -d postgres -v ON_ERROR_STOP=1 -c "CREATE DATABASE $scratch"

export DIFFYMAN_DB_URL="postgres://diffyman:${DIFFYMAN_PG_PASSWORD}@127.0.0.1:5433/$scratch"
export DATABASE_URL="$DIFFYMAN_DB_URL"
unset SQLX_OFFLINE

(cd crates/diffyman-db && cargo sqlx migrate run)

if [[ ${1:-} == --check ]]; then
  cargo sqlx prepare --workspace --check -- --all-targets
else
  cargo sqlx prepare --workspace -- --all-targets
fi
