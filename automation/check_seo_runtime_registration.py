#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ACTIVITIES = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "mod.rs"
WORKFLOWS_MOD = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "mod.rs"
SEO_WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "seo_site_build.rs"
TEMPORAL_STARTER = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "bin" / "temporal_starter.rs"
SEO_STEPS_DIR = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src"
SEO_APPLICATION = ROOT / "app" / "rust" / "crates" / "seo_application" / "src"
PAYLOAD_STORE = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "proto_runtime_payload_store.rs"

SEO_STEP_NAMES = [
    "serp_ingest",
    "serp_normalize",
    "opportunity_build",
    "ia_build",
    "link_recommend",
    "draft_assemble",
    "draft_qa",
    "cms_publish",
    "rebuild_detect",
]

APPLICATION_OWNERS = {
    "serp_ingest": ("planning.rs", "serp_ingest_step::execute"),
    "serp_normalize": ("planning.rs", "serp_normalize_step::execute"),
    "opportunity_build": ("planning.rs", "opportunity_build_step::execute"),
    "ia_build": ("planning.rs", "ia_build_step::execute"),
    "link_recommend": ("planning.rs", "link_recommend_step::execute"),
    "draft_assemble": ("drafting.rs", "draft_assemble_step::execute"),
    "draft_qa": ("drafting.rs", "draft_qa_step::execute"),
    "cms_publish": ("review_publish.rs", "cms_publish_step::execute"),
    "rebuild_detect": ("rebuild_detect.rs", "pub async fn execute"),
}


def main() -> int:
    activities = ACTIVITIES.read_text(encoding="utf-8")
    workflows_mod = WORKFLOWS_MOD.read_text(encoding="utf-8")
    seo_workflow = SEO_WORKFLOW.read_text(encoding="utf-8") if SEO_WORKFLOW.exists() else ""
    temporal_starter = TEMPORAL_STARTER.read_text(encoding="utf-8")
    application_modules = {
        path.name: path.read_text(encoding="utf-8")
        for path in sorted(SEO_APPLICATION.glob("*.rs"))
    }
    payload_store = PAYLOAD_STORE.read_text(encoding="utf-8")
    failures: list[str] = []
    if "load_seo_site_build_input" not in activities:
        failures.append("missing Temporal activity `load_seo_site_build_input`")
    if "load_seo_site_build_input" not in seo_workflow:
        failures.append("SeoSiteBuildWorkflow does not load runtime DB input first")
    for step in SEO_STEP_NAMES:
        module_path = SEO_STEPS_DIR / f"{step}_step.rs"
        if not module_path.exists():
            failures.append(f"missing seo_steps module `{module_path.relative_to(ROOT)}`")
        if f"run_{step}_step" not in activities:
            failures.append(f"missing Temporal activity `run_{step}_step`")
        owner_module, owner_token = APPLICATION_OWNERS[step]
        owner_text = application_modules.get(owner_module, "")
        if owner_token not in owner_text and f"run_{step}" not in owner_text:
            failures.append(f"missing seo_application orchestration for `{step}`")
        if f"\"{step}\"" not in activities:
            failures.append(f"activity `{step}` is not ledger-backed with execute_step step_name")
        if f"run_{step}_step" not in seo_workflow:
            failures.append(f"SeoSiteBuildWorkflow does not call `run_{step}_step`")
    for payload in [
        "SeoSiteBuildInputPayload",
        "SerpIngestInputPayload",
        "SerpIngestOutputPayload",
        "SerpNormalizeInputPayload",
        "OpportunityBuildInputPayload",
        "IaBuildInputPayload",
        "LinkRecommendInputPayload",
        "DraftAssembleInputPayload",
        "DraftQaInputPayload",
        "CmsPublishInputPayload",
        "RebuildDetectInputPayload",
    ]:
        if payload not in payload_store:
            failures.append(f"runtime payload store missing `{payload}`")
    if "mod seo_site_build;" not in workflows_mod or "seo_site_build::register" not in workflows_mod:
        failures.append("SeoSiteBuildWorkflow is not registered in worker options")
    if "SeoSiteBuild" not in temporal_starter or "SeoSiteBuildWorkflow" not in temporal_starter:
        failures.append("temporal_starter cannot start SeoSiteBuildWorkflow")
    if "SeoSiteBuildInputPayload" not in temporal_starter or "input_payload" not in temporal_starter:
        failures.append("temporal_starter does not persist typed SeoSiteBuild input")
    if failures:
        print("SEO_RUNTIME_REGISTRATION: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_RUNTIME_REGISTRATION: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
