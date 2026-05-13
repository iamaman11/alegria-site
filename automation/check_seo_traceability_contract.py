#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PROTO = ROOT / "app/contracts/proto/temporal_payloads.proto"
DRAFT_ASSEMBLE = ROOT / "app/rust/crates/seo_steps/src/draft_assemble_step.rs"
DRAFT_QA = ROOT / "app/rust/crates/seo_steps/src/draft_qa_step.rs"
ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs"
SCHEMA = ROOT / "app/db/schema.sql"


def main() -> int:
    proto = PROTO.read_text(encoding="utf-8")
    assemble = DRAFT_ASSEMBLE.read_text(encoding="utf-8")
    qa = DRAFT_QA.read_text(encoding="utf-8")
    adapter = ADAPTER.read_text(encoding="utf-8")
    schema = SCHEMA.read_text(encoding="utf-8")
    failures: list[str] = []

    for needle in [
        "SeoVerifiedFactSupportState",
        "SeoTraceabilityEntryState",
        "repeated SeoTraceabilityEntryState traceability_entries",
        "repeated SeoVerifiedFactSupportState verified_support",
    ]:
        if needle not in proto:
            failures.append(f"proto traceability contract missing `{needle}`")

    for needle in [
        "verified_fact",
        "verified_summary",
        "traceability_entries",
        "support_refs",
    ]:
        if needle not in assemble:
            failures.append(f"draft assembly missing traceability needle `{needle}`")

    for needle in [
        "traceability_entries",
        "unsupported_factual_fragment",
        "missing_traceability_manifest",
    ]:
        if needle not in qa:
            failures.append(f"draft QA missing traceability gate `{needle}`")

    if "traceability_manifest" not in adapter or "traceability_manifest" not in schema:
        failures.append("draft traceability is not persisted to site.page_drafts")

    if failures:
        print("SEO_TRACEABILITY_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_TRACEABILITY_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
