#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
PROTO = ROOT / "app/contracts/proto/temporal_payloads.proto"
EXECUTION = ROOT / "app/rust/crates/seo_application/src/execution.rs"
WORKFLOW = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build.rs"
CLI = ROOT / "app/rust/services/cli_tools/src/main.rs"
STARTER = ROOT / "app/rust/services/temporal/src/bin/temporal_starter.rs"


def read_with_includes(path: Path) -> str:
    if not path.exists():
        return ""
    text = path.read_text(encoding="utf-8")
    import re
    parent = path.parent
    for match in re.finditer(r'include!\("([^"]+)"\);', text):
        include_path = parent / match.group(1)
        if include_path.exists():
            text += "\n" + read_with_includes(include_path)
    return text


def main() -> int:
    proto = read_with_includes(PROTO)
    execution = read_with_includes(EXECUTION)
    workflow = read_with_includes(WORKFLOW)
    cli = read_with_includes(CLI)
    starter = read_with_includes(STARTER)

    failures: list[str] = []

    if "string run_mode = 8;" not in proto:
        failures.append("SeoSiteBuildInputPayload missing additive `run_mode` field in proto")

    for needle in [
        'SEO_RUN_MODE_DRY_RUN: &str = "dry_run"',
        'SEO_RUN_MODE_CRAWL_ONLY: &str = "crawl_only"',
        'SEO_RUN_MODE_DRAFT_ONLY: &str = "draft_only"',
        'SEO_RUN_MODE_PUBLISH_WITH_HITL: &str = "publish_with_hitl"',
        'SEO_RUN_MODE_FULL_AUTO_AFTER_APPROVAL: &str = "full_auto_after_approval"',
        'normalize_run_mode(',
        'scenario_kind_for_run_mode(',
        'policy_for_run_mode(',
    ]:
        if needle not in execution:
            failures.append(f"execution contract missing `{needle}`")

    if "let normalized_run_mode = normalize_run_mode(&site_input.run_mode);" not in workflow:
        failures.append("SeoSiteBuildWorkflow does not normalize `site_input.run_mode` before branching")
    if "scenario_kind_for_run_mode(normalized_run_mode)" not in workflow:
        failures.append("SeoSiteBuildWorkflow does not derive scenario from normalized run_mode")
    if "policy_for_run_mode(normalized_run_mode, SeoExecutionMode::TemporalDurable)" not in workflow:
        failures.append("SeoSiteBuildWorkflow does not derive policy from normalized run_mode")
    if "run_mode_for_scenario(" not in cli:
        failures.append("cli_tools does not persist run_mode contract for seo-run path")
    if 'default_value = "publish_with_hitl"' not in starter:
        failures.append("temporal_starter missing compatibility default for run_mode")
    if "normalize_run_mode(&run_mode)" not in starter:
        failures.append("temporal_starter does not normalize run_mode before registration")

    if failures:
        print("RUN_MODES_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("RUN_MODES_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
