#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

log() { printf '[gate] %s\n' "$*"; }
die() { printf '[gate][FAIL] %s\n' "$*" >&2; exit 1; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

need_cmd docker
need_cmd cargo
need_cmd python3
need_cmd curl

DB_DSN="postgres://postgres:postgres_password@localhost:5433/alegria"
WORKER_MODE="${TEMPORAL_GATE_WORKER_MODE:-local}" # local | docker
SKIP_BUILD="${TEMPORAL_GATE_SKIP_BUILD:-1}"       # 1 skip docker build, 0 build
WORKER_PID=""
METRICS_URL="${TEMPORAL_GATE_METRICS_URL:-http://localhost:9464/metrics}"

wf_status() {
  local wf_id="$1"
  docker exec alegria_temporal temporal -o json workflow describe --workflow-id "$wf_id" 2>/dev/null \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("workflowExecutionInfo",{}).get("status","UNKNOWN"))'
}

wait_wf_closed() {
  local wf_id="$1"
  local timeout_s="${2:-180}"
  local elapsed=0
  while [ "$elapsed" -lt "$timeout_s" ]; do
    local st
    st="$(wf_status "$wf_id" || true)"
    if [ "$st" = "WORKFLOW_EXECUTION_STATUS_COMPLETED" ]; then
      return 0
    fi
    if [ "$st" = "WORKFLOW_EXECUTION_STATUS_FAILED" ] || [ "$st" = "WORKFLOW_EXECUTION_STATUS_TERMINATED" ] || [ "$st" = "WORKFLOW_EXECUTION_STATUS_TIMED_OUT" ]; then
      return 2
    fi
    sleep 2
    elapsed=$((elapsed + 2))
  done
  return 1
}

db_scalar() {
  local sql="$1"
  docker exec -e PGPASSWORD=postgres_password alegria_postgres \
    psql -U postgres -d alegria -t -A -c "$sql"
}

blake3_hex() {
  (
    cd app/rust
    cargo run -q -p cli_tools -- compute-bytes-hash --hex "$1"
  )
}

encode_fact_input_hex() {
  (
    cd app/rust
    cargo run -q -p cli_tools -- encode-fact-input --sections-json "$1"
  )
}

encode_validation_input_hex() {
  (
    cd app/rust
    cargo run -q -p cli_tools -- encode-validation-input \
      --required-links-json "$1" \
      --required-keys-json "$2" \
      --used-rule-keys-json "$3" \
      --used-fact-keys-json "$4" \
      --url-norm "$5"
  )
}

apply_business_schema() {
  log "applying business schema"
  docker exec -i -e PGPASSWORD=postgres_password alegria_postgres \
    psql -U postgres -d alegria >/dev/null < app/db/schema.sql
}

start_services() {
  if [ "$WORKER_MODE" = "docker" ]; then
    log "starting infra and temporal worker (docker mode)"
    if [ "$SKIP_BUILD" = "0" ]; then
      log "building temporal-worker image from current sources"
      docker compose build temporal-worker >/dev/null
    fi
    docker compose up -d postgres pgbouncer postgres-temporal temporal-server temporal-ui temporal-worker prometheus grafana
  else
    log "starting infra (local worker mode)"
    docker compose up -d postgres postgres-temporal temporal-server temporal-ui prometheus grafana
    log "starting local temporal_worker from current Rust sources"
    (
      cd app/rust
      TEMPORAL_URL=http://localhost:7233 \
      DATABASE_URL="$DB_DSN" \
      RUST_LOG=info \
      cargo run -q -p temporal_worker --bin temporal_worker \
      > /tmp/alegria_temporal_worker_gate.log 2>&1
    ) &
    WORKER_PID="$!"
    # Wait until worker can serve workflows (ping checks client + server reachability).
    local i=0
    until (cd app/rust && cargo run -q -p temporal_worker --bin temporal_starter -- ping >/dev/null 2>&1); do
      i=$((i + 1))
      [ "$i" -lt 30 ] || die "local temporal_worker did not become ready; see /tmp/alegria_temporal_worker_gate.log"
      sleep 1
    done
  fi
}

