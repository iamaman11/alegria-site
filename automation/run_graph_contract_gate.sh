#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPORT_PATH="${GRAPH_CONTRACT_GATE_REPORT_PATH:-/tmp/graph_contract_gate_current.json}"
DATABASE_URL="${DATABASE_URL:-postgres://postgres:postgres_password@localhost:5433/alegria}"
LOG_PATH="${GRAPH_CONTRACT_GATE_LOG_PATH:-/tmp/graph_contract_gate_current.log}"
BOOTSTRAP_LOCAL_INFRA="${GRAPH_CONTRACT_GATE_BOOTSTRAP_LOCAL_INFRA:-1}"
if [[ -n "${GRAPH_CONTRACT_GATE_CLI_TARGET_DIR:-}" ]]; then
  CLI_TARGET_DIR="$GRAPH_CONTRACT_GATE_CLI_TARGET_DIR"
  CLEANUP_TARGET_DIR=false
else
  CLI_TARGET_DIR="/tmp/graph-contract-gate-cli-$$"
  CLEANUP_TARGET_DIR=true
fi

cd "$ROOT_DIR"

compose_cmd() {
  if docker compose version >/dev/null 2>&1; then
    docker compose "$@"
  else
    docker-compose "$@"
  fi
}

wait_postgres_ready() {
  local timeout_s="${1:-45}"
  local elapsed=0
  while [[ "$elapsed" -lt "$timeout_s" ]]; do
    if docker exec -e PGPASSWORD=postgres_password alegria_postgres \
      pg_isready -U postgres -d alegria >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  return 1
}

if [[ "$BOOTSTRAP_LOCAL_INFRA" == "1" ]]; then
  if command -v docker >/dev/null 2>&1; then
    compose_cmd up -d postgres neo4j >/dev/null
    wait_postgres_ready 45 || true
  fi
fi

export RETRIEVAL_CAPABILITY_REQUIRED="${RETRIEVAL_CAPABILITY_REQUIRED:-false}"
export GRAPH_CAPABILITY_REQUIRED="${GRAPH_CAPABILITY_REQUIRED:-true}"
export NEO4J_SYNC_REQUIRED="${NEO4J_SYNC_REQUIRED:-true}"
export GRAPH_QUERY_REQUIRED="${GRAPH_QUERY_REQUIRED:-true}"
export GRAPH_GDS_REQUIRED="${GRAPH_GDS_REQUIRED:-true}"

if [[ "$CLEANUP_TARGET_DIR" == "true" ]]; then
  trap 'rm -rf "$CLI_TARGET_DIR"' EXIT
fi

rm -f "$REPORT_PATH" "$LOG_PATH"

DATABASE_URL="$DATABASE_URL" \
  python3 automation/bootstrap_graph_contract_projections.py

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
    --projection-max-lag-ms "${GRAPH_CONTRACT_PROJECTION_MAX_LAG_MS:-86400000}" \
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
reason = "seo_preflight failed before graph contract evaluation"
if log.exists():
    tail = log.read_text(encoding="utf-8", errors="replace").strip().splitlines()
    if tail:
        reason = tail[-1]
required = [
    "keyword_cluster",
    "serp_pattern",
    "page_blueprint",
    "page_node",
    "content_gap",
    "link_recommendation",
    "page_brief",
]
payload = {
    "artifact_id": "seo_preflight",
    "status": "blocked_graph_contract",
    "graph_contract_status": "blocked_graph_contract",
    "context_key": "es:tourist:by",
    "normalized_profile": "unknown",
    "graph_capability_required": True,
    "neo4j_sync_required": True,
    "graph_query_required": True,
    "graph_gds_required": True,
    "neo4j_ready": False,
    "graph_query_ready": False,
    "graph_gds_ready": False,
    "graph_projection_contract_ready": False,
    "graph_block_reason": reason,
    "required_graph_projections": [
        {
            "projection_name": name,
            "exists": False,
            "fresh": False,
            "projection_complete": False,
            "point_count": 0,
            "last_materialized_at": None,
            "last_source_change_at": None,
            "lag_seconds": None,
        }
        for name in required
    ],
}
report.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
PY
fi

python3 automation/check_graph_contract_gate.py "$REPORT_PATH"
cat "$LOG_PATH"

python3 - <<'PY' "$REPORT_PATH" "$status"
import json
import sys
from pathlib import Path

report = Path(sys.argv[1])
exit_code = int(sys.argv[2])
payload = json.loads(report.read_text(encoding="utf-8"))
status = payload.get("graph_contract_status", "unknown")
print(f"GRAPH_CONTRACT_GATE: {status}")
if exit_code != 0:
    sys.exit(exit_code)
PY
