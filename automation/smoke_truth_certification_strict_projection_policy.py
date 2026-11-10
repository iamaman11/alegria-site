#!/usr/bin/env python3
"""Reject every projection-baseline change except stricter no-truth blocking."""
from __future__ import annotations

from check_truth_certification_regression import monotonic_no_truth_projection_block


def main() -> int:
    old = {
        "verified_rule_set": [],
        "projection_verdict": "allow",
        "draft_verdict": "blocked",
        "publish_verdict": "not_attempted",
        "rebuild_impact_emitted": False,
    }
    strict = {**old, "projection_verdict": "blocked"}
    assert monotonic_no_truth_projection_block(old, strict)
    for invalid in (
        {**strict, "verified_rule_set": ["fabricated_truth"]},
        {**strict, "draft_verdict": "allow"},
        {**strict, "publish_verdict": "allow"},
        {**strict, "rebuild_impact_emitted": True},
        {**strict, "projection_verdict": "allow"},
    ):
        assert not monotonic_no_truth_projection_block(old, invalid)
    assert not monotonic_no_truth_projection_block(
        {**old, "verified_rule_set": ["real_truth"]}, strict
    )
    print("TRUTH_CERT_MONOTONIC_PROJECTION_POLICY: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
