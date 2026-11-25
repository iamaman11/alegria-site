#!/usr/bin/env python3
"""Deterministic, fail-closed Rust and PR-owner impact for the canonical workspace.

The selected job owner set is checked by an exact-SHA mandatory CI gate;
unmapped executable inputs conservatively retain all established owners.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import tomllib
from collections import deque
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
WORKSPACE = ROOT / "app" / "rust"
SCHEMA_VERSION = 5

# Includes changes to generated contracts, integration fixtures, build tooling,
# container contents and runtime configuration; they are NOT Rust crate-local.
GLOBAL_PREFIXES = (
    ".github/",
    "automation/",
    "app/db/",
    # Shared live PG/Qdrant test setup feeds *both* E2E owner packages.
    # A fixture change cannot be assigned only to the new A14 package.
    "app/rust/crates/integration_data_fixtures/",
    "contracts/",
    "analytics_lab/",
    "infra/",
)
GLOBAL_FILES = {
    "docker-compose.yml",
    "app/rust/Cargo.toml",
    "app/rust/rust-toolchain.toml",
    "rust-toolchain.toml",
    # The concrete typed PG adapter root changes multiple application ports.
    # Keep such edits full-source until per-trait impact can be proven.
    "app/rust/crates/pg_runtime_ports/src/lib.rs",
}

# Only contracts with a verified generator/consumer are scoped. Other schemas
# and wire inputs retain the existing fail-closed full-workspace behavior.
KNOWN_CONTRACT_CRATES = {
    "app/contracts/proto/read_api.proto": "contracts",
    "app/contracts/proto/rules.proto": "contracts",
    "app/contracts/proto/condition.proto": "contracts",
    "app/contracts/proto/sync.proto": "contracts",
    "app/contracts/proto/temporal_payloads.proto": "contracts",
}
KNOWN_CONTRACT_MANIFEST = "automation/PROTO_CONTRACTS.md"

# These files do not change a Rust compilation unit. Historical run evidence
# can still be checked by the ordinary source-policy CI owners, but must not be
# treated as a live Truth fixture or proof that Rust behavior passed.
DOCUMENTATION_PREFIXES = ("docs/",)
DOCUMENTATION_FILES = {"README.md", "CONTRIBUTING.md"}
# These Markdown files are consumed as policy inputs by source acceptance
# scripts (including opt-in deep checks); they are not merely explanatory.
SOURCE_POLICY_DOCUMENTS = {
    "docs/INDEX.md",
    "docs/OPS_RUNTIME_RUNBOOK.md",
    "docs/OPS_TEMPORAL_PRODUCTION_GATE.md",
    "docs/OPS_TEMPORAL_BUILD_MODES.md",
    "docs/SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md",
    "docs/V6_Support_Process_Registry.md",
    "docs/V6_Expert_Truth_Graph_Runtime.md",
    "docs/V6_SeoSiteBuildWorkflow_Working_Plan.md",
}

# These existing Markdown files document operator/architecture procedures.
# They are not Rust source, CI drivers or Worker delivery inputs. The fast
# `verify` owner validates their source-policy wording; the generated
# PROTO_CONTRACTS.md is deliberately excluded because it is a live contract
# inventory and retains its separate fail-closed generator rules below.
# Unknown automation files, including newly added Markdown, remain global.
AUTOMATION_SOURCE_DOCUMENTS = frozenset({
    "automation/00_ENTRYPOINT.md",
    "automation/FSM_TYPED_PHASE_IMPERATIVE_PLAN.md",
    "automation/INTEGRITY_RUNBOOK.md",
    "automation/LAYER_MAP.md",
    "automation/NEW_OPERATION_PLAYBOOK.md",
    "automation/OPS_CANONICALS.md",
    "automation/PROJECT_NAVIGATION_MAP.md",
    "automation/README.md",
})

# Non-Rust inputs with a known downstream image. Do not make every release
# script change recompile the entire Cargo workspace.
WORKER_PACKAGING_FILES = {
    "automation/deploy_seo_static_cloudflare.py",
    "automation/check_public_static_output.py",
    "automation/check_public_http_readback.py",
    "automation/check_first_site_release.py",
}

# This explanatory file is not copied into the Worker build context or used
# by any release command. Keep the list shared with the context materializer;
# every other static asset still has its real image/CI owner.
WORKER_NONDELIVERY_FILES = frozenset({
    "infra/cloudflare/seo-static/README.md",
})

# Explicitly verified CI-only inputs do not modify a Cargo compilation unit,
# runtime image or live E2E fixture. Their source-level contract is executed by
# the mandatory `verify` owner. Keep ci.yml, the impact planner itself, unknown
# workflows and arbitrary automation scripts fail-closed: they can change what
# the required gate actually executes.
CI_ONLY_VERIFY_FILES = {
    "automation/check_ci_workflow_contract.py",
    "automation/check_expert_core_activation_contract.py",
    "automation/report_sccache_failures.py",
}

# Release packaging has a narrower input graph than PR verification. These
# reviewed maintenance files do not enter the shipped Worker Docker context;
# they can still trigger fail-closed FULL PR CI if they alter selection logic.
# Unknown or mixed inputs must retain the impact planner's image decision.
NON_WORKER_DELIVERY_INPUTS = frozenset({
    ".github/workflows/ci.yml",
    "automation/check_ci_workflow_contract.py",
    "automation/check_worker_dependency_skeleton.py",
    "automation/check_expert_core_activation_contract.py",
    "automation/check_ci_impact.py",
    "automation/check_ci_merge_provenance.py",
    "automation/check_ci_cargo_cache.py",
    "automation/check_ci_rust_owned_check.py",
    "automation/check_cargo_feedback.py",
    "automation/dev_rust_feedback.py",
    "automation/cargo_feedback.py",
    "automation/run_scoped_cargo_feedback.py",
    "automation/rust_ci_artifacts.py",
    "automation/check_rust_ci_artifacts.py",
    "automation/run_main_cache_prime.py",
    "automation/report_sccache_failures.py",
    "automation/report_ci_cargo_cache.py",
    "automation/ci_impact.py",
    "automation/ci_fast_verify.sh",
    "automation/run_rust_owned_check.sh",
    "automation/check_windows_staging_rollback.py",
    "infra/local/windows_staging.ps1",
    "docs/ci-fast-path.md",
    "docs/ARCHITECTURE_BOUNDARIES.md",
}) | WORKER_NONDELIVERY_FILES | AUTOMATION_SOURCE_DOCUMENTS

TRUTH_FIXTURE_PREFIX = "app/rust/crates/integration_harness/fixtures/visa_truth_certification/"
TRUTH_FIXTURE_ID = re.compile(r"[a-z][a-z0-9_]*\Z")

# Publishing A17 exercises the real PostgreSQL transaction and a replaceable
# controlled HTTP delivery adapter. Reuse this same impact report in local/CI;
# unrelated leaf-crate edits must not pay for a Docker-backed publication test.
PUBLISHING_A17_PATHS = {
    "app/rust/crates/seo_domain/src/publishing.rs",
    # Publication decisions now belong to the domain, not Temporal step
    # modules. Keep the actual license/materialization consumers in A17.
    "app/rust/crates/seo_domain/src/publishing/license.rs",
    "app/rust/crates/seo_domain/src/publishing/materialization.rs",
    "app/rust/crates/seo_domain/src/render_preview.rs",
    "app/rust/crates/seo_steps/src/finalize_publish_step.rs",
    "app/rust/crates/seo_steps/src/licensing_gate_step.rs",
    "app/rust/crates/seo_steps/src/publish_materialize_step.rs",
    "app/rust/crates/seo_steps/src/render_preview_validate_step.rs",
    "app/rust/crates/seo_application/src/review_publish.rs",
    "app/rust/crates/seo_ports/src/lib.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/cms_publish.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/finalize_publish.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/publication_release.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/renderer_refresh.rs",
    "app/rust/crates/integration_harness/src/tests/publication_delivery_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/publication_delivery_tests.rs",
}

# A14 exercises one real PostgreSQL lease/outbox and Qdrant materialization.
# The same decision is consumed by local feedback and the canonical CI runner.
OUTBOX_A14_PATHS = {
    "app/rust/crates/infrastructure/src/adapters/verified_rule_projection_adapter.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/projection_runtime_store.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_hitl_adapter.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_step_ledger_adapter.rs",
    "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter/source_registry.rs",
    "app/rust/services/temporal/src/activities/operations/truth_write_and_fixture_tests.rs",
    "app/rust/services/outbox_worker/src/worker.rs",
    "app/rust/services/outbox_worker/src/materialize.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_outbox_adapter.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_runtime_outbox_adapter.rs",
    "app/rust/crates/projection_delivery/src/projection_materialize_adapter.rs",
    "app/rust/crates/projection_delivery/src/neo4j_materialization_adapter.rs",
    "app/rust/crates/voyage_api/src/lib.rs",
    "app/rust/crates/postgres_store/src/sqlx_projection_rows.rs",
    "app/rust/services/temporal/src/activities/operations/runtime_support_ops.rs",
    "app/rust/crates/integration_outbox_e2e/src/lib.rs",
    "app/rust/crates/integration_outbox_e2e/src/tests/outbox_delivery_tests.rs",
    "app/rust/crates/integration_outbox_e2e/src/tests/pg_migration_upgrade_tests.rs",
}

# Truth adjudication is a real PostgreSQL/evidence operation distinct from
# publication, outbox delivery and full Temporal restart. Its targeted E2E
# must be selected by the same impact model, including for test-only changes.
TRUTH_ADJUDICATION_PATHS = {
    "app/rust/crates/truth_domain/src/contradictions.rs",
    "app/rust/crates/truth_domain/src/truth_governance.rs",
    "app/rust/crates/truth_domain/src/review_decisions.rs",
    "app/rust/services/temporal/src/activities/operations/truth_extraction_ops/contradiction_adjudication.rs",
    "app/rust/services/temporal/src/activities/runtime.rs",
    "app/rust/crates/integration_harness/src/tests/truth_runtime_tests.rs",
    "app/rust/crates/integration_harness/src/tests/synthetic_runtime_helpers.rs",
    "app/rust/crates/integration_pg_app_e2e/src/lib.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/truth_adjudication_tests.rs",
}

# An actual PostgreSQL revision-bound human review exercises the domain
# decision, typed port, SQL transaction and active-workflow ownership. It is
# distinct from the public release/readback A17 integration.
CMS_REVIEW_PATHS = {
    "app/rust/crates/seo_domain/src/cms_revision.rs",
    "app/rust/crates/seo_domain/src/review_decision.rs",
    "app/rust/crates/seo_application/src/cms_review.rs",
    "app/rust/crates/seo_application/src/review_publish.rs",
    "app/rust/crates/seo_application/src/scenario/page_phase.rs",
    "app/rust/crates/seo_steps/src/cms_review_wire.rs",
    "app/rust/crates/seo_steps/src/cms_publish_step.rs",
    "app/rust/crates/seo_ports/src/lib.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/human_review_decision.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/types_review_helpers.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/cms_publish.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_static_site_adapter.rs",
    "app/rust/crates/infrastructure/src/adapters/static_site_builder_adapter.rs",
    "app/rust/crates/pg_runtime_ports/src/draft_publish_ports.rs",
    "app/rust/services/temporal/src/activities/registry/mod_registry_impl.rs",
    "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover.rs",
    "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover/page_publish.rs",
    "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover/page_publish.rs",
    "app/rust/services/cli_tools/src/cli/cms_decision.rs",
    "app/rust/crates/integration_harness/src/tests/manual_llm_temporal_tests.rs",
    "app/rust/crates/integration_harness/src/tests/provider_runtime_surface_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/lib.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/cms_review_pg_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/cms_publish_pg_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/cms_cross_scope_pg_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/cms_navigation_pg_tests.rs",
}

# The full-scenario and fault-injected PostgreSQL tests exercise both
# successful Draft QA persistence and atomic rollback/reject-missing-draft.
DRAFT_QA_SCENARIO_PATHS = {
    "app/rust/crates/seo_steps/src/editorial_wire.rs",
    "app/rust/crates/seo_steps/src/page_wire.rs",
    "app/rust/crates/seo_domain/src/editorial.rs",
    "app/rust/crates/seo_domain/src/page_readiness.rs",
    "app/rust/crates/seo_domain/src/content_contract.rs",
    "app/rust/crates/seo_domain/src/draft_quality.rs",
    "app/rust/crates/seo_application/src/drafting.rs",
    "app/rust/crates/seo_steps/src/draft_qa_step.rs",
    "app/rust/crates/seo_steps/src/content_contract_validate_step.rs",
    "app/rust/crates/infrastructure/src/adapters/editorial_llm_adapter/generated_overlay.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/draft_publish_rebuild_persistence.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/shared_helpers.rs",
    "app/rust/crates/integration_harness/src/tests/scenario_parity_tests.rs",
    "app/rust/crates/integration_harness/src/tests/qa_persistence_tests.rs",
    "app/rust/crates/integration_temporal_e2e/src/tests/scenario_parity_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/qa_persistence_tests.rs",
}

# Real PostgreSQL rollback checks for planning/link-recommendation writes.
# Keep them in the existing architecture-and-rust job; the integration harness
# owns the test, while this registry only selects its exact Cargo filter.
PLANNING_ATOMICITY_PATHS = {
    "app/rust/crates/seo_steps/src/global_site_reconcile_step.rs",
    "app/rust/crates/seo_domain/src/link_recommendation.rs",
    "app/rust/crates/pg_runtime_ports/src/planning_ports.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/planning_persistence.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/navigation_persistence.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/shared_helpers.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/projection_status.rs",
    "app/rust/crates/integration_harness/src/tests/provider_runtime_surface_tests.rs",
    "app/rust/crates/integration_pg_app_e2e/src/lib.rs",
    "app/rust/crates/integration_pg_app_e2e/src/tests/planning_atomic_pg_tests.rs",
}


# Both live Temporal restart and Truth certification spawn these actual Rust
# processes. Prepare them once in the ordinary Cargo target, not recursively
# inside the integration harness's isolated nested compilation directory.
WORKER_E2E_PREPARE = (
    "cargo", "build", "--locked",
    "-p", "temporal_worker", "-p", "outbox_worker",
    "--bin", "temporal_worker", "--bin", "outbox_worker",
)
WORKER_E2E_RUNTIME_BINARIES = {
    "INTEGRATION_TEMPORAL_WORKER_BIN": "temporal_worker",
    "INTEGRATION_OUTBOX_WORKER_BIN": "outbox_worker",
}
# The A14 acceptance suite now launches the actual Outbox process. The old
# in-process Rust function isn't equivalent to independent Worker lifecycle.
OUTBOX_E2E_PREPARE = (
    "cargo", "build", "--locked", "-p", "outbox_worker", "--bin", "outbox_worker",
)

# The new pure DAG Temporal worker owns a different queue and has no provider
# processes. Never pull the legacy worker/outbox binaries into this test's
# preparation: both branches are intentionally independently deployable.
PURE_V3_E2E_PREPARE = (
    "cargo", "build", "--locked",
    "-p", "temporal_orchestrator", "--bin", "temporal_orchestrator",
)
PURE_V3_E2E_RUNTIME_BINARIES = {
    "INTEGRATION_PURE_DAG_ORCHESTRATOR_BIN": "temporal_orchestrator",
}

PG_V3_TEMPORAL_PREPARE = (
    "cargo", "build", "--locked", "-p", "temporal_pg_effects",
    "--bin", "temporal_pg_effects",
)
PG_V3_TEMPORAL_RUNTIME_BINARIES = {
    "INTEGRATION_PG_EFFECT_WORKER_BIN": "temporal_pg_effects",
}

# The mandatory V4 parent->child test consumes *both* independently compiled
# binaries from the existing content-verified V3 artifact. The scoped path
# builds them together; the full PR runner merely installs that one artifact.
PG_V4_PARENT_CHILD_PREPARE = (
    "cargo", "build", "--locked",
    "-p", "temporal_orchestrator", "-p", "temporal_pg_effects",
    "--bin", "temporal_orchestrator", "--bin", "temporal_pg_effects",
)
QDRANT_V7_PARENT_CHILD_PREPARE = (
    "cargo", "build", "--locked",
    "-p", "temporal_orchestrator", "-p", "temporal_qdrant_effects",
    "--bin", "temporal_orchestrator", "--bin", "temporal_qdrant_effects",
    "--features", "temporal_orchestrator/legacy-qdrant-v1",
)
QDRANT_V7_PARENT_CHILD_RUNTIME_BINARIES = {
    "INTEGRATION_EFFECT_DAG_WORKER_BIN": "temporal_orchestrator_legacy_qdrant_v1",
    "INTEGRATION_QDRANT_EFFECT_WORKER_BIN": "temporal_qdrant_effects",
}
MIXED_V6_NATIVE_OWNER_PREPARE = (
    "cargo", "build", "--locked",
    "-p", "temporal_orchestrator", "-p", "temporal_pg_effects",
    "-p", "temporal_qdrant_effects",
    "--bin", "temporal_orchestrator", "--bin", "temporal_pg_effects",
    "--bin", "temporal_qdrant_effects",
)
MIXED_V6_NATIVE_RUNTIME_BINARIES = {
    # Mixed V6/V7 exercises the provider-free generic parent, NEVER the
    # historical Qdrant V1-featured executable used by the frozen V8 suite.
    "INTEGRATION_EFFECT_DAG_WORKER_BIN": "temporal_orchestrator",
    "INTEGRATION_QDRANT_EFFECT_WORKER_BIN": "temporal_qdrant_effects",
    **PG_V3_TEMPORAL_RUNTIME_BINARIES,
}
PG_V4_PARENT_CHILD_RUNTIME_BINARIES = {
    **PG_V3_TEMPORAL_RUNTIME_BINARIES,
    "INTEGRATION_EFFECT_DAG_WORKER_BIN": "temporal_orchestrator",
}

# A13 reads only admissible, unscoped verified facts from disposable PostgreSQL
# through the production SQLx reader and the active SEO generation operation.
KNOWLEDGE_A13_PATHS = {
    "app/rust/crates/infrastructure/src/adapters/verified_rule_projection_adapter.rs",
    "app/rust/crates/seo_ports/src/knowledge.rs",
    "app/rust/crates/seo_ports/src/lib.rs",
    "app/rust/crates/seo_application/src/knowledge.rs",
    "app/rust/crates/runtime_composition/src/adapters/canonical_vector_search_adapter.rs",
    "app/rust/crates/runtime_composition/src/adapters/semantic_neighbor_search_adapter.rs",
    "app/rust/crates/seo_application/src/context_bundle.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_context_bundle_adapter.rs",
    "app/rust/crates/pg_runtime_ports/src/draft_publish_ports.rs",
    "app/rust/crates/runtime_composition/src/adapters/seo_ports_sqlx_adapter/types_and_helpers.rs",
    "app/rust/crates/runtime_composition/src/adapters/seo_ports_sqlx_adapter/source_context_projection_ports.rs",
    "app/rust/crates/seo_application/src/drafting.rs",
    "app/rust/services/temporal/src/activities/registry/mod_registry_impl.rs",
    "app/rust/crates/integration_harness/src/tests/knowledge_read_tests.rs",
    "app/rust/crates/integration_qdrant_e2e/src/tests/knowledge_read_tests.rs",
    "app/rust/services/temporal/src/activities/operations/truth_extraction_ops/triple_builder.rs",
    "app/rust/services/temporal/src/activities/operations/truth_extraction_ops/completeness_resolution.rs",
    "app/rust/services/temporal/src/activities/operations/truth_extraction_ops/contradiction_adjudication.rs",
}


# Canonical owners of *executable* Rust-backed integration suites. The exact
# commands are consumed by CI, never reconstructed in the runner/YAML. All
# paths here name live production consumers or the suite's actual inputs.
SUITE_OWNERS = {
    "planning_atomicity": {
        "paths": PLANNING_ATOMICITY_PATHS,
        "prefixes": (),
        "lock_crates": {"infrastructure", "seo_application"},
        "on_unknown_global_input": True,
        "command": (
            "cargo", "test", "-p", "integration_pg_app_e2e", "--features", "e2e",
            "--lib", "planning_atomic_", "--locked", "--", "--nocapture",
        ),
    },
    "draft_qa_scenario": {
        "paths": DRAFT_QA_SCENARIO_PATHS,
        "prefixes": (),
        "lock_crates": {"seo_domain", "runtime_models", "seo_application", "seo_steps"},
        "on_unknown_global_input": True,
        "command": (
            "cargo", "test", "-p", "integration_temporal_e2e", "--features", "e2e",
            "--lib", "draft_qa_synthetic_full_scenario_persists_page_draft_on_full_harness",
            "--locked", "--", "--nocapture",
        ),
    },
    "draft_qa_pg_persistence": {
        "paths": DRAFT_QA_SCENARIO_PATHS,
        "prefixes": ("app/rust/crates/integration_pg_app_e2e/",),
        "lock_crates": {"seo_domain", "runtime_models", "seo_application", "seo_steps"},
        "on_unknown_global_input": True,
        "command": (
            "cargo", "test", "-p", "integration_pg_app_e2e", "--features", "e2e",
            "--lib", "draft_qa_persistence_rolls_back_late_failure_and_rejects_missing_draft",
            "--locked", "--", "--nocapture",
        ),
    },
    "cms_review_revision": {
        "paths": CMS_REVIEW_PATHS,
        "prefixes": (),
        "lock_crates": {"seo_domain", "seo_application", "seo_ports", "infrastructure"},
        "on_unknown_global_input": True,
        "command": (
            "cargo", "test", "-p", "integration_pg_app_e2e", "--features", "e2e",
            "--lib", "cms_revision_",
            "--locked", "--", "--nocapture",
        ),
    },
    "publishing_a17": {
        "paths": PUBLISHING_A17_PATHS,
        "prefixes": (),
        "lock_crates": {"seo_domain", "seo_application", "infrastructure"},
        "on_unknown_global_input": True,
        "command": (
            "cargo", "test", "-p", "integration_pg_app_e2e", "--features", "e2e",
            "--lib", "publishing_a17_postgres_outbox_http_faults_and_recovery",
            "--locked", "--", "--nocapture",
        ),
    },
    "outbox_a14": {
        "paths": OUTBOX_A14_PATHS,
        "prefixes": ("app/rust/services/outbox_worker/src/", "app/rust/crates/integration_outbox_e2e/"),
        "lock_crates": {"outbox_worker", "infrastructure"},
        "on_unknown_global_input": True,
        "prepare": OUTBOX_E2E_PREPARE,
        "runtime_binaries": {"INTEGRATION_OUTBOX_WORKER_BIN": "outbox_worker"},
        "command": (
            "cargo", "test", "-p", "integration_outbox_e2e", "--features", "e2e",
            "--lib", "outbox_a14_", "--locked", "--", "--nocapture",
        ),
    },
    "truth_adjudication_integrity": {
        "paths": TRUTH_ADJUDICATION_PATHS,
        "prefixes": (),
        "lock_crates": {"truth_domain"},
        "on_unknown_global_input": True,
        "command": (
            "cargo", "test", "-p", "integration_pg_app_e2e", "--features", "e2e",
            "--lib", "truth_adjudication_", "--locked", "--", "--nocapture",
        ),
    },
    "knowledge_a13": {
        "paths": KNOWLEDGE_A13_PATHS,
        "prefixes": ("app/rust/crates/integration_qdrant_e2e/",),
        "lock_crates": {"seo_application", "seo_ports", "vector_qdrant", "infrastructure"},
        "on_unknown_global_input": True,
        "command": (
            "cargo", "test", "-p", "integration_qdrant_e2e", "--features", "e2e",
            "--lib", "knowledge_a13_", "--locked", "--", "--nocapture",
        ),
    },
    "truth_certification": {
        "paths": set(),
        "prefixes": (TRUTH_FIXTURE_PREFIX,),
        "on_unknown_global_input": False,
        "prepare": WORKER_E2E_PREPARE,
        "runtime_binaries": WORKER_E2E_RUNTIME_BINARIES,
        "command": (
            "cargo", "test", "-p", "integration_harness", "--features", "e2e",
            "--lib", "truth_certification_suite_matches_fixture_expectations",
            "--locked", "--", "--nocapture",
        ),
    },
    "graph_query": {
        "paths": {
            "app/rust/crates/runtime_composition/src/adapters/graph_capability_adapter.rs",
            # Shared test-container setup is also the Neo4j uniqueness
            # bootstrap boundary. Changing it must run the real graph race.
            "app/rust/crates/integration_harness/src/containers.rs",
            "app/rust/crates/integration_harness/src/tests/provider_runtime_surface_tests.rs",
            "app/rust/crates/integration_graph_e2e/src/lib.rs",
            "app/rust/crates/integration_graph_e2e/src/tests/graph_capability_probe_tests.rs",
            "app/rust/crates/integration_graph_e2e/src/tests/graph_capability_link_projection_tests.rs",
            "app/rust/crates/integration_graph_fixtures/src/lib.rs",
            "app/rust/crates/graph_capability/src/lib.rs",
        },
        "prefixes": ("app/rust/crates/graph_neo4j/", "app/rust/crates/integration_graph_e2e/",
                     "app/rust/crates/integration_graph_fixtures/", "app/rust/crates/graph_capability/",
                     "app/rust/crates/integration_test_env/"),
        "lock_crates": {"graph_neo4j"},
        "on_unknown_global_input": False,
        "command": (
            "cargo", "test", "-p", "integration_graph_e2e", "--features", "e2e",
            # Both the context-key probe and real Neo4j scoped-edge regression
            # share the graph capability prefix. This selects BOTH without
            # adding a suite, job or unguarded provider-side traversal.
            "--lib", "graph_capability_", "--locked", "--", "--nocapture",
        ),
    },
    "vector_projection": {
        "paths": {"app/rust/crates/integration_qdrant_e2e/src/lib.rs",
                  "app/rust/crates/integration_qdrant_e2e/src/tests/qdrant_rest_projection_tests.rs"},
        "prefixes": ("app/rust/crates/vector_qdrant/", "app/rust/crates/integration_qdrant_e2e/"),
        "lock_crates": {"vector_qdrant"},
        "on_unknown_global_input": False,
        "command": (
            "cargo", "test", "-p", "integration_qdrant_e2e", "--features", "e2e",
            "--lib", "qdrant_rest_outbox_upsert_is_visible_to_grpc_retrieval",
            "--locked", "--", "--nocapture",
        ),
    },
    # This is a distinct real PostgreSQL + Qdrant external-effect contract.
    # Preserve the older REST-to-gRPC test as its own exact filter: broadening
    # it would silently mask a missing newly admitted/recoverable projection.
    "vector_admitted_projection": {
        "paths": {
            "app/db/schema.sql",
            "app/db/migrations/20261125_0010_qdrant_projection_operator_ledger.sql",
            "app/db/migrations/20261125_0012_qdrant_projection_immutable_receipts_and_revocation.sql",
            "app/db/migrations/20261125_0014_qdrant_projection_run_status_start_fence.sql",
            "app/rust/crates/integration_qdrant_e2e/src/lib.rs",
            "app/rust/crates/integration_qdrant_e2e/src/tests/admitted_qdrant_projection_tests.rs",
        },
        "prefixes": (
            "app/rust/crates/projection_delivery/",
            "app/rust/crates/integration_qdrant_e2e/",
            "app/rust/crates/vector_qdrant/",
        ),
        "lock_crates": {"projection_delivery", "vector_qdrant"},
        "on_unknown_global_input": False,
        "command": (
            "cargo", "test", "-p", "integration_qdrant_e2e", "--features", "e2e",
            "--lib", "admitted_qdrant_keyword_projection_recovers_after_receipt_failure_without_second_rpc",
            "--locked", "--", "--nocapture",
        ),
    },
    # One native Temporal parent/child execution over a real PG admission and
    # Qdrant mutation. It is NOT a broadening of the REST-only vector shard:
    # distinct test executable and two exact-source independent Worker owners.
    "temporal_qdrant_v8": {
        "paths": {
            "app/db/schema.sql",
            "app/db/migrations/20261125_0010_qdrant_projection_operator_ledger.sql",
            "app/db/migrations/20261125_0012_qdrant_projection_immutable_receipts_and_revocation.sql",
            "app/db/migrations/20261125_0014_qdrant_projection_run_status_start_fence.sql",
            "app/rust/crates/integration_qdrant_temporal_e2e/Cargo.toml",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/lib.rs",
        },
        "prefixes": (
            "app/rust/crates/integration_qdrant_temporal_e2e/",
            "app/rust/services/temporal_qdrant_effects/",
            "app/rust/services/temporal_orchestrator/",
            "app/rust/crates/projection_delivery/",
            "app/rust/crates/vector_qdrant/",
            "app/rust/crates/operation_protocol/",
            "app/rust/crates/qdrant_contracts/",
        ),
        "lock_crates": {"temporal_orchestrator", "temporal_qdrant_effects",
                        "projection_delivery", "vector_qdrant", "operation_protocol",
                        "qdrant_contracts"},
        "on_unknown_global_input": True,
        "prepare": QDRANT_V7_PARENT_CHILD_PREPARE,
        "runtime_binaries": QDRANT_V7_PARENT_CHILD_RUNTIME_BINARIES,
        # Scoped developer feedback uses the same exact reviewed feature-
        # gated source but Cargo names both outputs temporal_orchestrator.
        # Stage the old V1 ABI under a separate immutable executable name.
        "stage_binaries": {"temporal_orchestrator_legacy_qdrant_v1": "temporal_orchestrator"},
        "command": (
            "cargo", "test", "-p", "integration_qdrant_temporal_e2e", "--features", "e2e",
            "--lib", "qdrant_v8_native_temporal_history_operator_real_effect",
            "--locked", "--", "--nocapture",
        ),
    },
    # Real independently admitted PG->Qdrant after-only native Temporal E2E,
    # a strict Produced-schema/cross-tenant/forged-route negative, and a
    # second valid plan with reversed provider order. All run in the EXISTING
    # mandatory Qdrant Temporal shard, reusing its
    # source-verified three Worker binaries. The unrelated PG receipt is never
    # pretended to be a Qdrant vector command or a Produced dataflow edge.
    "temporal_mixed_v6_contract": {
        "paths": {
            "app/rust/crates/integration_qdrant_temporal_e2e/src/lib.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_mixed_owner_tests.rs",
            "app/db/migrations/20261125_0017_qdrant_mixed_owner_v2.sql",
            "app/db/migrations/20261125_0018_pg_generic_owner_v1.sql",
            "app/db/migrations/20261125_0019_qdrant_generic_owner_v3.sql",
            "app/db/migrations/20261125_0020_operation_dag_run_governance_v1.sql",
        },
        "prefixes": (
            "app/rust/crates/integration_qdrant_temporal_e2e/",
            "app/rust/crates/operation_protocol/",
            "app/rust/crates/qdrant_contracts/",
            "app/rust/crates/truth_domain/",
            "app/rust/crates/seo_application/src/operation_adapter/",
        ),
        "lock_crates": {"operation_protocol", "qdrant_contracts", "truth_domain",
                        "seo_application", "temporal_orchestrator",
                        "temporal_qdrant_effects", "temporal_pg_effects"},
        "on_unknown_global_input": True,
        "prepare": MIXED_V6_NATIVE_OWNER_PREPARE,
        "runtime_binaries": MIXED_V6_NATIVE_RUNTIME_BINARIES,
        "command": (
            "cargo", "test", "-p", "integration_qdrant_temporal_e2e", "--features", "e2e",
            "--lib", "mixed_v6_",
            "--locked", "--", "--nocapture",
        ),
    },
    # True native PG-produced VectorProjectionV1 -> Temporal Child result ->
    # independently admitted Qdrant Produced consumer, plus a fail-closed
    # contract/tenant preflight. Preserve the older after-only V6 shard: both
    # are mandatory and represent DIFFERENT guarantees.
    "temporal_mixed_v7_produced": {
        "paths": {
            "app/rust/crates/integration_qdrant_temporal_e2e/src/lib.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_mixed_owner_tests.rs",
            "app/db/migrations/20261125_0018_pg_generic_owner_v1.sql",
            "app/db/migrations/20261125_0019_qdrant_generic_owner_v3.sql",
            "app/db/migrations/20261125_0020_operation_dag_run_governance_v1.sql",
            "app/db/migrations/20261125_0021_pg_vector_projection_v1.sql",
        },
        "prefixes": (
            "app/rust/crates/integration_qdrant_temporal_e2e/",
            "app/rust/crates/operation_protocol/",
            "app/rust/crates/projection_contracts/",
            "app/rust/crates/qdrant_contracts/",
            "app/rust/crates/postgres_store/",
            "app/rust/crates/seo_application/src/operation_adapter/",
            "app/rust/services/temporal_orchestrator/",
            "app/rust/services/temporal_pg_effects/",
            "app/rust/services/temporal_qdrant_effects/",
        ),
        "lock_crates": {"operation_protocol", "projection_contracts", "postgres_store",
                        "qdrant_contracts", "seo_application", "temporal_orchestrator",
                        "temporal_qdrant_effects", "temporal_pg_effects"},
        "on_unknown_global_input": True,
        "prepare": MIXED_V6_NATIVE_OWNER_PREPARE,
        "runtime_binaries": MIXED_V6_NATIVE_RUNTIME_BINARIES,
        "command": (
            "cargo", "test", "-p", "integration_qdrant_temporal_e2e", "--features", "e2e",
            "--lib", "mixed_v7_",
            "--locked", "--", "--nocapture",
        ),
    },
    # NEW exact native V2 test: separate generic business authority, all
    # owner ValidateOnly Children before first effect, real PG Produced ->
    # Temporal -> Qdrant plus an independent parallel PG/Qdrant plan and
    # malformed downstream command rejected with ZERO upstream effects.
    # This is a mandatory dedicated shard, not a contract-only substitute.
    "temporal_mixed_v2_native": {
        "paths": {
            "app/rust/crates/integration_qdrant_temporal_e2e/src/lib.rs",
            "app/rust/crates/integration_qdrant_temporal_e2e/src/tests/native_temporal_mixed_owner_tests.rs",
            "app/db/migrations/20261125_0020_operation_dag_run_governance_v1.sql",
            "app/db/migrations/20261125_0021_pg_vector_projection_v1.sql",
            "app/db/migrations/20261125_0022_pg_owner_v2_admission.sql",
            "app/db/migrations/20261125_0023_qdrant_owner_v2_admission.sql",
        },
        "prefixes": (
            "app/rust/crates/integration_qdrant_temporal_e2e/",
            "app/rust/crates/operation_protocol/",
            "app/rust/crates/projection_contracts/",
            "app/rust/crates/qdrant_contracts/",
            "app/rust/crates/qdrant_effects_core/",
            "app/rust/crates/postgres_store/",
            "app/rust/crates/seo_application/src/operation_adapter/",
            "app/rust/services/temporal_orchestrator/",
            "app/rust/services/temporal_pg_effects/",
            "app/rust/services/temporal_qdrant_effects/",
        ),
        "lock_crates": {"operation_protocol", "projection_contracts", "qdrant_contracts",
                        "qdrant_effects_core", "postgres_store", "seo_application",
                        "temporal_orchestrator", "temporal_pg_effects",
                        "temporal_qdrant_effects"},
        "on_unknown_global_input": True,
        "prepare": MIXED_V6_NATIVE_OWNER_PREPARE,
        "runtime_binaries": MIXED_V6_NATIVE_RUNTIME_BINARIES,
        "command": (
            "cargo", "test", "-p", "integration_qdrant_temporal_e2e", "--features", "e2e",
            "--lib", "mixed_v2_native_",
            "--locked", "--", "--nocapture",
        ),
    },
    "temporal_restart": {
        "paths": {
            "app/rust/services/temporal/Cargo.toml",
            "app/contracts/proto/temporal_payloads.proto",
            "app/rust/crates/integration_harness/src/tests/manual_llm_temporal_tests.rs",
            "app/rust/crates/integration_temporal_e2e/src/tests/manual_llm_temporal_tests.rs",
            "app/rust/crates/integration_temporal_e2e/src/lib.rs",
            "app/rust/crates/integration_temporal_fixtures/src/lib.rs",
            # These domain/editorial decisions are executed by the real
            # canonical Temporal page workflow even when its orchestration
            # source is untouched. A policy-only diff must still exercise it.
            "app/rust/crates/seo_steps/src/editorial_wire.rs",
            "app/rust/crates/seo_steps/src/page_wire.rs",
            "app/rust/crates/seo_domain/src/editorial.rs",
            "app/rust/crates/seo_domain/src/page_readiness.rs",
            "app/rust/crates/seo_domain/src/content_contract.rs",
            "app/rust/crates/seo_domain/src/draft_quality.rs",
            "app/rust/crates/seo_domain/src/link_recommendation.rs",
            "app/rust/crates/seo_application/src/drafting.rs",
            "app/rust/crates/seo_steps/src/draft_qa_step.rs",
            "app/rust/crates/seo_steps/src/content_contract_validate_step.rs",
            "app/rust/crates/infrastructure/src/adapters/editorial_llm_adapter/generated_overlay.rs",
        },
        "prefixes": ("app/rust/services/temporal/src/", "app/rust/crates/integration_temporal_e2e/",
                     "app/rust/crates/integration_temporal_fixtures/"),
        "lock_crates": {"temporal_worker"},
        # This separate live E2E also opens the canonical happy-path JSON and
        # its HTML pages. Do not let fixture-only narrowing skip its consumer.
        "fixture_ids": {"happy_path_publish_allowed"},
        "on_unknown_global_input": False,
        # Build the actual processes in the SAME Cargo target as the preceding
        # workspace tests. The harness already accepts these binary paths;
        # otherwise it compiles both workers again in an isolated target.
        "prepare": WORKER_E2E_PREPARE,
        "runtime_binaries": WORKER_E2E_RUNTIME_BINARIES,
        "command": (
            "cargo", "test", "-p", "integration_temporal_e2e", "--features", "e2e",
            "--lib", "canonical_temporal_",
            "--locked", "--", "--nocapture",
        ),
    },
    "pure_dag_v3": {
        "paths": {
            "app/rust/services/temporal_orchestrator/Cargo.toml",
            "app/rust/crates/integration_harness/src/lib.rs",
            "app/rust/crates/integration_harness/src/tests/pure_v3_temporal_tests.rs",
            "app/rust/crates/integration_temporal_e2e/src/tests/pure_v3_temporal_tests.rs",
            "app/rust/crates/temporal_gateway/src/lib.rs",
            # The Workflow calls this real executor; changes to immutable
            # plan validation and payload handling require real History replay.
            "app/rust/crates/seo_application/src/operation_adapter.rs",
            "app/rust/crates/seo_application/src/execution/durable_plan.rs",
        },
        "prefixes": (
            "app/rust/services/temporal_orchestrator/src/",
            "app/rust/crates/seo_application/src/operation_adapter/",
            "app/rust/crates/operation_protocol/src/",
        ),
        "lock_crates": {"temporal_orchestrator", "temporal_gateway"},
        # Unlike legacy certification, V3 is the mandatory source-to-runtime
        # queue-isolation check for a new executable; exercise it when a global
        # CI/runtime input changes, even when the diff cannot be mapped.
        "on_unknown_global_input": True,
        "prepare": PURE_V3_E2E_PREPARE,
        "runtime_binaries": PURE_V3_E2E_RUNTIME_BINARIES,
        "command": (
            "cargo", "test", "-p", "integration_temporal_e2e", "--features", "e2e",
            "--lib", "pure_v3_", "--locked", "--", "--nocapture",
        ),
    },
    "pg_effect_v3": {
        "paths": {
            "app/db/schema.sql",
            "app/db/migrations/20261124_0003_pg_truth_review_v3.sql",
            "app/db/migrations/20261125_0001_raw_truth_source_writer_fence.sql",
            "app/db/migrations/20261125_0002_execution_run_truth_context_fence.sql",
            "app/db/migrations/20261125_0003_pg_effect_parent_child_v4.sql",
            "app/db/migrations/20261125_0004_truth_reconcile_owner_capability.sql",
            "app/db/migrations/20261125_0005_verified_truth_step_context_fence.sql",
            "app/db/migrations/20261125_0006_truth_hitl_writer_fence.sql",
            "app/db/migrations/20261125_0007_authoritative_tenant_truth_rls.sql",
            "app/db/migrations/20261125_0008_truth_source_authority_fence.sql",
            "app/db/migrations/20261125_0009_tenant_ingress_writer_provenance.sql",
            "app/db/migrations/20261125_0011_tenant_operator_truncate_fence.sql",
            "app/db/migrations/20261125_0013_tenant_quarantine_preflight_v1.sql",
            "app/rust/crates/postgres_store/src/truth_review_effect_v3.rs",
            "app/rust/crates/postgres_store/src/sqlx_hitl_adapter.rs",
            "app/rust/crates/postgres_store/src/lib.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/lib.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v3_truth_review_effect_tests.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v3_admission_race_tests.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v3_operator_cli_tests.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v5_ordered_children_tests.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v6_tenant_ingress_tests.rs",
        },
        "prefixes": (
            "app/rust/crates/integration_pg_v3_e2e/",
            "app/rust/services/temporal_pg_effects/",
            "app/rust/services/temporal_orchestrator/",
            "app/rust/crates/seo_application/src/operation_adapter/",
            "app/rust/crates/truth_domain/src/review_effect_contract",
            "app/rust/crates/operation_protocol/src/",
        ),
        "lock_crates": {
            "temporal_pg_effects", "temporal_orchestrator", "seo_application",
            "truth_domain", "postgres_store", "operation_protocol",
        },
        # The PostgreSQL-only fenced Activity owner must remain tested even
        # when a global schema/CI input cannot be safely narrowed.
        "on_unknown_global_input": True,
        # Includes live parent/child V4 Temporal+PG: the two exact-source V3
        # binary owners are mandatory, with no legacy Worker fallback.
        "prepare": PG_V4_PARENT_CHILD_PREPARE,
        "runtime_binaries": PG_V4_PARENT_CHILD_RUNTIME_BINARIES,
        "command": (
            "cargo", "test", "-p", "integration_pg_v3_e2e", "--features", "e2e",
            "--lib", "pg_v3_", "--locked", "--", "--nocapture",
        ),
    },
    "pg_temporal_v3": {
        "paths": {
            "app/rust/crates/integration_pg_v3_e2e/src/lib.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v3_temporal_cross_queue_tests.rs",
            "app/rust/crates/integration_pg_v3_e2e/src/tests/pg_v3_truth_review_effect_tests.rs",
            "app/rust/crates/postgres_store/src/truth_review_effect_v3.rs",
            "app/db/schema.sql",
            "app/db/migrations/20261124_0003_pg_truth_review_v3.sql",
            "app/db/migrations/20261125_0001_raw_truth_source_writer_fence.sql",
            "app/db/migrations/20261125_0002_execution_run_truth_context_fence.sql",
            "app/db/migrations/20261125_0003_pg_effect_parent_child_v4.sql",
            "app/db/migrations/20261125_0004_truth_reconcile_owner_capability.sql",
            "app/db/migrations/20261125_0005_verified_truth_step_context_fence.sql",
        },
        "prefixes": (
            "app/rust/crates/integration_pg_v3_e2e/",
            "app/rust/services/temporal_pg_effects/",
            "app/rust/crates/operation_protocol/src/",
            "app/rust/crates/temporal_gateway/src/",
        ),
        "lock_crates": {"temporal_pg_effects", "temporal_gateway", "postgres_store"},
        "on_unknown_global_input": True,
        "prepare": PG_V3_TEMPORAL_PREPARE,
        "runtime_binaries": PG_V3_TEMPORAL_RUNTIME_BINARIES,
        "command": (
            "cargo", "test", "-p", "integration_pg_v3_e2e", "--features", "e2e",
            "--lib", "temporal_effect_v3_", "--locked", "--", "--nocapture",
        ),
    },
}

# Only the image that is actually built, source-verified and attested by this
# CI has a release selector. Other services have executable Rust/Compose
# owners, not hypothetical CI image outputs. Their Dockerfile changes still
# receive the fail-closed PR gate via ci_jobs_for_change.
IMAGE_OWNERS = {
    "temporal-worker": {
        "crates": {"temporal_worker", "cli_tools"},
        "paths": WORKER_PACKAGING_FILES | {"app/rust/services/temporal/Dockerfile"},
        "prefixes": ("infra/cloudflare/seo-static/",),
    },
}


def worker_release_decision(report: dict[str, Any], changed: list[str], *, manual: bool) -> dict[str, Any]:
    """Reuse the single impact report for an exact accepted-main image decision.

    CI control changes still get full PR acceptance. They must not rebuild the
    unchanged Worker on main merely because PR selection is conservative.
    """
    status = report.get("image_status", {}).get("temporal-worker")
    if (report.get("schema_version") != SCHEMA_VERSION
            or report.get("mandatory_full_ci_gate") is not True
            or status not in ("selected", "not_applicable")
            or (status == "selected") != ("temporal-worker" in report.get("selected_images", []))
            or not changed or any(not isinstance(path, str) or not path for path in changed)
            or sorted(set(changed)) != report.get("changed_paths")
            or not isinstance(manual, bool)):
        raise ValueError("WORKER_IMAGE_SELECTION=FAIL invalid exact-source impact")
    if manual:
        required, reason = True, "explicit_accepted_main_rebuild"
    elif set(changed) <= NON_WORKER_DELIVERY_INPUTS:
        required, reason = False, "ci_operator_maintenance_only"
    else:
        required, reason = status == "selected", "shared_impact_registry"
    # Buildx's reusable builder layers are source-specific, not just keyed by
    # Cargo manifests. Even a source-only Rust edit must export the updated
    # binary layer: a subsequent static-assets-only release reuses it. Pure
    # delivered assets need only rebase unchanged Rust/Node dependency layers;
    # exporting the whole builder cache again would be duplicate work.
    # An explicit rebuild, an unknown input, or ANY executable/compiler input
    # conservatively keeps cache export enabled.
    static_delivery_only = all(
        path in WORKER_PACKAGING_FILES
        or (path.startswith("infra/cloudflare/seo-static/")
            and Path(path).name not in {"package.json", "package-lock.json", "npm-shrinkwrap.json"}
            and path not in WORKER_NONDELIVERY_FILES)
        for path in changed
    )
    return {
        "worker_required": required,
        "cache_export_required": required and (manual or not static_delivery_only),
        "reason": reason,
        "changed_paths": sorted(set(changed)),
    }


def main_cache_prime_required(changed: list[str]) -> bool:
    """Warm Rust objects only for compilation inputs, including rename sources.

    The main job obtains these paths from ``changed_paths``; its old
    ``git diff --name-only`` dropped the source of a rename. Test-only harness
    changes are verified by PR CI and never need a second compiler warm-up.
    """
    if not changed or any(not isinstance(path, str) or not path for path in changed):
        raise ValueError("MAIN_CACHE_PRIME=FAIL invalid changed paths")
    return any(
        path.startswith("app/rust/")
        and not path.startswith(("app/rust/crates/integration_harness/src/tests/",
                                 "app/rust/crates/integration_outbox_e2e/src/tests/"))
        and not path.startswith(TRUTH_FIXTURE_PREFIX)
        and not path.endswith("/Dockerfile")
        for path in changed
    )

# GitHub job IDs, not display names. The PR gate checks every one of these;
# only this planner can mark an owner not applicable to an exact source diff.
CI_PR_OWNERS = (
    "architecture-and-rust",
    "rust-e2e-graph-harness",
    "rust-e2e-qdrant-harness",
    "rust-e2e-qdrant-temporal-harness",
    "rust-e2e-pg-app-harness",
    "rust-e2e-temporal-harness",
    "rust-e2e-pg-harness",
    "rust-e2e-outbox-harness",
    "rust-e2e-truth-harness",
    "rust-e2e-legacy-binaries",
    "rust-e2e-v3-binaries",
    "rust-e2e-qdrant-effects-binaries",
    "rust-graph-e2e",
    "rust-qdrant-e2e",
    "rust-qdrant-temporal-e2e",
    "rust-pg-app-e2e",
    "rust-temporal-legacy-e2e",
    "rust-temporal-v3-e2e",
    "rust-pg-e2e",
    "rust-outbox-e2e",
    "rust-truth-e2e",
    "product-truth",
    "rust-security",
    "cloudflare-static-build",
    "stack-temporal-postgres18",
    "stack-storage-graph",
    "verify",
    "windows-operator-smoke",
)


def ci_jobs_for_change(
    paths: list[str], scope: str, suites: set[str], images: set[str],
) -> dict[str, list[str]]:
    """One fail-closed reasoned selection for the existing PR job owners.

    Integration suites already run in the Rust owner. The two independent
    Compose boot checks are for *deployment configuration*, not a substitute
    for the disposable PostgreSQL/Qdrant/Neo4j adapters exercised by suites.
    """
    if scope == "full":
        required = {name: ["fail-closed: global or unknown impact"] for name in CI_PR_OWNERS}
        # Optional Truth certification still has one real broad-harness test;
        # build/execute that physical compiler only when a live fixture
        # selects this suite. No omitted test is silently reported as green.
        if "truth_certification" not in suites:
            del required["rust-e2e-truth-harness"]
            del required["rust-truth-e2e"]
        return required

    changed = set(paths)
    selected: dict[str, list[str]] = {}

    def require(name: str, why: str) -> None:
        selected.setdefault(name, []).append(why)

    if scope == "scoped" or suites:
        for name in ("architecture-and-rust", "rust-security", "verify"):
            require(name, "affected Rust workspace or integration suite")

    if changed & SOURCE_POLICY_DOCUMENTS:
        require("product-truth", "Markdown is a source-policy input, not just explanatory documentation")
        require("verify", "source-policy documents are read by acceptance scripts")

    if changed & AUTOMATION_SOURCE_DOCUMENTS:
        require("verify", "reviewed automation Markdown is checked by source-policy contracts")

    if changed & KNOWN_CONTRACT_CRATES.keys():
        require("product-truth", "generated protobuf contracts feed live source-policy checks")

    if any(path.startswith("docs/runs/") for path in changed):
        require("product-truth", "historical Truth evidence is consumed by Python policy checks")
        require("verify", "historical run evidence is checked by source contracts")

    truth_prefixes = (
        "app/rust/crates/truth_domain/", "app/rust/crates/acquisition/",
        "app/rust/services/temporal/", TRUTH_FIXTURE_PREFIX,
    )
    if any(path.startswith(truth_prefixes) for path in changed) or "truth_certification" in suites:
        require("product-truth", "live Acquisition/Truth/Temporal source or fixture")

    if any((path.startswith("infra/cloudflare/seo-static/") and path not in WORKER_NONDELIVERY_FILES)
           or path in WORKER_PACKAGING_FILES for path in changed):
        require("cloudflare-static-build", "Cloudflare static or delivered Python asset")
    if changed & WORKER_PACKAGING_FILES:
        require("product-truth", "first-site delivery and readback scripts feed Product smoke")
        require("verify", "delivered Python assets require source policy and lint")

    if changed & CI_ONLY_VERIFY_FILES:
        require("verify", "known CI-only file: validated by mandatory source-policy contracts")

    if any(path.startswith(("infra/analytics_lab/", "app/analytics_lab/")) for path in changed):
        require("verify", "analytics-lab Python is checked by source policy and Ruff")

    if "temporal-worker" in images:
        require("windows-operator-smoke", "changed Worker delivery artifact")

    if any(path == "docker-compose.yml" or path.startswith("infra/temporal/") for path in changed):
        require("stack-temporal-postgres18", "Temporal/Compose deployment configuration")
    if any(path == "docker-compose.yml" or path.startswith("infra/postgres/") or
           path.startswith("infra/qdrant/") or path.startswith("infra/neo4j/") for path in changed):
        require("stack-storage-graph", "storage/graph Compose deployment configuration")

    if any(path.startswith("automation/") and path.endswith(".ps1") or
           path.startswith("automation/check_windows_") for path in changed):
        require("windows-operator-smoke", "native Windows operator contract")

    # A Dockerfile is an executable delivery input. No narrow PR job currently
    # builds these images, so do not label it equivalent to Markdown or infer
    # release acceptance from the image impact report.
    if any(path.endswith("/Dockerfile") for path in changed):
        return {name: ["fail-closed: Dockerfile delivery owner not isolated"] for name in CI_PR_OWNERS}

    # New narrow allowlists must name the executable owner explicitly. This
    # catches future additions to the non-Rust `scope=none` classification.
    uncovered = (changed - SOURCE_POLICY_DOCUMENTS - AUTOMATION_SOURCE_DOCUMENTS
                 - DOCUMENTATION_FILES - CI_ONLY_VERIFY_FILES - WORKER_NONDELIVERY_FILES)
    uncovered = {path for path in uncovered
                 if not (path.startswith(DOCUMENTATION_PREFIXES) and path.endswith(".md"))
                 and not (path.startswith("docs/runs/") and path.endswith(".json"))
                 and not path.startswith(("infra/analytics_lab/", "app/analytics_lab/"))
                 and not path.startswith("infra/cloudflare/seo-static/")
                 and not path.startswith(TRUTH_FIXTURE_PREFIX)
                 and path not in WORKER_PACKAGING_FILES}
    if scope == "none" and uncovered:
        return {name: ["fail-closed: unowned non-Rust input"] for name in CI_PR_OWNERS}

    return {name: selected[name] for name in CI_PR_OWNERS if name in selected}


def truth_fixture_ids_for_paths(
    changed_paths: list[str], *, root: Path = ROOT,
) -> tuple[list[str] | None, list[str]]:
    """Resolve live HTML consumers from the same JSON manifests as Rust.

    `None` means run *all* certification fixtures. Never scope changed
    manifests, deletes/renames, malformed inventories or unowned HTML by guess.
    """
    changed = sorted(path for path in set(changed_paths) if path.startswith(TRUTH_FIXTURE_PREFIX))
    if not changed:
        return None, ["full: no changed live HTML input to resolve"]
    if any(not path.endswith(".html") for path in changed):
        return None, ["full: fixture manifest or non-HTML input changed"]

    folder = root / TRUTH_FIXTURE_PREFIX
    manifests = sorted(folder.glob("*.json"))
    if not manifests:
        return None, ["full: no readable Truth fixture manifests"]

    references: dict[str, set[str]] = {}
    seen_ids: set[str] = set()
    try:
        for manifest in manifests:
            data = json.loads(manifest.read_text(encoding="utf-8"))
            fixture_id = data["fixture_id"]
            pages = data["source_inputs"]["pages"]
            if (not isinstance(fixture_id, str) or not TRUTH_FIXTURE_ID.fullmatch(fixture_id)
                    or fixture_id != manifest.stem or fixture_id in seen_ids
                    or not isinstance(pages, list) or not pages):
                raise ValueError("invalid or duplicate fixture ID/pages")
            seen_ids.add(fixture_id)
            for page in pages:
                html_file = page["html_file"]
                if (not isinstance(html_file, str) or Path(html_file).name != html_file
                        or not html_file.endswith(".html")
                        or not (folder / html_file).is_file()):
                    raise ValueError("unsafe or missing referenced HTML input")
                references.setdefault(TRUTH_FIXTURE_PREFIX + html_file, set()).add(fixture_id)
    except (OSError, ValueError, TypeError, KeyError) as error:
        return None, [f"full: incomplete Truth fixture inventory ({type(error).__name__})"]

    unmatched = sorted(set(changed) - set(references))
    if unmatched:
        return None, ["full: changed HTML lacks a live JSON fixture consumer: " + ", ".join(unmatched)]

    fixture_ids = sorted(set().union(*(references[path] for path in changed)))
    return fixture_ids, ["changed live HTML is read by: " + ", ".join(fixture_ids)]


def suite_environment_for_report(report: dict[str, Any], name: str) -> dict[str, str]:
    """Validate test filters before passing them to the real Cargo process."""
    scoped = report.get("suite_fixture_ids")
    selected = report.get("selected_suites")
    if not isinstance(scoped, dict) or not isinstance(selected, list) or any(
        suite != "truth_certification" or suite not in selected for suite in scoped
    ):
        raise ValueError("invalid integration fixture owner selection")
    if name != "truth_certification" or name not in scoped:
        return {}
    ids = scoped[name]
    if (not isinstance(ids, list) or not ids
            or any(not isinstance(value, str) or not TRUTH_FIXTURE_ID.fullmatch(value) for value in ids)
            or len(set(ids)) != len(ids)):
        raise ValueError("invalid exact Truth fixture IDs")
    return {"TRUTH_CERT_FIXTURE_IDS": ",".join(ids)}


def suite_commands_for_report(report: dict[str, Any]) -> list[tuple[str, list[str]]]:
    """Validate and materialize the planner's selected real integration tests."""
    if report.get("schema_version") != SCHEMA_VERSION or report.get("mandatory_full_ci_gate") is not True:
        raise ValueError("invalid impact report: mandatory CI gate missing")
    suites = report.get("selected_suites")
    if not isinstance(suites, list) or len(suites) != len(set(suites)) or any(
        not isinstance(name, str) or name not in SUITE_OWNERS for name in suites
    ):
        raise ValueError("impact report has missing, duplicate or unknown integration suites")
    if not isinstance(report.get("suite_fixture_ids"), dict):
        raise ValueError("impact report lacks typed fixture owner selection")
    for name in suites:
        suite_environment_for_report(report, name)
    if report.get("suite_fixture_ids") and "truth_certification" not in suites:
        raise ValueError("fixture selection without its executable suite")
    return [(name, list(SUITE_OWNERS[name]["command"])) for name in suites]


