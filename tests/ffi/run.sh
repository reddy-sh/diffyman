#!/usr/bin/env bash
# Builds the C ABI (crates/diffyman-ffi) and the C test program, runs the
# program's self-checks, then checks every corpus case through it
# (tests/ffi/check_corpus.py).
#
# Usage: tests/ffi/run.sh
#
# Environment: CARGO (default cargo), CC (default cc), PYTHON (default
# python3), PROFILE (debug or release, default debug), CARGO_TARGET_DIR.
# The program is built with AddressSanitizer and UndefinedBehaviorSanitizer
# when the compiler supports them.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cargo="${CARGO:-cargo}"
cc="${CC:-cc}"
python="${PYTHON:-python3}"
profile="${PROFILE:-debug}"

case "$profile" in
  debug) "$cargo" build --locked --manifest-path "$repo/Cargo.toml" -p diffyman-ffi ;;
  release) "$cargo" build --locked --release --manifest-path "$repo/Cargo.toml" -p diffyman-ffi ;;
  *) echo "PROFILE must be debug or release, not $profile" >&2; exit 2 ;;
esac

lib_dir="${CARGO_TARGET_DIR:-$repo/target}/$profile"
out="$lib_dir/ffi-test"
mkdir -p "$out"

flags=(-std=c11 -Wall -Wextra -Wpedantic -Werror -g)
sanitizers=(-fsanitize=address,undefined -fno-omit-frame-pointer)
if printf 'int main(void) { return 0; }\n' \
    | "$cc" -x c "${sanitizers[@]}" -o "$out/probe" - 2>/dev/null; then
  flags+=("${sanitizers[@]}")
  echo "building test_ffi with AddressSanitizer and UndefinedBehaviorSanitizer"
else
  echo "building test_ffi without sanitizers ($cc does not support them)"
fi
rm -f "$out/probe"

# Links the shared library (the linker prefers it over the static one in the
# same folder); the rpath lets the program find it without installing it.
"$cc" "${flags[@]}" \
  -I "$repo/crates/diffyman-ffi/include" \
  "$repo/tests/ffi/test_ffi.c" \
  -L "$lib_dir" -ldiffyman_ffi -Wl,-rpath,"$lib_dir" \
  -o "$out/test_ffi"

"$out/test_ffi"
"$python" "$repo/tests/ffi/check_corpus.py" "$out/test_ffi"
