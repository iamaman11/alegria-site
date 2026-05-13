#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROTO = ROOT / "app/contracts/proto/temporal_payloads.proto"
DRAFT = ROOT / "app/rust/crates/seo_steps/src/draft_assemble_step.rs"
QA = ROOT / "app/rust/crates/seo_steps/src/draft_qa_step.rs"
CMS = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter.rs"


def require(text: str, needle: str, label: str, failures: list[str]) -> None:
    if needle not in text:
        failures.append(f"missing {label}: `{needle}`")


def main() -> int:
    failures: list[str] = []
    proto = PROTO.read_text(encoding="utf-8")
    draft = DRAFT.read_text(encoding="utf-8")
    qa = QA.read_text(encoding="utf-8")
    cms = CMS.read_text(encoding="utf-8")

    for message in [
        "message ClaimLedgerEntry",
        "message RenderedContentBlock",
        "message EditorialBrief",
        "message LlmDraftRequest",
        "message LlmDraftCandidate",
    ]:
        require(proto, message, "proto message", failures)
    for field in [
        "repeated ClaimLedgerEntry claim_ledger",
        "repeated RenderedContentBlock content_blocks",
        "string llm_provider_key",
        "string generation_request_key",
    ]:
        require(proto, field, "draft state field", failures)
    for needle in [
        "claim_ledger",
        "content_blocks",
        "unsupported_factual_fragment",
        "verified_fact",
        "verified_summary",
    ]:
        require(draft, needle, "draft claim ledger implementation", failures)
    for needle in [
        "missing_claim_ledger",
        "unsupported_claim_ledger_entry",
        "missing_rendered_content_blocks",
        "missing_schema_markup",
    ]:
        require(qa, needle, "QA claim ledger gate", failures)
    for needle in [
        '"claim_ledger"',
        '"content_blocks"',
        '"headless_cms_blocks@1"',
    ]:
        require(cms, needle, "CMS headless payload persistence", failures)

    if failures:
        print("SEO_CLAIM_LEDGER_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_CLAIM_LEDGER_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