# The full CI already runs independent workspace units in verify. Running
# these real, Worker-independent E2E suites there uses the idle second runner
# while architecture retains the Worker binaries and Temporal regressions.
# Scoped changes keep their existing single-owner architecture fast path.
FULL_VERIFY_E2E_SUITES = frozenset({
    "draft_qa_scenario", "draft_qa_pg_persistence", "knowledge_a13", "outbox_a14",
    "planning_atomicity", "publishing_a17", "cms_review_revision",
    "truth_adjudication_integrity", "pure_dag_v3", "pg_effect_v3",
})


# Full-scope live suites execute on genuinely separate GitHub runners. This is
# one static, typed routing table, NOT another operation registry: all the
# executable commands and selection rules remain owned by SUITE_OWNERS.
# A row is (source suite, exact Rust test-name filter, expected test count).
# Each source suite is covered once except the canonical Temporal suite, whose
# three individually audited test names deliberately run on two independent
# servers/runners. Actual test counts are asserted by the binary runner, not
# guessed from a successful subprocess exit status.
E2E_ARCH_SHARDS = (
    "graph", "pg_temporal", "temporal_manual", "temporal_blocked",
    "vector", "temporal_qdrant", "truth_certification",
)
E2E_VERIFY_SHARDS = (
    "qdrant_knowledge", "pg_app_truth", "pg_app_cms", "pg_app_planning",
    "pg_app_delivery", "temporal_draft", "temporal_pure", "verify_d", "outbox",
)
# The business-suite source owner (architecture/verify) is orthogonal to its
# physical Cargo compiler and independent provider runner. Each shard belongs
# to exactly one runtime job: none may wait for a broad harness dependency.
E2E_GRAPH_SHARDS = ("graph",)
E2E_QDRANT_SHARDS = ("vector", "qdrant_knowledge")
E2E_QDRANT_TEMPORAL_SHARDS = ("temporal_qdrant",)
E2E_PG_APP_SHARDS = ("pg_app_truth", "pg_app_cms", "pg_app_planning", "pg_app_delivery")
E2E_TEMPORAL_LEGACY_SHARDS = ("temporal_manual", "temporal_blocked", "temporal_draft")
E2E_TEMPORAL_V3_SHARDS = ("temporal_pure",)
E2E_PG_SHARDS = ("pg_temporal", "verify_d")
E2E_OUTBOX_SHARDS = ("outbox",)
E2E_TRUTH_SHARDS = ("truth_certification",)
E2E_RUNNER_PARTITIONS = {
    "rust-graph-e2e": E2E_GRAPH_SHARDS,
    "rust-qdrant-e2e": E2E_QDRANT_SHARDS,
    "rust-qdrant-temporal-e2e": E2E_QDRANT_TEMPORAL_SHARDS,
    "rust-pg-app-e2e": E2E_PG_APP_SHARDS,
    "rust-temporal-legacy-e2e": E2E_TEMPORAL_LEGACY_SHARDS,
    "rust-temporal-v3-e2e": E2E_TEMPORAL_V3_SHARDS,
    "rust-pg-e2e": E2E_PG_SHARDS,
    "rust-outbox-e2e": E2E_OUTBOX_SHARDS,
    "rust-truth-e2e": E2E_TRUTH_SHARDS,
}
if (set().union(*(set(shards) for shards in E2E_RUNNER_PARTITIONS.values()))
        != set(E2E_ARCH_SHARDS) | set(E2E_VERIFY_SHARDS)
        or sum(map(len, E2E_RUNNER_PARTITIONS.values())) != len(E2E_ARCH_SHARDS) + len(E2E_VERIFY_SHARDS)):
    raise ValueError("independent E2E runner partition lost a required shard")
