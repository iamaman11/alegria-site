#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SEO_RUNTIME = ROOT / "app/rust/crates/seo_application/src/seo_runtime.rs"
DRAFTING = ROOT / "app/rust/crates/seo_application/src/drafting.rs"
VERIFIED_SUPPORT = (
    ROOT
    / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/verified_support.rs"
)


def main() -> int:
    seo_runtime = SEO_RUNTIME.read_text(encoding="utf-8")
    drafting = DRAFTING.read_text(encoding="utf-8")
    verified_support = VERIFIED_SUPPORT.read_text(encoding="utf-8")
    failures: list[str] = []

    for needle in [
        "pub fn truth_admissibility_gate(",
        "truth_admissibility_gate failed",
        "support row",
    ]:
        if needle not in seo_runtime:
            failures.append(f"seo_runtime missing `{needle}`")

    if "truth_admissibility_gate(" not in drafting:
        failures.append(
            "drafting.rs does not enforce truth_admissibility_gate before draft_assemble"
        )

    for needle in [
        "AND (r.effective_from IS NULL OR r.effective_from <= CURRENT_DATE)",
        "AND (r.effective_to IS NULL OR r.effective_to >= CURRENT_DATE)",
        "AND COALESCE(r.freshness_class, 'unknown') = 'fresh'",
        "COALESCE(r.publish_admissibility, 'not_admissible') = 'admissible'",
        "COALESCE(r.evidence_quote, '') <> ''",
        "COALESCE(r.source_snapshot_hash, '') <> ''",
    ]:
        if needle not in verified_support:
            failures.append(f"verified support read path missing `{needle}`")

    if failures:
        print("TRUTH_ADMISSIBILITY_GATE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("TRUTH_ADMISSIBILITY_GATE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
