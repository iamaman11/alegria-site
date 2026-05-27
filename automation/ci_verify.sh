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

echo "[4a/50] Support process registry"
python3 automation/check_support_process_registry.py

echo "[4b/50] Support surface execution"
python3 automation/check_support_surface_execution.py

echo "[4c/50] Release/restore gate surface"
python3 automation/check_release_restore_gate_surface.py

echo "[4d/50] Voyage retrieval surface"
python3 automation/check_voyage_retrieval_surface.py

echo "[4d1/50] Voyage evaluation bundle"
python3 automation/check_voyage_evaluation_bundle.py

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

echo "[12/50] SEO steps boundary policy"
python3 automation/check_seo_steps_boundary.py

echo "[13/50] SEO steps no-SQL"
python3 automation/check_seo_steps_no_sql.py

echo "[14/50] SEO steps no-JSON-boundary"
python3 automation/check_seo_steps_no_json_boundary.py

echo "[15/50] SEO steps external SDK ban"
python3 automation/check_seo_steps_external_sdk_ban.py

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

echo "[35a/53] SEO identity docs contract"
python3 automation/check_seo_identity_docs.py

echo "[35b/53] SEO canonical core ownership"
python3 automation/check_seo_canonical_core.py

echo "[35c/54] SEO layering"
python3 automation/check_seo_layering.py

echo "[35c2/54] SEO crate graph"
python3 automation/check_seo_crate_graph.py

echo "[35d/55] seo_steps infra surface"
python3 automation/check_seo_steps_infra_surface.py

echo "[35e/56] Temporal SEO planning cluster boundary"
python3 automation/check_temporal_seo_activity_surface.py

echo "[35e1/56] Temporal workflow execution plan"
python3 automation/check_temporal_workflow_execution_plan.py

echo "[35f/57] Temporal step catalog SEO surface"
python3 automation/check_temporal_step_catalog_surface.py

echo "[35f1/57] Temporal workflow catalog support surfaces"
python3 automation/check_temporal_workflow_catalog_support.py

echo "[35g/58] CLI tools SEO mutation surface"
python3 automation/check_cli_tools_seo_surface.py

echo "[35g1/58] Cutover shadow verification surface"
python3 automation/check_shadow_verification_surface.py

echo "[35g2/58] Cutover convergence policy"
python3 automation/check_cutover_convergence_policy.py

echo "[35g3/58] Truth certification regression gate"
bash automation/run_truth_certification_gate.sh

echo "[35g4/58] Truth governance policy"
python3 automation/check_truth_governance_policy.py

echo "[35h/59] SEO architecture wording"
python3 automation/check_seo_architecture_wording.py

echo "[36/56] SEO no LLM SDK in seo_steps"
python3 automation/check_no_llm_sdk_in_seo_steps.py

echo "[37/53] SEO claim ledger contract"
python3 automation/check_seo_claim_ledger_contract.py

echo "[37a/53] Extraction runtime contract"
python3 automation/check_extraction_runtime_contract.py

echo "[37b/53] Verified storage invariants"
python3 automation/check_verified_storage_invariants.py

echo "[37c/53] Truth admissibility gate"
python3 automation/check_truth_admissibility_gate.py

echo "[37d/53] Extracted rule candidates schema"
python3 automation/check_extracted_rule_candidates_schema.py

echo "[37e/53] Truth extraction provider contract"
python3 automation/check_truth_extraction_provider_contract.py

echo "[37f/53] LLM extraction contract"
python3 automation/check_llm_extraction_contract.py

echo "[37g/53] Truth validators contract"
python3 automation/check_truth_validators_contract.py

echo "[37h/53] Truth adjudication contract"
python3 automation/check_truth_adjudication_contract.py

echo "[37e/53] SEO provenance invariants"
python3 automation/check_seo_provenance_invariants.py

echo "[38/53] Extraction coverage"
python3 automation/check_extraction_coverage.py

echo "[38a/53] Expert core activation contract"
python3 automation/check_expert_core_activation_contract.py

echo "[39/53] Extraction coverage smoke"
python3 automation/smoke_extraction_coverage.py

echo "[40/53] Contradiction handling"
python3 automation/check_contradiction_handling.py

echo "[41/53] Contradiction handling smoke"
python3 automation/smoke_contradiction_handling.py

echo "[42/53] PII redaction pre-LLM"
python3 automation/check_pii_redaction_pre_llm.py

echo "[43/53] PII redaction pre-LLM smoke"
python3 automation/smoke_pii_redaction_pre_llm.py

echo "[44/53] Licensing gate"
python3 automation/check_licensing_gate.py

echo "[45/53] Licensing gate smoke"
python3 automation/smoke_licensing_gate.py

echo "[46/53] Deterministic fallback publish blocking"
python3 automation/check_deterministic_fallback_publish_blocking.py

echo "[47/53] Deterministic fallback publish blocking smoke"
python3 automation/smoke_deterministic_fallback_publish_blocking.py

echo "[48/53] HITL operator surface"
python3 automation/check_hitl_operator_surface.py

echo "[49/53] HITL operator surface smoke"
python3 automation/smoke_hitl_operator_surface.py

echo "[50/53] Headless CMS publish flow smoke"
python3 automation/smoke_headless_cms_publish_flow.py

echo "[51/53] Expert draft LLM fixture smoke"
python3 automation/smoke_expert_draft_llm_fixture.py

echo "[52/53] SEO site-build fixture smoke"
python3 automation/smoke_seo_site_build_fixture.py

echo "[52a/53] Support bundle required smoke"
python3 automation/smoke_support_bundle_required.py

