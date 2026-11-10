#!/usr/bin/env bash
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 <dump.sql>" >&2; exit 1; }
FILE="$1"

[ "${ALEGRIA_ALLOW_DESTRUCTIVE_RESTORE:-}" = "YES" ] || {
  echo "[restore][BLOCKED] live-schema restore requires ALEGRIA_ALLOW_DESTRUCTIVE_RESTORE=YES" >&2
  exit 2
}
[ -s "$FILE" ] || { echo "[restore][BLOCKED] dump file missing or empty" >&2; exit 2; }

docker exec -e PGPASSWORD=postgres_password alegria_postgres \
  psql -U postgres -d alegria -c "DROP SCHEMA IF EXISTS raw CASCADE; DROP SCHEMA IF EXISTS kb CASCADE; DROP SCHEMA IF EXISTS extracted CASCADE; DROP SCHEMA IF EXISTS verified CASCADE; DROP SCHEMA IF EXISTS system CASCADE; DROP SCHEMA IF EXISTS pipeline CASCADE; DROP SCHEMA IF EXISTS site CASCADE; DROP SCHEMA IF EXISTS serp CASCADE; DROP SCHEMA IF EXISTS monitoring CASCADE;"

docker exec -i -e PGPASSWORD=postgres_password alegria_postgres \
  psql -U postgres -d alegria < "$FILE"