assert_metrics() {
  log "checking metrics endpoint"
  local body
  body="$(curl -fsS "$METRICS_URL")" || die "metrics endpoint unavailable: $METRICS_URL"
  grep -q "activity_duration_seconds" <<<"$body" || die "missing activity_duration_seconds metric"
  grep -q "workflow_completions_total" <<<"$body" || die "missing workflow_completions_total metric"
  grep -q "step_execution_reused_total" <<<"$body" || die "missing step_execution_reused_total metric"
}

assert_invariants() {
  log "checking DB/runtime invariants"
  apply_business_schema

  local idx_count
  idx_count="$(db_scalar "select count(*) from pg_indexes where schemaname='system' and tablename='sync_outbox' and indexname='idx_sync_outbox_dedup';" | tr -d '[:space:]')"
  [ "$idx_count" = "1" ] || die "missing unique dedup index idx_sync_outbox_dedup"

  local src_count
  src_count="$(db_scalar "select count(*) from kb.sources where status='active';" | tr -d '[:space:]')"
  if [ "${src_count:-0}" -lt 1 ]; then
    log "seeding minimal active source row (smoke bootstrap)"
    db_scalar "insert into kb.sources (source_key, source_type, source_label, base_url, trust_level, status) values ('smoke_source','internal','Smoke Source','https://example.com',3,'active') on conflict (source_key) do nothing;" >/dev/null
  fi

  # Required concepts for deterministic smoke extract path.
  db_scalar "insert into kb.concepts (concept_key, concept_type, label_ru, status) values ('consular_fee','fee','Консульский сбор','active') on conflict (concept_key) do nothing;" >/dev/null
  db_scalar "insert into kb.concepts (concept_key, concept_type, label_ru, status) values ('passport','document','Паспорт','active') on conflict (concept_key) do nothing;" >/dev/null
  db_scalar "insert into kb.concepts (concept_key, concept_type, label_ru, status) values ('medical_insurance','document','Медицинская страховка','active') on conflict (concept_key) do nothing;" >/dev/null
  db_scalar "insert into kb.visa_contexts (context_key, country_code, visa_family, visa_subtype, citizenship_code, status) values ('pl:work:by','pl','work',null,'by','active') on conflict (context_key) do nothing;" >/dev/null
}

run_ping_and_demo() {
  log "ping + demo-hitl"
  (cd app/rust && cargo run -q -p temporal_worker --bin temporal_starter -- ping) >/dev/null
  local out
  out="$(cd app/rust && cargo run -q -p temporal_worker --bin temporal_starter -- demo-hitl)"
  printf '%s\n' "$out" | grep -q "demo_hitl_ok" || die "demo-hitl failed"
}

