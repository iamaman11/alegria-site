#!/usr/bin/env python3
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1]
PROTO = ROOT / "app/contracts/proto/temporal_payloads.proto"
EXECUTION = ROOT / "app/rust/crates/seo_application/src/execution.rs"
WORKFLOW = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover.rs"
CLI = ROOT / "app/rust/services/cli_tools/src/main.rs"
CLI_MANIFEST = ROOT / "app/rust/services/cli_tools/Cargo.toml"
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
        failures.append("canonical workflow does not normalize `site_input.run_mode` before branching")
    if "scenario_kind_for_run_mode(normalized_run_mode)" not in workflow:
        failures.append("canonical workflow does not derive scenario from normalized run_mode")
    if "policy_for_run_mode(normalized_run_mode, SeoExecutionMode::TemporalDurable)" not in workflow:
        failures.append("canonical workflow does not derive policy from normalized run_mode")
    # The fixed direct CLI scenario was intentionally retired in #94. It must
    # never silently run a different plan or mutate SQL/provider state. The
    # canonical Temporal starter remains responsible for persisted run_mode.
    for variant in ("SeoRun", "CrawlPendingSources"):
        fail_closed = (
            rf"Command::{variant}\s*\{{\s*\.\.\s*\}}\s*=>\s*\{{"
            rf"\s*return\s+Err\(UnsupportedDirectSiteCommand::{variant}\.into\(\)\);"
        )
        if re.search(fail_closed, cli) is None:
            failures.append(f"cli_tools {variant} does not fail closed before I/O")
    if "CLI_DIRECT_SITE_COMMAND_DISABLED:" not in cli:
        failures.append("retired CLI site commands lack typed operator diagnostic")
    if "SqlxSeoRuntimeRepository" in cli or "execute_site_build_scenario(" in cli:
        failures.append("cli_tools still links legacy cross-provider direct SEO scenario")
    if "runtime_composition" in CLI_MANIFEST.read_text(encoding="utf-8"):
        failures.append("CLI manifest still depends on graph/vector runtime_composition")
    if 'include!("cli/seo_run.rs");' in cli:
        failures.append("CLI still compiles retired direct SEO execution")
    if 'default_value = "publish_with_hitl"' not in starter:
        failures.append("temporal_starter missing default run_mode")
    if "normalize_run_mode(&run_mode)" not in starter:
        failures.append("temporal_starter does not normalize run_mode before registration")
    if "SeoSiteBuildCanonicalCutoverWorkflow" not in starter:
        failures.append("Temporal canonical workflow starter is not reachable")

    if failures:
        print("RUN_MODES_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("RUN_MODES_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
