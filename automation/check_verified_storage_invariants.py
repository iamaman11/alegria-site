#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app/db/schema.sql"
SEO_ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs"
SMOKE = ROOT / "automation/smoke_real_provider_minimal_scope.py"
PROD_GATE = ROOT / "automation/temporal_production_gate.sh"


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "sqlx_seo_adapter.rs":
        sub_dir = path.parent / "sqlx_seo_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    schema = SCHEMA.read_text(encoding="utf-8")
    seo_adapter = read_file_resolved(SEO_ADAPTER)
    smoke = SMOKE.read_text(encoding="utf-8")
    prod_gate = PROD_GATE.read_text(encoding="utf-8")
    failures: list[str] = []

    for needle in [
        "CREATE TABLE IF NOT EXISTS extracted.rule_candidates",
        "rule_candidate_id      TEXT PRIMARY KEY",
        "evidence_section_id    BIGINT NOT NULL",
        "publish_admissibility TEXT NOT NULL DEFAULT 'not_admissible'",
        "freshness_class TEXT NOT NULL DEFAULT 'unknown'",
        "completeness_class TEXT NOT NULL DEFAULT 'unknown'",
        "verified_rule_instances_rule_candidate_fk",
        "verified_rule_instances_evidence_section_fk",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")

    for needle in [
        "COALESCE(r.publish_admissibility, 'not_admissible') = 'admissible'",
        "r.evidence_section_id IS NOT NULL",
        "COALESCE(r.evidence_quote, '') <> ''",
        "COALESCE(r.source_snapshot_hash, '') <> ''",
    ]:
        if needle not in seo_adapter:
            failures.append(f"seo adapter missing admissibility filter `{needle}`")

    if "publish_admissibility" not in smoke:
        failures.append("live-provider smoke bootstrap is not seeding admissible verified truth")
    if "publish_admissibility" not in prod_gate:
        failures.append("temporal production gate bootstrap is not seeding admissible verified truth")

    if failures:
        print("VERIFIED_STORAGE_INVARIANTS: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("VERIFIED_STORAGE_INVARIANTS: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
