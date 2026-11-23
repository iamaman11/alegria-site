#!/usr/bin/env python3
"""Offline regression fixtures for the Rust dependency impact analyzer."""

from __future__ import annotations

import unittest
import json
from tempfile import TemporaryDirectory
from pathlib import Path
from unittest.mock import patch

from ci_impact import (
    CI_PR_OWNERS, SUITE_OWNERS, TRUTH_FIXTURE_PREFIX, analyze, changed_paths,
    affected_crates_for_lock_change, declared_workspace_owners, parsed_lock_graph,
    suite_commands_for_report, suite_environment_for_report, truth_fixture_ids_for_paths,
    workspace_graph,
)
from dev_rust_feedback import commands_for_report
from run_scoped_cargo_feedback import require_clean_truth_filter_environment


class ImpactFixtures(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path("/synthetic/repo")
        self.owners = {
            "app/rust/crates/contracts": "contracts",
            "app/rust/crates/primitives": "primitives",
            "app/rust/crates/graph_neo4j": "graph_neo4j",
            "app/rust/crates/http_client": "http_client",
            "app/rust/crates/infrastructure": "infrastructure",
            "app/rust/crates/integration_harness": "integration_harness",
            "app/rust/crates/seo_application": "seo_application",
            "app/rust/crates/seo_domain": "seo_domain",
            "app/rust/crates/seo_ports": "seo_ports",
            "app/rust/crates/seo_steps": "seo_steps",
            "app/rust/crates/vector_qdrant": "vector_qdrant",
            "app/rust/services/gsc_sync": "gsc_sync",
            "app/rust/services/cli_tools": "cli_tools",
            "app/rust/services/analytics_svc": "analytics_svc",
            "app/rust/services/temporal": "temporal_worker",
            "app/rust/services/outbox_worker": "outbox_worker",
        }
        self.reverse = {
            "contracts": {"primitives"},
            "primitives": {"temporal_worker"},
            "graph_neo4j": {"analytics_svc", "temporal_worker"},
            "http_client": {"gsc_sync"},
            "infrastructure": set(),
            "integration_harness": set(),
            "seo_application": set(),
            "seo_domain": {"seo_application", "seo_ports"},
            "seo_ports": set(),
            "seo_steps": {"seo_application", "temporal_worker"},
            "vector_qdrant": {"temporal_worker"},
            "gsc_sync": set(),
            "cli_tools": set(),
            "analytics_svc": set(),
            "temporal_worker": set(),
            "outbox_worker": set(),
        }

    def impact(self, *paths: str, lock_snapshots: tuple[str, str] | None = None) -> dict:
        return analyze(
            list(paths), self.owners, self.reverse, head="HEAD", base="BASE",
            lock_snapshots=lock_snapshots,
        )

    @staticmethod
    def lock(*packages: tuple[str, str, tuple[str, ...]]) -> str:
        """Minimal real-format lockfiles, including local workspace packages."""
        result = ["version = 4"]
        for name, version, dependencies in packages:
            result.extend(("", "[[package]]", f"name = {json.dumps(name)}", f"version = {json.dumps(version)}"))
            if version != "0.1.0":
                result.append('source = "registry+https://github.com/rust-lang/crates.io-index"')
            result.append(f"dependencies = {json.dumps(list(dependencies))}")
        return "\n".join(result) + "\n"

    def test_lock_only_temporal_dependency_is_consumer_aware(self) -> None:
        old = self.lock(("temporal_worker", "0.1.0", ("temporalio-client 1.0.0",)),
                        ("temporalio-client", "1.0.0", ()))
        new = self.lock(("temporal_worker", "0.1.0", ("temporalio-client 1.1.0",)),
                        ("temporalio-client", "1.1.0", ()))
        impact = self.impact("app/rust/Cargo.lock", lock_snapshots=(old, new))
        self.assertEqual(impact["scope"], "scoped")
        self.assertEqual(impact["affected_crates"], ["temporal_worker"])
        self.assertEqual(impact["selected_suites"], ["temporal_restart"])
        self.assertEqual(impact["selected_images"], ["temporal-worker"])
        self.assertNotIn("stack-storage-graph", impact["selected_ci_jobs"])
        self.assertIn("Cargo.lock", str(impact["reasons"]["temporal_worker"]))

    def test_lock_shared_external_dependency_includes_all_consumers(self) -> None:
        old = self.lock(("temporal_worker", "0.1.0", ("shared 1.0.0",)),
                        ("outbox_worker", "0.1.0", ("shared 1.0.0",)),
                        ("shared", "1.0.0", ()))
        new = self.lock(("temporal_worker", "0.1.0", ("shared 1.1.0",)),
                        ("outbox_worker", "0.1.0", ("shared 1.1.0",)),
                        ("shared", "1.1.0", ()))
        impact = self.impact("app/rust/Cargo.lock", lock_snapshots=(old, new))
        self.assertEqual(set(impact["affected_crates"]), {"temporal_worker", "outbox_worker"})
        self.assertEqual(set(impact["selected_images"]), {"temporal-worker", "outbox-worker"})
        self.assertEqual(set(impact["selected_suites"]), {"temporal_restart", "outbox_a14"})

    def test_removed_lock_dependency_uses_old_graph(self) -> None:
        old = self.lock(("temporal_worker", "0.1.0", ("removed 1.0.0",)),
                        ("removed", "1.0.0", ()))
        new = self.lock(("temporal_worker", "0.1.0", ()))
        self.assertEqual(affected_crates_for_lock_change(old, new, set(self.reverse))[0], {"temporal_worker"})
        impact = self.impact("app/rust/Cargo.lock", lock_snapshots=(old, new))
        self.assertEqual(impact["selected_suites"], ["temporal_restart"])

    def test_manifest_and_lock_change_do_not_force_full(self) -> None:
        old = self.lock(("temporal_worker", "0.1.0", ("sdk 1.0.0",)), ("sdk", "1.0.0", ()))
        new = self.lock(("temporal_worker", "0.1.0", ("sdk 1.1.0",)), ("sdk", "1.1.0", ()))
        impact = self.impact("app/rust/Cargo.lock", "app/rust/services/temporal/Cargo.toml",
                             lock_snapshots=(old, new))
        self.assertEqual(impact["scope"], "scoped")
        self.assertEqual(impact["affected_crates"], ["temporal_worker"])

    def test_known_protobuf_contract_tracks_workspace_and_live_consumers(self) -> None:
        result = self.impact("app/contracts/proto/temporal_payloads.proto")
        self.assertEqual(result["scope"], "scoped")
        self.assertEqual(set(result["affected_crates"]), {"contracts", "primitives", "temporal_worker"})
        self.assertEqual(result["selected_suites"], ["temporal_restart"])
        self.assertIn("product-truth", result["selected_ci_jobs"])

    def test_known_proto_with_generated_inventory_stays_scoped(self) -> None:
        result = self.impact(
            "app/contracts/proto/temporal_payloads.proto", "automation/PROTO_CONTRACTS.md",
        )
        self.assertEqual(result["scope"], "scoped")
        self.assertEqual(set(result["affected_crates"]), {"contracts", "primitives", "temporal_worker"})
        self.assertEqual(result["selected_suites"], ["temporal_restart"])
        self.assertIn("generated manifest", result["classified_inputs"]["automation/PROTO_CONTRACTS.md"])

    def test_orphan_or_unknown_generated_contract_inventory_fails_closed(self) -> None:
        for paths in (
            ("automation/PROTO_CONTRACTS.md",),
            ("automation/PROTO_CONTRACTS.md", "app/contracts/proto/unknown.proto"),
            ("automation/PROTO_CONTRACTS.md", "app/contracts/proto/temporal_payloads.proto",
             "automation/gen_proto_contracts.py"),
        ):
            with self.subTest(paths=paths):
                self.assertEqual(self.impact(*paths)["scope"], "full")

    def test_unknown_schema_contract_and_toolchain_fail_closed(self) -> None:
        for path in ("app/contracts/proto/unknown.proto", "app/db/new_schema.sql", "app/rust/rust-toolchain.toml"):
            with self.subTest(path=path):
                self.assertEqual(self.impact(path)["scope"], "full")

    def test_invalid_missing_or_ambiguous_lock_graph_fails_closed(self) -> None:
        valid = self.lock(("temporal_worker", "0.1.0", ("sdk 1.0.0",)), ("sdk", "1.0.0", ()))
        invalid = ("[[package]]\nname = ", self.lock(("temporal_worker", "0.1.0", ("missing",))),
                   self.lock(("sdk", "1.0.0", ()), ("sdk", "1.0.0", ())))
        for before in invalid:
            with self.subTest(before=before[:35]):
                impact = self.impact("app/rust/Cargo.lock", lock_snapshots=(before, valid))
                self.assertEqual(impact["scope"], "full")
                self.assertTrue(impact["full_scope_reasons"])

    def test_no_resolved_lock_change_is_not_mistaken_for_no_impact(self) -> None:
        locked = self.lock(("temporal_worker", "0.1.0", ()))
        impact = self.impact("app/rust/Cargo.lock", lock_snapshots=(locked, locked))
        self.assertEqual(impact["scope"], "full")

    def test_real_workspace_lock_graph_is_complete(self) -> None:
        lock = (Path(__file__).resolve().parents[1] / "app/rust/Cargo.lock").read_text(encoding="utf-8")
        version, packages, reverse = parsed_lock_graph(lock)
        self.assertEqual(version, 4)
        self.assertGreater(len(packages), len(self.owners))
        self.assertTrue(any(dependents for dependents in reverse.values()))

    def test_locked_checksum_change_selects_same_consumers(self) -> None:
        old = self.lock(("temporal_worker", "0.1.0", ("sdk 1.0.0",)), ("sdk", "1.0.0", ()))
        new = old.replace('dependencies = []', 'checksum = "changed"\ndependencies = []')
        result = self.impact("app/rust/Cargo.lock", lock_snapshots=(old, new))
        self.assertEqual(result["affected_crates"], ["temporal_worker"])

    def test_cargo_metadata_failure_forces_full_and_preserves_all_crates(self) -> None:
        owners, reverse = declared_workspace_owners()
        self.assertGreaterEqual(len(owners), 20)
        result = analyze(
            ["app/rust/Cargo.lock"], owners, reverse, base="BASE", head="HEAD",
            forced_full_reasons=("full: cargo metadata failure",),
        )
        self.assertEqual(result["scope"], "full")
        self.assertEqual(set(result["affected_crates"]), set(reverse))
        self.assertEqual(set(result["selected_ci_jobs"]), set(CI_PR_OWNERS))

    def test_isolated_gsc_does_not_trigger_other_services(self) -> None:
        result = self.impact("app/rust/services/gsc_sync/src/main.rs")
        self.assertEqual(result["affected_crates"], ["gsc_sync"])
        self.assertTrue(result["mandatory_full_ci_gate"])
        self.assertEqual(result["selected_suites"], [])
        self.assertEqual(result["selected_images"], [])

    def test_temporal_source_selects_real_restart_e2e_and_worker_image(self) -> None:
        result = self.impact("app/rust/services/temporal/src/main.rs")
        self.assertEqual(result["affected_crates"], ["temporal_worker"])
        self.assertEqual(result["selected_suites"], ["temporal_restart"])
        self.assertEqual(result["selected_images"], ["temporal-worker"])
        selected = SUITE_OWNERS[result["selected_suites"][0]]
        self.assertEqual(selected["prepare"][:3], ("cargo", "build", "--locked"))
        self.assertEqual(set(selected["runtime_binaries"]), {
            "INTEGRATION_TEMPORAL_WORKER_BIN", "INTEGRATION_OUTBOX_WORKER_BIN",
        })
        self.assertNotIn("--target-dir", selected["prepare"])

    def test_publishing_change_selects_real_a17_integration(self) -> None:
        result = self.impact("app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/finalize_publish.rs")
        self.assertEqual(result["selected_suites"], ["publishing_a17"])

    def test_cms_review_domain_port_and_sql_select_real_revision_e2e(self) -> None:
        for path in (
            "app/rust/crates/seo_domain/src/review_decision.rs",
            "app/rust/crates/seo_application/src/cms_review.rs",
            "app/rust/services/cli_tools/src/cli/cms_decision.rs",
            "app/rust/crates/seo_application/src/review_publish.rs",
            "app/rust/crates/seo_steps/src/cms_review_wire.rs",
            "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/types_review_helpers.rs",
            "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/human_review_decision.rs",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertEqual(result["analysis"], "scoped")
                self.assertIn("cms_review_revision", result["selected_suites"])
        suite = SUITE_OWNERS["cms_review_revision"]
        self.assertIn(
            "published_page_review_requires_exact_pending_revision_and_active_workflow",
            suite["command"],
        )

    def test_outbox_change_selects_real_a14_integration(self) -> None:
        result = self.impact("app/rust/crates/infrastructure/src/adapters/sqlx_outbox_adapter.rs")
        self.assertEqual(result["selected_suites"], ["outbox_a14"])

    def test_verified_rule_projection_effects_select_real_outbox_and_knowledge_integrations(self) -> None:
        path = "app/rust/crates/infrastructure/src/adapters/verified_rule_projection_adapter.rs"
        result = self.impact(path)
        self.assertEqual(result["analysis"], "scoped")
        self.assertEqual(result["selected_suites"], ["knowledge_a13", "outbox_a14"])

    def test_knowledge_read_change_selects_postgres_a13(self) -> None:
        result = self.impact("app/rust/crates/seo_application/src/knowledge.rs")
        self.assertEqual(result["selected_suites"], ["knowledge_a13"])

    def test_real_neighbor_adapter_change_selects_existing_qdrant_a13(self) -> None:
        result = self.impact("app/rust/crates/infrastructure/src/adapters/semantic_neighbor_search_adapter.rs")
        self.assertEqual(result["analysis"], "scoped")
        self.assertEqual(result["selected_suites"], ["knowledge_a13"])

    def test_editorial_source_context_port_changes_select_real_qdrant_a13(self) -> None:
        for path in (
            "app/rust/crates/seo_application/src/drafting.rs",
            "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter/source_context_projection_ports.rs",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertEqual(result["analysis"], "scoped")
                self.assertIn("knowledge_a13", result["selected_suites"])

    def test_unknown_global_change_fails_closed_to_a17(self) -> None:
        result = self.impact("app/db/schema.sql")
        self.assertEqual(result["analysis"], "full")
        self.assertIn("publishing_a17", result["selected_suites"])
        self.assertIn("outbox_a14", result["selected_suites"])
        self.assertIn("knowledge_a13", result["selected_suites"])

    def test_isolated_analytics(self) -> None:
        self.assertEqual(self.impact("app/rust/services/analytics_svc/src/main.rs")["affected_services"], ["analytics_svc"])

    def test_graph_change_includes_reverse_consumers(self) -> None:
        got = self.impact("app/rust/crates/graph_neo4j/src/lib.rs")
        self.assertEqual(set(got["affected_crates"]), {"graph_neo4j", "analytics_svc", "temporal_worker"})
        self.assertEqual(got["selected_suites"], ["graph_query"])

    def test_http_change_includes_gsc_only(self) -> None:
        self.assertEqual(set(self.impact("app/rust/crates/http_client/src/lib.rs")["affected_crates"]), {"http_client", "gsc_sync"})

    def test_vector_change_does_not_rebuild_unrelated_analytics_or_gsc(self) -> None:
        result = self.impact("app/rust/crates/vector_qdrant/src/lib.rs")
        self.assertEqual(
            set(result["affected_crates"]),
            {"vector_qdrant", "temporal_worker"},
        )
        self.assertEqual(result["selected_suites"], ["vector_projection"])

    def test_common_contracts_are_transitive(self) -> None:
        self.assertEqual(set(self.impact("app/rust/crates/contracts/src/lib.rs")["affected_crates"]), {"contracts", "primitives", "temporal_worker"})

    def test_global_inputs_fail_closed(self) -> None:
        paths = (
            "contracts/proto/temporal_payloads.proto",
            "app/db/schema.sql",
            ".github/workflows/ci.yml",
            "automation/check_seo_crate_graph.py",
            "docker-compose.yml",
            "analytics_lab/proto/analytics.proto",
        )
        for path in paths:
            with self.subTest(path=path):
                outcome = self.impact(path)
                self.assertEqual(outcome["analysis"], "full")
                self.assertEqual(len(outcome["affected_crates"]), len(self.reverse))

    def test_unknown_file_is_full_scope(self) -> None:
        self.assertEqual(self.impact("unmapped/thing.txt")["analysis"], "full")

    def test_explanatory_markdown_needs_no_rust_suite_or_image(self) -> None:
        outcome = self.impact("docs/ARCHITECTURE_BOUNDARIES.md")
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_suites"], [])
        self.assertEqual(outcome["selected_images"], [])
        self.assertEqual(outcome["selected_ci_jobs"], [])
        self.assertEqual(set(outcome["ci_job_status"].values()), {"not_applicable"})
        self.assertIn("documentation", outcome["classified_inputs"]["docs/ARCHITECTURE_BOUNDARIES.md"])

    def test_historical_truth_report_is_not_a_live_rust_fixture(self) -> None:
        outcome = self.impact("docs/runs/r3_contradiction_handling.json")
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_suites"], [])
        self.assertEqual(outcome["selected_ci_jobs"], ["product-truth", "verify"])
        self.assertIn("historical", outcome["classified_inputs"]["docs/runs/r3_contradiction_handling.json"])

    def test_policy_markdown_still_runs_source_acceptance_without_rust_or_docker(self) -> None:
        outcome = self.impact("docs/OPS_RUNTIME_RUNBOOK.md")
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_ci_jobs"], ["product-truth", "verify"])
        self.assertEqual(outcome["selected_images"], [])
        self.assertIn("source-policy", outcome["classified_inputs"]["docs/OPS_RUNTIME_RUNBOOK.md"])

    def test_analytics_lab_python_uses_python_owner_not_rust(self) -> None:
        outcome = self.impact("infra/analytics_lab/src/runner.py")
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_ci_jobs"], ["verify"])

    def test_container_delivery_input_without_narrow_owner_fails_closed(self) -> None:
        for path in ("app/rust/services/temporal/Dockerfile", "infra/analytics_lab/Dockerfile"):
            with self.subTest(path=path):
                outcome = self.impact(path)
                self.assertEqual(set(outcome["selected_ci_jobs"]), set(CI_PR_OWNERS))

    def test_live_truth_fixture_selects_its_real_e2e_without_other_images(self) -> None:
        path = "app/rust/crates/integration_harness/fixtures/visa_truth_certification/happy_path_publish_allowed_source.html"
        outcome = self.impact(path)
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["direct_crates"], [])
        self.assertEqual(outcome["affected_crates"], [])
        self.assertEqual(outcome["selected_suites"], ["temporal_restart", "truth_certification"])
        self.assertEqual(commands_for_report(outcome, check_only=False), [])
        self.assertEqual(outcome["suite_fixture_ids"], {"truth_certification": ["happy_path_publish_allowed"]})
        self.assertEqual(SUITE_OWNERS["temporal_restart"]["prepare"],
                         SUITE_OWNERS["truth_certification"]["prepare"])
        self.assertEqual(SUITE_OWNERS["temporal_restart"]["runtime_binaries"],
                         SUITE_OWNERS["truth_certification"]["runtime_binaries"])
        self.assertEqual(suite_environment_for_report(outcome, "truth_certification"), {
            "TRUTH_CERT_FIXTURE_IDS": "happy_path_publish_allowed",
        })
        self.assertEqual(outcome["selected_images"], [])
        self.assertIn("architecture-and-rust", outcome["selected_ci_jobs"])
        self.assertIn("product-truth", outcome["selected_ci_jobs"])
        self.assertNotIn("stack-temporal-postgres18", outcome["selected_ci_jobs"])
        self.assertNotIn("stack-storage-graph", outcome["selected_ci_jobs"])
        selected_commands = dict(suite_commands_for_report(outcome))
        self.assertIn("temporal_restart", selected_commands)
        command = selected_commands["truth_certification"]
        self.assertIn("truth_certification_suite_matches_fixture_expectations", command)

    def test_unrelated_live_html_selects_only_truth_e2e_without_manual_restart(self) -> None:
        outcome = self.impact(TRUTH_FIXTURE_PREFIX + "agency_override_denied.html")
        self.assertEqual(outcome["selected_suites"], ["truth_certification"])
        self.assertEqual(outcome["suite_fixture_ids"], {"truth_certification": ["agency_override_denied"]})
        self.assertEqual(commands_for_report(outcome, check_only=False), [])

    def test_truth_manifest_change_or_unowned_html_runs_full_suite(self) -> None:
        for path in (
            TRUTH_FIXTURE_PREFIX + "happy_path_publish_allowed.json",
            TRUTH_FIXTURE_PREFIX + "unowned_fixture.html",
        ):
            with self.subTest(path=path):
                outcome = self.impact(path)
                self.assertEqual(outcome["selected_suites"], ["temporal_restart", "truth_certification"])
                self.assertEqual(outcome["suite_fixture_ids"], {})
                self.assertIn("full:", outcome["suite_fixture_reasons"]["truth_certification"][0])
                self.assertEqual(suite_environment_for_report(outcome, "truth_certification"), {})

    def test_shared_html_input_selects_every_manifest_consumer(self) -> None:
        with TemporaryDirectory() as tmp:
            folder = Path(tmp) / TRUTH_FIXTURE_PREFIX
            folder.mkdir(parents=True)
            (folder / "shared.html").write_text("<html></html>", encoding="utf-8")
            for fixture_id in ("consumer_a", "consumer_b"):
                (folder / (fixture_id + ".json")).write_text(json.dumps({
                    "fixture_id": fixture_id,
                    "source_inputs": {"pages": [{"html_file": "shared.html"}]},
                }), encoding="utf-8")
            ids, reasons = truth_fixture_ids_for_paths(
                [TRUTH_FIXTURE_PREFIX + "shared.html"], root=Path(tmp),
            )
            self.assertEqual(ids, ["consumer_a", "consumer_b"])
            self.assertIn("consumer_b", reasons[0])

    def test_incomplete_manifest_never_selects_one_fixture_silently(self) -> None:
        with TemporaryDirectory() as tmp:
            folder = Path(tmp) / TRUTH_FIXTURE_PREFIX
            folder.mkdir(parents=True)
            (folder / "source.html").write_text("<html></html>", encoding="utf-8")
            (folder / "bad.json").write_text("{malformed", encoding="utf-8")
            ids, reasons = truth_fixture_ids_for_paths(
                [TRUTH_FIXTURE_PREFIX + "source.html"], root=Path(tmp),
            )
            self.assertIsNone(ids)
            self.assertIn("full:", reasons[0])

    def test_malformed_or_unowned_filter_plan_is_rejected(self) -> None:
        outcome = self.impact(TRUTH_FIXTURE_PREFIX + "happy_path_publish_allowed_source.html")
        for ids in ([], ["bad,other"], ["happy_path_publish_allowed"] * 2, [123]):
            with self.subTest(ids=ids):
                invalid = dict(outcome, suite_fixture_ids={"truth_certification": ids})
                with self.assertRaises(ValueError):
                    suite_commands_for_report(invalid)
        invalid = dict(outcome, suite_fixture_ids={"unknown": ["happy_path_publish_allowed"]})
        with self.assertRaises(ValueError):
            suite_commands_for_report(invalid)

    def test_external_fixture_filter_cannot_override_exact_source_plan(self) -> None:
        for variable in ("TRUTH_CERT_LIMIT", "TRUTH_CERT_FIXTURE_IDS"):
            with self.subTest(variable=variable):
                with self.assertRaises(ValueError):
                    require_clean_truth_filter_environment(
                        "truth_certification", environment={variable: "1"},
                    )
        require_clean_truth_filter_environment("truth_certification", environment={})
        require_clean_truth_filter_environment("knowledge_a13", environment={"TRUTH_CERT_LIMIT": "1"})

    def test_delivery_script_selects_temporal_worker_image_without_rust(self) -> None:
        path = "automation/deploy_seo_static_cloudflare.py"
        outcome = self.impact(path)
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_suites"], [])
        self.assertEqual(outcome["selected_images"], ["temporal-worker"])
        self.assertNotIn("architecture-and-rust", outcome["selected_ci_jobs"])
        self.assertIn("cloudflare-static-build", outcome["selected_ci_jobs"])
        self.assertIn("product-truth", outcome["selected_ci_jobs"])
        self.assertIn("verify", outcome["selected_ci_jobs"])
        self.assertIn(path, outcome["image_reasons"]["temporal-worker"][0])

    def test_unrelated_service_edit_selects_no_temporal_image(self) -> None:
        outcome = self.impact("app/rust/services/analytics_svc/src/main.rs")
        self.assertEqual(outcome["selected_images"], ["analytics-svc"])
        self.assertEqual(outcome["image_status"]["temporal-worker"], "not_applicable")

    def test_outbox_service_change_runs_real_outbox_suite_and_own_image(self) -> None:
        outcome = self.impact("app/rust/services/outbox_worker/src/worker.rs")
        self.assertEqual(outcome["selected_suites"], ["outbox_a14"])
        self.assertEqual(outcome["selected_images"], ["outbox-worker"])

    def test_cloudflare_static_is_worker_image_input_without_cargo(self) -> None:
        outcome = self.impact("infra/cloudflare/seo-static/package.json")
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_images"], ["temporal-worker"])

    def test_planner_or_ci_driver_change_selects_every_registered_suite_and_image(self) -> None:
        outcome = self.impact("automation/ci_impact.py")
        self.assertEqual(outcome["scope"], "full")
        self.assertEqual(len(outcome["selected_suites"]), 4)
        self.assertEqual(len(outcome["selected_images"]), 4)
        self.assertEqual(set(outcome["selected_ci_jobs"]), set(CI_PR_OWNERS))
        self.assertTrue(outcome["full_scope_reasons"])

    def test_unknown_or_missing_suite_in_report_is_rejected(self) -> None:
        outcome = self.impact("app/rust/services/gsc_sync/src/main.rs")
        outcome.pop("selected_suites")
        with self.assertRaises(ValueError):
            suite_commands_for_report(outcome)
        outcome["selected_suites"] = ["invented"]
        with self.assertRaises(ValueError):
            suite_commands_for_report(outcome)

    def test_multiple_changes_include_all_consumers(self) -> None:
        self.assertEqual(
            set(self.impact("app/rust/services/gsc_sync/src/main.rs", "app/rust/crates/graph_neo4j/src/lib.rs")["affected_crates"]),
            {"gsc_sync", "graph_neo4j", "analytics_svc", "temporal_worker"},
        )

    def test_optional_dev_and_build_path_dependencies_in_graph(self) -> None:
        packages = [
            {"id": "a", "name": "contracts", "manifest_path": "/synthetic/repo/app/rust/crates/contracts/Cargo.toml", "dependencies": []},
            {"id": "b", "name": "gsc_sync", "manifest_path": "/synthetic/repo/app/rust/services/gsc_sync/Cargo.toml", "dependencies": [
                {"name": "contracts", "kind": "dev", "optional": True, "path": "/synthetic/repo/app/rust/crates/contracts"}
            ]},
        ]
        owners, reverse = workspace_graph({"workspace_members": ["a", "b"], "packages": packages}, root=self.root)
        self.assertEqual(owners["app/rust/services/gsc_sync"], "gsc_sync")
        self.assertEqual(reverse["contracts"], {"gsc_sync"})

    def test_git_rename_includes_old_and_new_paths(self) -> None:
        with patch("subprocess.check_output", return_value=b"R100\0app/rust/crates/graph_neo4j/src/old.rs\0app/rust/crates/graph_neo4j/src/new.rs\0"):
            self.assertEqual(
                changed_paths("BASE", "HEAD", root=self.root),
                ["app/rust/crates/graph_neo4j/src/new.rs", "app/rust/crates/graph_neo4j/src/old.rs"],
            )

    def test_git_deletion_is_retained(self) -> None:
        with patch("subprocess.check_output", return_value=b"D\0app/rust/services/gsc_sync/src/main.rs\0"):
            self.assertEqual(changed_paths("BASE", "HEAD", root=self.root), ["app/rust/services/gsc_sync/src/main.rs"])

    def test_invalid_git_data_fails_closed(self) -> None:
        with patch("subprocess.check_output", return_value=b"R100\0old.rs\0"):
            with self.assertRaises(ValueError):
                changed_paths("BASE", "HEAD", root=self.root)


if __name__ == "__main__":
    unittest.main()
