#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ACTIVITIES = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "mod.rs"
WORKFLOWS_MOD = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "mod.rs"
SEO_WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "seo_site_build_canonical_cutover.rs"
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


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "mod.rs" and "activities" in path.parts:
        registry_file = path.parent / "registry" / "mod_registry_impl.rs"
        if registry_file.exists():
            text += "\n" + registry_file.read_text(encoding="utf-8")
    elif path.name == "proto_runtime_payload_store.rs":
        sub_dir = path.parent / "proto_runtime_payload_store"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "temporal_starter.rs":
        sub_dir = path.parent / "temporal_starter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "seo_site_build_canonical_cutover.rs":
        for part in sorted(path.parent.glob("seo_site_build_canonical_cutover/*.rs")):
            text += "\n" + part.read_text(encoding="utf-8")
    return text


def main() -> int:
    activities = read_file_resolved(ACTIVITIES)
    workflows_mod = WORKFLOWS_MOD.read_text(encoding="utf-8")
    seo_workflow = read_file_resolved(SEO_WORKFLOW)
    temporal_starter = read_file_resolved(TEMPORAL_STARTER)
    application_modules = {}
    for path in sorted(SEO_APPLICATION.glob("*.rs")):
        text = path.read_text(encoding="utf-8")
        if path.name == "planning.rs":
            sub_dir = path.parent / "planning"
            if sub_dir.is_dir():
                for sub_file in sub_dir.glob("*.rs"):
                    text += "\n" + sub_file.read_text(encoding="utf-8")
        application_modules[path.name] = text
    payload_store = read_file_resolved(PAYLOAD_STORE)
    failures: list[str] = []
    if "load_seo_site_build_input" not in activities:
        failures.append("missing Temporal activity `load_seo_site_build_input`")
    if "AlegriaActivities::load_seo_site_build_input" not in seo_workflow:
        failures.append("canonical workflow does not load persisted runtime input")
    if "seo_application::rebuild_detect::execute(" not in activities:
        failures.append("Temporal rebuild activity must delegate to seo_application::rebuild_detect")
    for step in SEO_STEP_NAMES:
        # Rebuild detection is implemented by the typed Application owner,
        # not by the deleted transport-era rebuild_detect_step facade.
        if step != "rebuild_detect":
            module_path = SEO_STEPS_DIR / f"{step}_step.rs"
            if not module_path.exists():
                failures.append(f"missing seo_steps module `{module_path.relative_to(ROOT)}`")
        if step == "cms_publish":
            for phase in ("run_cms_request_review_step", "run_cms_publish_approved_step"):
                if phase not in activities:
                    failures.append(f"missing canonical Temporal activity `{phase}`")
        elif f"run_{step}_step" not in activities:
            failures.append(f"missing Temporal activity `run_{step}_step`")
        owner_module, owner_token = APPLICATION_OWNERS[step]
        owner_text = application_modules.get(owner_module, "")
        if owner_token not in owner_text and f"run_{step}" not in owner_text:
            failures.append(f"missing seo_application orchestration for `{step}`")
        if step == "cms_publish":
            for phase in ("cms_request_review", "cms_publish_approved"):
                if f'"{phase}"' not in activities:
                    failures.append(f"canonical CMS activity `{phase}` is not ledger-backed")
        elif f'"{step}"' not in activities:
            failures.append(f"activity `{step}` is not ledger-backed with execute_step step_name")
        if step == "cms_publish":
            for phase in ("run_cms_request_review_step", "run_cms_publish_approved_step"):
                if phase not in seo_workflow:
                    failures.append(f"canonical workflow does not call `{phase}`")
        elif f"run_{step}_step" not in seo_workflow:
            failures.append(f"canonical workflow does not call `run_{step}_step`")
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
    if "mod seo_site_build_canonical_cutover;" not in workflows_mod or "seo_site_build_canonical_cutover::register" not in workflows_mod:
        failures.append("canonical SeoSiteBuild workflow is not registered in worker options")
    if "mod seo_site_build;" in workflows_mod or "mod content_generation;" in workflows_mod:
        failures.append("removed legacy workflows must not be registered")
    if "SeoSiteBuildCanonicalCutoverWorkflow" not in temporal_starter:
        failures.append("temporal_starter cannot start canonical SeoSiteBuild workflow")
    if "register_site_build_input(" not in temporal_starter or "SeoSiteBuildRegistrationRequest" not in temporal_starter:
        failures.append("temporal_starter does not use shared SeoSiteBuild registration path")
    if failures:
        print("SEO_RUNTIME_REGISTRATION: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_RUNTIME_REGISTRATION: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
