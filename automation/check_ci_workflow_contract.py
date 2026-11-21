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
    assert "toJSON(needs)" in workflow and 'result != "success"' in workflow
    gate = workflow.split("  gate:\n", 1)[1]
    for owner in OWNERS:
        assert f"  {owner}:\n" in workflow, owner
        assert f"      - {owner}\n" in gate, owner
    assert len(re.findall(r"(?m)^    if: github.event_name == 'pull_request'$", workflow)) == len(OWNERS)

    # Main verifies exact source + the accepted PR before release/deployment.
    assert "commits/{merge_sha}/pulls" in workflow
    assert "actions/runs?head_sha={source_sha}" in workflow
    assert "job[\"conclusion\"]" in workflow
    assert "temporal-worker-docker" not in workflow
    assert release.startswith("name: worker-image\n")
    assert "  push:\n    branches: [main]\n" in release
    assert "  workflow_dispatch:\n" in release
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
    assert "cargo test --workspace --locked" in architecture
    assert "--features e2e --locked --lib retained_corpus_reuses_exact_paid_result" in architecture
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
    # all 16 Truth fixtures and Temporal/manual E2E. It is not a PR blocker.
    assert deep.startswith("name: ci-deep\n")
    assert "  workflow_dispatch:\n" in deep
    assert "  pull_request:\n" not in deep
    assert "bash automation/ci_verify.sh all" in deep
    assert "cargo test --workspace --locked" in deep
    assert "cargo test -p integration_harness --features e2e" in deep
    assert "bash automation/run_truth_certification_gate.sh" in legacy
    assert "python3 automation/smoke_manual_llm_queue.py" in legacy


if __name__ == "__main__":
    check()
    print("FAST_CI_AND_OPT_IN_DEEP_CONTRACT=PASS")
