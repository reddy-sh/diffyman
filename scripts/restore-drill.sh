#!/usr/bin/env zsh
# Restore drill: proves that the latest backup can be restored (docs/operations.md).
#
# Usage: scripts/restore-drill.sh [--dump FILE] [--dir DIR] [--store-dir DIR | --skip-blobs]
#                                 [--sample N] [--database NAME] [--host-pg HOST:PORT]
#   --dump FILE          the dump to restore (default: the newest in --dir)
#   --dir DIR            backup folder (default ~/diffyman-data/backups)
#   --store-dir DIR      root of the file:// blob store (default ~/diffyman-data/blobs)
#   --skip-blobs         do not check blobs (for example with an s3:// store)
#   --sample N           how many blob rows to check against the store (default 20)
#   --database NAME      the live database the dump came from, compared with
#                        the restored copy (default diffyman)
#   --host-pg HOST:PORT  use the psql/pg_restore on PATH against HOST:PORT
#
# Restores the dump into a temporary database (diffyman_drill_<time>) in the
# same server, compares row counts with the live database, checks that a
# random sample of blob rows has a matching object (size, and SHA-256 for
# objects up to 64 MiB), then drops the temporary database, also on failure.
# A sampled blob that is missing from the store only counts as a failure while
# the live database still has its row: blobs deleted after the backup are
# expected to be gone. The live database is only read.
set -euo pipefail

repo_root=${0:A:h:h}
source "$repo_root/scripts/lib/ops.zsh"

backup_dir=$HOME/diffyman-data/backups
store_dir=$HOME/diffyman-data/blobs
dump=
sample=20
check_blobs=1
live_db=diffyman
while (( $# )); do
  case $1 in
    --database) live_db=${2:?--database needs a name}; shift 2 ;;
    --dump) dump=${2:?--dump needs a file}; shift 2 ;;
    --dir) backup_dir=${2:?--dir needs a folder}; shift 2 ;;
    --store-dir) store_dir=${2:?--store-dir needs a folder}; shift 2 ;;
    --skip-blobs) check_blobs=0; shift ;;
    --sample) sample=${2:?--sample needs a number}; shift 2 ;;
    --host-pg) ops_use_host_pg "${2:?--host-pg needs HOST:PORT}"; shift 2 ;;
    -h|--help) ops_usage "$0"; exit 0 ;;
    *) ops_die "unknown argument $1 (see --help)" ;;
  esac
