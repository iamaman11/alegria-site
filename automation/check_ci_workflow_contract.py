#!/usr/bin/env python3
"""Small offline contract for unified fast PR, deep acceptance and OCI release.

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
    rust_deep = workflow.split("  rust-regressions:\n", 1)[1].split("  integration-e2e:\n", 1)[0]
    integration_deep = workflow.split("  integration-e2e:\n", 1)[1].split("  comprehensive:\n", 1)[0]
    deep = workflow.split("  comprehensive:\n", 1)[1].split("  package:\n", 1)[0]
    release = workflow.split("  package:\n", 1)[1]
    legacy = (ROOT / "automation/ci_verify.sh").read_text(encoding="utf-8")
    assert workflow.startswith("name: ci\n")
    assert "  pull_request:\n" in workflow
    assert "  push:\n    branches: [main]\n" in workflow
    assert "  workflow_dispatch:\n" in workflow
    assert "ready_for_review" in workflow and "converted_to_draft" in workflow
    assert not (ROOT / ".github/workflows/ci-deep.yml").exists()
    assert not (ROOT / ".github/workflows/worker-image.yml").exists()
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
    assert "needs: [gate]" in release
    assert "needs.comprehensive.result" not in release
    assert "SOURCE_SHA: ${{ github.event.pull_request.head.sha }}" in release
    assert "file: app/rust/services/temporal/Dockerfile" in release
    assert "by.alegria.source-head-sha=${{ env.SOURCE_SHA }}" in release
    assert "uses: actions/attest@v4" in release
    assert "temporal-worker-image-${{ env.SOURCE_SHA }}" in release
    assert '"rust-regressions", "integration-e2e", "comprehensive", "package"' in gate
    assert 'git fetch --depth=1 origin "${CI_SOURCE_SHA}"' in workflow
    # A fresh runner must not waste ~3 minutes compiling the audit CLI itself.
    assert "cargo install cargo-audit" not in workflow
    assert "cargo-audit-x86_64-unknown-linux-gnu-v0.23.0.tgz" in workflow
    assert "b77993532b253fcbf550996d7716465ec3d734d4e74e7f87bfb65a19add0fb59" in workflow

    # No repeated expensive Rust compilation in three independent PR runners.
    architecture = workflow.split("  architecture-and-rust:\n", 1)[1].split("  rust-contract-smokes:\n", 1)[0]
    rust_smokes = workflow.split("  rust-contract-smokes:\n", 1)[1].split("  product-truth:\n", 1)[0]
    product = workflow.split("  product-truth:\n", 1)[1].split("  rust-security:\n", 1)[0]
    verify = workflow.split("  verify:\n", 1)[1].split("  windows-operator-smoke:\n", 1)[0]
    assert "cargo test -p runtime_models --locked" in architecture
    # Drafts retain their fast unit smoke. Ready PRs already run the same
    # model crate in the required full workspace owner after the fast gate.
    assert "if: github.event.pull_request.draft == true && steps.rust_impact.outputs.deep_required == 'true'" in architecture
    assert "deep_required: ${{ steps.rust_impact.outputs.deep_required }}" in architecture
    assert "affected_packages: ${{ steps.rust_impact.outputs.affected_packages }}" in architecture
    assert "id: rust_impact" in architecture
    assert "CI_AFFECTED_PACKAGES: ${{ steps.rust_impact.outputs.affected_packages }}" in architecture
    assert 'command = ["cargo", "test", "--locked"]' in architecture
    assert "CARGO_IMPACT=NO_RUST_CHANGE" in architecture
    assert "CARGO_PROFILE_DEV_DEBUG" not in architecture
    assert "cargo test --workspace" not in architecture
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

    # The independent Rust/SQL, real Integration, Truth and package owners
    # run in parallel. All four are mandatory for accepted provenance.
    for group in (rust_deep, integration_deep, deep):
        assert "github.event.pull_request.draft == false" in group
        assert "ref: ${{ env.CI_SOURCE_SHA }}" in group
        assert "CI_SOURCE_SHA: ${{ github.event.pull_request.head.sha || github.sha }}" in group
        assert "needs: [gate, architecture-and-rust]" in group
        assert "needs.architecture-and-rust.outputs.deep_required == 'true'" in group
        assert 'CARGO_PROFILE_DEV_DEBUG: "line-tables-only"' in group
        assert 'CARGO_PROFILE_TEST_DEBUG: "line-tables-only"' in group
        assert "RUSTFLAGS:" not in group  # Stable Rust already uses its native lld linker.
    assert all('CI_WORKSPACE_TESTED: "true"' in group for group in (rust_deep, integration_deep))
    assert 'CI_WORKSPACE_TESTED: "true"' not in deep
    # The scoped Truth reader/writer test needs the disposable hosted DB.
    # Enable it in the required workspace owner, never in the fixture-only job.
    assert 'ALEGRIA_SCOPED_TRUTH_TEST: "true"' in rust_deep
    assert 'ALEGRIA_SCOPED_TRUTH_TEST' not in deep
    assert 'DATABASE_URL: postgres://alegria_ci:alegria_ci_only@127.0.0.1:5432/alegria_ci' in rust_deep
    assert "CI_DEEP_SCOPE=NOT_AFFECTED_BY_SOURCE_CHANGE" in deep
    for group, steps in (
        (rust_deep, ("Workspace unit tests", "All retained-source/editorial PostgreSQL regressions", "Full architecture and Rust-backed contracts")),
        (integration_deep, ("Full Temporal/LLM and semantic integration E2E",)),
        (deep, ("Real Qdrant transport and all 16 Truth Worker fixtures",)),
    ):
        for step in steps:
            assert f"      - name: {step}\n        if: github.event_name == 'workflow_dispatch' || needs.architecture-and-rust.outputs.deep_required == 'true'" in group
    assert rust_deep.count("bash automation/ci_verify.sh contracts") == 1
    assert deep.count("run: bash automation/run_truth_certification_gate.sh") == 1
    assert "bash automation/ci_verify.sh product" not in deep
    assert integration_deep.count("bash automation/ci_verify.sh integration") == 1
    assert "bash automation/ci_verify.sh integration" not in deep
    assert "bash automation/ci_verify.sh all" not in workflow
    assert "cargo test --workspace --locked" in rust_deep
    assert "cargo test -p integration_harness --features e2e" in rust_deep
    assert "cargo test --workspace --locked" not in deep
    assert "cargo test -p integration_harness --features e2e" not in deep
    assert 'WHOLE_PAGE_SEMANTIC_TARGET_DIR: ${{ github.workspace }}/app/rust/target' in integration_deep
    assert "CI_WORKSPACE_TESTED: \"true\"" in integration_deep
    assert "bash automation/run_truth_certification_gate.sh" in legacy
    assert "bash automation/ci_verify.sh product-policy" in fast
    assert '"$PHASE" == "product-policy"' in legacy
    assert "run_product_rust_smoke" in legacy
    assert 'python3 automation/smoke_contradiction_handling.py' not in fast
    assert "services:\n" not in deep
    assert "postgresql-client" not in deep
    assert "python -m pip install" not in deep
    assert "PostgresHarness::start().await?" in (ROOT / "app/rust/crates/integration_harness/src/containers.rs").read_text()
    assert "python3 automation/smoke_manual_llm_queue.py" in legacy
    truth = (ROOT / "automation/run_truth_certification_gate.sh").read_text(encoding="utf-8")
    manual = (ROOT / "automation/smoke_manual_llm_queue.py").read_text(encoding="utf-8")
    semantic = (ROOT / "automation/run_whole_page_semantic_gate.sh").read_text(encoding="utf-8")
    assert 'bash automation/run_truth_prepare.sh "$SUITE_TARGET_DIR"' in truth
    assert 'if env.get(temporal_key) and env.get(outbox_key)' in manual
    assert 'TARGET_DIR="${WHOLE_PAGE_SEMANTIC_TARGET_DIR:-' in semantic
    assert "INTEGRATION_TEMPORAL_WORKER_BIN:" not in integration_deep
    assert "INTEGRATION_OUTBOX_WORKER_BIN:" not in integration_deep
    assert '"cargo", "build", "--locked", "-p", "temporal_worker", "-p", "outbox_worker"' in manual
    # Real workspace tests are owned by a mandatory sibling. Standalone local
    # audits still execute their own Rust checks without this setting.
    assert '${CI_WORKSPACE_TESTED:-}' in legacy
    run_report = (ROOT / "automation/smoke_run_report_completeness.py").read_text(encoding="utf-8")
    assert 'os.environ.get("CI_WORKSPACE_TESTED") == "true"' in run_report
    assert '"full_harness"' in run_report


if __name__ == "__main__":
    check()
    print("FAST_CI_AND_OPT_IN_DEEP_CONTRACT=PASS")
