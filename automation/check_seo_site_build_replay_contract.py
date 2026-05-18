#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build.rs"
SQLX_ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs"
STARTER = ROOT / "app/rust/services/temporal/src/bin/temporal_starter.rs"


def require_contains(text: str, needle: str, failures: list[str], label: str) -> None:
    if needle not in text:
        failures.append(f"{label} missing `{needle}`")


def require_order(
    text: str,
    first: str,
    second: str,
    failures: list[str],
    label: str,
) -> None:
    try:
        first_idx = text.index(first)
        second_idx = text.index(second)
    except ValueError as exc:
        failures.append(f"{label} missing token for order check: {exc}")
        return
    if first_idx >= second_idx:
        failures.append(f"{label} order invalid: `{first}` must appear before `{second}`")


def main() -> int:
    workflow = WORKFLOW.read_text(encoding="utf-8")
    sqlx_adapter = SQLX_ADAPTER.read_text(encoding="utf-8")
    starter = STARTER.read_text(encoding="utf-8")
    failures: list[str] = []

    require_contains(
        workflow,
        "let normalized_run_mode = normalize_run_mode(&site_input.run_mode);",
        failures,
        "workflow",
    )
    require_contains(
        workflow,
        "let scenario = scenario_kind_for_run_mode(normalized_run_mode);",
        failures,
        "workflow",
    )
    require_contains(
        workflow,
        "policy_for_run_mode(normalized_run_mode, SeoExecutionMode::TemporalDurable)",
        failures,
        "workflow",
    )
    require_contains(workflow, '#[signal(name = "pause")]', failures, "workflow")
    require_contains(workflow, '#[signal(name = "resume")]', failures, "workflow")
    require_contains(workflow, 'ctx.state_mut(|s| s.phase = "done:crawl_ingest_only".to_string());', failures, "workflow")

    require_order(
        workflow,
        "AlegriaActivities::load_seo_site_build_input",
        "let normalized_run_mode = normalize_run_mode(&site_input.run_mode);",
        failures,
        "workflow",
    )
    require_order(
        workflow,
        "let normalized_run_mode = normalize_run_mode(&site_input.run_mode);",
        "let plan = build_execution_plan(&request);",
        failures,
        "workflow",
    )

    require_contains(
        sqlx_adapter,
        'if input.run_mode.trim().is_empty() {',
        failures,
        "sqlx_seo_adapter",
    )
    require_contains(
        sqlx_adapter,
        'input.run_mode = "publish_with_hitl".to_string();',
        failures,
        "sqlx_seo_adapter",
    )
    require_order(
        sqlx_adapter,
        'if input.run_mode.trim().is_empty() {',
        "Ok(input)",
        failures,
        "sqlx_seo_adapter",
    )

    require_contains(
        starter,
        'default_value = "publish_with_hitl"',
        failures,
        "temporal_starter",
    )
    require_contains(
        starter,
        "normalize_run_mode(&run_mode)",
        failures,
        "temporal_starter",
    )

    if failures:
        print("SEO_SITE_BUILD_REPLAY_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SEO_SITE_BUILD_REPLAY_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
