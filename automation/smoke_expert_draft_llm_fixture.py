#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/editorial_llm_adapter.rs"
ACTIVITIES = ROOT / "app/rust/services/temporal/src/activities/mod.rs"
WORKFLOW = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build.rs"
PROTO = ROOT / "app/contracts/proto/temporal_payloads.proto"


def main() -> int:
    failures: list[str] = []
    adapter = ADAPTER.read_text(encoding="utf-8")
    activities = ACTIVITIES.read_text(encoding="utf-8")
    workflow = WORKFLOW.read_text(encoding="utf-8")
    proto = PROTO.read_text(encoding="utf-8")
    for needle in [
        "trait EditorialLlmClient",
        "OpenAiEditorialClient",
        "AnthropicEditorialClient",
        "GeminiEditorialClient",
        "LocalCompatibleEditorialClient",
        "DeterministicEditorialClient",
        "generate_editorial_draft",
    ]:
        if needle not in adapter:
            failures.append(f"editorial LLM adapter missing `{needle}`")
    for needle in [
        "run_editorial_draft_generate",
        "EditorialDraftGenerateInputPayload",
        "EditorialDraftGenerateOutputPayload",
    ]:
        if needle not in activities:
            failures.append(f"Temporal activities missing `{needle}`")
    for needle in [
        "editorial_draft_generate",
        "llm_candidate: editorial_candidate.candidate",
    ]:
        if needle not in workflow:
            failures.append(f"SEO workflow missing `{needle}`")
    for needle in [
        "message EditorialDraftGenerateInputPayload",
        "message EditorialDraftGenerateOutputPayload",
        "message LlmDraftCandidate",
    ]:
        if needle not in proto:
            failures.append(f"proto missing `{needle}`")

    if failures:
        print("EXPERT_DRAFT_LLM_FIXTURE_SMOKE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("EXPERT_DRAFT_LLM_FIXTURE_SMOKE: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
