#!/usr/bin/env bash
# End-to-end benchmark: the release `diffyman` CLI against `git diff --no-index`
# and `diff` on generated inputs. Prints a markdown table on stdout (progress
# goes to stderr).
#
#   scripts/bench.sh [--bin PATH] [--data DIR] [--sizes "N ..."] [--runs N]
#
#   --bin PATH    the CLI to measure. Default: build it with
#                 `cargo build --release --locked -p diffyman-cli` and use
#                 target/release/diffyman.
#   --data DIR    where inputs and outputs go. Default: $TMPDIR/diffyman-bench.
#                 Inputs are generated once and reused while their file exists.
#   --sizes LIST  line counts per side. Default: "10000 100000 1000000".
#   --runs N      runs per command (default 3). The table shows the median wall
#                 time and the largest peak memory.
#
# Cases, for each size N:
#   text-N    N lines per side; about 1% of lines replaced, 0.5% deleted and
#             0.5% inserted (the same edit rates as the criterion benchmarks).
#   moved-N   the same edits, and pairs of 6 to 20 line blocks swapped (one
#             pair per 4000 lines).
# and once:
#   binary-10m  10 MiB of AES-CTR keystream (fixed key, so the same bytes on
#               every machine) with 256 bytes overwritten in place.
#
# The generator is a Park-Miller generator in awk: its arithmetic is exact in
# any awk, so every machine and awk gives the same inputs. Each command writes
# its output to a file under DATA/out, as a user would. `git` runs without
# system or global configuration, so local settings (diff.algorithm, a pager,
# an external diff) do not change what it does. `diff` and `git` without
# `--binary` only report that binary files differ, so for binary-10m they do
# much less work than a byte diff.
#
# Wall time comes from bash's `time` (millisecond resolution). Peak memory is
# the macOS "peak memory footprint" of `/usr/bin/time -l`, or the maximum
# resident set size of GNU `/usr/bin/time -v` on Linux; it is "-" when
# neither is available.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
bin=
data=${TMPDIR:-/tmp}
data=${data%/}/diffyman-bench
sizes="10000 100000 1000000"
runs=3

while [ "$#" -gt 0 ]; do
  case $1 in
    --bin) bin=$2; shift 2 ;;
    --data) data=$2; shift 2 ;;
    --sizes) sizes=$2; shift 2 ;;
    --runs) runs=$2; shift 2 ;;
    -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1 (see --help)" >&2; exit 2 ;;
  esac
done

case $runs in
  ''|*[!0-9]*|0) echo "--runs must be a positive integer" >&2; exit 2 ;;
esac
for n in $sizes; do
  case $n in
    ''|*[!0-9]*|0) echo "--sizes must list positive integers" >&2; exit 2 ;;
  esac
done

if [ -z "$bin" ]; then
  echo "building the release CLI" >&2
  cargo build --release --locked -p diffyman-cli --manifest-path "$root/Cargo.toml" >&2
  bin=$root/target/release/diffyman
fi
if [ ! -x "$bin" ]; then
  echo "not an executable: $bin" >&2
  exit 2
fi

export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL=/dev/null
export LC_ALL=C
mkdir -p "$data/in" "$data/out"

# --- Inputs -----------------------------------------------------------------

# gen_text N MOVES LEFT RIGHT
gen_text() {
  awk -v n="$1" -v moves="$2" -v left="$3" -v right="$4" '
    function rnd(k) { seed = (seed * 16807) % 2147483647; return seed % k }
    function mkline(num,   s, k, i) {
      s = sprintf("%08d", num)
      k = 3 + rnd(6)
      for (i = 0; i < k; i++) s = s " " w[rnd(32)]
      return s " = " rnd(1000) ";"
    }
    BEGIN {
      split("alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega value count result index buffer offset length status", t, " ")
      for (i = 1; i <= 32; i++) w[i - 1] = t[i]
      seed = 20261 + n
      m = 0
      for (i = 0; i < n; i++) {
        l = mkline(i)
        print l > left
        r = rnd(200)
        if (r < 2) R[m++] = l " // edited"
        else if (r == 2) { }
        else if (r == 3) { R[m++] = l; R[m++] = mkline(n + i) }
        else R[m++] = l
      }
      if (moves) {
        for (b = 0; b < int(n / 4000); b++) {
          len = 6 + rnd(15)
          if (m < 4 * len) break
          a = rnd(m - len); c = rnd(m - len)
          if (a > c) { x = a; a = c; c = x }
          if (c - a < len) continue
          for (i = 0; i < len; i++) { x = R[a + i]; R[a + i] = R[c + i]; R[c + i] = x }
        }
      }
      for (j = 0; j < m; j++) print R[j] > right
    }'
}

