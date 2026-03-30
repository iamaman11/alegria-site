#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "== Alegria Expert Verification =="

echo "[1/35] Python compile check"
python3 -m compileall -q automation infra/analytics_lab/src

echo "[2/35] Python scope check (no forbidden imports)"
python3 automation/check_python_scope.py

echo "[3/35] Docs layout check (flat docs/, no docs/knowledge drift)"
python3 automation/check_docs_layout.py

echo "[4/35] R5-R8 boundary check (no Python hot-path violations)"
bash automation/check_python_rust_boundary.sh

echo "[5/35] Rust adapter boundary check (strict layering)"
python3 automation/check_rust_adapter_boundaries.py

echo "[6/35] Rust layer isolation check"
python3 automation/check_rust_layer_isolation.py

echo "[7/35] Primitives purity"
python3 automation/check_primitives_purity.py

echo "[8/35] Temporal runtime purity"
python3 automation/check_temporal_runtime_purity.py

echo "[9/35] Layer dependency matrix"
python3 automation/check_layer_dependency_matrix.py

echo "[10/35] Runtime type families"
python3 automation/check_runtime_type_families.py

echo "[11/35] Use-cases boundary policy"
python3 automation/check_use_cases_boundary.py

echo "[12/35] Use-cases no-SQL"
python3 automation/check_use_cases_no_sql.py

echo "[13/35] Use-cases no-JSON-boundary"
python3 automation/check_use_cases_no_json_boundary.py

echo "[14/35] Use-cases external SDK ban"
python3 automation/check_use_cases_external_sdk_ban.py

echo "[15/35] Allowed boundary points"
python3 automation/check_allowed_boundary_points.py

echo "[16/35] JSON boundary policy (deny-by-default)"
python3 automation/check_json_boundary_policy.py

echo "[17/35] Services/temporal boundary"
python3 automation/check_services_temporal_boundary.py

echo "[18/35] Proto contract verify"
bash automation/contract_verify.sh

echo "[19/35] Reliability contracts check"
python3 automation/check_reliability_contracts.py

echo "[20/35] Domain error usage policy"
python3 automation/check_domain_error_usage.py

echo "[21/35] Step execution contract"
python3 automation/check_step_execution_contract.py

echo "[22/35] Step contract completeness"
python3 automation/check_step_contract_completeness.py

echo "[23/35] Temporal build-id policy"
python3 automation/check_temporal_build_id_policy.py

echo "[24/35] Metrics contract"
python3 automation/check_metrics_contract.py

echo "[25/35] Reconcile fail-safe contract"
python3 automation/check_reconcile_failsafe.py

echo "[26/35] Fail-safe smoke: broken schema -> DLQ"
python3 automation/smoke_broken_schema_to_dlq.py

echo "[27/35] Fail-safe smoke: exhausted retry -> DLQ"
python3 automation/smoke_exhausted_retry_to_dlq.py

echo "[28/35] Fail-safe smoke: stale outbox reclaim"
python3 automation/smoke_stale_outbox_reclaim.py

echo "[29/35] Fail-safe smoke: pending HITL not failure"
python3 automation/smoke_pending_hitl_not_failure.py

echo "[30/35] Backup/restore layout"
python3 automation/check_backup_restore_layout.py

echo "[31/35] SQLx offline contract"
python3 automation/check_sqlx_offline_contract.py

echo "[32/35] End-to-end invariant checks (schema + JSONL data)"
python3 automation/check_end_to_end_invariants.py

echo "[33/35] Consistency audit report"
python3 automation/check_expert_consistency.py \
  --run-boundary \
  --run-rust-adapters \
  --run-invariants \
  --run-json-boundary \
  --report-json automation/reports/consistency_report.json

echo "[34/35] Rust workspace check"
(cd app/rust && SQLX_OFFLINE=true cargo check -q)

echo "[35/35] Rust migration contract + fact verifier parity"
SQLX_OFFLINE=true cargo run -q --manifest-path app/rust/services/cli_tools/Cargo.toml -- \
  check-rust-migration-contract \
  --root . \
  --report-json automation/reports/rust_migration_contract.rust.json

SQLX_OFFLINE=true cargo run -q --manifest-path app/rust/services/cli_tools/Cargo.toml -- \
  check-fact-verifier-parity \
  --root . \
  --report-json automation/reports/fact_verifier_parity.rust.json

echo "All checks passed."
