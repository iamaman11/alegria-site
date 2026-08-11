#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (
    ROOT
    / "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover.rs"
)
ACTIVITIES = ROOT / "app/rust/services/temporal/src/activities/mod.rs"
SEO_RUNTIME = ROOT / "app/rust/crates/seo_application/src/seo_runtime.rs"
SUPPORT_ADAPTER = (
    ROOT
    / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/verified_support.rs"
)
PROJECTION_PORTS = (
    ROOT
    / "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter/source_context_projection_ports.rs"
)
STARTER = ROOT / "app/rust/services/temporal/src/bin/temporal_starter.rs"


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
    failures: list[str] = []
    workflow = WORKFLOW.read_text(encoding="utf-8")
    activities = read_file_resolved(ACTIVITIES)
    seo_runtime = SEO_RUNTIME.read_text(encoding="utf-8")
    support_adapter = SUPPORT_ADAPTER.read_text(encoding="utf-8")
    projection_ports = PROJECTION_PORTS.read_text(encoding="utf-8")
    starter = read_file_resolved(STARTER)

    initial_support = workflow.find('"load_verified_support_bundle.initial"')
    truth_write = workflow.find('"verified_truth_write"')
    graph_gate = workflow.find('"graph_admissibility_gate"')
    if min(initial_support, truth_write, graph_gate) < 0:
        failures.append(
            "canonical workflow must expose initial support, verified_truth_write, and graph_admissibility_gate"
        )
    elif not initial_support < truth_write < graph_gate:
        failures.append(
            "canonical workflow ordering must be initial support -> verified_truth_write -> graph admissibility"
        )

    if "pub async fn load_verified_support_bundle" not in activities:
        failures.append("activities missing `load_verified_support_bundle`")
    if "seo_application::seo_runtime::load_verified_support_bundle" not in activities:
        failures.append("Temporal activity must use shared seo_application support loader")

    loader_start = seo_runtime.find("pub async fn load_verified_support_bundle")
    gate_start = seo_runtime.find("pub fn truth_admissibility_gate")
    if loader_start < 0 or gate_start < 0 or gate_start <= loader_start:
        failures.append("seo_runtime support loader/gate layout is missing")
    else:
        loader_body = seo_runtime[loader_start:gate_start]
        if "repo.load_verified_support_bundle(request).await" not in loader_body:
            failures.append("application support loader must remain a read-only repository call")
        if "truth_admissibility_gate(" in loader_body:
            failures.append(
                "application support loader still fail-closes initial zero-truth bootstrap"
            )
    if "initial_support_read_allows_zero_truth_bootstrap" not in seo_runtime:
        failures.append("seo_runtime missing zero-truth bootstrap regression test")

    for needle in [
        "pub async fn load_verified_support_bundle(",
        '"bundle_state": if support_count == 0 { "empty_bootstrap_candidate" } else { "ready" }',
    ]:
        if needle not in support_adapter:
            failures.append(f"support adapter missing `{needle}`")

    for forbidden in [
        "if resolution.included.is_empty()",
        "verified support bundle is empty for context_key",
    ]:
        if forbidden in support_adapter:
            failures.append(
                f"initial support loader still fail-closes zero-truth bootstrap via `{forbidden}`"
            )

    for needle in [
        "verified_truth_write_done",
        "FROM pipeline.step_executions",
        "step_name = 'verified_truth_write'",
        "sqlx_seo_adapter::load_verified_support_bundle(",
        "post-truth-write verified support bundle is empty; graph projection and planning are blocked",
    ]:
        if needle not in projection_ports:
            failures.append(f"post-truth support gate missing `{needle}`")

    if "register_site_build_input(" not in starter:
        failures.append("starter should remain thin and use shared registration path")

    if failures:
        print("SUPPORT_BUNDLE_BOOTSTRAP: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SUPPORT_BUNDLE_BOOTSTRAP: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