# gen_binary SIZE LEFT RIGHT
gen_binary() {
  if command -v openssl >/dev/null 2>&1; then
    head -c "$1" /dev/zero \
      | openssl enc -aes-128-ctr -nosalt \
          -K 000102030405060708090a0b0c0d0e0f -iv 00000000000000000000000000000000 \
      >"$2"
  else
    echo "openssl not found: using /dev/urandom (inputs differ between runs)" >&2
    head -c "$1" /dev/urandom >"$2"
  fi
  cp "$2" "$3"
  awk -v size="$1" 'BEGIN { seed = 7; for (i = 0; i < 256; i++) { seed = (seed * 16807) % 2147483647; print seed % size } }' \
    | while read -r offset; do
        printf '\377' | dd of="$3" bs=1 seek="$offset" conv=notrunc 2>/dev/null
      done
}

# Generator version in the file names, so a changed generator never reuses
# stale inputs.
gv=g1
cases=
for n in $sizes; do
  for kind in text moved; do
    name=$kind-$n
    left=$data/in/$name.$gv.left
    right=$data/in/$name.$gv.right
    if [ ! -s "$left" ] || [ ! -s "$right" ]; then
      echo "generating $name" >&2
      moves=0
      [ "$kind" = moved ] && moves=1
      gen_text "$n" "$moves" "$left.tmp" "$right.tmp"
      mv "$left.tmp" "$left"
      mv "$right.tmp" "$right"
    fi
    cases="$cases $name"
  done
done
name=binary-10m
if [ ! -s "$data/in/$name.$gv.left" ] || [ ! -s "$data/in/$name.$gv.right" ]; then
  echo "generating $name" >&2
  gen_binary 10485760 "$data/in/$name.$gv.left.tmp" "$data/in/$name.$gv.right.tmp"
  mv "$data/in/$name.$gv.left.tmp" "$data/in/$name.$gv.left"
  mv "$data/in/$name.$gv.right.tmp" "$data/in/$name.$gv.right"
fi
cases="$cases $name"

# --- Commands ---------------------------------------------------------------

# tools_for CASE: the tools measured for a case.
tools_for() {
  case $1 in
    text-*) echo "dm-json dm-text dm-unified git git-histogram diff" ;;
    moved-*) echo "dm-json dm-json-moves dm-unified git git-histogram git-color-moved diff" ;;
    binary-*) echo "dm-json git git-binary diff" ;;
  esac
}

# label TOOL: the command as shown in the table.
label() {
  case $1 in
    dm-json) echo 'diffyman diff --json' ;;
    dm-json-moves) echo 'diffyman diff --json --detect-moves' ;;
    dm-text) echo 'diffyman diff' ;;
    dm-unified) echo 'diffyman diff --format unified' ;;
    git) echo 'git diff --no-index' ;;
    git-histogram) echo 'git diff --no-index --histogram' ;;
    git-color-moved) echo 'git diff --no-index --color=always --color-moved' ;;
    git-binary) echo 'git diff --no-index --binary' ;;
    diff) echo 'diff -u' ;;
  esac
}

# set_cmd TOOL LEFT RIGHT: sets CMD to the argument vector.
set_cmd() {
  local g=(git --no-pager diff --no-index --no-ext-diff --no-textconv)
  case $1 in
    dm-json) CMD=("$bin" diff "$2" "$3" --json) ;;
    dm-json-moves) CMD=("$bin" diff "$2" "$3" --json --detect-moves) ;;
    dm-text) CMD=("$bin" diff "$2" "$3" --color never) ;;
    dm-unified) CMD=("$bin" diff "$2" "$3" --format unified --color never) ;;
    git) CMD=("${g[@]}" --no-color "$2" "$3") ;;
    git-histogram) CMD=("${g[@]}" --no-color --histogram "$2" "$3") ;;
    git-color-moved) CMD=("${g[@]}" --color=always --color-moved=default "$2" "$3") ;;
    git-binary) CMD=("${g[@]}" --no-color --binary "$2" "$3") ;;
    diff) CMD=(diff -u "$2" "$3") ;;
  esac
}

