#!/bin/sh
# Installs the release files in clean containers, the way the quickstarts
# say, and runs each quickstart's examples with them (docs/release.md,
# "Install check").
#
#   scripts/release/install-check.sh [--keep] [--remove-images] [--workdir DIR] ARTIFACTS [LANGUAGE...]
#
# ARTIFACTS is a folder with the release files of a run (searched
# recursively, so a folder of downloaded workflow artifacts works), for the
# architecture of the Docker engine (arm64 or amd64). LANGUAGE is one or
# more of the following (default: all of them):
#
#   cli      debian:trixie-slim   diffyman-VERSION-ARCH-unknown-linux-musl.tar.gz
#   rust     rust:1.99.1          diffyman*-VERSION.crate (all eight), installed
#                                 with `cargo install --locked diffyman-cli` from a
#                                 local copy of crates.io (no network)
#   python   python:3.12-slim     diffyman-VERSION-cp311-abi3-manylinux_2_28_ARCH.whl
#   node     node:22-slim         diffyman-node-VERSION.tgz and
#                                 diffyman-node-linux-ARCH-gnu-VERSION.tgz (npm pack)
#   browser  node:22-slim         diffyman-wasm-VERSION.tgz and diffyman-react-VERSION.tgz
#   java     eclipse-temurin:17-jre  diffyman-VERSION.jar
#   c        debian:trixie-slim   libdiffyman_ffi.so (not a release file:
#                                 cargo build --release -p diffyman-ffi for Linux)
#
# Each language gets a new container that sees only its files (read-only)
# and a script made from its quickstart by quickstart_script.py: it
# installs the files, then runs every example of the page and compares what
# each prints with the page. The time is measured from the container's start
# to its end; images are pulled before. Containers run without a network,
# except for cli and c (apt-get installs jq, patch and gcc) and browser (npm
# installs React and Vite).
#
# The host needs docker, python3, and for rust also cargo (it copies the
# crates' dependencies from crates.io into the folder the container reads),
# for java javac (17 or newer) and Jackson in ~/.m2 (mvn verify in
# bindings/java downloads it). --keep keeps the work folder, --remove-images
# removes the images this run pulled. Exit status 0 when every language
# passed.

# shellcheck disable=SC2329 # called through "stage_$language" and the EXIT trap
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
python=${PYTHON:-python3}
cargo=${CARGO:-cargo}

IMAGE_DEBIAN=debian:trixie-slim
IMAGE_RUST=rust:1.99.1
IMAGE_PYTHON=python:3.12-slim
IMAGE_NODE=node:22-slim
IMAGE_JAVA=eclipse-temurin:17-jre
ALL="cli rust python node browser java c"

usage() {
  echo "usage: $0 [--keep] [--remove-images] [--workdir DIR] ARTIFACTS [LANGUAGE...]" >&2
  echo "languages: $ALL" >&2
  exit 2
}

keep=false
remove_images=false
workdir=
while [ "$#" -gt 0 ]; do
  case $1 in
    --keep) keep=true ;;
    --remove-images) remove_images=true ;;
    --workdir)
      [ "$#" -ge 2 ] || usage
      workdir=$2
      shift
      ;;
    -h | --help) usage ;;
    -*) usage ;;
    *) break ;;
  esac
  shift
done
[ "$#" -ge 1 ] || usage
artifacts=$(cd "$1" && pwd)
shift
# shellcheck disable=SC2086 # one word per language
[ "$#" -gt 0 ] || set -- $ALL
for language in "$@"; do
  case " $ALL " in
    *" $language "*) ;;
    *) echo "unknown language: $language" >&2; usage ;;
  esac
done

# A new folder of its own, inside --workdir if given: the cleanup removes it.
if [ -n "$workdir" ]; then
  mkdir -p "$workdir"
  work=$(mktemp -d "$(cd "$workdir" && pwd)/install-check.XXXXXX")
else
  work=$(mktemp -d "${TMPDIR:-/tmp}/diffyman-install-check.XXXXXX")
fi
: > "$work/results.txt"
: > "$work/pulled-images.txt"
container=

cleanup() {
  if [ -n "$container" ]; then
    docker rm -f "$container" > /dev/null 2>&1 || true
  fi
  if [ "$remove_images" = true ]; then
    while read -r image; do
      docker image rm "$image" > /dev/null 2>&1 || true
    done < "$work/pulled-images.txt"
  fi
  if [ "$keep" = true ]; then
    echo "work folder kept: $work"
  else
    rm -rf "$work"
  fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

case $(docker version --format '{{.Server.Arch}}') in
  arm64 | aarch64) rust_arch=aarch64 npm_arch=arm64 ;;
  amd64 | x86_64) rust_arch=x86_64 npm_arch=x64 ;;
  *) echo "unsupported Docker architecture" >&2; exit 2 ;;
