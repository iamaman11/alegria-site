#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

if [ -f infra/local/dev_runtime.env ]; then
  set -a
  # shellcheck disable=SC1091
  source infra/local/dev_runtime.env
  set +a
fi

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

wait_temporal_cli_ready() {
  local timeout_s="${1:-60}"
  local elapsed=0
  while [ "$elapsed" -lt "$timeout_s" ]; do
    if docker exec alegria_temporal temporal --address "$TEMPORAL_CLI_ADDRESS" operator cluster health >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    elapsed=$((elapsed + 2))
  done
  die "Temporal CLI did not become ready at $TEMPORAL_CLI_ADDRESS"
}

db_scalar() {
  local sql="$1"
  docker exec -e PGPASSWORD=postgres_password alegria_postgres \
    psql -q -U postgres -d alegria -t -A -c "$sql"
}

table_exists() {
  local schema="$1"
  local table="$2"
  db_scalar "select exists (select 1 from information_schema.tables where table_schema='${schema}' and table_name='${table}');" | tr -d '[:space:]'
}

assert_db_baseline() {
  local verdict
  verdict="$(python3 automation/check_local_runtime_db_baseline.py --database-url "$DB_DSN" 2>&1)" || {
    printf '%s\n' "$verdict" >&2
    die "local DB is not baseline-ready for migration-first runtime path"
  }
  printf '%s\n' "$verdict"
}

