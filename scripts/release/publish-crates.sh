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
# needed. Locally, add --allow-dirty for an uncommitted tree.
#
# --publish uploads them one by one; it is what the release workflow's
# approved, non-dry run executes (docs/release.md), with a token in
# CARGO_REGISTRY_TOKEN. A crate whose VERSION is already on crates.io is
# skipped, so a run that stopped halfway can be repeated. cargo waits until
# each crate is in the index before the next one is uploaded.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
python=${PYTHON:-python3}

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

for crate in $crates; do
  status=$(curl -sS -o /dev/null -w '%{http_code}' \
    -A "diffyman release (https://github.com/reddy-sh/diffyman)" \
    "https://crates.io/api/v1/crates/$crate/$version")
  if [ "$status" = 200 ]; then
    echo "$crate $version is already on crates.io; skipped"
    continue
  fi
  echo "publishing $crate $version"
  cargo publish --locked -p "$crate" "$@"
done
