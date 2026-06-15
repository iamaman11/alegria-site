#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/editorial_llm_adapter.rs"
ACTIVITIES = ROOT / "app/rust/services/temporal/src/activities/mod.rs"
WORKFLOW = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build.rs"
SCENARIO = ROOT / "app/rust/crates/seo_application/src/scenario.rs"
PROTO = ROOT / "app/contracts/proto/temporal_payloads.proto"


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "editorial_llm_adapter.rs":
        sub_dir = path.parent / "editorial_llm_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "mod.rs" and "activities" in path.parts:
        registry_file = path.parent / "registry" / "mod_registry_impl.rs"
        if registry_file.exists():
            text += "\n" + registry_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    failures: list[str] = []
    adapter = read_file_resolved(ADAPTER)
    activities = read_file_resolved(ACTIVITIES)
    workflow = WORKFLOW.read_text(encoding="utf-8")
    scenario = SCENARIO.read_text(encoding="utf-8")
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
    if "editorial_draft_generate" not in workflow:
        failures.append("SEO workflow missing `editorial_draft_generate`")
    if (
        "llm_candidate: editorial_ref.candidate.clone()" not in workflow
        and "llm_candidate: editorial_candidate.candidate.clone()" not in scenario
    ):
        failures.append("shared SEO execution path missing editorial candidate handoff into draft normalize")
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
