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
        "quarantine_by_relation": {
            relation: {"unattributed": 0, "inconsistent": 0}
            for relation in transition.QUARANTINE_RELATIONS
        },
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
            "pipeline.tenant_historical_quarantine_v1()", "quarantine_by_relation",
            "left join raw.pages", "left join raw.sections",
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
            }, "quarantine_by_relation": postgres()["quarantine_by_relation"]})

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

    def test_ten_relation_quarantine_census_positive_or_incomplete_blocks(self):
        p = postgres(historical_quarantine_unattributed=1)
        p["quarantine_by_relation"]["raw.pages"]["unattributed"] = 1
        reasons = transition.blockers(temporal(), p)
        self.assertTrue(any("historical_quarantine_unattributed" in reason
                            for reason in reasons))
        p["historical_quarantine_unattributed"] = 0
        with self.assertRaisesRegex(transition.EvidenceError, "sum differs"):
            transition.blockers(temporal(), p)
        p = postgres()
        p["quarantine_by_relation"].pop("pipeline.qdrant_projection_v1_attempts")
        with self.assertRaisesRegex(transition.EvidenceError, "ten authoritative"):
            transition.blockers(temporal(), p)

    def test_historical_quarantine_inconsistent_rows_block_drain(self):
        p = postgres(historical_quarantine_inconsistent=2)
        p["quarantine_by_relation"]["pipeline.pg_effect_v3_receipts"]["inconsistent"] = 2
        self.assertTrue(any("historical_quarantine_inconsistent" in reason
                            for reason in transition.blockers(temporal(), p)))

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
        for service in ("temporal", "temporal_orchestrator", "temporal_pg_effects",
                        "temporal_qdrant_effects"):
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
        self.assertEqual(result["native_replayers"], {
            "orchestrator": {
                "sha256": hashlib.sha256(self.binary.read_bytes()).hexdigest(),
                "size_bytes": self.binary.stat().st_size,
            },
        })
        self.assertIn("independent_server_history_export_identity_and_completeness",
                      result["unverified_external_gates"])
        args, kwargs = command.call_args
        self.assertEqual(args[0][1], "--replay-v5-effect-history")
        self.assertEqual(Path(args[0][0]).name, "replayer-orchestrator")
        self.assertNotEqual(args[0][0], str(self.binary))
        self.assertFalse(Path(args[0][0]).exists(), "protected executable copy must be removed")
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

    def test_versioned_qdrant_parent_and_child_use_different_native_replayers(self):
        for workflow_type, queue, binary_owner, argument, marker in (
            ("AlegriaQdrantProjectionDagV1Workflow", "alegria-qdrant-dag-v1",
             "orchestrator", "--replay-qdrant-effect-history", "CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS"),
            ("QdrantKeywordProjectionV1Workflow", "alegria-qdrant-command-v1",
             "qdrant", "--replay-history", "CI_TEMPORAL_QDRANT_EFFECT_REPLAY=PASS"),
        ):
            with self.subTest(workflow_type=workflow_type):
                self.capture, self.manifest, history = replay_fixture(
                    workflow_type=workflow_type, task_queue=queue,
                )
                (self.root / "history.json").write_bytes(history)
                self.binaries = {binary_owner: self.binary}
                with patch.object(transition, "exact_source", return_value={"sha": SHA, "tree": TREE}), \
                     patch.object(transition, "run_command", return_value=marker) as command:
                    result = transition.replay_audit(
                        self.capture, self.manifest,
                        history_root=self.root, binaries=self.binaries, clock=NOW,
                    )
                self.assertEqual(result["replay_count"], 1)
                self.assertFalse(result["cutover_authorized"])
                argv = command.call_args.args[0]
                self.assertEqual(argv[1], argument)
                self.assertEqual(Path(argv[0]).name, f"replayer-{binary_owner}")
                self.assertFalse(Path(argv[0]).exists())
                self.assertNotEqual(argv[2], str(self.root / "history.json"))

    def test_new_pg_qdrant_native_owner_and_validate_histories_replay_on_exact_queue(self):
        routes = (
            ("PgTruthReviewReconcileV5Workflow", "alegria-pg-command-v5",
             "pg", "--replay-v5-history", "CI_TEMPORAL_PG_EFFECT_V5_REPLAY=PASS"),
            ("AlegriaOwnerOperationV1Workflow", "alegria-pg-owner-v1",
             "pg", "--replay-owner-history", "CI_TEMPORAL_PG_OWNER_REPLAY=PASS"),
            ("AlegriaOwnerValidateV1Workflow", "alegria-pg-owner-v1",
             "pg", "--replay-owner-history", "CI_TEMPORAL_PG_OWNER_REPLAY=PASS"),
            ("QdrantKeywordProjectionV2Workflow", "alegria-qdrant-command-v2",
             "qdrant", "--replay-history-v2", "CI_TEMPORAL_QDRANT_EFFECT_V2_REPLAY=PASS"),
            ("AlegriaOwnerOperationV1Workflow", "alegria-qdrant-owner-v1",
             "qdrant", "--replay-history-generic-v1",
             "CI_TEMPORAL_QDRANT_GENERIC_OWNER_V1_REPLAY=PASS"),
            ("AlegriaOwnerValidateV1Workflow", "alegria-qdrant-owner-v1",
             "qdrant", "--replay-history-generic-v1",
             "CI_TEMPORAL_QDRANT_GENERIC_OWNER_V1_REPLAY=PASS"),
        )
        for workflow_type, queue, owner, flag, marker in routes:
            with self.subTest(workflow_type=workflow_type, queue=queue):
                self.capture, self.manifest, history = replay_fixture(
                    workflow_type=workflow_type, task_queue=queue,
                )
                (self.root / "history.json").write_bytes(history)
                self.binaries = {owner: self.binary}
                with patch.object(transition, "exact_source", return_value={"sha": SHA, "tree": TREE}), \
                     patch.object(transition, "run_command", return_value=marker) as command:
                    result = transition.replay_audit(
                        self.capture, self.manifest, history_root=self.root,
                        binaries=self.binaries, clock=NOW,
                    )
                self.assertEqual(result["status"], "OFFLINE_SDK_REPLAY_CANDIDATE_ONLY")
                self.assertFalse(result["cutover_authorized"])
                self.assertEqual(result["replay_count"], 1)
                argv = command.call_args.args[0]
                self.assertEqual(argv[1], flag)
                self.assertEqual(Path(argv[0]).name, f"replayer-{owner}")
                self.assertFalse(Path(argv[0]).exists())
                self.assertFalse(Path(argv[2]).exists())
                self.assertTrue(set(command.call_args.kwargs["env"]) <= set(transition.REPLAY_ENV_ALLOW))

    def test_new_owner_wrong_queue_and_catalog_dependent_parents_fail_closed(self):
        for workflow_type, queue in (
            ("AlegriaOwnerOperationV1Workflow", "alegria-pg-command-v3"),
            ("AlegriaOwnerValidateV1Workflow", "alegria-pg-command-v5"),
            ("AlegriaOwnerValidateV1Workflow", "alegria-qdrant-command-v2"),
            ("QdrantKeywordProjectionV2Workflow", "alegria-qdrant-owner-v1"),
            ("PgTruthReviewReconcileV5Workflow", "alegria-pg-command-v3"),
            # Generic parents require an independently bound immutable owner
            # catalog; the Python scrubbed-environment replayer has none.
            ("AlegriaOperationDagV1Workflow", "alegria-operation-dag-v1"),
            ("AlegriaOperationDagV2Workflow", "alegria-operation-dag-v2"),
        ):
            with self.subTest(workflow_type=workflow_type, queue=queue):
                self.capture, self.manifest, history = replay_fixture(
                    workflow_type=workflow_type, task_queue=queue,
                )
                (self.root / "history.json").write_bytes(history)
                with patch.object(transition, "exact_source", return_value={"sha": SHA, "tree": TREE}), \
                     patch.object(transition, "run_command") as command, \
                     self.assertRaisesRegex(transition.EvidenceError, "lacks an SDK replay route"):
                    transition.replay_audit(
                        self.capture, self.manifest, history_root=self.root,
                        binaries={"pg": self.binary, "qdrant": self.binary}, clock=NOW,
                    )
                command.assert_not_called()

    def test_same_generic_workflow_type_replays_with_two_independent_owner_binaries(self):
        self.capture, self.manifest, pg_history = replay_fixture(
            workflow_type="AlegriaOwnerOperationV1Workflow",
            task_queue="alegria-pg-owner-v1",
        )
        (self.root / "history.json").write_bytes(pg_history)
        q_history = json.dumps({"events": [{
            "eventType": "EVENT_TYPE_WORKFLOW_EXECUTION_STARTED",
            "workflowExecutionStartedEventAttributes": {
                "workflowType": {"name": "AlegriaOwnerOperationV1Workflow"},
                "taskQueue": {"name": "alegria-qdrant-owner-v1"},
            },
        }]}, sort_keys=True).encode("utf-8")
        (self.root / "qdrant-history.json").write_bytes(q_history)
        q_item = dict(self.manifest["histories"][0])
        q_item.update({
            "workflow_id": "other-owner-workflow", "run_id": "other-owner-run",
            "task_queue": "alegria-qdrant-owner-v1", "file": "qdrant-history.json",
            "sha256": hashlib.sha256(q_history).hexdigest(),
        })
        self.manifest["histories"].append(q_item)
        counts = self.capture["temporal"]["legacy_workflow_types"]
        counts["AlegriaOwnerOperationV1Workflow"]["total"] = 2
        self.capture["temporal"]["total_all_before"] = 2
        self.capture["temporal"]["total_all_after"] = 2
        self.capture["blockers"] = transition.blockers(
            self.capture["temporal"], self.capture["postgres"])
        q_binary = self.root / "temporal_qdrant_effects"
        q_binary.write_bytes(b"independently built Qdrant owner replayer")
        q_binary.chmod(0o700)

        def native_marker(argv, *, env):
            self.assertEqual(set(env) - set(transition.REPLAY_ENV_ALLOW), set())
            return {
                "--replay-owner-history": "CI_TEMPORAL_PG_OWNER_REPLAY=PASS",
                "--replay-history-generic-v1": "CI_TEMPORAL_QDRANT_GENERIC_OWNER_V1_REPLAY=PASS",
            }[argv[1]]

        with patch.object(transition, "exact_source", return_value={"sha": SHA, "tree": TREE}), \
             patch.object(transition, "run_command", side_effect=native_marker) as commands:
            result = transition.replay_audit(
                self.capture, self.manifest, history_root=self.root,
                binaries={"pg": self.binary, "qdrant": q_binary}, clock=NOW,
            )
        self.assertEqual(result["replayed_by_type"]["AlegriaOwnerOperationV1Workflow"], 2)
        self.assertEqual(set(result["native_replayers"]), {"pg", "qdrant"})
        self.assertEqual({call.args[0][1] for call in commands.call_args_list}, {
            "--replay-owner-history", "--replay-history-generic-v1",
        })
        self.assertEqual(result["status"], "OFFLINE_SDK_REPLAY_CANDIDATE_ONLY")
        self.assertFalse(result["cutover_authorized"])

    def test_native_generic_replay_cli_registration_has_queue_correct_workflows(self):
        services = Path(__file__).resolve().parents[1] / "app/rust/services"
        pg = (services / "temporal_pg_effects/src/main.rs").read_text(encoding="utf-8")
        qdrant = (services / "temporal_qdrant_effects/src/main.rs").read_text(encoding="utf-8")
        generic = (services / "temporal_qdrant_effects/src/owner_generic_v1.rs").read_text(
            encoding="utf-8")
        self.assertIn('OsStr::new("--replay-owner-history")', pg)
        self.assertIn('OsStr::new("--replay-v5-history")', pg)
        self.assertIn('replay_history_for_queue(std::path::Path::new(&path), PG_OWNER_TASK_QUEUE_V1)', pg)
        self.assertIn('replay_history_for_queue(std::path::Path::new(&path), V5_CHILD_COMMAND_TASK_QUEUE)', pg)
        self.assertIn('options.register_workflow::<AlegriaOwnerValidateV1Workflow>()?', pg)
        self.assertIn('OsStr::new("--replay-history-v2")', qdrant)
        self.assertIn('OsStr::new("--replay-history-generic-v1")', qdrant)
        self.assertIn('task_queue(QDRANT_GENERIC_OWNER_QUEUE_V1.to_owned())', generic)
        self.assertIn('options.register_workflow::<AlegriaOwnerValidateV1Workflow>()?', generic)

    def test_sdk_replayer_receives_verified_immutable_executable_bytes(self):
        observed = {}

        def swap_original_binary(argv, *, env):
            copied = Path(argv[0])
            observed["bytes"] = copied.read_bytes()
            observed["mode"] = copied.stat().st_mode & 0o777
            self.binary.write_bytes(b"modified after immutable SDK copy")
            observed["after"] = copied.read_bytes()
            observed["path"] = copied
            return "CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS"

        original = self.binary.read_bytes()
        with patch.object(transition, "exact_source", return_value={"sha": SHA, "tree": TREE}), \
             patch.object(transition, "run_command", side_effect=swap_original_binary):
            result = transition.replay_audit(
                self.capture, self.manifest, history_root=self.root,
                binaries=self.binaries, clock=NOW,
            )
        self.assertEqual(observed["bytes"], original)
        self.assertEqual(observed["after"], original)
        self.assertEqual(observed["mode"], 0o500)
        self.assertFalse(observed["path"].exists())
        self.assertEqual(result["native_replayers"]["orchestrator"]["sha256"],
                         hashlib.sha256(original).hexdigest())

    def test_replay_rejects_symlinked_oversized_or_empty_native_binary(self):
        original = self.binary
        symlink = self.root / "binary-symlink"
        symlink.symlink_to(original)
        self.binaries["orchestrator"] = symlink
        with self.assertRaisesRegex(transition.EvidenceError, "required executable"):
            self.audit()
        self.binaries["orchestrator"] = original
        with patch.object(transition, "MAX_REPLAYER_BYTES", 1), \
             self.assertRaisesRegex(transition.EvidenceError, "size exceeds"):
            self.audit()
        original.write_bytes(b"")
        with self.assertRaisesRegex(transition.EvidenceError, "size exceeds"):
            self.audit()

    def test_history_type_and_queue_come_from_first_actual_start_event(self):
        self.assertEqual(transition.history_type_and_queue(self.history),
                         ("AlegriaAdmittedEffectDagV3Workflow", "alegria-effect-dag-v5"))
        for invalid in (b"not JSON", b"{}", b'{"events": []}',
                        b'{"events": [{"eventType": "EVENT_TYPE_WORKFLOW_TASK_SCHEDULED"}]}'):
            with self.subTest(invalid=invalid), self.assertRaises(transition.EvidenceError):
                transition.history_type_and_queue(invalid)

    def test_sdk_protobuf_nested_oneof_history_start_layout_is_supported(self):
        # Rust E2E obtains WorkflowHistory::to_json(), which serializes the
        # HistoryEvent.attributes oneof, unlike flat Temporal CLI JSON.
        for started_key in (
            "WorkflowExecutionStartedEventAttributes",
            "workflowExecutionStartedEventAttributes",
            "workflow_execution_started_event_attributes",
        ):
            payload = {
                "events": [{
                    "event_type": 1,
                    "attributes": {started_key: {
                        "workflow_type": {"name": "AlegriaAdmittedEffectDagV3Workflow"},
                        "task_queue": {"name": "alegria-effect-dag-v5"},
                    }},
                }],
            }
            with self.subTest(started_key=started_key):
                self.assertEqual(transition.history_type_and_queue(
                    json.dumps(payload).encode()),
                    ("AlegriaAdmittedEffectDagV3Workflow", "alegria-effect-dag-v5"))
        flat = json.loads(self.history)
        flat["events"][0]["attributes"] = {
            "WorkflowExecutionStartedEventAttributes": flat["events"][0][
                "workflowExecutionStartedEventAttributes"],
        }
        with self.assertRaisesRegex(transition.EvidenceError, "ambiguous"):
            transition.history_type_and_queue(json.dumps(flat).encode())

    def test_history_rejects_dual_root_and_event_type_aliases(self):
        nested = json.loads(self.history)
        nested["history"] = {"events": list(nested["events"])}
        with self.assertRaisesRegex(transition.EvidenceError, "ambiguous"):
            transition.history_type_and_queue(json.dumps(nested).encode())
        aliases = json.loads(self.history)
        aliases["events"][0]["event_type"] = "EVENT_TYPE_WORKFLOW_TASK_SCHEDULED"
        with self.assertRaisesRegex(transition.EvidenceError, "ambiguous"):
            transition.history_type_and_queue(json.dumps(aliases).encode())

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
            ("AlegriaPureOperationDagV2Workflow", "alegria-pure-dag-v4"),
            ("AlegriaCheckedEffectDagV1Workflow", "alegria-pure-dag-v3"),
            ("AlegriaAdmittedEffectDagV2Workflow", "alegria-effect-dag-v4"),
            ("AlegriaAdmittedEffectDagV3Workflow", "alegria-effect-dag-v5"),
            ("PgTruthReviewReconcileV3Workflow", "alegria-pg-command-v3"),
            ("PgTruthReviewReconcileV4Workflow", "alegria-pg-command-v3"),
            ("PgTruthReviewReconcileV5Workflow", "alegria-pg-command-v5"),
            ("AlegriaOwnerOperationV1Workflow", "alegria-pg-owner-v1"),
            ("AlegriaOwnerValidateV1Workflow", "alegria-pg-owner-v1"),
            ("AlegriaQdrantProjectionDagV1Workflow", "alegria-qdrant-dag-v1"),
            ("QdrantKeywordProjectionV1Workflow", "alegria-qdrant-command-v1"),
            ("QdrantKeywordProjectionV2Workflow", "alegria-qdrant-command-v2"),
            ("AlegriaOwnerOperationV1Workflow", "alegria-qdrant-owner-v1"),
            ("AlegriaOwnerValidateV1Workflow", "alegria-qdrant-owner-v1"),
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