echo "[52b/53] Incremental publish path smoke"
python3 automation/smoke_incremental_publish_path.py

echo "[52c/53] Rebuild dependency graph"
python3 automation/check_rebuild_dependency_graph.py

echo "[52d/53] Rebuild dependency graph smoke"
python3 automation/smoke_rebuild_dependency_graph.py

echo "[52e/53] Rebuild queue and execution path"
python3 automation/check_rebuild_queue_execution_path.py

echo "[52f/53] Rebuild queue and execution path smoke"
python3 automation/smoke_rebuild_queue_execution_path.py

echo "[52g/53] Global navigation policy"
python3 automation/check_global_navigation_policy.py

echo "[52h/53] Global navigation policy smoke"
python3 automation/smoke_global_navigation_policy.py

echo "[52i/53] Dependency-aware rebuild smoke"
python3 automation/smoke_dependency_aware_rebuild.py

echo "[52j/53] Editorial provider routing contract"
python3 automation/check_editorial_provider_routing.py

echo "[52k/53] SEO migration parity"
python3 automation/check_seo_migration_parity.py

echo "[52k1/53] Execution run blob contract"
python3 automation/check_execution_run_blob_contract.py

echo "[52l/53] Launch gate no legacy content"
python3 automation/check_launch_gate_no_legacy_content.py

echo "[52m/53] Legacy workflow quarantine"
python3 automation/check_legacy_workflow_quarantine.py

echo "[52n/53] Legacy replay evidence schema"
python3 automation/check_seo_legacy_replay_evidence_schema.py

echo "[52o/53] Legacy replay inventory contract"
python3 automation/check_seo_legacy_replay_inventory_contract.py

echo "[52p/53] Legacy replay environment probe"
python3 automation/check_seo_legacy_replay_environment_probe.py

echo "[52q/53] Local operational evidence schema"
python3 automation/check_local_operational_evidence_schema.py

echo "[41/53] Temporal build-id policy"
python3 automation/check_temporal_build_id_policy.py

echo "[42a/53] Quality policy registry and scoring"
python3 automation/check_quality_policy_registry.py

echo "[42b/53] Quality policy registry and scoring smoke"
python3 automation/smoke_quality_policy_registry.py

echo "[42c/53] Run report completeness"
python3 automation/check_run_report_completeness.py

echo "[42d/53] Run report completeness smoke"
python3 automation/smoke_run_report_completeness.py

echo "[42e/53] Metrics contract"
python3 automation/check_metrics_contract.py

echo "[43/57] Run modes contract"
python3 automation/check_run_modes_contract.py

echo "[44/57] SEO site-build replay contract"
python3 automation/check_seo_site_build_replay_contract.py

echo "[45/57] SEO rollout/compat contract"
python3 automation/check_seo_rollout_compat_contract.py

echo "[46/58] Projection barrier run-scope contract"
python3 automation/check_projection_barrier_run_scoped.py

echo "[47/59] Run-id event stamping contract"
python3 automation/check_run_id_event_stamping.py

echo "[48/60] Reconcile fail-safe contract"
python3 automation/check_reconcile_failsafe.py

echo "[49/60] Fail-safe smoke: broken schema -> DLQ"
python3 automation/smoke_broken_schema_to_dlq.py

echo "[50/60] Fail-safe smoke: exhausted retry -> DLQ"
python3 automation/smoke_exhausted_retry_to_dlq.py

echo "[51/60] Fail-safe smoke: stale outbox reclaim"
python3 automation/smoke_stale_outbox_reclaim.py

echo "[52/60] Fail-safe smoke: pending HITL not failure"
python3 automation/smoke_pending_hitl_not_failure.py

echo "[53/60] Backup/restore layout"
python3 automation/check_backup_restore_layout.py

echo "[54/60] SQLx offline contract"
python3 automation/check_sqlx_offline_contract.py

echo "[55/60] End-to-end invariant checks (schema + JSONL data)"
python3 automation/check_end_to_end_invariants.py

echo "[56/60] Consistency audit report"
python3 automation/check_expert_consistency.py \
  --run-boundary \
  --run-rust-adapters \
  --run-invariants \
  --run-json-boundary \
  --report-json automation/reports/consistency_report.json

echo "[57/60] Integration harness presence"
python3 automation/check_integration_harness_present.py

echo "[58/60] SEO use-case acceptance test"
(cd app/rust && SQLX_OFFLINE=true cargo test -q -p seo_steps seo_steps_form_publish_ready_pipeline)

echo "[59/60] Integration harness stub test"
(cd app/rust && cargo test -q -p integration_harness)

echo "[60/60] Integration harness e2e surface compile"
(cd app/rust && cargo test -q -p integration_harness --features e2e --no-run)

echo "[60a/60] Live-provider smoke evidence schema"
python3 automation/check_live_provider_smoke_evidence_schema.py

echo "[60b/60] Crawler invariants contract"
python3 automation/check_crawler_invariants.py

echo "[60c/60] Crawler robots-policy smoke"
python3 automation/smoke_crawler_robots_policy.py

echo "[60d/60] Crawl provenance contract"
python3 automation/check_crawl_provenance_contract.py

echo "[60e/60] Crawl dedup contract"
python3 automation/check_crawl_dedup_contract.py

echo "[60f/60] SERP intelligence contract"
python3 automation/check_serp_intelligence_contract.py

echo "[60g/60] SERP top10 pattern smoke"
python3 automation/smoke_serp_top10_pattern_pipeline.py

echo "[60h/60] Whole-page semantic fixture gate"
bash automation/run_whole_page_semantic_gate.sh

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
