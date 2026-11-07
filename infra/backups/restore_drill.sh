#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

BUSINESS_DUMP="$("${ROOT_DIR}/infra/backups/backup_business_pg.sh")"
TEMPORAL_DUMP="$("${ROOT_DIR}/infra/backups/backup_temporal_pg.sh")"

"${ROOT_DIR}/infra/backups/restore_business_pg.sh" "$BUSINESS_DUMP"
"${ROOT_DIR}/infra/backups/restore_temporal_pg.sh" "$TEMPORAL_DUMP"

"${ROOT_DIR}/automation/temporal_production_gate.sh"
