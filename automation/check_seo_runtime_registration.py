#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ACTIVITIES = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "mod.rs"
WORKFLOWS_MOD = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "mod.rs"
SEO_WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "seo_site_build.rs"
TEMPORAL_STARTER = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "bin" / "temporal_starter.rs"
STEP_CATALOG = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "step_catalog.rs"
USE_CASES = ROOT / "app" / "rust" / "crates" / "use_cases" / "src"
PAYLOAD_STORE = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "proto_runtime_payload_store.rs"

SEO_STEPS = [
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


def main() -> int:
    activities = ACTIVITIES.read_text(encoding="utf-8")
    workflows_mod = WORKFLOWS_MOD.read_text(encoding="utf-8")
    seo_workflow = SEO_WORKFLOW.read_text(encoding="utf-8") if SEO_WORKFLOW.exists() else ""
    temporal_starter = TEMPORAL_STARTER.read_text(encoding="utf-8")
    step_catalog = STEP_CATALOG.read_text(encoding="utf-8")
    payload_store = PAYLOAD_STORE.read_text(encoding="utf-8")
    failures: list[str] = []
    if "load_seo_site_build_input" not in activities:
        failures.append("missing Temporal activity `load_seo_site_build_input`")
    if "load_seo_site_build_input" not in seo_workflow:
        failures.append("SeoSiteBuildWorkflow does not load runtime DB input first")
    for step in SEO_STEPS:
        module_path = USE_CASES / f"{step}_step.rs"
        if not module_path.exists():
            failures.append(f"missing use_case module `{module_path.relative_to(ROOT)}`")
        if f"run_{step}_step" not in activities:
            failures.append(f"missing Temporal activity `run_{step}_step`")
        if f"run_{step}" not in step_catalog:
            failures.append(f"missing step_catalog function `run_{step}`")
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
