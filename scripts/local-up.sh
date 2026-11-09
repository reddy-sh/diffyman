#!/usr/bin/env zsh
# Start the local diffyman stack (Postgres 18 on 127.0.0.1:5433).
# Usage: scripts/local-up.sh [extra compose profiles, e.g. s3 obs]
# Creates ~/diffyman-data and a gitignored .env.local with random secrets on first run.
set -euo pipefail

repo_root=${0:A:h:h}
cd "$repo_root"

data_dir="$HOME/diffyman-data"
mkdir -p "$data_dir/blobs" "$data_dir/tmp" "$data_dir/backups" "$data_dir/corpus"

if [[ ! -f .env.local ]]; then
  umask 077
  {
    print "DIFFYMAN_PG_PASSWORD=$(openssl rand -hex 16)"
    print "DIFFYMAN_S3_TEST_KEY=dm$(openssl rand -hex 8)"
    print "DIFFYMAN_S3_TEST_SECRET=$(openssl rand -hex 20)"
  } > .env.local
  print "Created .env.local (mode 600)"
fi

profiles=()
for p in "$@"; do profiles+=(--profile "$p"); done

# The s3 profile pulls an image: refuse when the disk is nearly full.
if (( ${argv[(Ie)s3]} )); then
  free_kb=$(df -Pk "$HOME" | awk 'NR == 2 { print $4 }')
  (( free_kb > 20 * 1024 * 1024 )) || { print -u2 "Less than 20 GB free; not pulling the s3 profile's image"; exit 1; }
fi

docker compose -f deploy/local/compose.yaml --env-file .env.local "${profiles[@]}" up -d --wait
docker compose -f deploy/local/compose.yaml --env-file .env.local exec -T postgres \
  psql -U diffyman -d diffyman -Atc 'select version();'

print "\nPostgres is ready on 127.0.0.1:5433. Connection string:"
print '  postgres://diffyman:$DIFFYMAN_PG_PASSWORD@127.0.0.1:5433/diffyman  (password in .env.local)'

if (( ${argv[(Ie)s3]} )); then
  scripts/s3-bucket.sh diffyman-test
  print "S3-compatible server on 127.0.0.1:9000 (console 127.0.0.1:9001)."
  print "Run the server against s3://diffyman-test with scripts/with-local-s3 (docs/operations.md)."
fi
