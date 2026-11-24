#!/usr/bin/env python3
"""Offline regression fixtures for the Rust dependency impact analyzer."""

from __future__ import annotations

import unittest
import json
from tempfile import TemporaryDirectory
from pathlib import Path
from unittest.mock import patch

from ci_impact import (
    CI_PR_OWNERS, SUITE_OWNERS, TRUTH_FIXTURE_PREFIX, WORKER_E2E_PREPARE, analyze, changed_paths,
    affected_crates_for_lock_change, declared_workspace_owners, parsed_lock_graph,
    main_cache_prime_required,
    suite_commands_for_report, suite_environment_for_report, truth_fixture_ids_for_paths,
    worker_release_decision, workspace_graph,
)
from dev_rust_feedback import commands_for_report
from run_main_cache_prime import prime_commands_for_report
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
            "app/rust/crates/runtime_models": "runtime_models",
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
            "runtime_models": {"seo_steps", "seo_application"},
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

    def cargo_metadata(self) -> dict:
        # Cargo metadata's target kinds, not a second manually maintained
        # Cargo dependency graph, decide what can produce reusable rlibs.
        binary_only = {"cli_tools", "gsc_sync", "temporal_worker", "analytics_svc"}
        packages = [
            {
                "id": f"workspace#{name}",
                "name": name,
                "targets": [{"kind": ["bin" if name in binary_only else "lib"]}],
            }
            for name in self.owners.values()
        ]
        return {
            "workspace_members": [pkg["id"] for pkg in packages],
            "packages": packages,
        }

    def impact(self, *paths: str, lock_snapshots: tuple[str, str] | None = None) -> dict:
        return analyze(
            list(paths), self.owners, self.reverse, head="HEAD", base="BASE",
            lock_snapshots=lock_snapshots,
        )

    def test_ci_only_main_merge_is_full_pr_but_not_a_new_worker_image(self) -> None:
        paths = [".github/workflows/ci.yml", "automation/ci_impact.py", "automation/check_ci_impact.py"]
        report = self.impact(*paths)
        self.assertEqual(report["scope"], "full")
        self.assertEqual(set(report["selected_ci_jobs"]), set(CI_PR_OWNERS))
        self.assertIn("temporal-worker", report["selected_images"])
        decision = worker_release_decision(report, paths, manual=False)
        self.assertFalse(decision["worker_required"])
        self.assertEqual(decision["reason"], "ci_operator_maintenance_only")

    def test_compiler_cache_and_feedback_tools_do_not_ship_into_worker(self) -> None:
        # A change to compiler orchestration still receives the full PR gate:
        # the release graph must not confuse it with a delivered Worker input.
        for path in (
            "automation/run_main_cache_prime.py",
            "automation/run_scoped_cargo_feedback.py",
            "automation/cargo_feedback.py",
            "automation/dev_rust_feedback.py",
            "automation/check_cargo_feedback.py",
        ):
            with self.subTest(path=path):
                report = self.impact(path)
                self.assertEqual(report["scope"], "full")
                self.assertEqual(set(report["selected_ci_jobs"]), set(CI_PR_OWNERS))
                self.assertFalse(worker_release_decision(report, [path], manual=False)["worker_required"])

        diagnostic = "automation/report_sccache_failures.py"
        report = self.impact(diagnostic)
        self.assertEqual(report["selected_ci_jobs"], ["verify"])
        self.assertFalse(worker_release_decision(report, [diagnostic], manual=False)["worker_required"])

    def test_worker_release_rebuilds_mixed_unknown_and_real_delivery_inputs(self) -> None:
        for paths in (
            [".github/workflows/ci.yml", "automation/deploy_seo_static_cloudflare.py"],
            [".github/workflows/ci.yml", "automation/new_runtime_script.py"],
            ["app/rust/services/temporal/src/main.rs"],
        ):
            with self.subTest(paths=paths):
                report = self.impact(*paths)
                decision = worker_release_decision(report, paths, manual=False)
                self.assertTrue(decision["worker_required"])
                self.assertEqual(decision["reason"], "shared_impact_registry")

    def test_worker_cache_export_preserves_source_only_then_asset_rebase(self) -> None:
        # Exporting after a Rust edit is mandatory: the next asset-only main
        # merge must find that compiled builder image in the shared cache.
        source = ["app/rust/services/temporal/src/main.rs"]
        first = worker_release_decision(self.impact(*source), source, manual=False)
        self.assertTrue(first["worker_required"])
        self.assertTrue(first["cache_export_required"])

        # Node dependency changes alter the other expensive reusable layer.
        manifest = ["infra/cloudflare/seo-static/package.json"]
        npm = worker_release_decision(self.impact(*manifest), manifest, manual=False)
        self.assertTrue(npm["worker_required"])
        self.assertTrue(npm["cache_export_required"])

        for assets in (
            ["infra/cloudflare/seo-static/build-entry.js"],
            ["automation/deploy_seo_static_cloudflare.py"],
            ["infra/cloudflare/seo-static/build-entry.js", "automation/check_first_site_release.py"],
        ):
            with self.subTest(assets=assets):
                second = worker_release_decision(self.impact(*assets), assets, manual=False)
                self.assertTrue(second["worker_required"])
                self.assertFalse(second["cache_export_required"])

    def test_worker_cache_export_unknown_source_and_manual_are_fail_closed(self) -> None:
        for changed in (
            ["infra/cloudflare/seo-static/package.json"],
            ["infra/cloudflare/seo-static/package-lock.json"],
            ["infra/cloudflare/seo-static/npm-shrinkwrap.json"],
            ["app/rust/services/temporal/Dockerfile"],
            ["app/contracts/proto/temporal_payloads.proto"],
            ["automation/prepare_worker_build_context.py"],
            ["automation/new_release_helper.py"],
            ["app/rust/services/temporal/src/main.rs", "infra/cloudflare/seo-static/build-entry.js"],
            ["infra/cloudflare/seo-static/build-entry.js", "automation/new_release_helper.py"],
        ):
            with self.subTest(changed=changed):
                decision = worker_release_decision(self.impact(*changed), changed, manual=False)
                self.assertTrue(decision["worker_required"])
                self.assertTrue(decision["cache_export_required"])

        # A malformed lock graph still exercises the fail-closed release path,
        # without trying to resolve the synthetic BASE/HEAD test refs.
        lock_changed = ["app/rust/Cargo.lock"]
        lock_report = self.impact(*lock_changed, lock_snapshots=("invalid", "invalid"))
        lock_decision = worker_release_decision(lock_report, lock_changed, manual=False)
        self.assertTrue(lock_decision["cache_export_required"])

        asset = ["infra/cloudflare/seo-static/build-entry.js"]
        manual = worker_release_decision(self.impact(*asset), asset, manual=True)
        self.assertTrue(manual["worker_required"])
        self.assertTrue(manual["cache_export_required"])
        ci_only = [".github/workflows/ci.yml"]
        skipped = worker_release_decision(self.impact(*ci_only), ci_only, manual=False)
        self.assertFalse(skipped["worker_required"])
        self.assertFalse(skipped["cache_export_required"])

    def test_architecture_source_contract_needs_only_python_verify(self) -> None:
        path = "automation/check_expert_core_activation_contract.py"
        report = self.impact(path)
        self.assertEqual(report["scope"], "none")
        self.assertEqual(report["selected_ci_jobs"], ["verify"])
        self.assertEqual(report["affected_crates"], [])
        self.assertEqual(report["selected_images"], [])
        self.assertFalse(worker_release_decision(report, [path], manual=False)["worker_required"])

        # A policy-file change cannot hide actual executable Rust inputs.
        source = "app/rust/services/temporal/src/main.rs"
        mixed = self.impact(path, source)
        self.assertEqual(mixed["scope"], "scoped")
        self.assertIn("architecture-and-rust", mixed["selected_ci_jobs"])
        self.assertTrue(worker_release_decision(mixed, [path, source], manual=False)["worker_required"])

    def test_main_cache_prime_uses_rename_aware_rust_source_paths(self) -> None:
        old = "app/rust/crates/truth_domain/src/old.rs"
        new = "docs/renamed.md"
        with patch("subprocess.check_output", return_value=f"R100\0{old}\0{new}\0".encode()):
            renamed = changed_paths("OLD", "NEW", root=self.root)
        self.assertTrue(main_cache_prime_required(renamed))
        self.assertTrue(main_cache_prime_required(["app/rust/Cargo.lock"]))
        self.assertFalse(main_cache_prime_required([".github/workflows/ci.yml"]))
        self.assertFalse(main_cache_prime_required(["infra/cloudflare/seo-static/package.json"]))
        self.assertFalse(main_cache_prime_required(["app/rust/crates/integration_harness/src/tests/foo.rs"]))
        self.assertFalse(main_cache_prime_required(["app/rust/services/temporal/Dockerfile"]))
        self.assertFalse(main_cache_prime_required([TRUTH_FIXTURE_PREFIX + "source.html"]))
        self.assertTrue(main_cache_prime_required(["app/rust/crates/integration_harness/src/lib.rs"]))
        self.assertTrue(main_cache_prime_required([
            "app/rust/crates/integration_harness/src/tests/foo.rs", old,
        ]))
        with self.assertRaises(ValueError):
            main_cache_prime_required([])

    def test_main_prime_compiles_cacheable_libraries_not_test_executables(self) -> None:
        report = self.impact("app/rust/crates/http_client/src/lib.rs")
        self.assertEqual(report["analysis"], "scoped")
        self.assertEqual(report["affected_crates"], ["gsc_sync", "http_client"])
        self.assertEqual(report["selected_suites"], [])
        self.assertEqual(prime_commands_for_report(report, self.cargo_metadata()), [
            ["cargo", "build", "--locked", "--lib", "--package", "http_client"],
        ])

    def test_main_prime_skips_binary_only_change_without_reusable_outputs(self) -> None:
        report = self.impact("app/rust/services/gsc_sync/src/main.rs")
        self.assertEqual(report["affected_crates"], ["gsc_sync"])
        self.assertEqual(report["selected_suites"], [])
        self.assertEqual(prime_commands_for_report(report, self.cargo_metadata()), [])

    def test_main_prime_compiles_e2e_feature_only_for_real_selected_suite(self) -> None:
        report = self.impact("app/rust/services/outbox_worker/src/worker.rs")
        self.assertEqual(report["selected_suites"], ["outbox_a14"])
        self.assertEqual(prime_commands_for_report(report, self.cargo_metadata()), [
            ["cargo", "build", "--locked", "--lib", "--package", "outbox_worker"],
            ["cargo", "build", "--locked", "--package", "integration_harness",
             "--features", "e2e", "--lib"],
        ])

    def test_main_prime_global_unknown_and_broken_lock_fall_back_to_full_workspace(self) -> None:
        for report in (
            self.impact("app/rust/Cargo.toml"),
            self.impact("app/rust/Cargo.lock", lock_snapshots=("broken lock", "still broken")),
        ):
            with self.subTest(reason=report["full_scope_reasons"]):
                self.assertEqual(report["analysis"], "full")
                commands = prime_commands_for_report(report, self.cargo_metadata())
                self.assertEqual(commands[0][:4], ["cargo", "build", "--locked", "--lib"])
                self.assertIn("--package", commands[0])
                self.assertNotIn("--no-run", str(commands))
                self.assertNotIn("--bin", str(commands))
                self.assertEqual(commands[-1], [
                    "cargo", "build", "--locked", "--package", "integration_harness",
                    "--features", "e2e", "--lib",
                ])

    def test_main_prime_scoped_lockfile_tracks_real_consumer_and_suite(self) -> None:
        old = self.lock(("temporal_worker", "0.1.0", ("temporalio-client 1.0.0",)),
                        ("temporalio-client", "1.0.0", ()))
        new = self.lock(("temporal_worker", "0.1.0", ("temporalio-client 1.1.0",)),
                        ("temporalio-client", "1.1.0", ()))
        report = self.impact("app/rust/Cargo.lock", lock_snapshots=(old, new))
        self.assertEqual(report["analysis"], "scoped")
        self.assertEqual(report["selected_suites"], ["temporal_restart"])
        self.assertEqual(prime_commands_for_report(report, self.cargo_metadata()), [
            ["cargo", "build", "--locked", "--package", "integration_harness",
             "--features", "e2e", "--lib"],
        ])

    def test_main_prime_does_not_recompile_non_cacheable_runtime_processes(self) -> None:
        report = self.impact(
            "app/rust/services/temporal/src/main.rs",
            TRUTH_FIXTURE_PREFIX + "happy_path_publish_allowed.html",
        )
        self.assertIn("truth_certification", report["selected_suites"])
        self.assertIn("temporal_restart", report["selected_suites"])
        commands = prime_commands_for_report(report, self.cargo_metadata())
        self.assertNotIn(list(WORKER_E2E_PREPARE), commands)
        self.assertEqual(commands[-1], [
            "cargo", "build", "--locked", "--package", "integration_harness",
            "--features", "e2e", "--lib",
        ])

    def test_main_prime_rejects_affected_crate_missing_from_cargo_metadata(self) -> None:
        report = self.impact("app/rust/crates/http_client/src/lib.rs")
        metadata = self.cargo_metadata()
        metadata["packages"] = [p for p in metadata["packages"] if p["name"] != "http_client"]
        with self.assertRaisesRegex(ValueError, "incomplete Cargo metadata"):
            prime_commands_for_report(report, metadata)

    def test_editorial_policy_only_changes_select_real_temporal_consumer(self) -> None:
        # A domain-only or step-only rule change can change the canonical
        # page outcome without touching Temporal workflow source. It must
        # select the live worker/restart path, not just a Cargo unit test.
        for path in (
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
        ):
            with self.subTest(path=path):
                report = self.impact(path)
                self.assertIn("temporal_restart", report["selected_suites"])
                self.assertNotIn("publishing_a17", report["selected_suites"])
                self.assertIn("architecture-and-rust", report["selected_ci_jobs"])
                if path in {
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
                }:
                    self.assertIn("draft_qa_scenario", report["selected_suites"])

        # Unit-level QA passes are not enough: ensure the bound E2E command
        # exercises the actual persistence path with its existing full harness.
        self.assertEqual(SUITE_OWNERS["draft_qa_scenario"]["command"][:7], (
            "cargo", "test", "-p", "integration_harness", "--features", "e2e",
            "--lib",
        ))
        # One Cargo test filter executes both the successful full scenario
        # and the SQL fault-injection rollback test, without a second job.
        self.assertIn("draft_qa_", SUITE_OWNERS["draft_qa_scenario"]["command"])
        repo = Path(__file__).resolve().parents[1]
        scenario_test = repo / "app/rust/crates/integration_harness/src/tests/scenario_parity_tests.rs"
        fault_test = repo / "app/rust/crates/integration_harness/src/tests/qa_persistence_tests.rs"
        self.assertIn("fn draft_qa_synthetic_full_scenario_persists_page_draft_on_full_harness()", scenario_test.read_text())
        self.assertIn("fn draft_qa_persistence_rolls_back_late_failure_and_rejects_missing_draft()", fault_test.read_text())
        self.assertIn("draft_qa_scenario", self.impact("app/rust/crates/integration_harness/src/tests/qa_persistence_tests.rs")["selected_suites"])
        self.assertIn("cms_revision_", SUITE_OWNERS["cms_review_revision"]["command"])
        cms_test = repo / "app/rust/crates/integration_harness/src/tests/provider_runtime_surface_tests.rs"
        self.assertIn("fn cms_revision_publish_keeps_verified_links_structured_without_duplicate_body_sections()", cms_test.read_text())
        self.assertIn("cms_atomic_artifact_fault", cms_test.read_text())

    def test_main_prime_no_compilation_for_runtime_fixture_only(self) -> None:
        report = self.impact(TRUTH_FIXTURE_PREFIX + "happy_path.html")
        self.assertEqual(report["analysis"], "none")
        self.assertIn("truth_certification", report["selected_suites"])
        # A fixture-only PR runs the selected live suite; the accepted-main
        # cache job is intentionally not scheduled for runtime-only bytes.
        self.assertFalse(main_cache_prime_required(report["changed_paths"]))

    def test_worker_readme_only_is_not_delivery_image_or_cloudflare_build(self) -> None:
        readme = "infra/cloudflare/seo-static/README.md"
        report = self.impact(readme)
        self.assertEqual(report["analysis"], "none")
        self.assertEqual(report["selected_images"], [])
        self.assertNotIn("cloudflare-static-build", report["selected_ci_jobs"])
        self.assertFalse(worker_release_decision(report, [readme], manual=False)["worker_required"])
        self.assertIn("non-delivery", report["classified_inputs"][readme])

        real_asset = "infra/cloudflare/seo-static/package.json"
        mixed = self.impact(readme, real_asset)
        self.assertIn("temporal-worker", mixed["selected_images"])
        self.assertIn("cloudflare-static-build", mixed["selected_ci_jobs"])
        self.assertTrue(worker_release_decision(mixed, [readme, real_asset], manual=False)["worker_required"])

    def test_explicit_worker_rebuild_overrides_maintenance_skip(self) -> None:
        paths = [".github/workflows/ci.yml"]
        decision = worker_release_decision(self.impact(*paths), paths, manual=True)
        self.assertTrue(decision["worker_required"])
        self.assertEqual(decision["reason"], "explicit_accepted_main_rebuild")

    def test_image_selector_rejects_unsafe_status_or_empty_main_diff(self) -> None:
        report = self.impact(".github/workflows/ci.yml")
        for changed, malformed in (
            ([], report),
            ([".github/workflows/ci.yml"], {**report, "image_status": {"temporal-worker": "skipped"}}),
            ([".github/workflows/ci.yml"], {**report, "selected_images": []}),
            (["docs/ci-fast-path.md"], report),
        ):
            with self.subTest(changed=changed, malformed=malformed):
                with self.assertRaises(ValueError):
                    worker_release_decision(malformed, changed, manual=False)

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
        self.assertEqual(impact["selected_images"], ["temporal-worker"])
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

    def test_domain_publication_license_and_materialization_keep_a17_owner(self) -> None:
        # Old Temporal orchestration steps are being moved under one domain
        # owner. Rename diffs include both paths; each side must be covered.
        for path in (
            "app/rust/crates/seo_steps/src/licensing_gate_step.rs",
            "app/rust/crates/seo_steps/src/publish_materialize_step.rs",
            "app/rust/crates/seo_steps/src/finalize_publish_step.rs",
            "app/rust/crates/seo_domain/src/publishing/license.rs",
            "app/rust/crates/seo_domain/src/publishing/materialization.rs",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertEqual(result["analysis"], "scoped")
                self.assertIn("publishing_a17", result["selected_suites"])
                self.assertIn("architecture-and-rust", result["selected_ci_jobs"])

    def test_render_preview_policy_and_wire_changes_select_publishing_a17(self) -> None:
        for path in (
            "app/rust/crates/seo_domain/src/render_preview.rs",
            "app/rust/crates/seo_steps/src/render_preview_validate_step.rs",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertEqual(result["scope"], "scoped")
                self.assertEqual(result["selected_suites"], ["publishing_a17"])
                self.assertIn("architecture-and-rust", result["selected_ci_jobs"])

    def test_cms_review_domain_port_and_sql_select_real_revision_e2e(self) -> None:
        for path in (
            "app/rust/crates/seo_domain/src/cms_revision.rs",
            "app/rust/crates/seo_domain/src/review_decision.rs",
            "app/rust/crates/seo_application/src/cms_review.rs",
            "app/rust/services/cli_tools/src/cli/cms_decision.rs",
            "app/rust/crates/seo_application/src/review_publish.rs",
            "app/rust/crates/seo_steps/src/cms_review_wire.rs",
            "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/types_review_helpers.rs",
            "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/human_review_decision.rs",
            "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover.rs",
            "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover/page_publish.rs",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertEqual(result["analysis"], "scoped")
                self.assertIn("cms_review_revision", result["selected_suites"])
                if path.startswith("app/rust/services/temporal/"):
                    self.assertIn("temporal_restart", result["selected_suites"])
        suite = SUITE_OWNERS["cms_review_revision"]
        self.assertIn(
            "cms_revision_",
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

    def test_truth_write_transaction_selects_actual_postgres_outbox_suite(self) -> None:
        for path in (
            "app/rust/services/temporal/src/activities/operations/truth_write_and_fixture_tests.rs",
            "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter/source_registry.rs",
            "app/rust/crates/infrastructure/src/adapters/sqlx_hitl_adapter.rs",
            "app/rust/crates/infrastructure/src/adapters/sqlx_step_ledger_adapter.rs",
            "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/projection_runtime_store.rs",
            "app/rust/crates/integration_harness/src/tests/outbox_delivery_tests.rs",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertIn("outbox_a14", result["selected_suites"])
                self.assertIn("outbox_a14_", dict(suite_commands_for_report(result))["outbox_a14"])

    def test_public_static_render_change_selects_real_cms_and_pg_suite(self) -> None:
        for path in (
            "app/rust/crates/infrastructure/src/adapters/sqlx_static_site_adapter.rs",
            "app/rust/crates/infrastructure/src/adapters/static_site_builder_adapter.rs",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertIn("cms_review_revision", result["selected_suites"])
                self.assertIn("cms_revision_", dict(suite_commands_for_report(result))["cms_review_revision"])

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

    def test_full_workspace_tests_can_use_existing_verify_owner_without_coverage_gap(self) -> None:
        # The two existing jobs may execute separate Cargo phases in parallel
        # only if every FULL plan requires both of them. Scoped Rust edits keep
        # their package-level feedback; non-Rust edits need no Cargo run.
        for path in (
            "app/rust/Cargo.toml",
            "app/db/schema.sql",
            ".github/workflows/ci.yml",
            "automation/ci_impact.py",
            "unmapped/new_input.txt",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertEqual(result["scope"], "full")
                self.assertEqual(result["ci_job_status"]["architecture-and-rust"], "required")
                self.assertEqual(result["ci_job_status"]["verify"], "required")
                self.assertEqual(commands_for_report(result, check_only=False), [
                    ["cargo", "test", "--workspace", "--locked"],
                ])

        scoped = self.impact("app/rust/crates/http_client/src/lib.rs")
        self.assertEqual(scoped["scope"], "scoped")
        self.assertIn("architecture-and-rust", scoped["selected_ci_jobs"])
        self.assertIn("verify", scoped["selected_ci_jobs"])
        self.assertNotIn("--workspace", str(commands_for_report(scoped, check_only=False)))

        for path in ("docs/ARCHITECTURE_BOUNDARIES.md", "automation/report_sccache_failures.py"):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertEqual(result["scope"], "none")
                self.assertEqual(commands_for_report(result, check_only=False), [])
                self.assertNotIn("architecture-and-rust", result["selected_ci_jobs"])

    def test_exact_ci_only_inputs_run_verify_without_rust_compilation(self) -> None:
        for path in (
            "automation/check_ci_workflow_contract.py",
            "automation/check_expert_core_activation_contract.py",
            "automation/report_sccache_failures.py",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertEqual(result["scope"], "none")
                self.assertEqual(result["affected_crates"], [])
                self.assertEqual(result["selected_suites"], [])
                self.assertEqual(result["selected_images"], [])
                self.assertEqual(result["selected_ci_jobs"], ["verify"])
                self.assertIn("known CI-only", result["classified_inputs"][path])

    def test_unknown_ci_and_impact_control_files_still_fail_closed(self) -> None:
        for path in (
            ".github/workflows/ci.yml",
            ".github/workflows/ci-deep.yml",
            ".github/workflows/rust-cache-prime.yml",
            ".github/workflows/worker-image.yml",
            ".github/workflows/future-build.yml",
            "automation/ci_impact.py",
            "automation/check_ci_impact.py",
            "automation/check_worker_dependency_skeleton.py",
            "automation/run_scoped_cargo_feedback.py",
            "automation/future_ci_driver.py",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertEqual(result["scope"], "full")
                self.assertEqual(set(result["selected_ci_jobs"]), set(CI_PR_OWNERS))

    def test_ci_only_file_does_not_hide_a_real_runtime_change(self) -> None:
        result = self.impact(
            "automation/check_ci_workflow_contract.py",
            "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover.rs",
        )
        self.assertEqual(result["scope"], "scoped")
        self.assertIn("temporal_restart", result["selected_suites"])
        self.assertIn("architecture-and-rust", result["selected_ci_jobs"])
        self.assertIn("verify", result["selected_ci_jobs"])
        self.assertNotIn("stack-storage-graph", result["selected_ci_jobs"])

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

    def test_reviewed_automation_markdown_needs_only_fast_verify(self) -> None:
        # PR #83 changed only this operator document but previously triggered
        # a full workspace Rust/PG/Temporal run (~230s Rust + 71s verify).
        # Preserve the policy check without treating Markdown as executable.
        for path in (
            "automation/00_ENTRYPOINT.md",
            "automation/FSM_TYPED_PHASE_IMPERATIVE_PLAN.md",
            "automation/INTEGRITY_RUNBOOK.md",
            "automation/LAYER_MAP.md",
            "automation/NEW_OPERATION_PLAYBOOK.md",
            "automation/OPS_CANONICALS.md",
            "automation/PROJECT_NAVIGATION_MAP.md",
            "automation/README.md",
        ):
            with self.subTest(path=path):
                result = self.impact(path)
                self.assertEqual(result["scope"], "none")
                self.assertEqual(result["affected_crates"], [])
                self.assertEqual(result["selected_suites"], [])
                self.assertEqual(result["selected_images"], [])
                self.assertEqual(result["selected_ci_jobs"], ["verify"])
                self.assertIn("source-policy Markdown", result["classified_inputs"][path])
                self.assertFalse(main_cache_prime_required([path]))
                self.assertFalse(worker_release_decision(result, [path], manual=False)["worker_required"])

    def test_automation_markdown_cannot_hide_executable_input(self) -> None:
        path = "automation/NEW_OPERATION_PLAYBOOK.md"
        # New unknown Markdown and policy generator inventory stay fail-closed.
        for unknown in ("automation/NEW_UNOWNED_POLICY.md", "automation/PROTO_CONTRACTS.md"):
            self.assertEqual(self.impact(unknown)["scope"], "full")
        # Mixed content selects the real runtime consumer, not just verify.
        result = self.impact(path, "app/rust/services/temporal/src/main.rs")
        self.assertEqual(result["scope"], "scoped")
        self.assertIn("architecture-and-rust", result["selected_ci_jobs"])
        self.assertIn("verify", result["selected_ci_jobs"])
        self.assertIn("temporal_restart", result["selected_suites"])
        self.assertTrue(worker_release_decision(result, result["changed_paths"], manual=False)["worker_required"])

    def test_draft_qa_sql_atomicity_change_selects_actual_postgres_rollback_suite(self) -> None:
        path = "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/draft_publish_rebuild_persistence.rs"
        outcome = self.impact(path)
        self.assertEqual(outcome["scope"], "scoped")
        self.assertIn("draft_qa_scenario", outcome["selected_suites"])
        self.assertIn("architecture-and-rust", outcome["selected_ci_jobs"])
        self.assertIn("draft_qa_", dict(suite_commands_for_report(outcome))["draft_qa_scenario"])

    def test_planning_and_projection_atomicity_select_actual_postgres_suite(self) -> None:
        for path in (
            "app/rust/crates/seo_steps/src/global_site_reconcile_step.rs",
            "app/rust/crates/seo_domain/src/link_recommendation.rs",
            "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter/planning_ports.rs",
            "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/planning_persistence.rs",
            "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/navigation_persistence.rs",
            "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/shared_helpers.rs",
            "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/projection_status.rs",
            "app/rust/crates/integration_harness/src/tests/provider_runtime_surface_tests.rs",
        ):
            with self.subTest(path=path):
                outcome = self.impact(path)
                self.assertIn("planning_atomicity", outcome["selected_suites"])
                self.assertIn("architecture-and-rust", outcome["selected_ci_jobs"])
                self.assertIn("planning_atomic_", dict(suite_commands_for_report(outcome))["planning_atomicity"])

        # One canonical Qdrant key builder is also exercised by Draft QA's
        # existing PostgreSQL retry and idempotency assertions.
        self.assertIn(
            "draft_qa_scenario",
            self.impact("app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/shared_helpers.rs")["selected_suites"],
        )

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
        self.assertIn("canonical_temporal_", selected_commands["temporal_restart"])
        command = selected_commands["truth_certification"]
        self.assertIn("truth_certification_suite_matches_fixture_expectations", command)

    def test_real_temporal_editorial_gate_e2e_selects_both_runtime_and_cms(self) -> None:
        outcome = self.impact("app/rust/crates/integration_harness/src/tests/manual_llm_temporal_tests.rs")
        self.assertIn("temporal_restart", outcome["selected_suites"])
        self.assertIn("cms_review_revision", outcome["selected_suites"])
        command = dict(suite_commands_for_report(outcome))["temporal_restart"]
        self.assertIn("canonical_temporal_", command)

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

    def test_unreleased_service_edit_does_not_claim_an_artifact(self) -> None:
        outcome = self.impact("app/rust/services/analytics_svc/src/main.rs")
        self.assertEqual(outcome["selected_images"], [])
        self.assertEqual(outcome["image_status"]["temporal-worker"], "not_applicable")

    def test_outbox_service_change_runs_real_suite_without_phantom_image(self) -> None:
        outcome = self.impact("app/rust/services/outbox_worker/src/worker.rs")
        self.assertEqual(outcome["selected_suites"], ["outbox_a14"])
        self.assertEqual(outcome["selected_images"], [])

    def test_unreleased_dockerfiles_remain_fail_closed_executable_inputs(self) -> None:
        # Dropping imaginary release-image outputs must never quietly classify
        # a real delivery Dockerfile as documentation or a safe no-op.
        for path in (
            "app/rust/services/outbox_worker/Dockerfile",
            "app/rust/services/analytics_svc/Dockerfile",
            "infra/analytics_lab/Dockerfile",
        ):
            with self.subTest(path=path):
                outcome = self.impact(path)
                self.assertEqual(outcome["selected_images"], [])
                self.assertEqual(set(outcome["selected_ci_jobs"]), set(CI_PR_OWNERS))

    def test_cloudflare_static_is_worker_image_input_without_cargo(self) -> None:
        asset = "infra/cloudflare/seo-static/package.json"
        outcome = self.impact(asset)
        self.assertEqual(outcome["scope"], "none")
        self.assertEqual(outcome["selected_images"], ["temporal-worker"])
        self.assertNotIn("architecture-and-rust", outcome["selected_ci_jobs"])
        self.assertIn("cloudflare-static-build", outcome["selected_ci_jobs"])
        self.assertTrue(worker_release_decision(outcome, [asset], manual=False)["worker_required"])
        self.assertFalse(main_cache_prime_required([asset]))

    def test_planner_or_ci_driver_change_selects_every_registered_suite_and_real_image(self) -> None:
        outcome = self.impact("automation/ci_impact.py")
        self.assertEqual(outcome["scope"], "full")
        self.assertEqual(
            set(outcome["selected_suites"]),
            {name for name, owner in SUITE_OWNERS.items() if owner["on_unknown_global_input"]},
        )
        self.assertEqual(outcome["selected_images"], ["temporal-worker"])
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

    def test_rename_aware_main_worker_selection_matches_exact_source_impact(self) -> None:
        old = "app/rust/services/temporal/src/old.rs"
        new = "app/rust/services/temporal/src/new.rs"
        with patch("subprocess.check_output", return_value=f"R100\0{old}\0{new}\0".encode()):
            changed = changed_paths("HEAD^1", "MERGE_SHA", root=self.root)
        self.assertEqual(changed, [new, old])
        report = self.impact(*changed)
        decision = worker_release_decision(report, changed, manual=False)
        self.assertTrue(decision["worker_required"])
        with self.assertRaisesRegex(ValueError, "invalid exact-source impact"):
            worker_release_decision(report, [new], manual=False)

    def test_git_deletion_is_retained(self) -> None:
        with patch("subprocess.check_output", return_value=b"D\0app/rust/services/gsc_sync/src/main.rs\0"):
            self.assertEqual(changed_paths("BASE", "HEAD", root=self.root), ["app/rust/services/gsc_sync/src/main.rs"])

    def test_invalid_git_data_fails_closed(self) -> None:
        with patch("subprocess.check_output", return_value=b"R100\0old.rs\0"):
            with self.assertRaises(ValueError):
                changed_paths("BASE", "HEAD", root=self.root)


if __name__ == "__main__":
    unittest.main()
