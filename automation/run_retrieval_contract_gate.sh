#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPORT_PATH="${RETRIEVAL_CONTRACT_GATE_REPORT_PATH:-/tmp/retrieval_contract_gate_current.json}"
DATABASE_URL="${DATABASE_URL:-postgres://postgres:postgres_password@localhost:5433/alegria}"
LOG_PATH="${RETRIEVAL_CONTRACT_GATE_LOG_PATH:-/tmp/retrieval_contract_gate_current.log}"
if [[ -n "${RETRIEVAL_CONTRACT_GATE_CLI_TARGET_DIR:-}" ]]; then
  CLI_TARGET_DIR="$RETRIEVAL_CONTRACT_GATE_CLI_TARGET_DIR"
  CLEANUP_TARGET_DIR=false
else
  CLI_TARGET_DIR="/tmp/retrieval-contract-gate-cli-$$"
  CLEANUP_TARGET_DIR=true
fi

cd "$ROOT_DIR"

export RETRIEVAL_CAPABILITY_REQUIRED="${RETRIEVAL_CAPABILITY_REQUIRED:-true}"
export CANONICAL_VECTOR_RETRIEVAL_REQUIRED="${CANONICAL_VECTOR_RETRIEVAL_REQUIRED:-true}"
export CONTEXTUAL_RAW_CHUNK_RETRIEVAL_REQUIRED="${CONTEXTUAL_RAW_CHUNK_RETRIEVAL_REQUIRED:-true}"
export VOYAGE_RERANK_REQUIRED="${VOYAGE_RERANK_REQUIRED:-true}"

if [[ "$CLEANUP_TARGET_DIR" == "true" ]]; then
  trap 'rm -rf "$CLI_TARGET_DIR"' EXIT
fi

rm -f "$REPORT_PATH" "$LOG_PATH"

set +e
(
  cd app/rust
  CARGO_TARGET_DIR="$CLI_TARGET_DIR" cargo run -q -p temporal_worker --bin temporal_starter -- \
    seo-preflight \
    --database-url "$DATABASE_URL" \
    --context-key "es:tourist:by" \
    --market "alegria-site" \
    --locale "ru-RU" \
    --country-code "ES" \
    --visa-type "tourist" \
    --applicant-profile "standard" \
    --report-json "$REPORT_PATH"
) >"$LOG_PATH" 2>&1
status=$?
set -e

if [[ ! -f "$REPORT_PATH" ]]; then
  python3 - <<'PY' "$REPORT_PATH" "$LOG_PATH"
import json
import sys
from pathlib import Path

report = Path(sys.argv[1])
log = Path(sys.argv[2])
reason = "seo_preflight failed before retrieval contract evaluation"
if log.exists():
    tail = log.read_text(encoding="utf-8", errors="replace").strip().splitlines()
    if tail:
        reason = tail[-1]
required = [
    "raw_chunks_4",
    "raw_chunks_ctx",
    "kb_canonical_4",
    "verified_rules_voyage4",
    "editorial_topics_voyage4",
    "seo_keyword_clusters_voyage4",
    "whole_page_advisory_prototypes",
]
payload = {
    "artifact_id": "seo_preflight",
    "status": "blocked_retrieval_contract",
    "context_key": "es:tourist:by",
    "normalized_profile": "unknown",
    "retrieval_capability_required": True,
    "canonical_vector_retrieval_required": True,
    "contextual_raw_chunk_retrieval_required": True,
    "voyage_rerank_required": True,
    "voyage_embeddings_ready": False,
    "voyage_contextualized_ready": False,
    "voyage_rerank_ready": False,
    "qdrant_ready": False,
    "qdrant_collection_contract_ready": False,
    "projection_blocked": True,
    "retrieval_block_reason": reason,
    "required_collections": [
        {
            "collection_name": name,
            "exists": False,
            "fresh": False,
            "projection_complete": False,
            "point_count": 0,
            "last_materialized_at": None,
            "last_source_change_at": None,
            "lag_seconds": None,
            "qdrant_collection_exists": False,
        }
        for name in required
    ],
}
report.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
PY
fi

python3 automation/check_retrieval_contract_gate.py "$REPORT_PATH"
cat "$LOG_PATH"

python3 - <<'PY' "$REPORT_PATH" "$status"
import json
import sys
from pathlib import Path

report = Path(sys.argv[1])
exit_code = int(sys.argv[2])
payload = json.loads(report.read_text(encoding="utf-8"))
status = payload.get("status", "unknown")
print(f"RETRIEVAL_CONTRACT_GATE: {status}")
if exit_code != 0:
    sys.exit(exit_code)
PY
