#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
PII_STEP = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "pii_redaction_prepass_step.rs"
DRAFT_ASSEMBLE = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "draft_assemble_step.rs"
FLOW_TESTS = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src" / "seo_flow_tests.rs"


def main() -> int:
    failures: list[str] = []
    pii_step = PII_STEP.read_text(encoding="utf-8")
    draft_assemble = DRAFT_ASSEMBLE.read_text(encoding="utf-8")
    flow_tests = FLOW_TESTS.read_text(encoding="utf-8")

    for needle in [
        "pub fn sanitize_text(",
        "pub fn sanitize_source_context_chunks(",
        "pub fn sanitize_support_bundle(",
        "pub fn sanitize_sections(",
        "pub fn sanitize_claim_ledger(",
        "pub fn sanitize_content_blocks(",
        "pub fn sanitize_candidate(",
        "[redacted:email]",
        "[redacted:phone]",
        "[redacted:passport]",
    ]:
        if needle not in pii_step:
            failures.append(f"pii prepass missing `{needle}`")

    for needle in [
        "sanitize_candidate",
        "sanitize_source_context_chunks(",
        "sanitize_support_bundle(",
        "sanitize_text(fragment.trim())",
    ]:
        if needle not in draft_assemble:
            failures.append(f"draft assemble missing `{needle}`")

    for needle in [
        "redacts_email_phone_and_passport",
    ]:
        if needle not in flow_tests and needle not in pii_step:
            failures.append(f"smoke/test surface missing `{needle}`")

    if failures:
        print("PII_REDACTION_PRELLM: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("PII_REDACTION_PRELLM: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
