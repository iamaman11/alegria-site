#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build.rs"
ACTIVITIES = ROOT / "app/rust/services/temporal/src/activities/mod.rs"
STARTER = ROOT / "app/rust/services/temporal/src/bin/temporal_starter.rs"


FORBIDDEN = [
    "default_scope",
    "seed_queries",
    "seed_factual_fragments",
    "Spain tourist visa",
    "spain tourist visa requirements",
]


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "mod.rs" and "activities" in path.parts:
        registry_file = path.parent / "registry" / "mod_registry_impl.rs"
        if registry_file.exists():
            text += "\n" + registry_file.read_text(encoding="utf-8")
    elif path.name == "temporal_starter.rs":
        sub_dir = path.parent / "temporal_starter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    workflow = WORKFLOW.read_text(encoding="utf-8")
    activities = read_file_resolved(ACTIVITIES)
    starter = read_file_resolved(STARTER)
    failures: list[str] = []

    for needle in FORBIDDEN:
        if needle in workflow:
            failures.append(f"SeoSiteBuildWorkflow still contains seed/demo fallback `{needle}`")

    for needle in [
        "load_seo_site_build_input",
        "SeoSiteBuildInputPayload",
        "SerpIngestInputPayload",
    ]:
        if needle not in workflow:
            failures.append(f"SeoSiteBuildWorkflow missing production input needle `{needle}`")

    if "load_seo_site_build_input" not in activities:
        failures.append("Temporal activities missing load_seo_site_build_input activity")
    if "register_site_build_input(" not in starter or "SeoSiteBuildRegistrationRequest" not in starter:
        failures.append("temporal_starter does not use shared typed SEO registration path")

    if failures:
        print("SEO_NO_SEED_WORKFLOW: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_NO_SEED_WORKFLOW: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
