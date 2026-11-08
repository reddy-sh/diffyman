#!/usr/bin/env bash
# Runs the real `diffyman` CLI on spike-A corpus cases and reports wall time and
# peak memory, one process per run.
#
#   spikes/line-diff-bench/cli-bench.sh BIN DATA_DIR OUT_DIR [CASE...]
#
# BIN is a release build of the CLI (target/release/diffyman), DATA_DIR holds
# the `<case>.left` / `<case>.right` pairs written by `gen`, and OUT_DIR
# receives each run's output and timing report. Cases default to `log-1g`,
# `log-100m`, `identical` and `same:log-1g`; the pseudo-case `same:<case>`
# diffs `<case>.left` against itself (the byte-identical fast path). Each case
# runs twice: `--json` (canonical document) and the default text diff, with
# output written to a file. Inputs are read once before each run, so the file
# cache is warm (as in the spike-A runs).
#
# Compare two OUT_DIRs (for example before and after a change) with
# `compare-outputs.py`: text must match byte for byte, JSON once `provenance`
# is removed.
#
# Memory is macOS "peak memory footprint" from `/usr/bin/time -l` (mapped input
# pages are not part of it). On Linux, GNU time's maximum resident set size is
# reported instead, which does count touched mapped pages.
set -euo pipefail

if [ "$#" -lt 3 ]; then
  echo "usage: $0 BIN DATA_DIR OUT_DIR [CASE...]" >&2
  exit 2
fi
bin=$1
data=$2
out=$3
shift 3
if [ "$#" -eq 0 ]; then
  set -- log-1g log-100m identical same:log-1g
fi
mkdir -p "$out"

printf '| Case | Output | Exit | Wall s | Peak MB | Output MB |\n|---|---|--:|--:|--:|--:|\n'
for case in "$@"; do
  name=${case/:/-}
  if [ "${case#same:}" != "$case" ]; then
    left="$data/${case#same:}.left"
    right=$left
  else
    left="$data/$case.left"
    right="$data/$case.right"
  fi
  for mode in json text; do
    flags=()
    if [ "$mode" = json ]; then
      flags=(--json)
    fi
    result="$out/$name.$mode.out"
    report="$out/$name.$mode.time"
    status=0
    cat "$left" "$right" >/dev/null
    if [ "$(uname)" = Darwin ]; then
      /usr/bin/time -l "$bin" diff "$left" "$right" ${flags[@]+"${flags[@]}"} \
        >"$result" 2>"$report" || status=$?
      wall=$(awk '/ real /{print $1}' "$report")
      peak=$(awk '/peak memory footprint/{printf "%.0f", $1 / 1048576}' "$report")
    else
      /usr/bin/time -v "$bin" diff "$left" "$right" ${flags[@]+"${flags[@]}"} \
        >"$result" 2>"$report" || status=$?
      wall=$(awk -F': ' '/Elapsed \(wall clock\)/{n = split($2, t, ":"); s = 0; for (i = 1; i <= n; i++) s = s * 60 + t[i]; print s}' "$report")
      peak=$(awk -F': ' '/Maximum resident set size/{printf "%.0f", $2 / 1024}' "$report")
    fi
    size=$(wc -c <"$result" | awk '{printf "%.1f", $1 / 1048576}')
    printf '| %s | %s | %s | %s | %s | %s |\n' "$case" "$mode" "$status" "$wall" "$peak" "$size"
  done
done
