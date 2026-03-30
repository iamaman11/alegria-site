#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
OUT_DIR="${ROOT_DIR}/infra/backups/artifacts"
mkdir -p "$OUT_DIR"

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
FILE="${OUT_DIR}/temporal_${STAMP}.sql"

docker exec -e PGPASSWORD=temporal_password alegria_postgres_temporal \
  pg_dump -U temporal -d temporal --no-owner --no-privileges > "$FILE"

echo "$FILE"