# Which /usr/bin/time flavor reports peak memory here.
memtool=none
if [ "$(uname)" = Darwin ] && [ -x /usr/bin/time ]; then
  memtool=bsd
elif [ -x /usr/bin/time ] && /usr/bin/time -v true >/dev/null 2>&1; then
  memtool=gnu
fi

# run_once OUT: runs CMD once with stdout to OUT; sets WALL (s), PEAK (MB),
# STATUS.
run_once() {
  local out=$1 wallf=$data/out/.wall memf=$data/out/.mem
  STATUS=0
  case $memtool in
    bsd)
      { TIMEFORMAT=%3R; time /usr/bin/time -l "${CMD[@]}" >"$out" 2>"$memf" || STATUS=$?; } 2>"$wallf"
      PEAK=$(awk '/peak memory footprint/ { printf "%.0f", $1 / 1048576 }' "$memf")
      ;;
    gnu)
      { TIMEFORMAT=%3R; time /usr/bin/time -v -o "$memf" "${CMD[@]}" >"$out" 2>/dev/null || STATUS=$?; } 2>"$wallf"
      PEAK=$(awk -F': ' '/Maximum resident set size/ { printf "%.0f", $2 / 1024 }' "$memf")
      ;;
    *)
      { TIMEFORMAT=%3R; time "${CMD[@]}" >"$out" 2>/dev/null || STATUS=$?; } 2>"$wallf"
      PEAK=-
      ;;
  esac
  WALL=$(tail -n 1 "$wallf")
  [ -n "$PEAK" ] || PEAK=-
}

# --- Run --------------------------------------------------------------------

machine="$(uname -sm)"
git_version=$(git --version 2>/dev/null || echo "git: not found")
diff_version=$(diff --version 2>/dev/null | head -n 1 || true)
[ -n "$diff_version" ] || diff_version="diff (no --version)"

printf '## diffyman end-to-end benchmark\n\n'
printf -- '- Machine: %s\n' "$machine"
printf -- '- Tools: %s; %s; %s\n' "$("$bin" --version)" "$git_version" "$diff_version"
printf -- '- Runs per command: %s (median wall time, largest peak memory)\n\n' "$runs"
printf '| Case | Input MB (both sides) | Command | Exit | Wall s | Peak MB | Output MB |\n'
printf '|---|--:|---|--:|--:|--:|--:|\n'

for name in $cases; do
  left=$data/in/$name.$gv.left
  right=$data/in/$name.$gv.right
  in_mb=$(cat "$left" "$right" | wc -c | awk '{ printf "%.1f", $1 / 1048576 }')
  for tool in $(tools_for "$name"); do
    echo "running $name: $(label "$tool")" >&2
    set_cmd "$tool" "$left" "$right"
    out=$data/out/$name.$tool.out
    walls=
    peak_max=-
    cat "$left" "$right" >/dev/null
    i=0
    while [ "$i" -lt "$runs" ]; do
      run_once "$out"
      walls="$walls$WALL"$'\n'
      if [ "$PEAK" != - ] && { [ "$peak_max" = - ] || [ "$PEAK" -gt "$peak_max" ]; }; then
        peak_max=$PEAK
      fi
      i=$((i + 1))
    done
    median=$(printf '%s' "$walls" | sort -n | awk '{ v[NR] = $1 } END { print v[int((NR + 1) / 2)] }')
    out_mb=$(wc -c <"$out" | awk '{ printf "%.1f", $1 / 1048576 }')
    # Exit 0 is identical and 1 different for every tool here; above 1 is an error.
    # shellcheck disable=SC2016 # the backticks are markdown code spans
    printf '| %s | %s | `%s` | %s | %s | %s | %s |\n' \
      "$name" "$in_mb" "$(label "$tool")" "$STATUS" "$median" "$peak_max" "$out_mb"
  done
done