esac

now() {
  "$python" -c 'import time; print(f"{time.time():.1f}")'
}

sha256() {
  if command -v sha256sum > /dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}

# find_one NAME_PATTERN [FIND_ARGS...]: the one file of ARTIFACTS that matches.
find_one() {
  pattern=$1
  shift
  found=$(find "$artifacts" -type f -name "$pattern" "$@" | LC_ALL=C sort)
  count=$(printf '%s' "$found" | grep -c . || true)
  if [ "$count" -ne 1 ]; then
    echo "expected one file named $pattern in $artifacts, found $count" >&2
    [ -z "$found" ] || printf '%s\n' "$found" >&2
    return 1
  fi
  printf '%s\n' "$found"
}

ensure_image() {
  if ! docker image inspect "$1" > /dev/null 2>&1; then
    echo "pulling $1"
    docker pull -q "$1" > /dev/null
    echo "$1" >> "$work/pulled-images.txt"
  fi
}

# quickstart PAGE ARGS...: the page's examples as a script, ctx/steps.sh.
quickstart() {
  page=$1
  shift
  "$python" "$here/quickstart_script.py" "$root/docs/quickstart/$page" -o "$ctx/steps.sh" "$@"
}

# stage_LANGUAGE: copies the files into $ctx/files, writes $ctx/run.sh and
# sets image, network and artifact.

stage_cli() {
  archive=$(find_one "diffyman-*-$rust_arch-unknown-linux-musl.tar.gz")
  cp "$archive" "$ctx/files/"
  artifact=$(basename "$archive")
  image=$IMAGE_DEBIAN network=bridge
  quickstart cli.md
  cat > "$ctx/run.sh" << EOF
set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -qq -y --no-install-recommends jq patch > /dev/null
mkdir -p /opt/diffyman
tar -xzf "/ctx/files/$artifact" -C /opt/diffyman --strip-components 1
export PATH="/opt/diffyman:\$PATH"
diffyman --version
sh /ctx/steps.sh
EOF
}

