#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MIGRATIONS_DIR = ROOT / "app" / "db" / "migrations"

REQUIRED_NEEDLES = [
    "verified_support_bundle",
    "content_block_plan",
    "draft_normalize_output",
    "content_contract_validation",
    "CREATE TABLE IF NOT EXISTS site.page_support_bindings",
    "CREATE TABLE IF NOT EXISTS site.publish_artifact_entries",
    "CREATE TABLE IF NOT EXISTS monitoring.seo_rebuild_dependencies",
    "CREATE TABLE IF NOT EXISTS extracted.rule_candidates",
    "publish_admissibility",
    "verified_rule_instances_rule_candidate_fk",
]


def main() -> int:
    if not MIGRATIONS_DIR.exists():
        print("SEO_MIGRATION_PARITY: FAILED")
        print(f"- missing migrations directory `{MIGRATIONS_DIR.relative_to(ROOT)}`")
        return 1

    combined = "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted(MIGRATIONS_DIR.glob("*.sql"))
    )
    failures: list[str] = []
    for needle in REQUIRED_NEEDLES:
        if needle not in combined:
            failures.append(f"missing migration needle `{needle}`")

    if failures:
        print("SEO_MIGRATION_PARITY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SEO_MIGRATION_PARITY: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
