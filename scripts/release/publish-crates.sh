#!/bin/sh
# Publishes the workspace crates to crates.io in dependency order, or checks
# that they would publish.
#
#   scripts/release/publish-crates.sh --dry-run [CARGO_ARGS...]
#   scripts/release/publish-crates.sh --publish VERSION [CARGO_ARGS...]
#
# The crates are those without `publish = false`, in the order of
# `versions.py crates` (each after the crates it depends on).
#
# --dry-run packages and builds every crate with `cargo publish --dry-run`
# in one call, so crates that depend on each other resolve against the
# packaged copies instead of crates.io. Nothing is uploaded and no token is
# needed. The .crate files stay in target/package/. Locally, add
# --allow-dirty for an uncommitted tree.
#
# --publish uploads them one by one; it is what the release workflow's
# approved, non-dry run executes (docs/release.md), with a token in
# CARGO_REGISTRY_TOKEN. A crate whose VERSION is already on crates.io is
# skipped, so a run that stopped halfway can be repeated and resumes with
# the first crate that is missing. cargo waits until each crate is in the
# index before the next one is uploaded.
#
# Rate limits: crates.io lets an account create a few new crates at once and
# then one more every 10 minutes (new versions of existing crates have a
# much larger allowance). An upload over the limit is refused with HTTP 429
# and the time to try again ("Please try again after <date>", the value of
# its Retry-After header). The script then waits until that time (or 10
# minutes when the answer names none) and uploads the same crate again, up
# to RETRIES times (default 8). The first release creates all eight crates,
# so it takes about half an hour.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
python=${PYTHON:-python3}
retries=${RETRIES:-8}
# crates.io's refill interval for new crates, used when a 429 names no time.
window=600
ua="diffyman release (https://github.com/reddy-sh/diffyman)"

usage() {
  echo "usage: $0 --dry-run [CARGO_ARGS...] | --publish VERSION [CARGO_ARGS...]" >&2
  exit 2
}

[ "$#" -ge 1 ] || usage
mode=$1
shift
case $mode in
  --dry-run) ;;
  --publish)
    [ "$#" -ge 1 ] || usage
    version=$1
    shift
    "$python" "$here/versions.py" check --expect "$version"
    ;;
  *) usage ;;
esac

crates=$("$python" "$here/versions.py" crates)
echo "crates, in publish order:"
echo "$crates"

cd "$root"
if [ "$mode" = --dry-run ]; then
  for crate in $crates; do
    set -- "$@" -p "$crate"
  done
  exec cargo publish --dry-run --locked "$@"
fi

# 0 when CRATE VERSION is on crates.io, 1 when it is not; anything else stops.
on_crates_io() {
  code=$(curl -sS -o /dev/null -w '%{http_code}' --retry 5 --retry-delay 10 -A "$ua" \
    "https://crates.io/api/v1/crates/$1/$2") || code=000
  case $code in
    200) return 0 ;;
    404) return 1 ;;
    *)
      echo "error: crates.io answered HTTP $code for $1 $2; try again later" >&2
      exit 1
      ;;
  esac
}

# The seconds to wait after a refused upload whose output is in FILE: until
# the time crates.io named, plus a margin, or the refill window.
wait_seconds() {
  "$python" - "$1" "$window" <<'EOF'
import re
import sys
import time
from email.utils import parsedate_to_datetime

text = open(sys.argv[1], encoding="utf-8", errors="replace").read()
match = re.search(r"try again after (\w{3}, \d{1,2} \w{3} \d{4} \d{2}:\d{2}:\d{2} GMT)", text)
seconds = int(sys.argv[2])
if match:
    seconds = max(0, int(parsedate_to_datetime(match.group(1)).timestamp() - time.time()))
print(seconds + 15)
EOF
}

log=$(mktemp)
trap 'rm -f "$log" "$log.status"' EXIT

for crate in $crates; do
  if on_crates_io "$crate" "$version"; then
    echo "$crate $version is already on crates.io; skipped"
    continue
  fi
  attempt=1
  while :; do
    echo "publishing $crate $version (attempt $attempt)"
    { status=0; cargo publish --locked -p "$crate" "$@" 2>&1 || status=$?; echo "$status" > "$log.status"; } | tee "$log"
    status=$(cat "$log.status")
    [ "$status" = 0 ] && break
    if ! grep -q -e "429 Too Many Requests" -e "status 429" "$log"; then
      # cargo may have uploaded the crate and failed afterwards (waiting for
      # the index, for example): then it is done.
      if on_crates_io "$crate" "$version"; then
        echo "$crate $version is on crates.io despite the error above"
        break
      fi
      exit "$status"
    fi
    if [ "$attempt" -ge "$retries" ]; then
      echo "error: crates.io still refuses $crate after $attempt attempts (rate limit); run the job again later, it resumes here" >&2
      exit 1
    fi
    seconds=$(wait_seconds "$log")
    if [ "$seconds" -gt 3600 ]; then
      echo "error: crates.io asks to wait $seconds s before $crate; run the job again then, it resumes here" >&2
      exit 1
    fi
    echo "crates.io's rate limit for new crates: waiting $seconds s before uploading $crate again"
    sleep "$seconds"
    attempt=$((attempt + 1))
  done
done
