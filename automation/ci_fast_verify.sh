#!/usr/bin/env bash
# Cheap PR acceptance: lightweight Rust model tests run in architecture-and-rust.
# Long-running Rust/SQL and real Temporal/Truth checks run in two independent
# mandatory jobs of the same .github/workflows/ci.yml (ci_verify.sh phases).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

case "${1:-}" in
  contracts)
    python3 -m compileall -q automation infra/analytics_lab/src
    python3 -m ruff check --select E4,E7,E9,F automation infra/analytics_lab/src
    python3 automation/check_ci_workflow_contract.py
    python3 automation/check_ci_cargo_cache.py
    python3 automation/check_python_scope.py
    python3 automation/check_compose_loopback_ports.py
    python3 automation/check_rust_adapter_boundaries.py
    python3 automation/check_rust_layer_isolation.py
    python3 automation/check_module_boundaries.py
    python3 automation/check_seo_crate_graph.py
    python3 automation/check_temporal_runtime_purity.py
    python3 automation/check_seo_publish_gates.py
    python3 automation/check_static_site_builder_contract.py
    python3 automation/gen_proto_contracts.py --out automation/PROTO_CONTRACTS.md --check
    ;;
  product)
    python3 automation/smoke_truth_certification_strict_projection_policy.py
    python3 automation/check_truth_governance_policy.py
    python3 automation/check_truth_admissibility_gate.py
    python3 automation/check_truth_validators_contract.py
    python3 automation/check_truth_adjudication_contract.py
    python3 automation/check_extraction_coverage.py
    python3 automation/check_contradiction_handling.py
    python3 automation/check_pii_redaction_pre_llm.py
    python3 automation/check_licensing_gate.py
    python3 automation/check_deterministic_fallback_publish_blocking.py
    python3 automation/check_rebuild_dependency_graph.py
    python3 automation/check_rebuild_queue_execution_path.py
    python3 automation/smoke_e2e_first_site.py
    python3 automation/smoke_first_site_doctor.py
    python3 automation/smoke_first_site_release.py
    ;;
  integration)
    python3 automation/check_temporal_build_id_policy.py
    python3 automation/check_quality_policy_registry.py
    python3 automation/check_run_report_completeness.py
    python3 automation/check_reconcile_failsafe.py
    python3 automation/check_sqlx_offline_contract.py
    python3 automation/check_end_to_end_invariants.py
    python3 automation/check_expert_consistency.py --run-boundary --run-invariants --run-json-boundary --report-json automation/reports/consistency_report.json
    python3 automation/check_integration_harness_present.py
    python3 automation/check_graph_contract_gate.py docs/runs/graph_contract_gate_evidence.json
    ;;
  *)
    echo "Usage: bash automation/ci_fast_verify.sh {contracts|product|integration}" >&2
    exit 2
    ;;
esac

echo "FAST_AUDIT_PHASE_PASS=$1"
