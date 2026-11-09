#!/bin/sh
set -eu

log() {
  printf '%s %s\n' "$(date -u +%H:%M:%S)" "$*" >&2
}

deploy() {
  log "building release $1"
  make release VERSION="$1"
  log "uploading artifacts"
  rsync -a dist/ "deploy@$TARGET:/srv/app/releases/$1/"
  log "done"
}

deploy "${1:?usage: deploy.sh VERSION}"
