#!/bin/sh
# Builds the JNI library (Cargo package diffyman-java), then the jar, and runs
# the Java tests. Extra arguments go to Maven, for example `-DskipTests`.
#
#   bindings/java/build.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
target=${CARGO_TARGET_DIR:-$root/target}

cargo build --release --locked -p diffyman-java --manifest-path "$root/Cargo.toml"
mvn -q -f "$here/pom.xml" verify -Ddiffyman.native.dir="$target/release" "$@"
