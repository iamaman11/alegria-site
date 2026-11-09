#!/usr/bin/env bash
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 <dump.sql>" >&2; exit 1; }
FILE="$1"

[ "${ALEGRIA_ALLOW_DESTRUCTIVE_RESTORE:-}" = "YES" ] || {
  echo "[restore][BLOCKED] live-schema restore requires ALEGRIA_ALLOW_DESTRUCTIVE_RESTORE=YES" >&2
  exit 2
}
[ -s "$FILE" ] || { echo "[restore][BLOCKED] dump file missing or empty" >&2; exit 2; }

docker exec -e PGPASSWORD=temporal_password alegria_postgres_temporal \
  psql -U temporal -d temporal -c "DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public;"

docker exec -i -e PGPASSWORD=temporal_password alegria_postgres_temporal \
  psql -U temporal -d temporal < "$FILE"
