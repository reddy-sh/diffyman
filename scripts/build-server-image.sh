#!/usr/bin/env zsh
# Builds the diffyman-server container image (deploy/docker/Dockerfile) and
# removes the build cache it created (docs/operations.md).
#
# Usage: scripts/build-server-image.sh [--features s3] [--tag TAG]
#   --features s3  build with s3:// support
#   --tag TAG      image tag (default diffyman-server:local)
#
# The build runs on a temporary buildx builder of its own, which is removed
# afterwards together with its whole cache, so no other build cache or image
# is touched. The buildkit image that builder pulled is removed too when it
# was not present before, and so is the previous image under TAG when the
# build replaced it and nothing uses it. Needs more than 20 GB of free disk.
set -euo pipefail

repo_root=${0:A:h:h}
source "$repo_root/scripts/lib/ops.zsh"

tag=diffyman-server:local
features=
while (( $# )); do
  case $1 in
    --features) features=${2:?--features needs a list}; shift 2 ;;
    --tag) tag=${2:?--tag needs a name}; shift 2 ;;
    -h|--help) ops_usage "$0"; exit 0 ;;
    *) ops_die "unknown argument $1 (see --help)" ;;
  esac
done
[[ -z $features || $features == s3 ]] || ops_die "--features accepts s3 only"

free_kb=$(df -Pk "$HOME" | awk 'NR == 2 { print $4 }')
(( free_kb > 20 * 1024 * 1024 )) || ops_die "less than 20 GB free on the home volume; not building"
ops_with_timeout 30 docker info >/dev/null 2>&1 || ops_die "Docker is not responding (is Docker Desktop running?)"

buildkit=moby/buildkit:v0.34.1@sha256:99368f3e46177c6405b9740e4461d91cee004128daeecca78d4ceac50ec1da5c
had_buildkit=0
docker image inspect "$buildkit" >/dev/null 2>&1 && had_buildkit=1
previous=$(docker image inspect -f '{{.Id}}' "$tag" 2>/dev/null || true)
builder=diffyman-build-$$

cleanup() {
  docker buildx rm --force "$builder" >/dev/null 2>&1 || true
  (( had_buildkit )) || docker image rm "$buildkit" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker buildx create --name "$builder" --driver docker-container \
  --driver-opt "image=$buildkit" >/dev/null
started=$SECONDS
docker buildx build --builder "$builder" --load \
  -f "$repo_root/deploy/docker/Dockerfile" \
  ${features:+--build-arg FEATURES=$features} \
  -t "$tag" "$repo_root"

current=$(docker image inspect -f '{{.Id}}' "$tag")
if [[ -n $previous && $previous != "$current" ]]; then
  docker image rm "$previous" >/dev/null 2>&1 || true
fi
size=$(docker image inspect -f '{{.Size}}' "$tag")
print -- "built $tag in $(( SECONDS - started ))s: $(( size / 1024 / 1024 )) MiB ($size bytes)"