run_fact_workflow() {
  local rid
  rid="$(cat /proc/sys/kernel/random/uuid)"
  log "fact workflow run_id=$rid"
  local sections_json input_hex input_hash
  sections_json='[{"content_md":"Паспорт обязателен. Консульский сбор 35 EUR. Медицинская страховка обязательна."}]'
  input_hex="$(encode_fact_input_hex "$sections_json")"
  input_hash="$(blake3_hex "$input_hex")"
  db_scalar "insert into pipeline.execution_runs (run_id, workflow_run_id, workflow_type, context_key, status) values ('$rid'::uuid, '$rid', 'extract_facts', 'pl:work:by', 'created');" >/dev/null
  db_scalar "insert into pipeline.execution_run_blobs (run_id, field_name, payload_type, schema_version, payload_bytes, payload_hash) values ('$rid'::uuid, 'input_payload', 'alegria.temporal.v1.FactExtractionInputPayload', 1, decode('$input_hex','hex'), '$input_hash');" >/dev/null
  (cd app/rust && cargo run -q -p temporal_worker --bin temporal_starter -- start --workflow fact-extraction --workflow-id "$rid") >/dev/null

  # Wait for HITL pause and resolve.
  local waited=0
  local hitl_seen=0
  while [ "$waited" -lt 120 ]; do
    local st
    st="$(wf_status "$rid" || true)"
    if [ "$st" = "WORKFLOW_EXECUTION_STATUS_COMPLETED" ]; then
      break
    fi
    if [ "$st" = "WORKFLOW_EXECUTION_STATUS_FAILED" ] || [ "$st" = "WORKFLOW_EXECUTION_STATUS_TERMINATED" ] || [ "$st" = "WORKFLOW_EXECUTION_STATUS_TIMED_OUT" ]; then
      break
    fi
    if docker exec alegria_temporal temporal workflow query --workflow-id "$rid" --name status 2>/dev/null | grep -q '"waiting_hitl":true'; then
      hitl_seen=1
      docker exec alegria_temporal temporal workflow signal --workflow-id "$rid" --name resume --input '{"decision":"approve","actor":"gate","notes":""}' >/dev/null
      break
    fi
    sleep 2
    waited=$((waited + 2))
  done

  # Fallback: even if query path is slow/timeout, push resume once.
  # Signal handler is idempotent for this test path.
  if [ "$hitl_seen" -eq 0 ]; then
    docker exec alegria_temporal temporal workflow signal --workflow-id "$rid" --name resume --input '{"decision":"approve","actor":"gate","notes":""}' >/dev/null 2>&1 || true
  fi

  if ! wait_wf_closed "$rid" 240; then
    die "fact workflow did not reach COMPLETED"
  fi

  local run_status
  run_status="$(db_scalar "select status from pipeline.execution_runs where run_id='$rid'::uuid;" | tr -d '[:space:]')"
  [ "$run_status" = "done" ] || die "fact execution_runs status is not done: $run_status"
}

run_content_workflow() {
  local rid
  rid="$(cat /proc/sys/kernel/random/uuid)"
  log "content workflow run_id=$rid"
  local input_hex input_hash
  input_hex="$(encode_validation_input_hex '[]' '[]' '[]' '[]' 'https://example.com/pl/work')"
  input_hash="$(blake3_hex "$input_hex")"
  db_scalar "insert into pipeline.execution_runs (run_id, workflow_run_id, workflow_type, context_key, status) values ('$rid'::uuid, '$rid', 'generate_content', 'pl:work:by', 'created');" >/dev/null
  db_scalar "insert into pipeline.execution_run_blobs (run_id, field_name, payload_type, schema_version, payload_bytes, payload_hash) values ('$rid'::uuid, 'input_payload', 'alegria.temporal.v1.ValidationInputPayload', 1, decode('$input_hex','hex'), '$input_hash');" >/dev/null
  (cd app/rust && cargo run -q -p temporal_worker --bin temporal_starter -- start --workflow content-generation --workflow-id "$rid") >/dev/null

  if ! wait_wf_closed "$rid" 180; then
    die "content workflow did not reach COMPLETED"
  fi

  local run_status
  run_status="$(db_scalar "select status from pipeline.execution_runs where run_id='$rid'::uuid;" | tr -d '[:space:]')"
  [ "$run_status" = "done" ] || die "content execution_runs status is not done: $run_status"
}

run_freshness_workflow() {
  local wf_id
  wf_id="freshness-check-$(cat /proc/sys/kernel/random/uuid)"
  log "freshness workflow workflow_id=$wf_id"

  (cd app/rust && cargo run -q -p temporal_worker --bin temporal_starter -- start --workflow freshness-check --workflow-id "$wf_id") >/dev/null
  if ! wait_wf_closed "$wf_id" 120; then
    die "freshness workflow did not reach COMPLETED"
  fi
}

main() {
  trap 'if [ -n "${WORKER_PID:-}" ]; then kill "${WORKER_PID}" >/dev/null 2>&1 || true; fi' EXIT
  start_services
  assert_invariants
  run_ping_and_demo
  run_fact_workflow
  run_content_workflow
  run_freshness_workflow
  assert_metrics
  log "PRODUCTION_GATE: PASS"
}

main "$@"
