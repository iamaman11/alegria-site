#!/usr/bin/env python3
"""Safety contracts for the read-only V1/V2 history/drain verifier."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import temporal_history_transition as transition


SHA = "b" * 40
TREE = "c" * 40
NOW = datetime(2026, 11, 25, 12, 0, tzinfo=timezone.utc)


def temporal(*, open_all: int = 0, old: int = 0, old_open: int = 0):
    history = {name: {"total": 0, "open": 0} for name in transition.LEGACY_TYPES}
    history[transition.LEGACY_TYPES[0]] = {"total": old, "open": old_open}
    return {
        "source": "live_temporal_server_visibility",
        "address": "temporal.test:7233", "namespace": "staging-site",
        "open_all_before": open_all, "open_all_after": open_all,
        "total_all_before": old, "total_all_after": old,
        "legacy_workflow_types": history,
    }


def postgres(**overrides):
    return {
        "source": "live_postgresql_all_tenant_read_only", "global_visibility": True,
        **{name: overrides.get(name, 0) for name in transition.COUNT_FIELDS},
    }


def snap(*, captured_at, pg=None, temporal_data=None):
    t = temporal_data or temporal()
    p = pg or postgres()
    reasons = transition.blockers(t, p)
    return {
        "schema_version": 1, "captured_at": captured_at.isoformat(),
        "environment": "staging", "source": {"sha": SHA, "tree": TREE},
        "temporal": t, "postgres": p,
        "status": "BLOCKED" if reasons else "DRAIN_CANDIDATE_ONLY",
        "blockers": reasons, "cutover_authorized": False,
    }


def replay_fixture(*, workflow_type="AlegriaAdmittedEffectDagV3Workflow",
                   task_queue="alegria-effect-dag-v5", history_count=1):
    """Create a captured census and a matching exported History envelope."""
    t = temporal()
    t["legacy_workflow_types"][workflow_type] = {"total": history_count, "open": 0}
    t["total_all_before"] = t["total_all_after"] = history_count
    capture = snap(captured_at=NOW - timedelta(seconds=10), temporal_data=t)
    history = json.dumps({"events": [{
        "eventType": "EVENT_TYPE_WORKFLOW_EXECUTION_STARTED",
        "workflowExecutionStartedEventAttributes": {
            "workflowType": {"name": workflow_type}, "taskQueue": {"name": task_queue},
        },
    }]}, sort_keys=True).encode("utf-8")
    manifest = {
        "schema_version": 1, "source": capture["source"],
        "environment": capture["environment"], "namespace": t["namespace"],
        "histories": [{
            "workflow_type": workflow_type, "task_queue": task_queue,
            "workflow_id": "workflow-1", "run_id": "run-1", "file": "history.json",
            "sha256": hashlib.sha256(history).hexdigest(),
        }],
    }
    return capture, manifest, history


class TemporalHistoryTransitionTests(unittest.TestCase):
    def test_inventory_sql_covers_authoritative_registry_and_receipts(self):
        sql = " ".join(transition.ROLE_SQL.split()).lower()
        for fragment in (
            "pipeline.tenant_context_ownership", "pipeline.pg_effect_v3_admissions",
            "pipeline.pg_effect_v3_receipts", "pipeline.execution_runs",
            "extracted.rule_candidates", "raw.sections", "raw.pages",
            "site.seo_hitl_tasks", "rolbypassrls", "tenant_id is distinct from",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, sql)

    def test_count_accepts_strict_temporal_integer_formats(self):
        self.assertEqual(transition.temporal_count('{"count": 0}'), 0)
        self.assertEqual(transition.temporal_count('{"count": "19"}'), 19)
        self.assertEqual(transition.temporal_count('{"count": 0, "groups": []}'), 0)

    def test_count_rejects_ambiguous_incomplete_or_boolean(self):
        for raw in ('[{}]', '{}', '{"count": -1}', '{"count": true}',
                    '{"count": 1.5}', '{"count": null}', '{"count": "01"}',
                    '{"count": 0, "other": 3}', '{"count": 0, "groups": [{"x":1}]}',
                    '{"count": "2e3"}', 'bad'):
            with self.subTest(raw=raw), self.assertRaises(transition.EvidenceError):
                transition.temporal_count(raw)

    def test_temporal_uses_server_count_before_and_after_all_type_counts(self):
        calls = []

        def command(argv, **kwargs):
            calls.append(argv)
            return '{"count": 0}'

        with patch.object(transition, "run_command", side_effect=command):
            result = transition.temporal_inventory("temporal.test:7233", "staging-site")
        self.assertEqual(len(calls), 4 + len(transition.LEGACY_TYPES) * 2)
        self.assertEqual(result["open_all_after"], 0)
        self.assertEqual(calls[0][0:3], ["temporal", "workflow", "count"])
        self.assertTrue(all("--output" in argv and "json" in argv for argv in calls))
        self.assertTrue(all("--namespace" in argv for argv in calls))

    def test_temporal_rejects_running_count_change(self):
        count = 0

        def command(argv, **kwargs):
            nonlocal count
            count += 1
            return '{"count": 1}' if count == 1 else '{"count": 0}'

        with patch.object(transition, "run_command", side_effect=command):
            with self.assertRaises(transition.EvidenceError):
                transition.temporal_inventory("temporal.test:7233", "staging-site")

    def test_live_temporal_rejects_unrecognized_completed_history(self):
        def command(argv, **kwargs):
            if "--query" not in argv:
                return '{"count": 1}'
            return '{"count": 0}'

        with patch.object(transition, "run_command", side_effect=command):
            with self.assertRaisesRegex(transition.EvidenceError, "unrecognized historical"):
                transition.temporal_inventory("temporal.test:7233", "staging-site")

    def test_temporal_rejects_token_in_address(self):
        with self.assertRaises(transition.EvidenceError):
            transition.temporal_inventory("https://user:password@temporal", "namespace")

    def test_pg_dsn_never_enters_argv_or_output(self):
        captured = {}

        def command(argv, **kwargs):
            captured["argv"] = argv
            captured["env"] = kwargs["env"]
            return json.dumps({"global_visibility": True, **{
                name: 0 for name in transition.COUNT_FIELDS
            }})

        with patch.dict("os.environ", {"ALEGRIA_HISTORY_AUDIT_DSN": "postgres://secret:123@pg/db"}), \
             patch.object(transition, "run_command", side_effect=command):
            result = transition.pg_inventory("ALEGRIA_HISTORY_AUDIT_DSN")
        self.assertEqual(result["unfinished_runs"], 0)
        self.assertNotIn("secret", " ".join(captured["argv"]))
        self.assertEqual(captured["env"]["PGDATABASE"], "postgres://secret:123@pg/db")
        self.assertNotIn("secret", json.dumps(result))

    def test_pg_rejects_rls_limited_role(self):
        payload = {"global_visibility": False, **{name: 0 for name in transition.COUNT_FIELDS}}
        with patch.dict("os.environ", {"ALEGRIA_HISTORY_AUDIT_DSN": "opaque"}), \
             patch.object(transition, "run_command", return_value=json.dumps(payload)):
            with self.assertRaises(transition.EvidenceError):
                transition.pg_inventory("ALEGRIA_HISTORY_AUDIT_DSN")

    def test_pg_rejects_missing_tenant_columns_and_schema(self):
        with patch.dict("os.environ", {"ALEGRIA_HISTORY_AUDIT_DSN": "opaque"}), \
             patch.object(transition, "run_command", side_effect=transition.EvidenceError("psql failed")):
            with self.assertRaises(transition.EvidenceError):
                transition.pg_inventory("ALEGRIA_HISTORY_AUDIT_DSN")

    def test_unowned_historical_data_blocks_drain(self):
        result = transition.blockers(temporal(), postgres(raw_pages_unowned=2))
        self.assertTrue(any("raw_pages_unowned" in s for s in result))

    def test_mismatched_cross_source_tenant_blocks_drain(self):
        result = transition.blockers(temporal(), postgres(candidate_source_mismatches=1))
        self.assertTrue(any("candidate_source_mismatches" in s for s in result))

    def test_unregistered_context_blocks_drain(self):
        result = transition.blockers(temporal(), postgres(unregistered_run_contexts=1))
        self.assertTrue(any("unregistered_run_contexts" in s for s in result))

    def test_cross_tenant_admission_or_receipt_blocks_drain(self):
        for name in ("admission_owner_mismatches", "receipt_owner_mismatches"):
            with self.subTest(name=name):
                result = transition.blockers(temporal(), postgres(**{name: 1}))
                self.assertTrue(any(name in s for s in result))

    def test_legacy_receipt_without_tenant_blocks_drain(self):
        result = transition.blockers(temporal(), postgres(receipts_unowned=1))
        self.assertTrue(any("receipts_unowned" in s for s in result))

    def test_nonterminal_run_blocks_drain(self):
        result = transition.blockers(temporal(), postgres(unfinished_runs=1))
        self.assertTrue(any("not terminal" in s for s in result))

    def test_historical_completed_workflow_still_requires_sdk_replay(self):
        result = transition.blockers(temporal(old=2), postgres())
        self.assertTrue(any("SDK replay" in s for s in result))

    def test_unknown_closed_type_blocks_drain_even_with_no_open_workflows(self):
        t = temporal()
        t["total_all_before"] = t["total_all_after"] = 1
        with self.assertRaises(transition.EvidenceError):
            transition.blockers(t, postgres())

    def test_known_inventory_covers_all_registered_temporal_workflows(self):
        import re
        root = Path(__file__).resolve().parents[1] / "app/rust/services"
        registered = set()
        for service in ("temporal", "temporal_orchestrator", "temporal_pg_effects"):
            for path in (root / service / "src").rglob("*.rs"):
                registered.update(re.findall(r"register_workflow::<([A-Za-z0-9_]+)>",
                                             path.read_text(encoding="utf-8")))
        self.assertTrue(registered)
        self.assertEqual(registered - set(transition.WORKFLOW_TYPES), set(),
                         "new Workflow Type must be added to the historical inventory")

    def test_missing_legacy_type_is_invalid_evidence(self):
        data = temporal()
        data["legacy_workflow_types"].pop(transition.LEGACY_TYPES[0])
        with self.assertRaises(transition.EvidenceError):
            transition.blockers(data, postgres())

    def test_capture_never_authorizes_cutover(self):
        with patch.object(transition, "exact_source", return_value={"sha": SHA, "tree": TREE}), \
             patch.object(transition, "temporal_inventory", return_value=temporal()), \
             patch.object(transition, "pg_inventory", return_value=postgres()):
            result = transition.capture(environment="staging", address="temporal.test:7233",
                                        namespace="staging-site", dsn_env="AUDIT_DSN", source_sha=SHA)
        self.assertEqual(result["status"], "DRAIN_CANDIDATE_ONLY")
        self.assertFalse(result["cutover_authorized"])
        self.assertIn("official_sdk_history_replay_and_worker_version_routing",
                      result["unverified_external_gates"])

    def test_pair_preconditions_pass_but_production_not_authorized(self):
        first = snap(captured_at=NOW - timedelta(seconds=75))
        second = snap(captured_at=NOW - timedelta(seconds=30))
        result = transition.check_pair(first, second, clock=NOW)
        self.assertEqual(result["status"], "DRAIN_PRECONDITIONS_CAPTURED")
        self.assertFalse(result["cutover_authorized"])
        self.assertTrue(result["still_required"])

    def test_pair_rejects_same_timestamp(self):
        first = snap(captured_at=NOW - timedelta(seconds=30))
        second = snap(captured_at=NOW - timedelta(seconds=30))
        self.assertEqual(transition.check_pair(first, second, clock=NOW)["status"], "BLOCKED")

    def test_pair_rejects_changed_server_and_source(self):
        first = snap(captured_at=NOW - timedelta(seconds=70))
        second = snap(captured_at=NOW - timedelta(seconds=30))
        second["source"] = {"sha": "e" * 40, "tree": TREE}
        second["temporal"]["namespace"] = "production"
        result = transition.check_pair(first, second, clock=NOW)
        self.assertEqual(result["status"], "BLOCKED")
        self.assertTrue(any("changed" in e for e in result["errors"]))

    def test_pair_rejects_forged_cutover_permission(self):
        first = snap(captured_at=NOW - timedelta(seconds=75))
        second = snap(captured_at=NOW - timedelta(seconds=30))
        second["cutover_authorized"] = True
        result = transition.check_pair(first, second, clock=NOW)
        self.assertEqual(result["status"], "BLOCKED")

    def test_pair_rechecks_untrusted_inventory_not_status_strings(self):
        first = snap(captured_at=NOW - timedelta(seconds=75))
        second = snap(captured_at=NOW - timedelta(seconds=30))
        second["postgres"]["execution_runs_unowned"] = 1
        result = transition.check_pair(first, second, clock=NOW)
        self.assertEqual(result["status"], "BLOCKED")

    def test_pair_rejects_non_global_pg_visibility(self):
        first = snap(captured_at=NOW - timedelta(seconds=75))
        second = snap(captured_at=NOW - timedelta(seconds=30))
        second["postgres"]["global_visibility"] = False
        result = transition.check_pair(first, second, clock=NOW)
        self.assertEqual(result["status"], "BLOCKED")

    def test_atomic_report_roundtrip(self):
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder) / "report.json"
            transition.atomic_json(output, {"status": "BLOCKED", "cutover_authorized": False})
            self.assertEqual(json.loads(output.read_text())["status"], "BLOCKED")
            self.assertEqual(list(Path(folder).glob("*.tmp")), [])


class TemporalOfflineReplayAuditTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)
        self.capture, self.manifest, self.history = replay_fixture()
        (self.root / "history.json").write_bytes(self.history)
        self.binary = self.root / "temporal_orchestrator"
        self.binary.write_bytes(b"fake executable for mocked test")
        self.binary.chmod(0o700)
        self.binaries = {"orchestrator": self.binary}

    def audit(self, *, marker="CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS"):
        with patch.object(transition, "exact_source", return_value={"sha": SHA, "tree": TREE}), \
             patch.object(transition, "run_command", return_value=marker) as run:
            result = transition.replay_audit(
                self.capture, self.manifest, history_root=self.root,
                binaries=self.binaries, clock=NOW,
            )
        return result, run

    def test_real_registered_sdk_replay_route_never_authorizes_cutover(self):
        result, command = self.audit()
        self.assertEqual(result["status"], "OFFLINE_SDK_REPLAY_CANDIDATE_ONLY")
        self.assertFalse(result["cutover_authorized"])
        self.assertEqual(result["replay_count"], 1)
        self.assertEqual(result["replayed_by_type"]["AlegriaAdmittedEffectDagV3Workflow"], 1)
        self.assertEqual(result["replayed_sha256"], [hashlib.sha256(self.history).hexdigest()])
        self.assertIn("independent_server_history_export_identity_and_completeness",
                      result["unverified_external_gates"])
        args, kwargs = command.call_args
        self.assertEqual(args[0][:2], [str(self.binary), "--replay-v5-effect-history"])
        self.assertNotEqual(args[0][2], str(self.root / "history.json"),
                            "SDK must only see a protected copy, not mutable export path")
        self.assertFalse(Path(args[0][2]).exists(), "protected replay copy must be removed")
        self.assertEqual(set(kwargs), {"env"})
        self.assertTrue(set(kwargs["env"]) <= set(transition.REPLAY_ENV_ALLOW))
        self.assertEqual(result["replayed_runs"], [{
            "workflow_type": "AlegriaAdmittedEffectDagV3Workflow",
            "task_queue": "alegria-effect-dag-v5", "workflow_id": "workflow-1",
            "run_id": "run-1", "sha256": hashlib.sha256(self.history).hexdigest(),
        }])

    def test_history_swap_after_hash_cannot_change_sdk_replayed_bytes(self):
        observed = {}

        def swap_mutable_export(argv, *, env):
            immutable_path = Path(argv[2])
            observed["copied_path"] = immutable_path
            observed["before"] = immutable_path.read_bytes()
            observed["copy_mode"] = immutable_path.stat().st_mode & 0o777
            (self.root / "history.json").write_bytes(b"malicious replacement after hash check")
            observed["after"] = immutable_path.read_bytes()
            return "CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS"

        with patch.object(transition, "exact_source", return_value={"sha": SHA, "tree": TREE}), \
             patch.object(transition, "run_command", side_effect=swap_mutable_export):
            result = transition.replay_audit(
                self.capture, self.manifest, history_root=self.root,
                binaries=self.binaries, clock=NOW,
            )
        self.assertEqual(observed["before"], self.history)
        self.assertEqual(observed["after"], self.history)
        self.assertEqual(observed["copy_mode"], 0o400)
        self.assertNotEqual(observed["copied_path"], self.root / "history.json")
        self.assertFalse(observed["copied_path"].exists(), "copy cleaned even after mutable export swap")
        self.assertEqual(result["replayed_runs"][0]["sha256"], hashlib.sha256(self.history).hexdigest())

    def test_protected_copy_is_cleaned_after_native_replay_failure(self):
        seen = []

        def replay_fail(argv, *, env):
            seen.append(Path(argv[2]))
            self.assertTrue(seen[-1].exists())
            return "not a native replay PASS marker"

        with patch.object(transition, "exact_source", return_value={"sha": SHA, "tree": TREE}), \
             patch.object(transition, "run_command", side_effect=replay_fail), \
             self.assertRaisesRegex(transition.EvidenceError, "did not attest PASS"):
            transition.replay_audit(self.capture, self.manifest, history_root=self.root,
                                    binaries=self.binaries, clock=NOW)
        self.assertEqual(len(seen), 1)
        self.assertFalse(seen[0].exists())

    def test_native_sdk_process_never_inherits_temporal_pg_or_provider_credentials(self):
        with patch.dict(os.environ, {
            "ALEGRIA_HISTORY_AUDIT_DSN": "postgres://secret@database",
            "TEMPORAL_API_KEY": "temporal-secret",
            "OPENAI_API_KEY": "provider-secret",
        }):
            _, command = self.audit()
        self.assertNotIn("ALEGRIA_HISTORY_AUDIT_DSN", command.call_args.kwargs["env"])
        self.assertNotIn("TEMPORAL_API_KEY", command.call_args.kwargs["env"])
        self.assertNotIn("OPENAI_API_KEY", command.call_args.kwargs["env"])

    def test_history_type_and_queue_come_from_first_actual_start_event(self):
        self.assertEqual(transition.history_type_and_queue(self.history),
                         ("AlegriaAdmittedEffectDagV3Workflow", "alegria-effect-dag-v5"))
        for invalid in (b"not JSON", b"{}", b'{"events": []}',
                        b'{"events": [{"eventType": "EVENT_TYPE_WORKFLOW_TASK_SCHEDULED"}]}'):
            with self.subTest(invalid=invalid), self.assertRaises(transition.EvidenceError):
                transition.history_type_and_queue(invalid)

    def test_missing_history_count_blocks_before_any_sdk_invocation(self):
        self.capture["temporal"]["legacy_workflow_types"]["AlegriaAdmittedEffectDagV3Workflow"]["total"] = 2
        self.capture["temporal"]["total_all_before"] = 2
        self.capture["temporal"]["total_all_after"] = 2
        with patch.object(transition, "exact_source", return_value={"sha": SHA, "tree": TREE}), \
             patch.object(transition, "run_command") as command, \
             self.assertRaisesRegex(transition.EvidenceError, "every counted"):
            transition.replay_audit(self.capture, self.manifest, history_root=self.root,
                                    binaries=self.binaries, clock=NOW)
        command.assert_not_called()

    def test_unsupported_retired_workflow_type_fails_closed(self):
        self.capture, self.manifest, history = replay_fixture(
            workflow_type="SeoSiteBuildWorkflow", task_queue="alegria-pipeline",
        )
        (self.root / "history.json").write_bytes(history)
        with patch.object(transition, "exact_source", return_value={"sha": SHA, "tree": TREE}), \
             patch.object(transition, "run_command") as command, \
             self.assertRaisesRegex(transition.EvidenceError, "lacks an SDK replay route"):
            transition.replay_audit(self.capture, self.manifest, history_root=self.root,
                                    binaries=self.binaries, clock=NOW)
        command.assert_not_called()

    def test_manifest_mismatched_task_queue_or_workflow_type_blocks(self):
        for field, modified in (
            ("task_queue", "alegria-pure-dag-v3"),
            ("workflow_type", "AlegriaAdmittedEffectDagV2Workflow"),
        ):
            with self.subTest(field=field):
                original = self.manifest["histories"][0][field]
                self.manifest["histories"][0][field] = modified
                with self.assertRaises(transition.EvidenceError):
                    self.audit()
                self.manifest["histories"][0][field] = original

    def test_manifest_hash_and_exported_bytes_must_match(self):
        self.manifest["histories"][0]["sha256"] = "a" * 64
        with self.assertRaisesRegex(transition.EvidenceError, "SHA-256"):
            self.audit()

    def test_manifest_duplicate_run_and_history_rejected(self):
        self.manifest["histories"].append(dict(self.manifest["histories"][0]))
        with self.assertRaisesRegex(transition.EvidenceError, "duplicate"):
            self.audit()

    def test_path_traversal_symlink_and_oversized_exports_rejected(self):
        self.manifest["histories"][0]["file"] = "../secrets.json"
        with self.assertRaisesRegex(transition.EvidenceError, "escapes export directory"):
            self.audit()
        self.manifest["histories"][0]["file"] = "sym.json"
        (self.root / "sym.json").symlink_to(self.root / "history.json")
        with self.assertRaisesRegex(transition.EvidenceError, "symlink"):
            self.audit()
        self.manifest["histories"][0]["file"] = "history.json"
        with patch.object(transition, "MAX_HISTORY_BYTES", 1), \
             self.assertRaisesRegex(transition.EvidenceError, "oversized"):
            self.audit()

    def test_pg_owner_mismatch_and_open_workflows_block_replay(self):
        self.capture["postgres"]["candidate_source_mismatches"] = 1
        with self.assertRaisesRegex(transition.EvidenceError, "PG tenant blockers"):
            self.audit()
        self.capture["postgres"]["candidate_source_mismatches"] = 0
        self.capture["temporal"]["legacy_workflow_types"]["AlegriaAdmittedEffectDagV3Workflow"]["open"] = 1
        self.capture["temporal"]["open_all_before"] = 1
        self.capture["temporal"]["open_all_after"] = 1
        with self.assertRaisesRegex(transition.EvidenceError, "Temporal drain"):
            self.audit()

    def test_capture_source_binding_and_freshness_immutable(self):
        self.manifest["source"] = {"sha": "e" * 40, "tree": TREE}
        with self.assertRaisesRegex(transition.EvidenceError, "bound to captured source"):
            self.audit()
        self.manifest["source"] = self.capture["source"]
        self.capture["captured_at"] = (NOW - timedelta(minutes=11)).isoformat()
        with self.assertRaisesRegex(transition.EvidenceError, "stale"):
            self.audit()

    def test_replay_rejects_boolean_schema_and_untyped_git_identity(self):
        self.manifest["schema_version"] = True
        with self.assertRaisesRegex(transition.EvidenceError, "manifest"):
            self.audit()
        self.manifest["schema_version"] = 1
        self.capture["source"]["sha"] = 3
        with self.assertRaisesRegex(transition.EvidenceError, "exact source identity"):
            self.audit()

    def test_native_replayer_must_really_return_exact_pass_marker(self):
        with self.assertRaisesRegex(transition.EvidenceError, "did not attest PASS"):
            self.audit(marker="ignored CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS unrelated")

    def test_unexecutable_binary_is_not_invoked(self):
        self.binary.chmod(0o600)
        with self.assertRaisesRegex(transition.EvidenceError, "required executable"):
            self.audit()

    def test_available_replay_routes_match_native_sdk_registered_workflows(self):
        self.assertEqual(set(transition.REPLAY_ROUTES) - {
            ("SeoSiteBuildCanonicalCutoverWorkflow", "alegria-pipeline"),
            ("SeoSiteBuildComposableV2Workflow", "alegria-pipeline"),
            ("AlegriaPureOperationDagV1Workflow", "alegria-pipeline"),
            ("AlegriaPureOperationDagV1Workflow", "alegria-pure-dag-v3"),
            ("AlegriaCheckedEffectDagV1Workflow", "alegria-pure-dag-v3"),
            ("AlegriaAdmittedEffectDagV2Workflow", "alegria-effect-dag-v4"),
            ("AlegriaAdmittedEffectDagV3Workflow", "alegria-effect-dag-v5"),
            ("PgTruthReviewReconcileV3Workflow", "alegria-pg-command-v3"),
            ("PgTruthReviewReconcileV4Workflow", "alegria-pg-command-v3"),
        }, set(), "SDK replay queue routes require explicit test coverage")
        self.assertTrue(set(t for t, _ in transition.REPLAY_ROUTES) <= set(transition.WORKFLOW_TYPES))

    def test_two_stable_historical_censuses_can_attach_replay_candidate(self):
        evidence, _ = self.audit()
        earlier = snap(
            captured_at=NOW - timedelta(seconds=75),
            temporal_data=self.capture["temporal"],
        )
        later = self.capture
        result = transition.check_pair(earlier, later, replay_evidence=evidence, clock=NOW)
        self.assertEqual(result["status"], "HISTORY_REPLAY_PRECONDITIONS_CAPTURED")
        self.assertFalse(result["cutover_authorized"])
        self.assertIn("independent_server_attestation_of_exported_history_and_run_identity",
                      result["still_required"])
        self.assertEqual(transition.check_pair(earlier, later, clock=NOW)["status"], "BLOCKED")

    def test_historical_pair_rejects_forged_replay_coverage(self):
        evidence, _ = self.audit()
        earlier = snap(
            captured_at=NOW - timedelta(seconds=75),
            temporal_data=self.capture["temporal"],
        )
        for mutate in (
            lambda: evidence.update({"cutover_authorized": True}),
            lambda: evidence.update({"replay_count": 2}),
            lambda: evidence.update({"replayed_by_type": {}}),
            lambda: evidence.update({"source": {"sha": "d" * 40, "tree": TREE}}),
            lambda: evidence.update({"replayed_runs": []}),
        ):
            with self.subTest(mutate=mutate):
                tested = dict(evidence)
                mutate()
                self.assertEqual(
                    transition.check_pair(earlier, self.capture,
                                          replay_evidence=evidence, clock=NOW)["status"],
                    "BLOCKED",
                )
                evidence = tested

    def test_historical_pair_rejects_forged_duplicate_run_mapping(self):
        self.capture, self.manifest, self.history = replay_fixture(history_count=2)
        (self.root / "history.json").write_bytes(self.history)
        second_history = json.dumps({"events": [{
            "eventType": "EVENT_TYPE_WORKFLOW_EXECUTION_STARTED",
            "workflowExecutionStartedEventAttributes": {
                "workflowType": {"name": "AlegriaAdmittedEffectDagV3Workflow"},
                "taskQueue": {"name": "alegria-effect-dag-v5"},
            },
            "eventId": 2,
        }]}).encode("utf-8")
        (self.root / "history-2.json").write_bytes(second_history)
        second = dict(self.manifest["histories"][0])
        second.update({
            "workflow_id": "workflow-2", "run_id": "run-2", "file": "history-2.json",
            "sha256": hashlib.sha256(second_history).hexdigest(),
        })
        self.manifest["histories"].append(second)
        evidence, _ = self.audit()
        self.assertEqual(evidence["replay_count"], 2)
        earlier = snap(captured_at=NOW - timedelta(seconds=75),
                       temporal_data=self.capture["temporal"])
        self.assertEqual(transition.check_pair(
            earlier, self.capture, replay_evidence=evidence, clock=NOW,
        )["status"], "HISTORY_REPLAY_PRECONDITIONS_CAPTURED")
        forged = dict(evidence)
        forged["replayed_runs"] = [dict(evidence["replayed_runs"][0]),
                                   dict(evidence["replayed_runs"][1])]
        forged["replayed_runs"][1]["workflow_id"] = forged["replayed_runs"][0]["workflow_id"]
        forged["replayed_runs"][1]["run_id"] = forged["replayed_runs"][0]["run_id"]
        result = transition.check_pair(earlier, self.capture,
                                       replay_evidence=forged, clock=NOW)
        self.assertEqual(result["status"], "BLOCKED")
        self.assertTrue(any("per-run" in reason for reason in result["errors"]))

    def test_historical_pair_rechecks_blockers_before_ignoring_replay_only(self):
        evidence, _ = self.audit()
        earlier = snap(
            captured_at=NOW - timedelta(seconds=75),
            temporal_data=self.capture["temporal"],
        )
        earlier["postgres"]["raw_pages_unowned"] = 1
        self.assertEqual(transition.check_pair(
            earlier, self.capture, replay_evidence=evidence, clock=NOW,
        )["status"], "BLOCKED")

    def test_read_only_cli_replay_and_compare_files_end_to_end(self):
        first = snap(captured_at=NOW - timedelta(seconds=75),
                     temporal_data=self.capture["temporal"])
        files = {
            "first.json": first,
            "capture.json": self.capture,
            "manifest.json": self.manifest,
        }
        for name, value in files.items():
            (self.root / name).write_text(json.dumps(value), encoding="utf-8")
        replay_report = self.root / "sdk-replay-result.json"
        compare_report = self.root / "preconditions-result.json"
        with patch.object(transition, "now_utc", return_value=NOW), \
             patch.object(transition, "exact_source", return_value={"sha": SHA, "tree": TREE}), \
             patch.object(transition, "run_command", return_value="CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS"):
            rc_replay = transition.main([
                "replay-audit", "--capture", str(self.root / "capture.json"),
                "--manifest", str(self.root / "manifest.json"),
                "--history-root", str(self.root),
                "--legacy-bin", str(self.binary), "--orchestrator-bin", str(self.binary),
                "--pg-bin", str(self.binary), "--output", str(replay_report),
            ])
            rc_compare = transition.main([
                "compare", "--earlier", str(self.root / "first.json"),
                "--later", str(self.root / "capture.json"),
                "--replay-evidence", str(replay_report), "--output", str(compare_report),
            ])
        self.assertEqual(rc_replay, 0)
        self.assertEqual(rc_compare, 0)
        self.assertEqual(json.loads(replay_report.read_text())["status"],
                         "OFFLINE_SDK_REPLAY_CANDIDATE_ONLY")
        report = json.loads(compare_report.read_text())
        self.assertEqual(report["status"], "HISTORY_REPLAY_PRECONDITIONS_CAPTURED")
        self.assertFalse(report["cutover_authorized"])


if __name__ == "__main__":
    unittest.main()
