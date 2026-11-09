#!/usr/bin/env zsh
# Nightly backup job for macOS (launchd) (docs/operations.md).
#
# Usage: scripts/install-backup-job.sh [--install | --uninstall] [--at HH:MM]
#   (no flag)    print the job definition and what --install would do; change nothing
#   --install    write ~/Library/LaunchAgents/sh.reddy.diffyman.backup.plist and load it
#   --uninstall  unload the job and remove its file
#   --at HH:MM   local time of the nightly run (default 03:15)
#
# The job runs scripts/backup-db.sh with its defaults (7-day rotation) and
# appends its output to ~/diffyman-data/backups/backup.log. A run missed while
# the Mac was asleep starts at the next wake. The job file holds paths only,
# never a password: backup-db.sh reads .env.local itself.
set -euo pipefail

repo_root=${0:A:h:h}
source "$repo_root/scripts/lib/ops.zsh"

label=sh.reddy.diffyman.backup
plist=$HOME/Library/LaunchAgents/$label.plist
log_file=$HOME/diffyman-data/backups/backup.log
action=print
at=03:15
while (( $# )); do
  case $1 in
    --install) action=install; shift ;;
    --uninstall) action=uninstall; shift ;;
    --at) at=${2:?--at needs HH:MM}; shift 2 ;;
    -h|--help) ops_usage "$0"; exit 0 ;;
    *) ops_die "unknown argument $1 (see --help)" ;;
  esac
done
[[ $at == ([01][0-9]|2[0-3]):[0-5][0-9] ]] || ops_die "--at expects HH:MM (24-hour)"
hour=$(( 10#${at%%:*} ))
minute=$(( 10#${at##*:} ))
[[ $OSTYPE == darwin* ]] || ops_die "launchd jobs are macOS only; use cron or a systemd timer elsewhere (docs/operations.md)"

xml_escape() {
  local s=$1
  s=${s//&/&amp;}
  s=${s//</&lt;}
  s=${s//>/&gt;}
  print -r -- "$s"
}

# launchd starts jobs with a minimal PATH; add the folders of the tools the
# backup needs (docker and its credential helpers, shasum).
path_dirs=(/usr/bin /bin /usr/sbin /sbin)
for tool in docker pg_dump shasum; do
  if found=$(command -v $tool 2>/dev/null); then
    path_dirs+=(${found:h})
  fi
done
path_dirs+=(/opt/homebrew/bin /usr/local/bin /Applications/Docker.app/Contents/Resources/bin)
job_path=${(j.:.)${(u)path_dirs}}

definition=$(cat <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$label</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/zsh</string>
    <string>$(xml_escape "$repo_root/scripts/backup-db.sh")</string>
  </array>
  <key>WorkingDirectory</key>
  <string>$(xml_escape "$repo_root")</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>$(xml_escape "$job_path")</string>
  </dict>
  <key>StartCalendarInterval</key>
  <dict>
    <key>Hour</key>
    <integer>$hour</integer>
    <key>Minute</key>
    <integer>$minute</integer>
  </dict>
  <key>RunAtLoad</key>
  <false/>
  <key>ProcessType</key>
  <string>Background</string>
  <key>LowPriorityIO</key>
  <true/>
  <key>Nice</key>
  <integer>10</integer>
  <key>StandardOutPath</key>
  <string>$(xml_escape "$log_file")</string>
  <key>StandardErrorPath</key>
  <string>$(xml_escape "$log_file")</string>
</dict>
</plist>
EOF
)

domain=gui/$(id -u)

case $action in
  print)
    print -r -- "$definition"
    print
    print -- "Nothing was changed. With --install this script would:"
    print -- "  1. write the definition above to $plist (mode 644) and check it with plutil -lint"
    print -- "  2. unload a previous version of $label, if one is loaded"
    print -- "  3. load it with: launchctl bootstrap $domain $plist"
    print -- "Then scripts/backup-db.sh runs every day at $at local time, keeps 7 days of"
    print -- "dumps in ~/diffyman-data/backups and logs to $log_file."
    print -- "Run once now with: launchctl kickstart $domain/$label"
    ;;
  install)
    mkdir -p "${plist:h}" "${log_file:h}"
    staged=$(mktemp "${plist:h}/.$label.XXXXXX")
    print -r -- "$definition" > "$staged"
    plutil -lint "$staged" >/dev/null || { rm -f -- "$staged"; ops_die "the generated job file is not a valid plist"; }
    chmod 644 "$staged"
    launchctl bootout "$domain/$label" 2>/dev/null || true
    mv -- "$staged" "$plist"
    launchctl bootstrap "$domain" "$plist"
    print -- "Installed $label: scripts/backup-db.sh runs daily at $at; log: $log_file"
    print -- "Run once now with: launchctl kickstart $domain/$label"
    ;;
  uninstall)
    launchctl bootout "$domain/$label" 2>/dev/null || true
    rm -f -- "$plist"
    print -- "Removed $label"
    ;;
esac
