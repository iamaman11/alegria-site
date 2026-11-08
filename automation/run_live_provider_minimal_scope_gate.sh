#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

cd "$ROOT_DIR"

python3 automation/check_truth_extraction_provider_ready.py || provider_ready_status=$?
provider_ready_status="${provider_ready_status:-0}"

set +e
python3 automation/smoke_real_provider_minimal_scope.py
smoke_status=$?
set -e

python3 automation/check_live_provider_smoke_evidence_schema.py

if [ "$smoke_status" -eq 0 ]; then
  echo "LIVE_PROVIDER_MINIMAL_SCOPE_GATE: OK"
  exit 0
fi

if [ "$smoke_status" -eq 2 ]; then
  echo "LIVE_PROVIDER_MINIMAL_SCOPE_GATE: PENDING_CREDENTIALS"
  exit 2
fi

if [ "$provider_ready_status" -eq 2 ]; then
  echo "LIVE_PROVIDER_MINIMAL_SCOPE_GATE: FAILED_AFTER_PROVIDER_PREFLIGHT"
else
  echo "LIVE_PROVIDER_MINIMAL_SCOPE_GATE: FAILED"
fi
exit 1