E2E_SHARD_RUNS: dict[str, tuple[tuple[str, str, int], ...]] = {
    "graph": (("graph_query", "graph_capability_", 2),),
    "pg_temporal": (("pg_temporal_v3", "temporal_effect_v3_", 1),),
    "temporal_manual": (
        ("temporal_restart", "canonical_temporal_manual_editorial_resumes_after_worker_restart", 1),
        ("temporal_restart", "canonical_temporal_activity_history_wire_shape", 1),
    ),
    "temporal_blocked": (
        ("temporal_restart", "canonical_temporal_blocked_review_ends_for_rework_without_publication", 1),
    ),
    "vector": (
        ("vector_projection", "qdrant_rest_outbox_upsert_is_visible_to_grpc_retrieval", 1),
        ("vector_admitted_projection", "admitted_qdrant_keyword_projection_recovers_after_receipt_failure_without_second_rpc", 1),
    ),
    "temporal_qdrant": (
        ("temporal_qdrant_v8", "qdrant_v8_native_temporal_history_operator_real_effect", 1),
        ("temporal_mixed_v6_contract", "mixed_v6_", 3),
        ("temporal_mixed_v7_produced", "mixed_v7_", 2),
        ("temporal_mixed_v2_native", "mixed_v2_native_", 1),
    ),
    "truth_certification": (
        ("truth_certification", "truth_certification_suite_matches_fixture_expectations", 1),
    ),
    "temporal_draft": (("draft_qa_scenario", "draft_qa_synthetic_full_scenario_persists_page_draft_on_full_harness", 1),),
    "temporal_pure": (("pure_dag_v3", "pure_v3_", 1),),
    "qdrant_knowledge": (("knowledge_a13", "knowledge_a13_", 3),),
    "pg_app_truth": (("truth_adjudication_integrity", "truth_adjudication_", 3),),
    "pg_app_cms": (("cms_review_revision", "cms_revision_", 4),),
    "pg_app_planning": (("planning_atomicity", "planning_atomic_", 6),),
    "pg_app_delivery": (
        ("draft_qa_pg_persistence", "draft_qa_persistence_rolls_back_late_failure_and_rejects_missing_draft", 1),
        ("publishing_a17", "publishing_a17_postgres_outbox_http_faults_and_recovery", 1),
    ),
    "outbox": (("outbox_a14", "outbox_a14_", 9),),
    # All prior V3/V4/V5 tests plus V6 real native LOGIN/tenant ingress.
    "verify_d": (("pg_effect_v3", "pg_v3_", 10),),
}


