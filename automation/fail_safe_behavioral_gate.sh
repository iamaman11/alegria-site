#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

python3 automation/smoke_broken_schema_to_dlq_behavioral.py
python3 automation/smoke_exhausted_retry_to_dlq_behavioral.py
python3 automation/smoke_stale_outbox_reclaim_behavioral.py
python3 automation/smoke_pending_hitl_not_failure_behavioral.py
