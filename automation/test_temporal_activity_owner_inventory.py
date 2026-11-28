#!/usr/bin/env python3
"""Negative contract tests for the immutable legacy Temporal Activity map."""

from __future__ import annotations

import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

import check_temporal_activity_owner_inventory as gate


class LegacyActivityInventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.registry = gate.REGISTRY.read_text(encoding="utf-8")
        cls.inventory = gate.INVENTORY.read_text(encoding="utf-8")
        cls.activities_mod = gate.ACTIVITIES_MOD.read_text(encoding="utf-8")
        cls.worker_main = gate.WORKER_MAIN.read_text(encoding="utf-8")
        cls.workflows_mod = gate.WORKFLOWS_MOD.read_text(encoding="utf-8")

    def errors(self, **overrides: str) -> list[str]:
        args = {
            "registry": self.registry,
            "inventory": self.inventory,
            "activities_mod": self.activities_mod,
            "worker_main": self.worker_main,
            "workflows_mod": self.workflows_mod,
        }
        args.update(overrides)
        return gate.violations(**args)

    def test_current_registered_activity_map_is_exact(self) -> None:
        self.assertFalse(self.errors())
        self.assertEqual(len(gate.ACTIVITY_METHOD.findall(self.registry)), 73)

    def test_new_registered_activity_must_have_owner(self) -> None:
        extra = self.registry + "\n#[activity]\npub async fn new_unregistered_effect(self: Arc<Self>) {}\n"
        self.assertIn("unowned registered legacy Activity: new_unregistered_effect", self.errors(registry=extra))

    def test_explicit_sdk_activity_name_override_is_blocked(self) -> None:
        tampered = self.registry.replace("#[activity]", '#[activity(name = "BROKEN_HISTORY")]', 1)
        self.assertIn("parameterized #[activity(...)]", " ".join(self.errors(registry=tampered)))

    def test_coordinated_method_and_inventory_rename_cannot_hide_history_drift(self) -> None:
        original = "run_page_utility_classifier_step"
        replaced = "run_page_utility_classifier_v99_step"
        reg = self.registry.replace(original, replaced)
        inventory = self.inventory.replace(original, replaced)
        self.assertIn("historical SDK Activity names changed", " ".join(
            self.errors(registry=reg, inventory=inventory)
        ))

    def test_unparsed_registered_activity_attribute_is_blocked(self) -> None:
        reg = self.registry + "\n#[activity]\nfn not_async_activity() {}\n"
        self.assertIn("SDK Activity attribute/parsed method count mismatch", " ".join(
            self.errors(registry=reg)
        ))

    def test_unregistered_owner_is_rejected(self) -> None:
        extra = self.inventory + "\nroute!(not_actually_registered => PostgresStore),\n"
        self.assertIn("stale legacy owner assignment", " ".join(self.errors(inventory=extra)))

    def test_duplicate_route_is_rejected(self) -> None:
        extra = self.inventory + "\nroute!(run_page_utility_classifier_step => Acquisition),\n"
        self.assertIn("duplicate Activity owner assignment", " ".join(self.errors(inventory=extra)))

    def test_unknown_typed_owner_is_rejected(self) -> None:
        other = self.inventory.replace(
            "route!(run_page_utility_classifier_step => Acquisition)",
            "route!(run_page_utility_classifier_step => NeverDefined)",
        )
        self.assertIn("unknown typed LogicalOwner::NeverDefined", " ".join(self.errors(inventory=other)))

    def test_new_mixed_effect_cannot_be_grandfathered(self) -> None:
        new_method = "\n#[activity]\npub async fn new_mixed_effect(self: Arc<Self>) {}\n"
        new_route = "\nroute!(new_mixed_effect => LegacyMixedRuntime),\n"
        self.assertIn("new Activity cannot enter legacy mixed-provider ownership", " ".join(
            self.errors(registry=self.registry + new_method, inventory=self.inventory + new_route)
        ))

    def test_existing_mixed_owner_cannot_be_paper_reclassified(self) -> None:
        rewritten = self.inventory.replace(
            "route!(run_publish_materialize_step => LegacyMixedRuntime)",
            "route!(run_publish_materialize_step => PostgresStore)",
        )
        self.assertIn("mixed legacy owner reclassified without explicit extraction/drain", " ".join(
            self.errors(inventory=rewritten)
        ))

    def test_task_queue_change_is_rejected_before_routing(self) -> None:
        other = self.worker_main.replace('build_worker_options("alegria-pipeline", acts)',
                                         'build_worker_options("another-queue", acts)')
        self.assertIn("legacy Worker Task Queue changed", " ".join(self.errors(worker_main=other)))

    def test_hidden_registrar_change_is_rejected(self) -> None:
        other = self.workflows_mod.replace(".register_activities(acts)", ".register_activities(other)")
        self.assertIn("legacy Activity registrar changed", " ".join(self.errors(workflows_mod=other)))

    def test_duplicate_sdk_registrar_is_rejected(self) -> None:
        other = self.workflows_mod + "\n// rogue second registration\n.register_activities(other)\n"
        self.assertIn("legacy Activity registrar changed", " ".join(self.errors(workflows_mod=other)))

    def test_pure_orchestrator_transitive_provider_dependency_forbidden(self) -> None:
        with TemporaryDirectory() as temp:
            root = Path(temp)
            pure = root / "pure"
            innocent = root / "innocent"
            pure.mkdir()
            innocent.mkdir()
            (pure / "Cargo.toml").write_text(
                '[package]\nname="temporal_orchestrator"\nversion="0.1.0"\n'
                '[dependencies]\ninnocent={path="../innocent"}\n', encoding="utf-8",
            )
            (innocent / "Cargo.toml").write_text(
                '[package]\nname="innocent"\nversion="0.1.0"\n'
                '[dependencies]\nsqlx="0.9"\n', encoding="utf-8",
            )
            errors = gate.pure_orchestrator_dependency_violations(pure / "Cargo.toml")
            self.assertIn("pure Temporal `innocent` imports forbidden provider `sqlx`", errors)

    def test_actual_pure_orchestrator_has_no_provider_closure(self) -> None:
        self.assertEqual(gate.pure_orchestrator_dependency_violations(), [])


if __name__ == "__main__":
    unittest.main()