stage_rust() {
  cli=$(find_one "diffyman-cli-*.crate")
  artifact=$(basename "$cli")
  version=${artifact#diffyman-cli-}
  version=${version%.crate}
  mkdir -p "$ctx/vendor" "$ctx/resolve"
  crates=$(find "$artifacts" -type f -name "diffyman*-$version.crate" | LC_ALL=C sort)
  set --
  old_ifs=$IFS
  IFS='
'
  for crate in $crates; do
    IFS=$old_ifs
    name=$(basename "$crate" .crate)
    tar -xzf "$crate" -C "$ctx/vendor"
    printf '{"files":{},"package":"%s"}\n' "$(sha256 "$crate")" > "$ctx/vendor/$name/.cargo-checksum.json"
    crate_name=${name%-"$version"}
    if [ "$crate_name" != diffyman-cli ]; then
      set -- "$@" --config "patch.crates-io.$crate_name.path=\"$ctx/vendor/$name\""
    fi
  done
  IFS=$old_ifs
  [ "$#" -eq 14 ] || { echo "expected the eight crates of version $version in $artifacts" >&2; return 1; }
  # The dependencies from crates.io, in the versions of the crate's Cargo.lock.
  tar -xzf "$cli" -C "$ctx/resolve"
  echo "copying the dependencies of $artifact from crates.io"
  "$cargo" vendor --quiet --versioned-dirs --manifest-path "$ctx/resolve/diffyman-cli-$version/Cargo.toml" \
    "$@" "$ctx/third-party" > /dev/null
  for dir in "$ctx"/third-party/*; do
    mv "$dir" "$ctx/vendor/"
  done
  rm -rf "$ctx/resolve" "$ctx/third-party"
  image=$IMAGE_RUST network=none
  quickstart cli.md --skip-run jq
  cat > "$ctx/run.sh" << 'EOF'
set -eu
cat >> "$CARGO_HOME/config.toml" << 'TOML'
[source.crates-io]
replace-with = "release-files"

[source.release-files]
directory = "/ctx/vendor"
TOML
cargo install --locked diffyman-cli
diffyman --version
sh /ctx/steps.sh
EOF
}

stage_python() {
  wheel=$(find_one "diffyman-*-manylinux_*_$rust_arch.whl")
  cp "$wheel" "$ctx/files/"
  artifact=$(basename "$wheel")
  image=$IMAGE_PYTHON network=none
  quickstart python.md --include-skip "pip install" --replace "path/to/diffyman/dist/diffyman-*.whl=/ctx/files/$artifact"
  cat > "$ctx/run.sh" << 'EOF'
set -eu
export PIP_DISABLE_PIP_VERSION_CHECK=1 PIP_ROOT_USER_ACTION=ignore PIP_NO_CACHE_DIR=1
sh /ctx/steps.sh
EOF
}

stage_node() {
  main=$(find_one "diffyman-node-[0-9]*.tgz")
  addon=$(find_one "diffyman-node-linux-$npm_arch-gnu-*.tgz")
  cp "$main" "$addon" "$ctx/files/"
  artifact="$(basename "$main") + $(basename "$addon")"
  image=$IMAGE_NODE network=none
  quickstart node.md --replace "\"\$DIFFYMAN_REPO/bindings/node\"=/ctx/files/$(basename "$main") /ctx/files/$(basename "$addon")"
  # Offline: npm skips the other platforms' optional packages at once instead
  # of retrying a registry the container cannot reach for minutes.
  cat > "$ctx/run.sh" << 'EOF'
set -eu
export npm_config_update_notifier=false npm_config_offline=true
sh /ctx/steps.sh
EOF
}

stage_browser() {
  wasm=$(find_one "diffyman-wasm-*.tgz")
  react=$(find_one "diffyman-react-*.tgz")
  cp "$wasm" "$react" "$ctx/files/"
  artifact="$(basename "$wasm") + $(basename "$react")"
  image=$IMAGE_NODE network=bridge
  quickstart browser-react.md --include-skip "npm install --save-exact" \
    --replace "\"\$DIFFYMAN_REPO/bindings/wasm/react\"=/ctx/files/$(basename "$react")" \
    --replace "\"\$DIFFYMAN_REPO/bindings/wasm/pkg-web\"=/ctx/files/$(basename "$wasm")"
  # The render blocks need a DOM; instead, the .wasm asset of the built page
  # diffs the page's two inputs, and the lines are printed as ChangedLines does.
  cat > "$ctx/engine-check.mjs" << 'EOF'
import { readdirSync, readFileSync } from "node:fs";
import { diff, initSync } from "@diffyman/wasm";

const assets = readdirSync("dist/assets");
const wasm = assets.filter((name) => name.endsWith(".wasm"));
if (wasm.length !== 1) throw new Error(`expected one .wasm asset in dist/assets, found ${wasm.length}`);
if (!assets.some((name) => name.endsWith(".js") && name.includes("worker"))) {
  throw new Error("no worker chunk in dist/assets");
}
initSync({ module: readFileSync(`dist/assets/${wasm[0]}`) });
const left = "apple\nbanana\ncherry\n";
const right = "apple\nblueberry\ncherry\ndate\n";
const doc = JSON.parse(diff(new TextEncoder().encode(left), new TextEncoder().encode(right), ""));
const lines = (text) => text.split(/\r\n|\r|\n/);
const [l, r] = [lines(left), lines(right)];
let text = `${doc.status}\n`;
for (const op of doc.hunks.flatMap((hunk) => hunk.ops)) {
  if (op.op !== "insert" && op.op !== "delete" && op.op !== "replace") continue;
  for (const line of l.slice(op.leftRange.start - 1, op.leftRange.start - 1 + op.leftRange.count)) text += `- ${line}\n`;
  for (const line of r.slice(op.rightRange.start - 1, op.rightRange.start - 1 + op.rightRange.count)) text += `+ ${line}\n`;
}
const want = "different\n- banana\n+ blueberry\n+ date\n";
if (text !== want) throw new Error(`the built page's engine printed\n${text}instead of\n${want}`);
console.log(`ok   the built page's ${wasm[0]} gives the lines of the page`);
EOF
  cat > "$ctx/run.sh" << 'EOF'
set -eu
export npm_config_update_notifier=false npm_config_audit=false npm_config_fund=false
sh /ctx/steps.sh
cp /ctx/engine-check.mjs .
node engine-check.mjs
EOF
}

stage_java() {
  jar=$(find_one "diffyman-[0-9]*.jar" ! -name "*-sources.jar" ! -name "*-javadoc.jar")
  cp "$jar" "$ctx/files/"
  artifact=$(basename "$jar")
  image=$IMAGE_JAVA network=none
  # A JRE has no compiler, so the source launcher (`java Example.java`) is
  # replaced by classes compiled here for Java 17.
  quickstart java.md --save-files "$ctx/src" \
    --replace "Example.java=-cp \"/ctx/classes:\$CLASSPATH\" Example" \
    --replace "Errors.java=-cp \"/ctx/classes:\$CLASSPATH\" Errors"
  m2="$HOME/.m2/repository/com/fasterxml/jackson/core"
  classpath="$jar"
  # The jar paths of the page's class path block (no spaces in them).
  # shellcheck disable=SC2013,SC2016
  for path in $(grep -o '\$JACKSON/[^"]*\.jar' "$root/docs/quickstart/java.md" | sed 's|^\$JACKSON/||'); do
    [ -f "$m2/$path" ] || { echo "missing $m2/$path: run mvn -f bindings/java/pom.xml verify" >&2; return 1; }
    mkdir -p "$ctx/jackson/$(dirname "$path")"
    cp "$m2/$path" "$ctx/jackson/$path"
    classpath="$classpath:$m2/$path"
  done
  javac --release 17 -nowarn -d "$ctx/classes" -cp "$classpath" "$ctx/src/Example.java" "$ctx/src/Errors.java"
  cat > "$ctx/run.sh" << EOF
set -eu
export DIFFYMAN_REPO=/opt/diffyman
mkdir -p "\$DIFFYMAN_REPO/bindings/java/target" "\$HOME/.m2/repository/com/fasterxml/jackson/core"
cp "/ctx/files/$artifact" "\$DIFFYMAN_REPO/bindings/java/target/"
cp -R /ctx/jackson/. "\$HOME/.m2/repository/com/fasterxml/jackson/core/"
java -version 2>&1 | head -n 1
sh /ctx/steps.sh
EOF
}

stage_c() {
  library=$(find_one "libdiffyman_ffi.so")
  cp "$library" "$ctx/files/"
  cp "$root/crates/diffyman-ffi/include/diffyman.h" "$ctx/"
  artifact="libdiffyman_ffi.so + diffyman.h"
  image=$IMAGE_DEBIAN network=bridge
  quickstart c.md
  cat > "$ctx/run.sh" << 'EOF'
set -eu
export DEBIAN_FRONTEND=noninteractive DIFFYMAN_REPO=/opt/diffyman
apt-get update -qq
apt-get install -qq -y --no-install-recommends gcc libc6-dev jq > /dev/null
mkdir -p "$DIFFYMAN_REPO/target/release" "$DIFFYMAN_REPO/crates/diffyman-ffi/include"
cp /ctx/files/libdiffyman_ffi.so "$DIFFYMAN_REPO/target/release/"
cp /ctx/diffyman.h "$DIFFYMAN_REPO/crates/diffyman-ffi/include/"
sh /ctx/steps.sh
EOF
}

failed=0
for language in "$@"; do
  ctx="$work/$language"
  rm -rf "$ctx"
  mkdir -p "$ctx/files"
  echo "== $language"
  # In a subshell of its own (not an `if` condition, which would turn off
  # set -e inside the function), so a missing file ends only this language.
  set +e
  (
    set -e
    "stage_$language"
    printf '%s\n' "$image" "$network" "$artifact" > "$ctx/staged.txt"
  )
  staged=$?
  set -e
  if [ "$staged" -eq 0 ]; then
    { read -r image; read -r network; read -r artifact; } < "$ctx/staged.txt"
  else
    echo "$language: could not stage the files" >&2
    printf '%s\t%s\t%s\t%s\n' "$language" "-" "-" "not run" >> "$work/results.txt"
    failed=1
    continue
  fi
  ensure_image "$image"
  container="diffyman-install-check-$language-$$"
  log="$work/$language.log"
  start=$(now)
  if docker run --rm --pull never --name "$container" --network "$network" \
    -v "$ctx:/ctx:ro" -w /work "$image" sh /ctx/run.sh > "$log" 2>&1; then
    status=ok
  else
    status=FAILED
    failed=1
  fi
  end=$(now)
  container=
  seconds=$("$python" -c 'import sys; print(f"{float(sys.argv[2]) - float(sys.argv[1]):.1f}")' "$start" "$end")
  if [ "$status" = ok ]; then
    echo "$language: $(grep -c '^ok ' "$log") checks passed, $(grep -c '^skip ' "$log") left out, ${seconds}s"
  else
    echo "$language: FAILED after ${seconds}s; the end of the log:" >&2
    tail -n 40 "$log" >&2
  fi
  printf '%s\t%s\t%s\t%s\n' "$language" "$artifact" "$seconds" "$status" >> "$work/results.txt"
done

echo
echo "| Language | Artifact | Install and examples (s) | Examples |"
echo "|---|---|---|---|"
while IFS="$(printf '\t')" read -r language artifact seconds status; do
  echo "| $language | $artifact | $seconds | $status |"
done < "$work/results.txt"
exit "$failed"
