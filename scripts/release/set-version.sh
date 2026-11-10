#!/bin/sh
# Sets the release version in every manifest, then checks that they agree:
# the Cargo workspace (all crates inherit it, so the CLI's --version and
# diffyman::ENGINE_VERSION follow) and both Cargo.lock files, the npm
# packages and their lockfiles, the Maven POM, docs/api/openapi.json and the
# jar names quoted in the Java docs. The Python and WASM packages take the
# Cargo version when they are built. See docs/release.md.
#
#   scripts/release/set-version.sh 0.1.0
#
# Needs Python 3.11 or later (PYTHON overrides `python3`). It changes files
# only; nothing is committed, tagged or published.
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: $0 MAJOR.MINOR.PATCH" >&2
  exit 2
fi

here=$(cd "$(dirname "$0")" && pwd)
exec "${PYTHON:-python3}" "$here/versions.py" set "$1"
