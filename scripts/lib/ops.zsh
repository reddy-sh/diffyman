# Shared helpers of the local operations scripts (docs/operations.md).
# Sourced, never run. Callers set `repo_root` and `set -euo pipefail` first.
#
# Postgres tools run in one of two ways:
#   compose (default)  inside the compose service `postgres`, over its local
#                      socket: the password never leaves the container.
#   host HOST:PORT     the psql/pg_dump/pg_restore on PATH, over TCP, with the
#                      password from .env.local in PGPASSWORD (never on a
#                      command line). For a native Postgres, and for testing.

setopt extended_glob

ops_compose=(docker compose -f "$repo_root/deploy/local/compose.yaml" --env-file "$repo_root/.env.local")
ops_pg_mode=compose
ops_pg_host=
ops_pg_port=

ops_die() {
  print -u2 -- "${ZSH_ARGZERO:t}: $*"
  exit 1
}

# Prints the comment block at the top of a script (its usage text).
ops_usage() {
  local line
  while IFS= read -r line; do
    [[ $line == '#!'* ]] && continue
    [[ $line == '#'* ]] || break
    line=${line#\#}
    print -r -- "${line# }"
  done < "$1"
}

ops_log() {
  print -- "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] $*"
}

# The value of KEY in .env.local, which must exist and be private (mode 600).
# Printed on stdout for capture by the caller; never logged.
ops_env_value() {
  local key=$1 file="$repo_root/.env.local" line value
  [[ -f $file ]] || ops_die "no .env.local: run scripts/local-up.sh first"
  local mode
  mode=$(stat -f %Lp "$file" 2>/dev/null || stat -c %a "$file")
  [[ $mode == 600 || $mode == 400 ]] || ops_die ".env.local must have mode 600 (it has $mode)"
  while IFS= read -r line || [[ -n $line ]]; do
    line=${line#export }
    [[ $line == "$key="* ]] || continue
    value=${line#*=}
    value=${value#[\"\']}
    value=${value%[\"\']}
    print -r -- "$value"
    return 0
  done < "$file"
  ops_die "$key is not set in .env.local"
}

# `--host-pg HOST:PORT` switches the Postgres tools to host mode.
ops_use_host_pg() {
  local target=$1
  [[ $target == ?*:<-> ]] || ops_die "--host-pg expects HOST:PORT"
  ops_pg_mode=host
  ops_pg_host=${target%:*}
  ops_pg_port=${target##*:}
  PGPASSWORD=$(ops_env_value DIFFYMAN_PG_PASSWORD)
  export PGPASSWORD
}

# Runs a command with a time limit (seconds); fails when it is exceeded. The
# docker CLI can hang when Docker Desktop is unhealthy, and a nightly job must
# not wait forever.
ops_with_timeout() {
  local limit=$1; shift
  "$@" &
  local pid=$! waited=0
  while kill -0 $pid 2>/dev/null; do
    if (( waited >= limit )); then
      kill $pid 2>/dev/null || true
      wait $pid 2>/dev/null || true
      return 124
    fi
    sleep 1
    (( waited += 1 ))
  done
  wait $pid
}

# Checks that the Postgres tools can run.
ops_pg_ready() {
  if [[ $ops_pg_mode == host ]]; then
    command -v pg_dump >/dev/null || ops_die "pg_dump is not on PATH"
    ops_pg psql -d postgres -Atqc 'select 1' >/dev/null \
      || ops_die "cannot connect to Postgres on $ops_pg_host:$ops_pg_port"
    return 0
  fi
  command -v docker >/dev/null || ops_die "docker is not on PATH"
  ops_with_timeout 30 docker info >/dev/null 2>&1 \
    || ops_die "Docker is not responding (is Docker Desktop running?)"
  local running
  running=$(ops_with_timeout 30 "${ops_compose[@]}" ps --status running --services 2>/dev/null) \
    || ops_die "docker compose did not answer"
  [[ $'\n'$running$'\n' == *$'\n'postgres$'\n'* ]] \
    || ops_die "the compose service postgres is not running: scripts/local-up.sh"
}

# Runs a Postgres client tool (psql, pg_dump, pg_restore) as user diffyman.
# Standard input is passed through.
ops_pg() {
  local tool=$1; shift
  if [[ $ops_pg_mode == host ]]; then
    "$tool" -h "$ops_pg_host" -p "$ops_pg_port" -U diffyman "$@"
  else
    "${ops_compose[@]}" exec -T postgres "$tool" -U diffyman "$@"
  fi
}

# Runs one SQL statement in DATABASE and prints unaligned, tuples-only rows.
ops_sql() {
  local database=$1 sql=$2
  ops_pg psql -X -q -v ON_ERROR_STOP=1 -d "$database" -At -F $'\t' -c "$sql"
}

# Exports the variables that point S3 clients (the aws CLI, and object_store
# in diffyman-server built with the `s3` feature) at the local S3-compatible
# server of the compose profile `s3`, with the test credentials from
# .env.local. An endpoint or region already set wins; the credentials always
# come from .env.local, and shared AWS profile files are ignored, so a real
# account's keys are never sent anywhere by these scripts.
ops_local_s3_env() {
  : ${AWS_ENDPOINT_URL_S3:=http://127.0.0.1:9000}
  : ${AWS_ALLOW_HTTP:=true}
  : ${AWS_REGION:=us-east-1}
  AWS_ACCESS_KEY_ID=$(ops_env_value DIFFYMAN_S3_TEST_KEY)
  AWS_SECRET_ACCESS_KEY=$(ops_env_value DIFFYMAN_S3_TEST_SECRET)
  AWS_DEFAULT_REGION=$AWS_REGION
  export AWS_ENDPOINT_URL_S3 AWS_ALLOW_HTTP AWS_REGION AWS_DEFAULT_REGION AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY
  export AWS_CONFIG_FILE=/dev/null AWS_SHARED_CREDENTIALS_FILE=/dev/null
  export AWS_EC2_METADATA_DISABLED=true AWS_PAGER=
  unset AWS_PROFILE AWS_SESSION_TOKEN
}

# The SHA-256 of a file, in lowercase hex.
ops_sha256() {
  local out
  out=$(shasum -a 256 "$1")
  print -r -- "${out%% *}"
}
