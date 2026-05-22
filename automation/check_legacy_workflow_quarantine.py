#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS_MOD = ROOT / "app/rust/services/temporal/src/workflows/mod.rs"
STARTER = ROOT / "app/rust/services/temporal/src/bin/temporal_starter.rs"
GATE = ROOT / "automation/temporal_production_gate.sh"
RUNBOOK = ROOT / "docs/OPS_RUNTIME_RUNBOOK.md"
PLAN = ROOT / "docs/SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md"


def main() -> int:
    workflows_mod = WORKFLOWS_MOD.read_text(encoding="utf-8")
    starter = STARTER.read_text(encoding="utf-8")
    gate = GATE.read_text(encoding="utf-8")
    runbook = RUNBOOK.read_text(encoding="utf-8")
    plan = PLAN.read_text(encoding="utf-8")

    failures: list[str] = []

    for needle in [
        'if std::env::var("ALLOW_LEGACY_CONTENT_WORKFLOW")',
        'if std::env::var("ALLOW_EXPERT_MIGRATION_WORKFLOWS")',
        "content_generation::register(&mut opts);",
        "expert_decomposed_extraction::register(&mut opts);",
        "expert_extraction::register(&mut opts);",
        "expert_projection::register(&mut opts);",
        "expert_semantic_slice::register(&mut opts);",
    ]:
        if needle not in workflows_mod:
            failures.append(f"workflows/mod.rs missing `{needle}`")

    for needle in [
        "WorkflowKind::ContentGeneration",
        "ContentGenerationWorkflow is legacy-only. Use SeoSiteBuildCanonicalCutoverWorkflow for production SEO generation.",
        "WorkflowKind::ExpertDecomposedExtraction",
        "WorkflowKind::ExpertExtraction",
        "WorkflowKind::ExpertProjection",
        "WorkflowKind::ExpertSemanticSlice",
        "Expert migration workflows are diagnostic-only. Set ALLOW_EXPERT_MIGRATION_WORKFLOWS=true for explicit migration diagnostics, or use SeoSiteBuildCanonicalCutoverWorkflow for canonical execution.",
    ]:
        if needle not in starter:
            failures.append(f"temporal_starter.rs missing `{needle}`")

    if "run_content_workflow" in gate:
        failures.append("temporal production gate still references run_content_workflow")
    if "content-generation" in gate:
        failures.append("temporal production gate still starts legacy content-generation workflow")
    if "run_seo_site_build_workflow" not in gate:
        failures.append("temporal production gate missing run_seo_site_build_workflow")

    for needle in [
        "`ContentGenerationWorkflow`:",
        "legacy-only",
        "keep disabled by default outside explicit legacy cutover testing",
        "migration-only diagnostic extraction surface",
        "migration-only diagnostic decomposition surface for the legacy extraction macro-step",
        "migration-only diagnostic extraction-plus-projection surface",
        "migration-only diagnostic real-section semantic surface",
    ]:
        if needle not in runbook:
            failures.append(f"OPS_RUNTIME_RUNBOOK.md missing `{needle}`")

    for needle in [
        "strict sequential execution",
        "Step 1 — Close `R0` Compatibility Perimeter",
    ]:
        if needle not in plan:
            failures.append(f"SUPERSITE plan missing `{needle}`")

    if failures:
        print("LEGACY_WORKFLOW_QUARANTINE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("LEGACY_WORKFLOW_QUARANTINE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