def e2e_shard_commands_for_report(
    report: dict[str, Any],
) -> dict[str, list[tuple[str, list[str], str, int]]]:
    """Exact-source, exhaustive full-CI shard plan; empty shards are NOT runnable.

    The caller publishes only nonempty shards to a GitHub matrix. Each
    selected suite and its actual test-name filters must be exercised once,
    with a known exact test count. Unknown/new suites fail the planning job
    until their test coverage is explicitly allocated.
    """
    if set(E2E_ARCH_SHARDS) | set(E2E_VERIFY_SHARDS) != set(E2E_SHARD_RUNS) or (
        set(E2E_ARCH_SHARDS) & set(E2E_VERIFY_SHARDS)
    ):
        raise ValueError("invalid unique full-scope E2E shard identity registry")
    if report.get("analysis") != "full" or report.get("scope") != "full":
        raise ValueError("full-scope E2E shard plan used by a non-full CI run")
    assignments = suite_commands_by_ci_job(report)
    expected_assignments = {
        "architecture-and-rust": dict(assignments["architecture-and-rust"]),
        "verify": dict(assignments["verify"]),
    }
    result: dict[str, list[tuple[str, list[str], str, int]]] = {}
    coverage: dict[str, list[str]] = {}
    for shard, specs in E2E_SHARD_RUNS.items():
        owner = "architecture-and-rust" if shard in E2E_ARCH_SHARDS else "verify"
        rows: list[tuple[str, list[str], str, int]] = []
        for name, test_filter, count in specs:
            if name not in SUITE_OWNERS or not isinstance(count, int) or count <= 0:
                raise ValueError(f"invalid E2E source suite/count in shard {shard}")
            command = list(SUITE_OWNERS[name]["command"])
            try:
                original_filter = command[command.index("--lib") + 1]
            except (ValueError, IndexError) as error:
                raise ValueError(f"missing canonical Rust lib filter for {name}") from error
            if not test_filter.startswith(original_filter):
                raise ValueError(f"shard {shard} illegally widens canonical filter for {name}")
            coverage.setdefault(name, []).append(test_filter)
            if name in expected_assignments[owner]:
                if command != expected_assignments[owner][name]:
                    raise ValueError(f"shard {shard} changed source suite {name} command")
                rows.append((name, command, test_filter, count))
            elif name in expected_assignments["verify" if owner == "architecture-and-rust" else "architecture-and-rust"]:
                raise ValueError(f"source suite {name} selected under wrong E2E CI owner")
        result[shard] = rows

    if set(coverage) != set(SUITE_OWNERS):
        raise ValueError("full-scope shard registry does not cover every source suite")
    for name, filters in coverage.items():
        if name == "temporal_restart":
            expected = {
                "canonical_temporal_manual_editorial_resumes_after_worker_restart",
                "canonical_temporal_blocked_review_ends_for_rework_without_publication",
                "canonical_temporal_activity_history_wire_shape",
            }
            if set(filters) != expected or len(filters) != len(expected):
                raise ValueError("historical Temporal restart tests have incomplete shard coverage")
        elif len(filters) != 1 or filters[0] != SUITE_OWNERS[name]["command"][7]:
            raise ValueError(f"suite {name} has missing, duplicate or rewritten E2E filter")
    actual = {
        "architecture-and-rust": {name for shard in E2E_ARCH_SHARDS for name, *_ in result[shard]},
        "verify": {name for shard in E2E_VERIFY_SHARDS for name, *_ in result[shard]},
    }
    for owner, commands in expected_assignments.items():
        if actual[owner] != set(commands):
            raise ValueError(f"full-scope E2E shard partition lost a mandatory {owner} suite")
    return result