done
[[ $sample == <1-1000> ]] || ops_die "--sample expects a number from 1 to 1000"
[[ $live_db == [a-z_][a-z0-9_]# ]] || ops_die "--database expects a lowercase name"

if [[ -z $dump ]]; then
  newest=("$backup_dir"/diffyman-*.dump(N.om[1]))
  (( ${#newest} )) || ops_die "no diffyman-*.dump in $backup_dir: run scripts/backup-db.sh"
  dump=${newest[1]}
fi
[[ -f $dump && -s $dump ]] || ops_die "$dump is not a dump file"

if [[ -f $dump.sha256 ]]; then
  expected=$(<"$dump.sha256")
  [[ ${expected%% *} == "$(ops_sha256 "$dump")" ]] || ops_die "${dump:t}: SHA-256 does not match ${dump:t}.sha256"
  ops_log "checksum ok: ${dump:t}"
else
  ops_log "no ${dump:t}.sha256 next to the dump; checksum not checked"
fi

ops_pg_ready
drill_db=diffyman_drill_$(date -u +%Y%m%d%H%M%S)
failures=0

drop_drill_db() {
  ops_sql postgres "DROP DATABASE IF EXISTS $drill_db WITH (FORCE)" >/dev/null \
    && ops_log "dropped $drill_db" \
    || print -u2 -- "warning: could not drop $drill_db; drop it by hand"
}
trap drop_drill_db EXIT

ops_log "restoring ${dump:t} into $drill_db ($ops_pg_mode)"
started=$SECONDS
ops_sql postgres "CREATE DATABASE $drill_db" >/dev/null
ops_pg pg_restore --no-owner --no-privileges --exit-on-error -d "$drill_db" < "$dump" \
  || ops_die "pg_restore failed"
ops_log "restore ok in $(( SECONDS - started ))s"

tables=(api_keys blobs jobs diffs usage audit)
present=$(ops_sql "$drill_db" "SELECT count(*) FROM pg_tables WHERE schemaname = 'public' AND tablename IN ('${(j:',':)tables}')")
if (( present == 0 )); then
  ops_log "the dump has no diffyman tables (the server has not migrated this database yet); restore mechanics verified, nothing else to check"
  exit 0
fi
(( present == ${#tables} )) || { print -u2 -- "FAIL: only $present of ${#tables} tables restored"; (( failures += 1 )); }

migration=$(ops_sql "$drill_db" "SELECT coalesce(max(version), 0) || ' ' || coalesce(bool_and(success), true) FROM _sqlx_migrations" 2>/dev/null || print -- "none")
ops_log "migrations in the dump: latest version ${migration% *}, all succeeded: ${migration#* }"
[[ $migration == *' false' ]] && { print -u2 -- "FAIL: a migration in the dump did not succeed"; (( failures += 1 )); }

print -- "table          restored      live"
for t in $tables; do
  restored=$(ops_sql "$drill_db" "SELECT count(*) FROM $t" 2>/dev/null || print -- "-")
  live=$(ops_sql "$live_db" "SELECT count(*) FROM $t" 2>/dev/null || print -- "-")
  print -- "${(r:14:)t} ${(l:8:)restored}  ${(l:8:)live}"
done

# Every diff and job in the dump must point at blob rows that are in the dump.
dangling=$(ops_sql "$drill_db" "SELECT
    (SELECT count(*) FROM diffs d WHERE NOT EXISTS (SELECT 1 FROM blobs b WHERE b.sha256 = d.document_blob))
  + (SELECT count(*) FROM jobs j WHERE NOT EXISTS (SELECT 1 FROM blobs b WHERE b.sha256 = j.left_blob)
                                    OR NOT EXISTS (SELECT 1 FROM blobs b WHERE b.sha256 = j.right_blob))")
(( dangling == 0 )) || { print -u2 -- "FAIL: $dangling row(s) reference a blob row that is not in the dump"; (( failures += 1 )); }

if (( check_blobs )); then
  [[ -d $store_dir ]] || ops_die "blob store folder $store_dir does not exist (use --skip-blobs for an s3:// store)"
  rows=$(ops_sql "$drill_db" "SELECT sha256, size FROM blobs ORDER BY random() LIMIT $sample")
  checked=0 hashed=0 gone=0 missing=0 bad=0
  for row in ${(f)rows}; do
    hash=${row%%$'\t'*}
    size=${row##*$'\t'}
    [[ $hash == [0-9a-f](#c64) ]] || { (( bad += 1 )); continue; }
    (( checked += 1 ))
    object=$store_dir/blobs/sha256/${hash[1,2]}/$hash
    if [[ ! -f $object ]]; then
      still_live=$(ops_sql "$live_db" "SELECT count(*) FROM blobs WHERE sha256 = '$hash'" 2>/dev/null || print -- 0)
      if (( still_live )); then
        print -u2 -- "FAIL: blob ${hash[1,12]}… has a live row but no object"
        (( missing += 1 ))
      else
        (( gone += 1 ))
      fi
      continue
    fi
    actual_size=$(stat -f %z "$object" 2>/dev/null || stat -c %s "$object")
    if [[ $actual_size != "$size" ]]; then
      print -u2 -- "FAIL: blob ${hash[1,12]}… is $actual_size bytes, its row says $size"
      (( bad += 1 ))
    elif (( size <= 64 * 1024 * 1024 )); then
      (( hashed += 1 ))
      [[ $(ops_sha256 "$object") == "$hash" ]] || { print -u2 -- "FAIL: blob ${hash[1,12]}… content does not match its SHA-256"; (( bad += 1 )); }
    fi
  done
  ops_log "blobs: $checked sampled, $hashed hashed, $gone deleted since the backup, $missing missing, $bad wrong"
  failures=$(( failures + missing + bad ))
else
  ops_log "blobs: not checked (--skip-blobs)"
fi

if (( failures )); then
  print -u2 -- "restore drill FAILED: $failures problem(s)"
  exit 1
fi
ops_log "restore drill passed"