blake3_hex() {
  (
    cd app/rust
    cargo run -q -p cli_tools -- compute-bytes-hash --hex "$1"
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

wait_sql_value_with_outbox_drain() {
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
    drain_open_outbox_events
    value="$(db_scalar "$sql" | tr -d '[:space:]' || true)"
    if [ "$value" = "$expected" ]; then
      return 0
    fi
    sleep 5
    elapsed=$((elapsed + 5))
  done
  return 1
}

seed_admissible_verified_rule() {
  local rule_instance_id="$1"
  local context_key="$2"
  local rule_type_key="$3"
  local concept_key="$4"
  local role_type="$5"
  local fragment_text="$6"
  local source_key="${7:-smoke_source}"
  local unique_token="${8:-$(cat /proc/sys/kernel/random/uuid)}"
  local page_id section_id snapshot_hash
  page_id="$(db_scalar "insert into raw.pages (url, domain, dtype, status_code, title, raw_html, raw_html_bytes, content_hash, content, processed) values ('https://example.com/${unique_token}','example.com','internal',200,'Smoke Source','<html><body><main><p>${fragment_text}</p></main></body></html>', length('<html><body><main><p>${fragment_text}</p></main></body></html>'), md5('<html><body><main><p>${fragment_text}</p></main></body></html>'), jsonb_build_object('markdown','${fragment_text}','headings','[]'::jsonb,'links','[]'::jsonb), true) returning id;" | tr -d '[:space:]')"
  section_id="$(db_scalar "insert into raw.sections (page_id, heading_path, heading_level, section_order, section_type, content_md, content_hash) values (${page_id}, 'Requirements', 1, 0, 'paragraph', '${fragment_text}', md5('${fragment_text}')) returning id;" | tr -d '[:space:]')"
  snapshot_hash="$(db_scalar "select content_hash from raw.pages where id=${page_id};" | tr -d '[:space:]')"
  db_scalar "insert into verified.rule_instances (rule_instance_id, context_key, rule_type_key, concept_key, role_type, params, status, source_key, confidence, evidence_section_id, evidence_quote, span_start, span_end, source_snapshot_hash, verification_method, adjudication_reason, publish_admissibility, freshness_class, completeness_class, registry_version, prompt_version, model_version, pipeline_version, effective_from) values ('${rule_instance_id}','${context_key}','${rule_type_key}','${concept_key}','${role_type}', jsonb_build_object('fragment_text','${fragment_text}'),'verified','${source_key}',1.0,${section_id},'${fragment_text}',0,length('${fragment_text}'),'${snapshot_hash}','bootstrap_seed','temporal_production_gate_seed','admissible','fresh','complete','registry@1','seed@1','none','temporal_production_gate@1',current_date) on conflict (rule_instance_id) do update set params=excluded.params, status=excluded.status, source_key=excluded.source_key, confidence=excluded.confidence, evidence_section_id=excluded.evidence_section_id, evidence_quote=excluded.evidence_quote, span_start=excluded.span_start, span_end=excluded.span_end, source_snapshot_hash=excluded.source_snapshot_hash, verification_method=excluded.verification_method, adjudication_reason=excluded.adjudication_reason, publish_admissibility=excluded.publish_admissibility, freshness_class=excluded.freshness_class, completeness_class=excluded.completeness_class, registry_version=excluded.registry_version, prompt_version=excluded.prompt_version, model_version=excluded.model_version, pipeline_version=excluded.pipeline_version, updated_at=now();" >/dev/null
}

apply_business_migrations() {
  log "checking local DB baseline"
  assert_db_baseline
  log "applying business migrations"
  local migration
  for migration in app/db/migrations/*.sql; do
    docker exec -i -e PGPASSWORD=postgres_password alegria_postgres \
      psql -v ON_ERROR_STOP=1 -U postgres -d alegria >/dev/null < "$migration"
  done
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
    # In local worker mode we intentionally avoid services that depend on the
    # dockerized temporal-worker image, otherwise compose may trigger an
    # unnecessary image build/pull and make the gate flaky.
    compose_cmd up -d postgres pgbouncer postgres-temporal temporal-server temporal-ui neo4j qdrant
    log "starting local temporal_worker from current Rust sources"
    (
      cd app/rust
      TEMPORAL_URL=http://localhost:7233 \
      DATABASE_URL="$DB_DSN" \
      QDRANT_URL="${QDRANT_URL:-http://localhost:6334}" \
      QDRANT_SKIP_COMPATIBILITY_CHECK="${QDRANT_SKIP_COMPATIBILITY_CHECK:-true}" \
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
  wait_temporal_cli_ready 60
}

assert_metrics() {
  log "checking metrics endpoint"
  local body
  body="$(curl -fsS "$METRICS_URL")" || die "metrics endpoint unavailable: $METRICS_URL"
  grep -q "activity_duration_seconds" <<<"$body" || die "missing activity_duration_seconds metric"
  grep -q "workflow_completions_total" <<<"$body" || die "missing workflow_completions_total metric"
  grep -q "step_execution_reused_total" <<<"$body" || die "missing step_execution_reused_total metric"
}

run_live_provider_gate() {
  log "running canonical Step 5 live-provider gate"
  if bash automation/run_live_provider_minimal_scope_gate.sh; then
    return 0
  fi
  local status=$?
  if [ "$status" = "2" ]; then
    die "canonical Step 5 live-provider gate is PENDING_CREDENTIALS; production gate cannot pass without a configured truth provider"
  fi
  die "canonical Step 5 live-provider gate failed"
}

drain_open_outbox_events() {
  local open_events
  open_events="$(db_scalar "select count(*)::text from system.sync_outbox where status in ('pending','processing') or (status='failed' and retry_count < 10);" | tr -d '[:space:]')"
  if [ "${open_events:-0}" -eq 0 ]; then
    return 0
  fi
  log "draining open outbox events count=$open_events"
  (
    cd app/rust
    DATABASE_URL="$DB_DSN" \
    QDRANT_URL="${QDRANT_URL:-http://localhost:6334}" \
    QDRANT_SKIP_COMPATIBILITY_CHECK="${QDRANT_SKIP_COMPATIBILITY_CHECK:-true}" \
    OUTBOX_MAX_CYCLES="${PRODUCTION_GATE_OUTBOX_DRAIN_MAX_CYCLES:-12}" \
    OUTBOX_WORKER_ID="production-gate-drain-$$" \
      cargo run -q -p outbox_worker --bin outbox_worker
  ) || die "outbox drain failed"
}

approve_review_pages_until_wf_closed() {
  local rid="$1"
  local timeout_s="${2:-600}"
  local elapsed=0
  while [ "$elapsed" -lt "$timeout_s" ]; do
    local wf_state
    wf_state="$(wf_status "$rid" || true)"
    if [ "$wf_state" = "WORKFLOW_EXECUTION_STATUS_COMPLETED" ]; then
      return 0
    fi
    if [ "$wf_state" = "WORKFLOW_EXECUTION_STATUS_FAILED" ] || [ "$wf_state" = "WORKFLOW_EXECUTION_STATUS_TERMINATED" ] || [ "$wf_state" = "WORKFLOW_EXECUTION_STATUS_TIMED_OUT" ]; then
      return 2
    fi

    drain_open_outbox_events

    local page_node_key
    page_node_key="$(db_scalar "
      select e.page_node_key
      from site.cms_publish_events e
      join site.cms_pages p on p.page_node_key = e.page_node_key
      where e.event_type = 'seo_page_review_requested'
        and e.event_payload ->> 'run_id' = '$rid'
        and p.current_status = 'review_required'
      order by e.occurred_at
      limit 1;" | tr -d '[:space:]')"
    if [ -n "$page_node_key" ]; then
      log "approving review-required page page_node_key=$page_node_key"
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
    fi

    sleep 5
    elapsed=$((elapsed + 5))
  done
  return 1
}

run_retrieval_contract_gate() {
  log "bootstrapping required retrieval collections"
  DATABASE_URL="$DB_DSN" \
  QDRANT_URL="${QDRANT_URL:-http://localhost:6334}" \
    python3 automation/bootstrap_retrieval_contract_collections.py || die "retrieval collection bootstrap failed"

  drain_open_outbox_events

  log "running hard-required retrieval contract gate"
  RETRIEVAL_CONTRACT_GATE_REPORT_PATH="${RETRIEVAL_CONTRACT_GATE_REPORT_PATH:-/tmp/retrieval_contract_gate_current.json}" \
  RETRIEVAL_CAPABILITY_REQUIRED=true \
  CANONICAL_VECTOR_RETRIEVAL_REQUIRED=true \
  CONTEXTUAL_RAW_CHUNK_RETRIEVAL_REQUIRED=true \
  VOYAGE_RERANK_REQUIRED=true \
    bash automation/run_retrieval_contract_gate.sh || die "hard-required retrieval contract gate failed"
}

run_graph_contract_gate() {
  log "running hard-required graph contract gate"
  GRAPH_CONTRACT_GATE_REPORT_PATH="${GRAPH_CONTRACT_GATE_REPORT_PATH:-/tmp/graph_contract_gate_current.json}" \
  GRAPH_CAPABILITY_REQUIRED=true \
  NEO4J_SYNC_REQUIRED=true \
  GRAPH_QUERY_REQUIRED=true \
  GRAPH_GDS_REQUIRED=true \
  RETRIEVAL_CAPABILITY_REQUIRED=false \
    bash automation/run_graph_contract_gate.sh || die "hard-required graph contract gate failed"
}

assert_invariants() {
  log "checking DB/runtime invariants"
  apply_business_migrations

  local idx_count
  idx_count="$(db_scalar "select count(*) from pg_indexes where schemaname='system' and tablename='sync_outbox' and indexname='idx_sync_outbox_dedup';" | tr -d '[:space:]')"
  [ "$idx_count" = "1" ] || die "missing unique dedup index idx_sync_outbox_dedup"

  local src_count
  src_count="$(db_scalar "select count(*) from kb.sources where status='active';" | tr -d '[:space:]')"
  if [ "${src_count:-0}" -lt 1 ]; then
    log "seeding minimal active source row (smoke bootstrap)"
    db_scalar "insert into kb.sources (source_key, source_type, source_label, base_url, trust_level, status) values ('smoke_source','internal','Smoke Source','https://example.com',3,'active') on conflict (source_key) do nothing;" >/dev/null
  fi

  # Required concepts for deterministic support bootstrap.
  db_scalar "insert into kb.concepts (concept_key, concept_type, label_ru, status) values ('consular_fee','fee','Консульский сбор','active') on conflict (concept_key) do nothing;" >/dev/null
  db_scalar "insert into kb.concepts (concept_key, concept_type, label_ru, status) values ('passport','document','Паспорт','active') on conflict (concept_key) do nothing;" >/dev/null
  db_scalar "insert into kb.concepts (concept_key, concept_type, label_ru, status) values ('medical_insurance','document','Медицинская страховка','active') on conflict (concept_key) do nothing;" >/dev/null
  db_scalar "insert into kb.concepts (concept_key, concept_type, label_ru, status) values ('standard_processing_time','timeline','Стандартный срок рассмотрения','active') on conflict (concept_key) do nothing;" >/dev/null
  db_scalar "insert into kb.concepts (concept_key, concept_type, label_ru, status) values ('standard_tourist_eligibility','rule','Стандартные условия туристической визы','active') on conflict (concept_key) do nothing;" >/dev/null
  db_scalar "insert into kb.concepts (concept_key, concept_type, label_ru, status) values ('standard_application_process','process','Порядок подачи заявления','active') on conflict (concept_key) do nothing;" >/dev/null
  db_scalar "insert into kb.concepts (concept_key, concept_type, label_ru, status) values ('standard_application_channel','location','Канал подачи заявления','active') on conflict (concept_key) do nothing;" >/dev/null
  db_scalar "insert into kb.visa_contexts (context_key, country_code, visa_family, visa_subtype, citizenship_code, status) values ('pl:work:by','pl','work',null,'by','active') on conflict (context_key) do nothing;" >/dev/null
  db_scalar "insert into kb.visa_contexts (context_key, country_code, visa_family, visa_subtype, citizenship_code, status) values ('es:tourist:by','ES','tourist',null,'BY','active') on conflict (context_key) do nothing;" >/dev/null
  db_scalar "insert into kb.visa_contexts (context_key, country_code, visa_family, visa_subtype, citizenship_code, status) values ('es:tourist:empty:by','ES','tourist','empty-gate','BY','active') on conflict (context_key) do nothing;" >/dev/null
  seed_admissible_verified_rule "gate-es-doc-passport" "es:tourist:by" "document_required" "passport" "document_required" "Valid passport is required for the application."
  seed_admissible_verified_rule "gate-es-fee-consular" "es:tourist:by" "fee_item" "consular_fee" "fee_item" "Consular fee is 35 EUR for the standard visa process."
  seed_admissible_verified_rule "gate-es-doc-insurance" "es:tourist:by" "document_required" "medical_insurance" "document_required" "Medical insurance covering the trip is required."
  seed_admissible_verified_rule "gate-es-timing-standard" "es:tourist:by" "timeline_item" "standard_processing_time" "timeline_item" "Standard tourist visa processing normally takes up to 15 calendar days after the application is lodged."
  seed_admissible_verified_rule "gate-es-eligibility-standard" "es:tourist:by" "eligibility_rule" "standard_tourist_eligibility" "eligibility_rule" "Applicants must satisfy the tourist visa eligibility conditions for the selected travel purpose."
  seed_admissible_verified_rule "gate-es-process-submit" "es:tourist:by" "step" "standard_application_process" "step" "Applicants submit the application with required documents and the visa fee through the official appointment or application channel."
  seed_admissible_verified_rule "gate-es-where-apply" "es:tourist:by" "where_to_apply" "standard_application_channel" "where_to_apply" "Tourist visa applications are submitted through the competent Spanish consular or visa application channel for the applicant location."
}

run_ping_and_demo() {
  log "ping production worker"
  (cd app/rust && cargo run -q -p temporal_worker --bin temporal_starter -- ping) >/dev/null
}

run_seo_site_build_workflow() {
  local rid
  rid="$(cat /proc/sys/kernel/random/uuid)"
  log "seo canonical-cutover workflow run_id=$rid"
  (cd app/rust && cargo run -q -p temporal_worker --bin temporal_starter -- start \
      --workflow seo-site-build-canonical-cutover \
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

  drain_open_outbox_events

  wait_sql_value_with_outbox_drain \
    "select count(*)::text from site.cms_publish_events where event_type='seo_page_review_requested' and event_payload ->> 'run_id' = '$rid';" \
    "1" 420 \
    || die "seo workflow did not reach review_requested"

  if ! approve_review_pages_until_wf_closed "$rid" 900; then
    die "seo workflow did not reach COMPLETED"
  fi

  local run_status
  run_status="$(db_scalar "select status from pipeline.execution_runs where run_id='$rid'::uuid;" | tr -d '[:space:]')"
  [ "$run_status" = "done" ] || die "seo execution_runs status is not done: $run_status"

  local page_node_key
  page_node_key="$(db_scalar "select page_node_key from site.cms_publish_events where event_type='seo_page_review_requested' and event_payload ->> 'run_id' = '$rid' order by occurred_at desc limit 1;" | tr -d '[:space:]')"
  [ -n "$page_node_key" ] || die "page_node_key for SEO review event not found"

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
  log "seo canonical-cutover empty-support workflow run_id=$rid"
  (cd app/rust && cargo run -q -p temporal_worker --bin temporal_starter -- start \
      --workflow seo-site-build-canonical-cutover \
      --workflow-id "$rid" \
      --database-url "$DB_DSN" \
      --context-key "es:student:by" \
      --market "alegria-site" \
      --locale "ru-RU" \
      --country-code "ES" \
      --visa-type "student" \
      --applicant-profile "standard" \
      --query "виза туристическая испания документы") >/dev/null

  local wait_result=0
  wait_wf_closed "$rid" 300 || wait_result=$?
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
  PRODUCTION_GATE_CLI_TARGET_DIR="${PRODUCTION_GATE_CLI_TARGET_DIR:-/tmp/temporal-production-gate-cli-target}"
  export RETRIEVAL_CONTRACT_GATE_CLI_TARGET_DIR="${RETRIEVAL_CONTRACT_GATE_CLI_TARGET_DIR:-$PRODUCTION_GATE_CLI_TARGET_DIR}"
  export GRAPH_CONTRACT_GATE_CLI_TARGET_DIR="${GRAPH_CONTRACT_GATE_CLI_TARGET_DIR:-$PRODUCTION_GATE_CLI_TARGET_DIR}"
  trap 'if [ -n "${WORKER_PID:-}" ]; then kill "${WORKER_PID}" >/dev/null 2>&1 || true; fi' EXIT
  start_services
  assert_invariants
  run_retrieval_contract_gate
  run_graph_contract_gate
  run_ping_and_demo
  run_seo_empty_support_failure
  run_seo_site_build_workflow
  run_freshness_workflow
  assert_metrics
  run_live_provider_gate
  log "PRODUCTION_GATE: PASS"
}

main "$@"