def suite_commands_by_ci_job(report: dict[str, Any]) -> dict[str, list[tuple[str, list[str]]]]:
    """Partition the *same selected suites* between existing PR owners exactly once.

    Impact still owns every suite's executable command and selection. This
    function only chooses where the already-mandatory command is executed.
    """
    commands = suite_commands_for_report(report)
    scope = report.get("analysis")
    if scope not in {"none", "scoped", "full"} or report.get("scope", scope) != scope:
        raise ValueError("invalid exact-source Cargo scope for suite assignment")
    selected = report.get("selected_ci_jobs")
    if (not isinstance(selected, list)
            or any(not isinstance(job, str) or job not in CI_PR_OWNERS for job in selected)
            or len(selected) != len(set(selected))):
        raise ValueError("missing selected CI owners for suites")
    if scope == "full":
        required = {"architecture-and-rust", "verify"}
    elif commands:
        required = {"architecture-and-rust"}
    else:
        required = set()
    if not required.issubset(selected):
        raise ValueError("selected suite CI owner is absent from the mandatory gate")
    assignments: dict[str, list[tuple[str, list[str]]]] = {
        "architecture-and-rust": [], "verify": [],
    }
    for name, command in commands:
        job = "verify" if scope == "full" and name in FULL_VERIFY_E2E_SUITES else "architecture-and-rust"
        assignments[job].append((name, command))
    return assignments


