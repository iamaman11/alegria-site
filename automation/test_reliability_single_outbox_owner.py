#!/usr/bin/env python3
"""Regression tests for the real, single-authority Outbox reliability guard."""

import unittest

from check_reliability_contracts import (
    TEMPORAL_RUNTIME_SUPPORT,
    temporal_projection_observer_failures,
)


class SingleOutboxOwnerContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.source = TEMPORAL_RUNTIME_SUPPORT.read_text(encoding="utf-8")

    def test_exact_source_has_one_status_only_temporal_observer(self) -> None:
        self.assertEqual(temporal_projection_observer_failures(self.source), [])

    def test_second_delivery_lease_is_forbidden(self) -> None:
        mutated = self.source.replace(
            "let drain_state =", "claim_outbox_batch_for_run_target();\n    let drain_state =", 1,
        )
        self.assertIn("claim_outbox_batch", " ".join(temporal_projection_observer_failures(mutated)))

    def test_done_cannot_ignore_pending_events(self) -> None:
        mutated = self.source.replace(
            "backlog.pending_events == 0 && backlog.failed_events == 0",
            "backlog.pending_events > 0 && backlog.failed_events == 0", 1,
        )
        self.assertIn("Drained requires zero pending", " ".join(temporal_projection_observer_failures(mutated)))

    def test_non_drained_failure_cannot_be_mapped_to_success(self) -> None:
        mutated = self.source.replace('Self::Blocked => "blocked"', 'Self::Blocked => "done"', 1)
        self.assertIn("Non-drained", " ".join(temporal_projection_observer_failures(mutated)))

    def test_uncertain_remote_outcome_must_block_after_timeout(self) -> None:
        mutated = self.source.replace(
            "backlog.uncertain_events > 0", "backlog.uncertain_events == 0", 1,
        )
        self.assertIn("uncertain provider effects", " ".join(temporal_projection_observer_failures(mutated)))


if __name__ == "__main__":
    unittest.main()
