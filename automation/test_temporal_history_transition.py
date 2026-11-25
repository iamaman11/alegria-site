#!/usr/bin/env python3
"""Safety contracts for the read-only V1/V2 history/drain verifier."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
import json
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


if __name__ == "__main__":
    unittest.main()