def parse_name_status(raw: bytes) -> list[str]:
    """Decode a NUL-delimited Git diff, keeping both rename paths and deletes."""
    fields = raw.decode("utf-8", errors="surrogateescape").split("\0")
    paths: set[str] = set()
    i = 0
    while i < len(fields):
        status = fields[i]
        i += 1
        if not status:
            continue
        if status[0] not in "ACDMRTUXB":
            raise ValueError(f"unsupported git diff status: {status!r}")
        count = 2 if status[0] in "RC" else 1
        if i + count > len(fields):
            raise ValueError("incomplete NUL-delimited git diff")
        for path in fields[i : i + count]:
            if not path or path.startswith("/") or ".." in Path(path).parts:
                raise ValueError(f"unsafe changed path: {path!r}")
            paths.add(path)
        i += count
    return sorted(paths)


def changed_paths(base: str, head: str, *, root: Path = ROOT) -> list[str]:
    """Return BOTH names in renames/copies, including deleted paths."""
    raw = subprocess.check_output(
        ["git", "diff", "--name-status", "-z", "--find-renames", base, head, "--"],
        cwd=root,
    )
    return parse_name_status(raw)


def changed_worktree_paths(base: str, *, root: Path = ROOT) -> list[str]:
    """Include staged, unstaged, renamed, deleted and not-yet-tracked local files."""
    raw = subprocess.check_output(
        ["git", "diff", "--name-status", "-z", "--find-renames", base, "--"],
        cwd=root,
    )
    tracked = set(parse_name_status(raw))
    untracked = subprocess.check_output(
        ["git", "ls-files", "--others", "--exclude-standard", "-z", "--"],
        cwd=root,
    )
    for path in untracked.decode("utf-8", errors="surrogateescape").split("\0"):
        if path:
            if path.startswith("/") or ".." in Path(path).parts:
                raise ValueError(f"unsafe untracked path: {path!r}")
            tracked.add(path)
    return sorted(tracked)


