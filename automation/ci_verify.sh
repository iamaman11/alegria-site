#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

if [[ -x "$ROOT_DIR/.venv/bin/python3" ]]; then
  export PATH="$ROOT_DIR/.venv/bin:$PATH"
fi

echo "== Alegria Expert Verification =="

echo "[1/50] Python compile check"
python3 -m compileall -q automation infra/analytics_lab/src

echo "[2/50] Python lint check"
python3 -m ruff check automation infra/analytics_lab/src

echo "[3/50] Python scope check (no forbidden imports)"
python3 automation/check_python_scope.py

echo "[4/50] Docs layout check (flat docs/, no docs/knowledge drift)"
python3 automation/check_docs_layout.py

echo "[5/50] R5-R8 boundary check (no Python hot-path violations)"
bash automation/check_python_rust_boundary.sh

echo "[6/50] Rust adapter boundary check (strict layering)"
python3 automation/check_rust_adapter_boundaries.py

echo "[7/50] Rust layer isolation check"
python3 automation/check_rust_layer_isolation.py

echo "[8/50] Primitives purity"
python3 automation/check_primitives_purity.py

echo "[9/50] Temporal runtime purity"
python3 automation/check_temporal_runtime_purity.py

echo "[10/50] Layer dependency matrix"
python3 automation/check_layer_dependency_matrix.py

echo "[11/50] Runtime type families"
python3 automation/check_runtime_type_families.py

echo "[12/50] Use-cases boundary policy"
python3 automation/check_use_cases_boundary.py

echo "[13/50] Use-cases no-SQL"
python3 automation/check_use_cases_no_sql.py

echo "[14/50] Use-cases no-JSON-boundary"
python3 automation/check_use_cases_no_json_boundary.py

echo "[15/50] Use-cases external SDK ban"
python3 automation/check_use_cases_external_sdk_ban.py

echo "[16/50] Allowed boundary points"
python3 automation/check_allowed_boundary_points.py

echo "[17/50] JSON boundary policy (deny-by-default)"
python3 automation/check_json_boundary_policy.py

echo "[18/50] Services/temporal boundary"
python3 automation/check_services_temporal_boundary.py

echo "[19/50] Proto contract verify"
bash automation/contract_verify.sh

echo "[20/50] Reliability contracts check"
python3 automation/check_reliability_contracts.py

echo "[21/50] Domain error usage policy"
python3 automation/check_domain_error_usage.py

echo "[22/50] Step execution contract"
python3 automation/check_step_execution_contract.py

echo "[23/50] Step contract completeness"
python3 automation/check_step_contract_completeness.py

echo "[24/50] Step catalog contract"
python3 automation/check_step_catalog_contract.py

echo "[25/50] Status model parity"
python3 automation/check_status_model_parity.py

echo "[26/50] SEO schema contract"
python3 automation/check_seo_schema_contract.py

echo "[27/50] SEO proto contract"
python3 automation/check_seo_proto_contract.py

echo "[28/50] SEO runtime registration"
python3 automation/check_seo_runtime_registration.py

echo "[29/50] SEO persistence contract"
python3 automation/check_seo_persistence_contract.py

echo "[30/50] SEO projection contract"
python3 automation/check_seo_projection_contract.py

echo "[31/50] SEO CMS contract"
python3 automation/check_seo_cms_contract.py

echo "[32/50] Static site builder contract"
python3 automation/check_static_site_builder_contract.py

echo "[33/50] SEO no-seed production workflow"
python3 automation/check_seo_no_seed_workflow.py

echo "[34/50] SEO publish gates"
python3 automation/check_seo_publish_gates.py

echo "[35/50] SEO traceability contract"
python3 automation/check_seo_traceability_contract.py

echo "[36/53] SEO no LLM SDK in use_cases"
python3 automation/check_no_llm_sdk_in_use_cases.py

echo "[37/53] SEO claim ledger contract"
python3 automation/check_seo_claim_ledger_contract.py

echo "[38/53] Headless CMS publish flow smoke"
python3 automation/smoke_headless_cms_publish_flow.py

echo "[39/53] Expert draft LLM fixture smoke"
python3 automation/smoke_expert_draft_llm_fixture.py

echo "[40/53] SEO site-build fixture smoke"
python3 automation/smoke_seo_site_build_fixture.py

echo "[40a/53] Support bundle required smoke"
python3 automation/smoke_support_bundle_required.py

echo "[40b/53] Incremental publish path smoke"
python3 automation/smoke_incremental_publish_path.py

echo "[40c/53] Dependency-aware rebuild smoke"
python3 automation/smoke_dependency_aware_rebuild.py

echo "[40d/53] Editorial provider routing contract"
python3 automation/check_editorial_provider_routing.py

echo "[40e/53] SEO migration parity"
python3 automation/check_seo_migration_parity.py

echo "[40f/53] Launch gate no legacy content"
python3 automation/check_launch_gate_no_legacy_content.py

echo "[41/53] Temporal build-id policy"
python3 automation/check_temporal_build_id_policy.py

echo "[42/53] Metrics contract"
python3 automation/check_metrics_contract.py

echo "[43/53] Reconcile fail-safe contract"
python3 automation/check_reconcile_failsafe.py

echo "[44/53] Fail-safe smoke: broken schema -> DLQ"
python3 automation/smoke_broken_schema_to_dlq.py

echo "[45/53] Fail-safe smoke: exhausted retry -> DLQ"
python3 automation/smoke_exhausted_retry_to_dlq.py

echo "[46/53] Fail-safe smoke: stale outbox reclaim"
python3 automation/smoke_stale_outbox_reclaim.py

echo "[47/53] Fail-safe smoke: pending HITL not failure"
python3 automation/smoke_pending_hitl_not_failure.py

echo "[48/53] Backup/restore layout"
python3 automation/check_backup_restore_layout.py

echo "[49/53] SQLx offline contract"
python3 automation/check_sqlx_offline_contract.py

echo "[50/53] End-to-end invariant checks (schema + JSONL data)"
python3 automation/check_end_to_end_invariants.py

echo "[51/53] Consistency audit report"
python3 automation/check_expert_consistency.py \
  --run-boundary \
  --run-rust-adapters \
  --run-invariants \
  --run-json-boundary \
  --report-json automation/reports/consistency_report.json

echo "[52/53] SEO use-case acceptance test"
(cd app/rust && SQLX_OFFLINE=true cargo test -q -p use_cases seo_steps_form_publish_ready_pipeline)

echo "[53/53] Rust workspace check"
(cd app/rust && SQLX_OFFLINE=true cargo check -q)

echo "[post] Rust migration contract + fact verifier parity"
SQLX_OFFLINE=true cargo run -q --manifest-path app/rust/services/cli_tools/Cargo.toml -- \
  check-rust-migration-contract \
  --root . \
  --report-json automation/reports/rust_migration_contract.rust.json

SQLX_OFFLINE=true cargo run -q --manifest-path app/rust/services/cli_tools/Cargo.toml -- \
  check-fact-verifier-parity \
  --root . \
  --report-json automation/reports/fact_verifier_parity.rust.json

echo "All checks passed."
