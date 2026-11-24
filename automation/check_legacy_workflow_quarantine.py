#!/usr/bin/env python3
"""Reject legacy workflow registration and verify *historical local* drain evidence.

This source-level release check is not a live production Temporal inventory.
An operator must establish target-environment drain before replacing a Worker.
"""

from __future__ import annotations

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / "app/rust/services/temporal/src/workflows/mod.rs"
STARTER = ROOT / "app/rust/services/temporal/src/bin/temporal_starter"
GATE = ROOT / "automation/temporal_production_gate.sh"
HISTORICAL = ROOT / "docs/runs/seo_site_build_legacy_replay_evidence.json"


def main() -> int:
    registrations = WORKFLOWS.read_text(encoding="utf-8")
    starter = "\n".join(path.read_text(encoding="utf-8") for path in sorted(STARTER.glob("*.rs")))
    gate = GATE.read_text(encoding="utf-8")
    failures: list[str] = []

    for legacy in (
        "mod content_generation;",
        "mod seo_site_build;",
        "content_generation::register",
        "seo_site_build::register",
        "ALLOW_LEGACY_CONTENT_WORKFLOW",
        "ALLOW_COMPAT_SEO_SITE_BUILD_WORKFLOW",
    ):
        if legacy in registrations:
            failures.append(f"removed legacy registration reintroduced: {legacy}")
    for legacy in ("WorkflowKind::ContentGeneration", "SeoSiteBuildLegacyCompat"):
        if legacy in starter:
            failures.append(f"removed legacy starter route reintroduced: {legacy}")
    if "seo_site_build_canonical_cutover::register(&mut opts)?;" not in registrations:
        failures.append("canonical SeoSiteBuild registration missing")
    if "run_seo_site_build_workflow" not in gate or "run_content_workflow" in gate:
        failures.append("production smoke entrypoint must select canonical site-build")

    try:
        historical = json.loads(HISTORICAL.read_text(encoding="utf-8"))
        inventory = historical["history_source"]["inventory"]
        if historical.get("evidence_status") != "PASS":
            failures.append("historical local legacy drain evidence is not PASS")
        for key in ("total_runs", "open_runs"):
            if type(inventory.get(key)) is not int or inventory[key] != 0:
                failures.append(f"historical local inventory {key} must be zero")
    except (OSError, ValueError, KeyError, TypeError) as exc:
        failures.append(f"historical local legacy drain evidence invalid: {exc}")

    if failures:
        print("LEGACY_WORKFLOW_QUARANTINE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("LEGACY_WORKFLOW_QUARANTINE: OK historical_local_only; production_drain_not_verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