LockPackage = tuple[str, str, str]  # name, version, source (empty for local packages)
LockSignature = tuple[str, tuple[str, ...]]  # checksum, resolved dependency entries


def parsed_lock_graph(
    contents: str,
) -> tuple[int, dict[LockPackage, LockSignature], dict[LockPackage, set[LockPackage]]]:
    """Read the *resolved* Cargo.lock graph, including external package edges.

    Unqualified dependencies can reference more than one locked version. In
    that case all matching versions are followed rather than guessing one.
    """
    document = tomllib.loads(contents)
    version = document.get("version")
    raw_packages = document.get("package")
    if version not in (3, 4) or not isinstance(raw_packages, list) or not raw_packages:
        raise ValueError("unsupported or empty Cargo.lock graph")

    packages: dict[LockPackage, LockSignature] = {}
    by_name: dict[str, set[LockPackage]] = {}
    for item in raw_packages:
        if not isinstance(item, dict):
            raise ValueError("invalid Cargo.lock package")
        name, package_version, source = item.get("name"), item.get("version"), item.get("source", "")
        checksum, dependencies = item.get("checksum", ""), item.get("dependencies", [])
        if (not isinstance(name, str) or not name or not isinstance(package_version, str)
                or not package_version or not isinstance(source, str) or not isinstance(checksum, str)
                or not isinstance(dependencies, list)
                or any(not isinstance(dep, str) or not dep for dep in dependencies)):
            raise ValueError("incomplete Cargo.lock package identity or dependencies")
        key: LockPackage = (name, package_version, source)
        if key in packages:
            raise ValueError(f"duplicate Cargo.lock package: {key}")
        packages[key] = (checksum, tuple(sorted(dependencies)))
        by_name.setdefault(name, set()).add(key)

    reverse: dict[LockPackage, set[LockPackage]] = {key: set() for key in packages}
    for dependent, (_, dependencies) in packages.items():
        for dependency in dependencies:
            fields = dependency.split(" ", 2)
            if len(fields) == 3 and not (fields[2].startswith("(") and fields[2].endswith(")")):
                raise ValueError(f"invalid Cargo.lock dependency: {dependency}")
            candidates = {
                key for key in by_name.get(fields[0], set())
                if (len(fields) < 2 or key[1] == fields[1])
                and (len(fields) < 3 or key[2] == fields[2][1:-1])
            }
            if not candidates:
                raise ValueError(f"unresolved Cargo.lock dependency: {dependent} -> {dependency}")
            for target in candidates:
                reverse[target].add(dependent)
    return version, packages, reverse


def affected_crates_for_lock_change(
    before: str, after: str, workspace_crates: set[str],
) -> tuple[set[str], list[str]]:
    """Use both resolved lock graphs so removals retain their old consumers."""
    old_version, old_packages, old_reverse = parsed_lock_graph(before)
    new_version, new_packages, new_reverse = parsed_lock_graph(after)
    if old_version != new_version:
        raise ValueError("Cargo.lock format version changed")
    changed = {
        key for key in old_packages.keys() | new_packages.keys()
        if old_packages.get(key) != new_packages.get(key)
    }
    if not changed:
        raise ValueError("Cargo.lock changed without a resolved package change")

    affected: set[str] = set()
    for packages, reverse in ((old_packages, old_reverse), (new_packages, new_reverse)):
        pending = deque(sorted(changed & packages.keys()))
        seen: set[LockPackage] = set()
        while pending:
            package = pending.popleft()
            if package in seen:
                continue
            seen.add(package)
            if not package[2] and package[0] in workspace_crates:
                affected.add(package[0])
            pending.extend(sorted(reverse[package] - seen))
    if not affected:
        raise ValueError("changed locked packages have no known workspace consumers")
    names = sorted({key[0] for key in changed})
    return affected, ["Cargo.lock resolved package change: " + ", ".join(names)]


def git_lock_contents(ref: str, *, root: Path = ROOT) -> str:
    return subprocess.check_output(
        ["git", "show", f"{ref}:app/rust/Cargo.lock"], cwd=root, text=True,
    )


def load_cargo_metadata(*, workspace: Path = WORKSPACE) -> dict[str, Any]:
    output = subprocess.check_output(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--no-deps",
            "--manifest-path",
            str(workspace / "Cargo.toml"),
        ],
        cwd=workspace,
        text=True,
    )
    return json.loads(output)


def workspace_graph(
    metadata: dict[str, Any], *, root: Path = ROOT
) -> tuple[dict[str, str], dict[str, set[str]]]:
    """Map local manifest directories to package names and reverse edges.

    Conservatively include normal, build, dev and optional edges. A workspace
    feature can enable a dependency later; omitting it would be a false-negative.
    """
    members = set(metadata["workspace_members"])
    packages = [pkg for pkg in metadata["packages"] if pkg["id"] in members]
    if not packages or len(packages) != len(members):
        raise ValueError("incomplete cargo metadata workspace member set")

    manifest_owner: dict[str, str] = {}
    by_absolute_manifest: dict[Path, str] = {}
    for pkg in packages:
        manifest = Path(pkg["manifest_path"]).resolve()
        directory = manifest.parent.relative_to(root.resolve()).as_posix()
        if pkg["name"] in manifest_owner.values() or directory in manifest_owner:
            raise ValueError("ambiguous workspace package name or directory")
        manifest_owner[directory] = pkg["name"]
        by_absolute_manifest[manifest] = pkg["name"]

    reverse = {name: set() for name in manifest_owner.values()}
    for pkg in packages:
        for dep in pkg["dependencies"]:
            if not dep.get("path"):
                continue
            dependent = by_absolute_manifest.get(
                (Path(dep["path"]) / "Cargo.toml").resolve()
            )
            if dependent is not None:
                reverse[dependent].add(pkg["name"])
    return manifest_owner, reverse


