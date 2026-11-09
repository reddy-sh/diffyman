#!/bin/sh
set -eu  

backup_dir=/var/backups/app
mkdir -p "$backup_dir"  
tar -czf "$backup_dir/app-$(date +%F).tgz" /srv/app
find "$backup_dir" -name '*.tgz' -mtime +30 -delete