class TemporalNonmutatingCutoverRehearsalTests(unittest.TestCase):
    def snapshots(self):
        earlier = snap(captured_at=NOW - timedelta(seconds=75))
        later = snap(captured_at=NOW - timedelta(seconds=10))
        return earlier, later

    @staticmethod
    def plan():
        return {
            "schema_version": 1, "environment": "staging", "namespace": "staging-site",
            "source": {"sha": SHA, "tree": TREE},
            "routes": [{
                "owner": "orchestrator", "task_queue": "alegria-effect-dag-v5",
                "from_build_id": "orchestrator-v5", "to_build_id": "orchestrator-v6",
                "rollback_build_id": "orchestrator-v5",
            }],
        }

    def test_rehearsal_validates_versioned_route_but_never_authorizes_cutover(self):
        earlier, later = self.snapshots()
        result = transition.rehearse_cutover_plan(earlier, later, self.plan(), clock=NOW)
        self.assertEqual(result["status"], "ROUTING_REHEARSAL_CANDIDATE_ONLY")
        self.assertFalse(result["cutover_authorized"])
        self.assertTrue(result["unverified_external_gates"])
        self.assertEqual(result["routing_changes_reviewed"][0]["rollback_build_id"],
                         "orchestrator-v5")

    def test_rehearsal_rejects_retained_history_route_even_if_replay_covered(self):
        earlier, later = self.snapshots()
        retained = later["temporal"]["legacy_workflow_types"]
        retained["AlegriaAdmittedEffectDagV3Workflow"]["total"] = 1
        later["temporal"]["total_all_before"] = later["temporal"]["total_all_after"] = 1
        earlier = snap(captured_at=NOW - timedelta(seconds=75),
                       temporal_data=later["temporal"])
        later = snap(captured_at=NOW - timedelta(seconds=10),
                     temporal_data=later["temporal"])
        replay = {
            "schema_version": 1, "status": "OFFLINE_SDK_REPLAY_CANDIDATE_ONLY",
            "cutover_authorized": False, "source": later["source"],
            "environment": "staging", "temporal_namespace": "staging-site",
            "replayed_by_type": {name: c["total"] for name, c in retained.items()},
            "replayed_sha256": ["a" * 64], "replay_count": 1,
            "native_replayers": {"orchestrator": {"sha256": "b" * 64, "size_bytes": 42}},
            "replayed_runs": [{
                "workflow_type": "AlegriaAdmittedEffectDagV3Workflow",
                "task_queue": "alegria-effect-dag-v5", "workflow_id": "id-1",
                "run_id": "run-1", "sha256": "a" * 64,
            }],
        }
        result = transition.rehearse_cutover_plan(
            earlier, later, self.plan(), replay_evidence=replay, clock=NOW,
        )
        self.assertEqual(result["status"], "BLOCKED")
        self.assertTrue(any("retained histories" in message for message in result["errors"]))

    def test_rehearsal_rejects_duplicate_queue_wrong_owner_and_missing_rollback(self):
        earlier, later = self.snapshots()
        for broken in ("duplicate", "owner", "rollback", "same-build", "unknown-queue"):
            with self.subTest(broken=broken):
                plan = self.plan()
                if broken == "duplicate":
                    plan["routes"].append(dict(plan["routes"][0]))
                elif broken == "owner":
                    plan["routes"][0]["owner"] = "pg"
                elif broken == "rollback":
                    plan["routes"][0]["rollback_build_id"] = "unrelated"
                elif broken == "unknown-queue":
                    plan["routes"][0]["task_queue"] = "unaudited-foreign-queue"
                else:
                    plan["routes"][0]["to_build_id"] = "orchestrator-v5"
                result = transition.rehearse_cutover_plan(earlier, later, plan, clock=NOW)
                self.assertEqual(result["status"], "BLOCKED")
                self.assertFalse(result["cutover_authorized"])
                if broken == "unknown-queue":
                    self.assertTrue(any("unregistered native Task Queue" in reason
                                        for reason in result["errors"]))

    def test_snapshot_and_replay_boolean_schema_never_equate_to_version_one(self):
        earlier, later = self.snapshots()
        earlier["schema_version"] = True
        self.assertEqual(transition.check_pair(earlier, later, clock=NOW)["status"], "BLOCKED")
        earlier["schema_version"] = 1
        replay = {
            "schema_version": True, "status": "OFFLINE_SDK_REPLAY_CANDIDATE_ONLY",
            "cutover_authorized": False, "source": later["source"],
            "environment": "staging", "temporal_namespace": "staging-site",
            "replayed_by_type": {name: 0 for name in transition.WORKFLOW_TYPES},
            "replayed_sha256": [], "replay_count": 0, "replayed_runs": [],
            "native_replayers": {},
        }
        self.assertEqual(transition.check_pair(earlier, later, replay_evidence=replay,
                                               clock=NOW)["status"], "BLOCKED")

    def test_rehearsal_is_blocked_on_unready_pg_ownership_or_stale_snapshots(self):
        earlier, later = self.snapshots()
        later["postgres"]["raw_pages_unowned"] = 1
        result = transition.rehearse_cutover_plan(earlier, later, self.plan(), clock=NOW)
        self.assertEqual(result["status"], "BLOCKED")
        later["postgres"]["raw_pages_unowned"] = 0
        later["captured_at"] = (NOW - timedelta(minutes=14)).isoformat()
        self.assertEqual(transition.rehearse_cutover_plan(
            earlier, later, self.plan(), clock=NOW,
        )["status"], "BLOCKED")

    def test_rehearsal_cli_only_writes_local_diagnostic_report(self):
        earlier, later = self.snapshots()
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for name, value in (("earlier", earlier), ("later", later), ("plan", self.plan())):
                (root / f"{name}.json").write_text(json.dumps(value), encoding="utf-8")
            output = root / "rehearsal.json"
            with patch.object(transition, "now_utc", return_value=NOW), \
                 patch.object(transition, "run_command") as command:
                result = transition.main([
                    "rehearse", "--earlier", str(root / "earlier.json"),
                    "--later", str(root / "later.json"),
                    "--routing-plan", str(root / "plan.json"), "--output", str(output),
                ])
            self.assertEqual(result, 0)
            command.assert_not_called()
            report = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(report["status"], "ROUTING_REHEARSAL_CANDIDATE_ONLY")
            self.assertFalse(report["cutover_authorized"])


class TemporalRollbackArtifactRehearsalTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)
        self.earlier = snap(captured_at=NOW - timedelta(seconds=75))
        self.later = snap(captured_at=NOW - timedelta(seconds=10))
        self.plan = TemporalNonmutatingCutoverRehearsalTests.plan()
        self.old = self.root / "old-worker"
        self.new = self.root / "new-worker"
        self.old.write_bytes(b"#!/bin/sh\nexit 1\n# old test-only worker\n")
        self.new.write_bytes(b"#!/bin/sh\nexit 1\n# candidate test-only worker\n")
        self.old.chmod(0o700)
        self.new.chmod(0o700)
        self.manifest = self.build_manifest()

    def build_manifest(self):
        return {
            "schema_version": 1,
            "source": self.plan["source"],
            "environment": self.plan["environment"],
            "namespace": self.plan["namespace"],
            "artifacts": [
                {"owner": "orchestrator", "task_queue": "alegria-effect-dag-v5",
                 "build_id": build_id, "role": role,
                 "file": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                 "size_bytes": path.stat().st_size}
                for build_id, role, path in (
                    ("orchestrator-v5", "rollback", self.old),
                    ("orchestrator-v6", "candidate", self.new),
                )
            ],
        }

    def rehearse(self):
        return transition.rehearse_rollback_artifacts(
            self.earlier, self.later, self.plan, self.manifest,
            artifact_root=self.root, clock=NOW,
        )

    def test_local_rollback_artifact_bytes_verified_but_cutover_not_authorized(self):
        result = self.rehearse()
        self.assertEqual(result["status"], "ROLLBACK_ARTIFACT_CANDIDATE_ONLY")
        self.assertFalse(result["cutover_authorized"])
        self.assertEqual(len(result["artifact_fingerprints"]), 2)
        self.assertEqual({item["role"] for item in result["artifact_fingerprints"]},
                         {"candidate", "rollback"})
        self.assertIn("production_tenant_FORCE_RLS_backfill_mTLS_and_release_authorization",
                      result["unverified_external_gates"])

    def test_rehearsal_manifest_requires_source_and_complete_exact_worker_coverage(self):
        for broken in ("source", "missing", "duplicate", "wrong-owner", "wrong-build",
                       "extra", "boolean-schema"):
            with self.subTest(broken=broken):
                self.manifest = self.build_manifest()
                if broken == "source":
                    self.manifest["source"] = {"sha": "a" * 40, "tree": TREE}
                elif broken == "missing":
                    self.manifest["artifacts"].pop()
                elif broken == "duplicate":
                    self.manifest["artifacts"].append(dict(self.manifest["artifacts"][0]))
                elif broken == "wrong-owner":
                    self.manifest["artifacts"][0]["owner"] = "pg"
                elif broken == "wrong-build":
                    self.manifest["artifacts"][0]["build_id"] = "unknown-v0"
                elif broken == "extra":
                    self.manifest["artifacts"].append({**self.manifest["artifacts"][0],
                                                       "role": "invented"})
                else:
                    self.manifest["schema_version"] = True
                with self.assertRaises(transition.EvidenceError):
                    self.rehearse()

    def test_rehearsal_never_accepts_forged_bytes_symlinks_or_escaped_paths(self):
        self.old.write_bytes(b"tampered rollback bytes")
        with self.assertRaisesRegex(transition.EvidenceError, "SHA-256 or size"):
            self.rehearse()
        self.old.write_bytes(b"#!/bin/sh\nexit 1\n# old test-only worker\n")
        symlink = self.root / "linked-old"
        symlink.symlink_to(self.old)
        self.manifest["artifacts"][0]["file"] = symlink.name
        with self.assertRaisesRegex(transition.EvidenceError, "symlink"):
            self.rehearse()
        self.manifest["artifacts"][0]["file"] = "../escaped"
        with self.assertRaisesRegex(transition.EvidenceError, "escapes"):
            self.rehearse()

    def test_rehearsal_rejects_same_bytes_and_nonexecutables(self):
        self.new.write_bytes(self.old.read_bytes())
        self.manifest = self.build_manifest()
        with self.assertRaisesRegex(transition.EvidenceError, "reused"):
            self.rehearse()
        self.new.write_bytes(b"#!/bin/sh\nexit 1\n# candidate test-only worker\n")
        self.manifest = self.build_manifest()
        self.new.chmod(0o600)
        with self.assertRaisesRegex(transition.EvidenceError, "not a regular executable"):
            self.rehearse()

    def test_rehearsal_cannot_skip_real_drain_or_history_replay(self):
        self.earlier["postgres"]["raw_pages_unowned"] = 1
        result = self.rehearse()
        self.assertEqual(result["status"], "BLOCKED")
        self.assertFalse(result["cutover_authorized"])
        self.earlier["postgres"]["raw_pages_unowned"] = 0
        historical = self.later["temporal"]["legacy_workflow_types"]
        historical["AlegriaAdmittedEffectDagV3Workflow"]["total"] = 1
        self.later["temporal"]["total_all_before"] = 1
        self.later["temporal"]["total_all_after"] = 1
        result = self.rehearse()
        self.assertEqual(result["status"], "BLOCKED")

    def test_two_queues_with_same_build_ids_require_distinct_route_attestations(self):
        self.plan["routes"].append({**self.plan["routes"][0],
                                   "task_queue": "alegria-pure-dag-v3"})
        with self.assertRaisesRegex(transition.EvidenceError, "count differs"):
            self.rehearse()
        # Four independently named entries must still not silently reuse the
        # same two binaries to claim four physical Worker instances.
        second = [{**entry, "task_queue": "alegria-pure-dag-v3"}
                  for entry in self.manifest["artifacts"]]
        self.manifest["artifacts"].extend(second)
        with self.assertRaisesRegex(transition.EvidenceError, "reused"):
            self.rehearse()

    def test_cross_owner_binary_digest_reuse_is_not_independent_evidence(self):
        self.plan["routes"].append({
            "owner": "pg", "task_queue": "alegria-pg-command-v3",
            "from_build_id": "pg-v3", "to_build_id": "pg-v4",
            "rollback_build_id": "pg-v3",
        })
        for source, role, build_id, name in (
            (self.old, "rollback", "pg-v3", "pg-old"),
            (self.new, "candidate", "pg-v4", "pg-new"),
        ):
            destination = self.root / name
            destination.write_bytes(source.read_bytes())
            destination.chmod(0o700)
            self.manifest["artifacts"].append({
                "owner": "pg", "task_queue": "alegria-pg-command-v3",
                "build_id": build_id, "role": role, "file": name,
                "sha256": hashlib.sha256(destination.read_bytes()).hexdigest(),
                "size_bytes": destination.stat().st_size,
            })
        with self.assertRaisesRegex(transition.EvidenceError, "reused"):
            self.rehearse()

    def test_cross_route_hard_link_alias_is_rejected(self):
        self.plan["routes"].append({
            "owner": "pg", "task_queue": "alegria-pg-command-v3",
            "from_build_id": "pg-v3", "to_build_id": "pg-v4",
            "rollback_build_id": "pg-v3",
        })
        linked = self.root / "linked-old"
        os.link(self.old, linked)
        independent = self.root / "pg-new"
        independent.write_bytes(b"#!/bin/sh\nexit 1\n# independent PG candidate\n")
        independent.chmod(0o700)
        for role, build_id, path in (
            ("rollback", "pg-v3", linked),
            ("candidate", "pg-v4", independent),
        ):
            self.manifest["artifacts"].append({
                "owner": "pg", "task_queue": "alegria-pg-command-v3",
                "build_id": build_id, "role": role, "file": path.name,
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "size_bytes": path.stat().st_size,
            })
        with self.assertRaisesRegex(transition.EvidenceError, "reused"):
            self.rehearse()

    def test_rehearsal_requires_owner_executable_and_no_shared_write_permissions(self):
        self.old.chmod(0o601)  # only world-executable, not owner-executable
        with self.assertRaisesRegex(transition.EvidenceError, "regular executable"):
            self.rehearse()
        self.old.chmod(0o722)  # group/world may write
        with self.assertRaisesRegex(transition.EvidenceError, "regular executable"):
            self.rehearse()

    def test_rehearsal_has_strict_route_and_file_work_budgets(self):
        self.plan["routes"] = [dict(self.plan["routes"][0])
                               for _ in range(transition.MAX_REHEARSAL_ROUTES + 1)]
        result = self.rehearse()
        self.assertEqual(result["status"], "BLOCKED")
        self.plan = TemporalNonmutatingCutoverRehearsalTests.plan()
        self.manifest["artifacts"] *= transition.MAX_REHEARSAL_ARTIFACTS
        with self.assertRaisesRegex(transition.EvidenceError, "manifest"):
            self.rehearse()

    def test_cli_rehearsal_does_not_run_or_contact_a_worker(self):
        for name, value in (
            ("earlier", self.earlier), ("later", self.later),
            ("plan", self.plan), ("artifacts", self.manifest),
        ):
            (self.root / f"{name}.json").write_text(json.dumps(value), encoding="utf-8")
        destination = self.root / "v8-read-only-result.json"
        with patch.object(transition, "now_utc", return_value=NOW), \
             patch.object(transition, "run_command") as commands:
            rc = transition.main([
                "rehearse-artifacts", "--earlier", str(self.root / "earlier.json"),
                "--later", str(self.root / "later.json"),
                "--routing-plan", str(self.root / "plan.json"),
                "--artifact-manifest", str(self.root / "artifacts.json"),
                "--artifact-root", str(self.root),
                "--output", str(destination),
            ])
        self.assertEqual(rc, 0)
        commands.assert_not_called()
        report = json.loads(destination.read_text(encoding="utf-8"))
        self.assertEqual(report["status"], "ROLLBACK_ARTIFACT_CANDIDATE_ONLY")
        self.assertFalse(report["cutover_authorized"])


if __name__ == "__main__":
    unittest.main()
