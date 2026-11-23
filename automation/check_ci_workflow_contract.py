#!/usr/bin/env python3
"""Small offline contract for fast PR CI and opt-in comprehensive CI.

The project is experimental: don't lock every test to a CI job forever.
Check actual gates and that expensive integration suites remain executable.
"""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
OWNERS = (
    "architecture-and-rust", "rust-contract-smokes", "product-truth",
    "rust-security", "cloudflare-static-build",
    "stack-temporal-postgres18", "stack-storage-graph", "verify",
    "windows-operator-smoke",
)


def check() -> None:
    workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    fast = (ROOT / "automation/ci_fast_verify.sh").read_text(encoding="utf-8")
    deep = (ROOT / ".github/workflows/ci-deep.yml").read_text(encoding="utf-8")
    release = (ROOT / ".github/workflows/worker-image.yml").read_text(encoding="utf-8")
    legacy = (ROOT / "automation/ci_verify.sh").read_text(encoding="utf-8")
    assert workflow.startswith("name: ci\n")
    assert "  pull_request:\n" in workflow
    assert "  push:\n    branches: [main]\n" in workflow
    assert "  gate:\n    name: CI\n    if: always()\n" in workflow
    assert "toJSON(needs)" in workflow and "CI_PLAN_SHA" in workflow
    assert "selected = json.loads(os.environ['SELECTED_CI_JOBS'])" in workflow
    gate = workflow.split("  gate:\n", 1)[1]
    assert "  impact:\n" in workflow
    assert "      - impact\n" in gate
    assert "run: python3 automation/check_docs_layout.py" in workflow
    assert "python3 automation/ci_impact.py --base" in workflow
    assert "CI_IMPACT=PASS selected=" in workflow
    for owner in OWNERS:
        assert f"  {owner}:\n" in workflow, owner
        assert f"      - {owner}\n" in gate, owner
        assert f"contains(fromJSON(needs.impact.outputs.selected_ci_jobs), '{owner}')" in workflow, owner
    assert len(re.findall(r"(?m)^    if: github.event_name == 'pull_request' && needs\.impact\.result == 'success' && contains\(", workflow)) == len(OWNERS)

    # Main verifies exact source + the accepted PR before release/deployment.
    assert "commits/{merge_sha}/pulls" in workflow
    assert "actions/runs?head_sha={source_sha}" in workflow
    assert "job[\"conclusion\"]" in workflow
    assert "temporal-worker-docker" not in workflow
    assert release.startswith("name: worker-image\n")
    assert "  push:\n    branches: [main]\n" in release
    assert "  workflow_dispatch:\n" in release
    assert "  select-image:\n" in release
    assert "python3 automation/ci_impact.py" in release
    assert "--head \"$SOURCE_SHA\"" in release
    assert "status = report.get('image_status', {}).get('temporal-worker')" in release
    assert "if: needs.select-image.outputs.worker_required == 'true'" in release
    assert "worker-image-impact-${{ env.SOURCE_SHA }}" in release
    assert "SOURCE_SHA: ${{ github.sha }}" in release
    assert "file: app/rust/services/temporal/Dockerfile" in release
    assert "by.alegria.source-head-sha=${{ env.SOURCE_SHA }}" in release
    assert "uses: actions/attest@v4" in release
    assert "temporal-worker-image-${{ env.SOURCE_SHA }}" in release
    # A fresh runner must not waste ~3 minutes compiling the audit CLI itself.
    assert "cargo install cargo-audit" not in workflow
    assert "cargo-audit-x86_64-unknown-linux-gnu-v0.23.0.tgz" in workflow
    assert "b77993532b253fcbf550996d7716465ec3d734d4e74e7f87bfb65a19add0fb59" in workflow

    # No repeated expensive Rust compilation in three independent PR runners.
    architecture = workflow.split("  architecture-and-rust:\n", 1)[1].split("  rust-contract-smokes:\n", 1)[0]
    rust_smokes = workflow.split("  rust-contract-smokes:\n", 1)[1].split("  product-truth:\n", 1)[0]
    product = workflow.split("  product-truth:\n", 1)[1].split("  rust-security:\n", 1)[0]
    verify = workflow.split("  verify:\n", 1)[1].split("  windows-operator-smoke:\n", 1)[0]
    assert "python3 automation/run_scoped_cargo_feedback.py" in architecture
    assert "cargo test -p integration_harness --features e2e" not in architecture
    assert "cargo test" not in rust_smokes and "cargo check" not in rust_smokes
    assert "cargo metadata --format-version 1 --no-deps --locked" in rust_smokes
    for phase, owner in (("contracts", verify), ("product", product), ("integration", verify)):
        invocation = f"bash automation/measure_ci_phase.sh {phase} bash automation/ci_fast_verify.sh {phase}"
        assert workflow.count(invocation) == 1, phase
        assert invocation in owner
        assert f"  {phase})" in fast
        assert f'"$PHASE" == "{phase}"' in legacy
    for block in (rust_smokes, product, verify):
        assert "bash automation/run_truth_certification_gate.sh" not in block
        assert "python3 automation/smoke_manual_llm_queue.py" not in block
        assert "bash automation/ci_verify.sh" not in block

    # Deep acceptance is available explicitly, with real Rust-backed checks,
    # all 16 Truth fixtures and Temporal/manual E2E. It must never start
    # automatically when a PR is marked Ready or synchronized: exact-head
    # scoped suites already run in the required `ci` workflow.
    assert deep.startswith("name: ci-deep\n")
    assert "  workflow_dispatch:\n" in deep
    assert "  pull_request:\n" not in deep
    assert "types: [ready_for_review" not in deep
    assert "github.event.pull_request.draft == false" not in deep
    assert "ref: ${{ github.sha }}" in deep
    assert "bash automation/ci_verify.sh all" in deep
    assert "cargo test --workspace --locked" in deep
    assert "cargo test -p integration_harness --features e2e" in deep
    assert "--lib cms_revision_" in deep
    assert "--lib published_page_review_requires_exact_pending_revision_and_active_workflow" not in deep
    assert "--lib cms_publish_keeps_verified_links_structured_without_duplicate_body_sections" not in deep
    assert "bash automation/run_truth_certification_gate.sh" in legacy
    assert "python3 automation/smoke_manual_llm_queue.py" in legacy


if __name__ == "__main__":
    check()
    print("FAST_CI_AND_OPT_IN_DEEP_CONTRACT=PASS")
