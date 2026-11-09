#!/usr/bin/env zsh
# Logical backup of the local diffyman database (docs/operations.md).
#
# Usage: scripts/backup-db.sh [--dir DIR] [--keep-days N] [--database NAME] [--host-pg HOST:PORT]
#   --dir DIR            where dumps go (default ~/diffyman-data/backups)
#   --database NAME      the database to dump (default diffyman; for example a
#                        test database kept by tests/server/run_e2e.py --keep,
#                        with its own --dir)
#   --keep-days N        remove dumps older than N days, always keeping the
#                        newest one (default 7)
#   --host-pg HOST:PORT  run the pg_dump on PATH against HOST:PORT instead of
#                        the one in the compose container
#
# Writes diffyman-<UTC time>.dump (pg_dump custom format, mode 600) and a
# matching .sha256 file. The dump is first written under a temporary name and
# kept only when pg_restore can read its table of contents back. Logs carry
# names, sizes and timings only; the password is never printed.
set -euo pipefail

repo_root=${0:A:h:h}
source "$repo_root/scripts/lib/ops.zsh"

backup_dir=$HOME/diffyman-data/backups
keep_days=7
database=diffyman
while (( $# )); do
  case $1 in
    --dir) backup_dir=${2:?--dir needs a folder}; shift 2 ;;
    --keep-days) keep_days=${2:?--keep-days needs a number}; shift 2 ;;
    --database) database=${2:?--database needs a name}; shift 2 ;;
    --host-pg) ops_use_host_pg "${2:?--host-pg needs HOST:PORT}"; shift 2 ;;
    -h|--help) ops_usage "$0"; exit 0 ;;
    *) ops_die "unknown argument $1 (see --help)" ;;
  esac
done
[[ $keep_days == <1-> ]] || ops_die "--keep-days expects a whole number of days, at least 1"
[[ $backup_dir == /* ]] || ops_die "--dir expects an absolute path"
[[ $database == [a-z_][a-z0-9_]# ]] || ops_die "--database expects a lowercase name"

umask 077
mkdir -p "$backup_dir"
chmod 700 "$backup_dir"
ops_pg_ready

name=diffyman-$(date -u +%Y%m%dT%H%M%SZ).dump
final=$backup_dir/$name
partial=$backup_dir/.$name.partial
[[ ! -e $final ]] || ops_die "$name already exists"
trap 'rm -f -- "$partial"' EXIT

ops_log "backup of $database started ($ops_pg_mode)"
started=$SECONDS
ops_pg pg_dump -d "$database" --format=custom --compress=6 > "$partial" \
  || ops_die "pg_dump failed"
[[ -s $partial ]] || ops_die "pg_dump wrote nothing"

toc=$(ops_pg pg_restore --list < "$partial") || ops_die "the dump does not read back"
entries=${#${(M)${(f)toc}:#[0-9]*}}

mv -- "$partial" "$final"
print -r -- "$(ops_sha256 "$final")  $name" > "$final.sha256"
size=$(stat -f %z "$final" 2>/dev/null || stat -c %s "$final")
ops_log "backup ok: $name, $size bytes, $entries entries, $(( SECONDS - started ))s"

# Rotation runs only after a dump succeeded and never removes the newest
# dump, so failing backups never empty the folder.
newest=("$backup_dir"/diffyman-*.dump(N.om[1]))
minutes=$(( keep_days * 1440 ))
removed=0
for old in "$backup_dir"/diffyman-*.dump(N.mm+$minutes); do
  [[ $old == "${newest[1]}" ]] && continue
  rm -f -- "$old" "$old.sha256"
  (( removed += 1 ))
done
kept=("$backup_dir"/diffyman-*.dump(N))
ops_log "rotation: removed $removed dump(s) older than $keep_days days, ${#kept} kept"
