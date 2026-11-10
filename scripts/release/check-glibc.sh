#!/bin/sh
# Fails when a Linux shared library needs a glibc newer than MAX (default
# 2.28, the floor of the manylinux_2_28 Python wheels): the highest GLIBC_x.y
# symbol version it references must be at most MAX. Needs `objdump`.
#
#   scripts/release/check-glibc.sh LIBRARY [MAX]
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  echo "usage: $0 LIBRARY [MAX]" >&2
  exit 2
fi
library=$1
max=${2:-2.28}

needed=$(objdump -T "$library" | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -t . -k 1,1n -k 2,2n -k 3,3n | tail -n 1)
if [ -z "$needed" ]; then
  echo "$library: references no versioned glibc symbol"
  exit 0
fi
highest=$(printf '%s\n%s\n' "$needed" "$max" | sort -t . -k 1,1n -k 2,2n -k 3,3n | tail -n 1)
if [ "$highest" != "$max" ]; then
  echo "$library needs glibc $needed, newer than $max" >&2
  exit 1
fi
echo "$library: needs glibc $needed (at most $max allowed)"
