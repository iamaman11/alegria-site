#!/usr/bin/env bash
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 <dump.sql>" >&2; exit 1; }
FILE="$1"

docker exec -e PGPASSWORD=temporal_password alegria_postgres_temporal \
  psql -U temporal -d temporal -c "DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public;"

docker exec -i -e PGPASSWORD=temporal_password alegria_postgres_temporal \
  psql -U temporal -d temporal < "$FILE"
