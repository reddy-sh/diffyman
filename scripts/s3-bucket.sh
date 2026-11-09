#!/usr/bin/env zsh
# Creates a bucket on the local S3-compatible server (compose profile `s3`)
# when it does not exist yet (docs/operations.md).
#
# Usage: scripts/s3-bucket.sh [BUCKET]   (default diffyman-test)
#
# Uses the aws CLI when it is installed, otherwise a small standard-library
# Python client (scripts/lib/s3_bucket.py, run with uv). Credentials are the
# test ones from .env.local (DIFFYMAN_S3_TEST_KEY / DIFFYMAN_S3_TEST_SECRET),
# passed in the environment only. Waits up to 60 seconds for the server.
set -euo pipefail

repo_root=${0:A:h:h}
source "$repo_root/scripts/lib/ops.zsh"

case ${1:-} in
  -h|--help) ops_usage "$0"; exit 0 ;;
esac
bucket=${1:-diffyman-test}
[[ $bucket == [a-z0-9][a-z0-9.-](#c1,61)[a-z0-9] ]] || ops_die "bucket names are 3-63 lowercase letters, digits, dots and dashes"
(( $# <= 1 )) || ops_die "expected at most one bucket name"

ops_local_s3_env

# Any HTTP answer (even 403) means the server is up.
for attempt in {1..60}; do
  curl -s -o /dev/null --noproxy '*' --max-time 2 "$AWS_ENDPOINT_URL_S3/" && break
  (( attempt == 60 )) && ops_die "no S3 server answers at $AWS_ENDPOINT_URL_S3 (scripts/local-up.sh s3)"
  sleep 1
done

if command -v aws >/dev/null; then
  if aws s3api head-bucket --bucket "$bucket" --endpoint-url "$AWS_ENDPOINT_URL_S3" >/dev/null 2>&1; then
    print -- "bucket $bucket exists"
  else
    aws s3api create-bucket --bucket "$bucket" --endpoint-url "$AWS_ENDPOINT_URL_S3" >/dev/null
    aws s3api head-bucket --bucket "$bucket" --endpoint-url "$AWS_ENDPOINT_URL_S3" >/dev/null
    print -- "bucket $bucket created"
  fi
elif command -v uv >/dev/null; then
  result=$(uv run --quiet --no-project --python 3.12 python -I "$repo_root/scripts/lib/s3_bucket.py" "$bucket")
  print -- "bucket $bucket $result"
else
  ops_die "needs the aws CLI or uv (for scripts/lib/s3_bucket.py)"
fi
print -- "Point diffyman-server at it with: scripts/with-local-s3 <command> (docs/operations.md)"
