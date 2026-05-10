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
TEMPORAL_CLI_ADDRESS="${TEMPORAL_GATE_CLI_ADDRESS:-127.0.0.1:7233}"

compose_cmd() {
  if docker compose version >/dev/null 2>&1; then
    docker compose "$@"
  else
    docker-compose "$@"
  fi
}

wf_status() {
  local wf_id="$1"
  docker exec alegria_temporal temporal --address "$TEMPORAL_CLI_ADDRESS" -o json workflow describe --workflow-id "$wf_id" 2>/dev/null \
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

wait_sql_value() {
  local sql="$1"
  local expected="$2"
  local timeout_s="${3:-120}"
  local elapsed=0
  while [ "$elapsed" -lt "$timeout_s" ]; do
    local value
    value="$(db_scalar "$sql" | tr -d '[:space:]' || true)"
    if [ "$value" = "$expected" ]; then
      return 0
    fi
    sleep 2
    elapsed=$((elapsed + 2))
  done
  return 1
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
      compose_cmd build temporal-worker >/dev/null
    fi
    compose_cmd up -d postgres pgbouncer postgres-temporal temporal-server temporal-ui temporal-worker prometheus grafana
  else
    log "starting infra (local worker mode)"
    compose_cmd up -d postgres postgres-temporal temporal-server temporal-ui prometheus grafana
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
  db_scalar "insert into kb.visa_contexts (context_key, country_code, visa_family, visa_subtype, citizenship_code, status) values ('es:tourist:by','ES','tourist',null,'BY','active') on conflict (context_key) do nothing;" >/dev/null
  db_scalar "insert into kb.visa_contexts (context_key, country_code, visa_family, visa_subtype, citizenship_code, status) values ('es:tourist:empty:by','ES','tourist','empty-gate','BY','active') on conflict (context_key) do nothing;" >/dev/null
  db_scalar "insert into verified.rule_instances (rule_instance_id, context_key, rule_type_key, concept_key, role_type, params, status, source_key, confidence, effective_from) values ('gate-es-doc-passport','es:tourist:by','document_required','passport','document_required','{\"fragment_text\":\"Valid passport is required for the application.\"}'::jsonb,'verified','smoke_source',1.0,current_date) on conflict (rule_instance_id) do nothing;" >/dev/null
  db_scalar "insert into verified.rule_instances (rule_instance_id, context_key, rule_type_key, concept_key, role_type, params, status, source_key, confidence, effective_from) values ('gate-es-fee-consular','es:tourist:by','fee_item','consular_fee','fee_item','{\"fragment_text\":\"Consular fee is 35 EUR for the standard visa process.\"}'::jsonb,'verified','smoke_source',1.0,current_date) on conflict (rule_instance_id) do nothing;" >/dev/null
  db_scalar "insert into verified.rule_instances (rule_instance_id, context_key, rule_type_key, concept_key, role_type, params, status, source_key, confidence, effective_from) values ('gate-es-doc-insurance','es:tourist:by','document_required','medical_insurance','document_required','{\"fragment_text\":\"Medical insurance covering the trip is required.\"}'::jsonb,'verified','smoke_source',1.0,current_date) on conflict (rule_instance_id) do nothing;" >/dev/null
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
    if docker exec alegria_temporal temporal --address "$TEMPORAL_CLI_ADDRESS" workflow query --workflow-id "$rid" --name status 2>/dev/null | grep -q '"waiting_hitl":true'; then
      hitl_seen=1
      docker exec alegria_temporal temporal --address "$TEMPORAL_CLI_ADDRESS" workflow signal --workflow-id "$rid" --name resume --input '{"decision":"approve","actor":"gate","notes":""}' >/dev/null
      break
    fi
    sleep 2
    waited=$((waited + 2))
  done

  # Fallback: even if query path is slow/timeout, push resume once.
  # Signal handler is idempotent for this test path.
  if [ "$hitl_seen" -eq 0 ]; then
    docker exec alegria_temporal temporal --address "$TEMPORAL_CLI_ADDRESS" workflow signal --workflow-id "$rid" --name resume --input '{"decision":"approve","actor":"gate","notes":""}' >/dev/null 2>&1 || true
  fi

  if ! wait_wf_closed "$rid" 240; then
    die "fact workflow did not reach COMPLETED"
  fi

  local run_status
  run_status="$(db_scalar "select status from pipeline.execution_runs where run_id='$rid'::uuid;" | tr -d '[:space:]')"
  [ "$run_status" = "done" ] || die "fact execution_runs status is not done: $run_status"
}

run_seo_site_build_workflow() {
  local rid
  rid="$(cat /proc/sys/kernel/random/uuid)"
  log "seo site-build workflow run_id=$rid"
  (cd app/rust && cargo run -q -p temporal_worker --bin temporal_starter -- start \
      --workflow seo-site-build \
      --workflow-id "$rid" \
      --database-url "$DB_DSN" \
      --context-key "es:tourist:by" \
      --market "alegria-site" \
      --locale "ru-RU" \
      --country-code "ES" \
      --visa-type "tourist" \
      --applicant-profile "standard" \
      --query "виза туристическая испания документы" \
      --query "виза туристическая испания стоимость" \
      --query "виза туристическая испания сроки") >/dev/null

  wait_sql_value \
    "select count(*)::text from pipeline.execution_run_blobs where run_id='$rid'::uuid and field_name='verified_support_bundle';" \
    "1" 120 \
    || die "verified_support_bundle blob was not persisted"

  wait_sql_value \
    "select count(*)::text from site.cms_publish_events where event_type='seo_page_review_requested' and event_payload ->> 'run_id' = '$rid';" \
    "1" 180 \
    || die "seo workflow did not reach review_requested"

  local page_node_key
  page_node_key="$(db_scalar "select page_node_key from site.cms_publish_events where event_type='seo_page_review_requested' and event_payload ->> 'run_id' = '$rid' order by occurred_at desc limit 1;" | tr -d '[:space:]')"
  [ -n "$page_node_key" ] || die "page_node_key for SEO review event not found"

  local cms_status
  cms_status="$(db_scalar "select current_status from site.cms_pages where page_node_key='$page_node_key';" | tr -d '[:space:]')"
  [ "$cms_status" = "review_required" ] || die "cms page did not enter review_required: $cms_status"

  (
    cd app/rust
    cargo run -q -p cli_tools -- cms-approve-publish \
      --database-url "$DB_DSN" \
      --page-node-key "$page_node_key" \
      --actor-role "seo_ops_gate" \
      --reason "temporal production gate approval" \
      --output-dir "app/rust/dist/static-site" \
      --base-url "https://example.com"
  ) >/dev/null

  if ! wait_wf_closed "$rid" 300; then
    die "seo workflow did not reach COMPLETED"
  fi

  local run_status
  run_status="$(db_scalar "select status from pipeline.execution_runs where run_id='$rid'::uuid;" | tr -d '[:space:]')"
  [ "$run_status" = "done" ] || die "seo execution_runs status is not done: $run_status"

  local artifact_status
  artifact_status="$(db_scalar "select status from site.publish_artifacts where page_node_key='$page_node_key' order by updated_at desc limit 1;" | tr -d '[:space:]')"
  [ "$artifact_status" = "ready" ] || die "publish artifact did not become ready: $artifact_status"

  local build_scope
  build_scope="$(db_scalar "select manifest_json ->> 'build_scope' from site.publish_artifacts where page_node_key='$page_node_key' order by updated_at desc limit 1;" | tr -d '[:space:]')"
  [ "$build_scope" = "page" ] || [ "$build_scope" = "site" ] || die "unexpected build_scope: $build_scope"

  local published_status
  published_status="$(db_scalar "select current_status from site.cms_pages where page_node_key='$page_node_key';" | tr -d '[:space:]')"
  [ "$published_status" = "published" ] || die "cms page did not become published: $published_status"

  local artifact_entries
  artifact_entries="$(db_scalar "select count(*)::text from site.publish_artifact_entries pae join site.publish_artifacts pa on pa.artifact_key = pae.artifact_key where pa.page_node_key='$page_node_key';" | tr -d '[:space:]')"
  [ "${artifact_entries:-0}" -ge 1 ] || die "publish artifact entries were not persisted"
}

run_seo_empty_support_failure() {
  local rid
  rid="$(cat /proc/sys/kernel/random/uuid)"
  log "seo empty-support workflow run_id=$rid"
  (cd app/rust && cargo run -q -p temporal_worker --bin temporal_starter -- start \
      --workflow seo-site-build \
      --workflow-id "$rid" \
      --database-url "$DB_DSN" \
      --context-key "es:tourist:empty:by" \
      --market "alegria-site" \
      --locale "ru-RU" \
      --country-code "ES" \
      --visa-type "tourist" \
      --applicant-profile "standard" \
      --query "виза туристическая испания документы") >/dev/null

  local wait_result=0
  wait_wf_closed "$rid" 120 || wait_result=$?
  [ "$wait_result" = "2" ] || die "empty-support seo workflow did not fail fast"
  local wf_state
  wf_state="$(wf_status "$rid" || true)"
  [ "$wf_state" = "WORKFLOW_EXECUTION_STATUS_FAILED" ] || die "empty-support workflow status is not FAILED: $wf_state"
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
  run_seo_empty_support_failure
  run_seo_site_build_workflow
  run_freshness_workflow
  assert_metrics
  log "PRODUCTION_GATE: PASS"
}

main "$@"