def declared_workspace_owners(
    *, root: Path = ROOT,
) -> tuple[dict[str, str], dict[str, set[str]]]:
    """Enumerate every member for a full fallback if Cargo metadata fails.

    No dependency edges are inferred here: the caller MUST force full scope.
    This lets a broken lockfile produce a visible fail-closed impact report.
    """
    workspace = root / "app/rust"
    manifest = tomllib.loads((workspace / "Cargo.toml").read_text(encoding="utf-8"))
    members = manifest.get("workspace", {}).get("members")
    if not isinstance(members, list) or not members:
        raise ValueError("workspace members cannot be recovered")
    owners: dict[str, str] = {}
    for member in members:
        if not isinstance(member, str) or Path(member).is_absolute() or ".." in Path(member).parts:
            raise ValueError("invalid workspace member in fallback")
        manifest_path = workspace / member / "Cargo.toml"
        package = tomllib.loads(manifest_path.read_text(encoding="utf-8")).get("package", {})
        name = package.get("name")
        if not isinstance(name, str) or not name or name in owners.values():
            raise ValueError("ambiguous workspace member in fallback")
        owners[manifest_path.parent.relative_to(root).as_posix()] = name
    return owners, {name: set() for name in owners.values()}


def analyze(
    paths: list[str],
    owners: dict[str, str],
    reverse: dict[str, set[str]],
    *,
    head: str,
    base: str,
    lock_snapshots: tuple[str, str] | None = None,
    forced_full_reasons: tuple[str, ...] = (),
) -> dict[str, Any]:
    """Build an auditable, no-false-negative conservative affected closure."""
    reason: dict[str, set[str]] = {name: set() for name in reverse}
    direct: set[str] = set()
    broad_reasons: list[str] = list(forced_full_reasons)
    classified_inputs: dict[str, str] = {}
    lock_changed = False

    for path in sorted(set(paths)):
        if path in WORKER_NONDELIVERY_FILES:
            classified_inputs[path] = "non-delivery documentation: excluded from Worker context"
            continue
        if path in DOCUMENTATION_FILES or (path.startswith(DOCUMENTATION_PREFIXES) and path.endswith(".md")):
            classified_inputs[path] = ("source-policy Markdown: no Rust compile or image input"
                                       if path in SOURCE_POLICY_DOCUMENTS
                                       else "documentation: no executable Rust or image input")
            continue
        if path in AUTOMATION_SOURCE_DOCUMENTS:
            classified_inputs[path] = "reviewed automation source-policy Markdown: no Rust or Worker input"
            continue
        if path.startswith("docs/runs/") and path.endswith(".json"):
            classified_inputs[path] = "historical run evidence: not a live test input or a Rust regression"
            continue
        if path.startswith(TRUTH_FIXTURE_PREFIX):
            # Runtime files are opened by the e2e harness, not compiled via
            # include_str!. The selected real suite compiles the required Rust
            # graph; an extra Cargo check/test is duplicate work.
            classified_inputs[path] = "live Truth runtime fixture: consumed by selected e2e suite"
            continue
        if path == "app/rust/Cargo.lock":
            lock_changed = True
            classified_inputs[path] = "resolved Cargo dependency graph: base and head"
            continue
        if path == KNOWN_CONTRACT_MANIFEST:
            # This checked-in generator output contains byte-level hashes of
            # the live proto inputs. A genuine proto change must update it;
            # that paired update is not an unrelated CI-planner modification.
            if set(paths) & KNOWN_CONTRACT_CRATES.keys() and "contracts" in reason:
                direct.add("contracts")
                reason["contracts"].add("generated contract inventory for changed known proto")
                classified_inputs[path] = "generated manifest paired with known protobuf source"
            else:
                broad_reasons.append("global: unpaired generated contract inventory")
            continue
        if path in KNOWN_CONTRACT_CRATES:
            owner = KNOWN_CONTRACT_CRATES[path]
            if owner not in reason:
                broad_reasons.append(f"unknown generated contract owner: {owner}")
            else:
                direct.add(owner)
                reason[owner].add(f"known generated contract: {path}")
                classified_inputs[path] = f"known protobuf generator owner: {owner}"
            continue
        if path in WORKER_PACKAGING_FILES or path.startswith("infra/cloudflare/seo-static/"):
            classified_inputs[path] = "known non-Rust input: temporal-worker image"
            continue
        if path in CI_ONLY_VERIFY_FILES:
            classified_inputs[path] = "known CI-only input: mandatory verify contract, no Rust compilation"
            continue
        if path.startswith(("infra/analytics_lab/", "app/analytics_lab/")):
            classified_inputs[path] = "known analytics-lab source/config: verified outside Rust"
            continue
        if path.endswith("/Dockerfile") and any(path in rule["paths"] for rule in IMAGE_OWNERS.values()):
            classified_inputs[path] = "known image Dockerfile: packaging only"
            continue
        if path in GLOBAL_FILES or path.startswith(GLOBAL_PREFIXES):
            broad_reasons.append(f"global input: {path}")
            continue
        matches = [(prefix, name) for prefix, name in owners.items() if path.startswith(prefix + "/")]
        if len(matches) != 1:
            broad_reasons.append(f"unmapped or ambiguous input: {path}")
            continue
        _, name = matches[0]
        direct.add(name)
        reason[name].add(f"changed: {path}")

    if lock_changed and not broad_reasons:
        try:
            before, after = lock_snapshots or (git_lock_contents(base), git_lock_contents(head))
            locked_crates, lock_reasons = affected_crates_for_lock_change(before, after, set(reason))
            for name in locked_crates:
                direct.add(name)
                reason[name].update(lock_reasons)
        except (ValueError, OSError, subprocess.CalledProcessError, UnicodeError, TypeError, KeyError) as error:
            broad_reasons.append(f"full: unresolved Cargo.lock consumer graph ({type(error).__name__}: {error})")

    if broad_reasons:
        affected = set(reverse)
        for name in affected:
            reason[name].add("fail-closed: common/unmapped input")
    else:
        affected = set(direct)
        queue = deque(sorted(direct))
        while queue:
            dependency = queue.popleft()
            for dependent in sorted(reverse[dependency]):
                reason[dependent].add(f"depends on: {dependency}")
                if dependent not in affected:
                    affected.add(dependent)
                    queue.append(dependent)

    services = sorted(
        name for directory, name in owners.items()
        if directory.startswith("app/rust/services/") and name in affected
    )
    scope = "full" if broad_reasons else "scoped" if affected else "none"

    suite_reasons: dict[str, list[str]] = {}
    shared_provider_fixture_changed = any(
        path.startswith("app/rust/crates/integration_data_fixtures/") for path in paths
    )
    for name, rule in SUITE_OWNERS.items():
        matched = [path for path in sorted(set(paths)) if path in rule["paths"] or path.startswith(rule["prefixes"])]
        # Both executable owners consume the same real provider fixtures;
        # changes to that common setup require ALL mandatory 38 live checks,
        # including architecture shards otherwise optional for global inputs.
        if shared_provider_fixture_changed and name != "truth_certification":
            matched.append("shared real provider fixture consumed by all mandatory E2E owners")
        if lock_changed and not broad_reasons:
            matched.extend(
                f"locked dependency affects: {crate}"
                for crate in sorted(rule.get("lock_crates", set()) & affected)
            )
        # Long service-specific E2E suites follow their known live owners; a
        # planner edit does not silently claim that they were run. The broad
        # Rust gate plus core A13/A14/A17 remain mandatory on unknown inputs.
        if broad_reasons and rule["on_unknown_global_input"]:
            matched = ["fail-closed: common/unmapped input"] + matched
        if matched:
            suite_reasons[name] = matched

    image_reasons: dict[str, list[str]] = {}
    for name, rule in IMAGE_OWNERS.items():
        matched = [f"affected build crate: {crate}" for crate in sorted(rule["crates"] & affected)]
        matched.extend(
            f"packaged input: {path}" for path in sorted(set(paths))
            if path not in WORKER_NONDELIVERY_FILES
            and (path in rule["paths"] or path.startswith(rule["prefixes"]))
        )
        if broad_reasons:
            matched.insert(0, "fail-closed: common/unmapped input")
        if matched:
            image_reasons[name] = matched

    suite_fixture_ids: dict[str, list[str]] = {}
    suite_fixture_reasons: dict[str, list[str]] = {}
    if "truth_certification" in suite_reasons:
        ids, reasons = truth_fixture_ids_for_paths(paths)
        suite_fixture_reasons["truth_certification"] = reasons
        if ids is not None:
            suite_fixture_ids["truth_certification"] = ids
        for name, rule in SUITE_OWNERS.items():
            consuming_ids = rule.get("fixture_ids", set())
            if consuming_ids and (ids is None or set(ids) & consuming_ids):
                suite_reasons.setdefault(name, []).append(
                    "live Truth input also feeds this E2E: " + ", ".join(sorted(consuming_ids))
                )

    ci_job_reasons = ci_jobs_for_change(paths, scope, set(suite_reasons), set(image_reasons))

    return {
        "schema_version": SCHEMA_VERSION,
        "base": base,
        "source_sha": head,
        "base_sha": base,
        "head_sha": head,
        "changed_paths": sorted(set(paths)),
        "scope": scope,
        "analysis": scope,
        "full_scope_reasons": sorted(broad_reasons),
        "classified_inputs": classified_inputs,
        "direct_crates": sorted(direct),
        "affected_crates": sorted(affected),
        "affected_services": services,
        "reasons": {pkg: sorted(reason[pkg]) for pkg in sorted(affected)},
        "selected_suites": sorted(suite_reasons),
        "suite_reasons": suite_reasons,
        "suite_fixture_ids": suite_fixture_ids,
        "suite_fixture_reasons": suite_fixture_reasons,
        "selected_images": sorted(image_reasons),
        "image_status": {
            name: "selected" if name in image_reasons else "not_applicable"
            for name in sorted(IMAGE_OWNERS)
        },
        "image_reasons": image_reasons,
        "selected_ci_jobs": sorted(ci_job_reasons),
        "ci_job_reasons": ci_job_reasons,
        "ci_job_status": {
            name: "required" if name in ci_job_reasons else "not_applicable"
            for name in CI_PR_OWNERS
        },
        # The aggregate must prove the selected-versus-skipped owner statuses.
        "mandatory_full_ci_gate": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, help="Fetched PR base commit SHA")
    parser.add_argument("--head", required=True, help="Exact CI commit SHA")
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()

    paths = changed_paths(args.base, args.head)
    forced_full_reasons: tuple[str, ...] = ()
    try:
        owners, reverse = workspace_graph(load_cargo_metadata())
    except (OSError, subprocess.CalledProcessError, ValueError, KeyError, TypeError) as error:
        if "app/rust/Cargo.lock" not in paths:
            raise
        owners, reverse = declared_workspace_owners()
        forced_full_reasons = (f"full: Cargo metadata unavailable ({type(error).__name__}: {error})",)
    report = analyze(
        paths, owners, reverse, base=args.base, head=args.head,
        forced_full_reasons=forced_full_reasons,
    )
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(
        f"RUST_IMPACT={report['analysis']} "
        f"direct={','.join(report['direct_crates']) or '-'} "
        f"affected={','.join(report['affected_crates']) or '-'} "
        f"report={args.report}"
    )
    for why in report["full_scope_reasons"]:
        print(f"  FULL_SCOPE: {why}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
