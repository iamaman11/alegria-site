#!/usr/bin/env python3
"""Small offline contract for fast PR CI and opt-in comprehensive CI.

The project is experimental: don't lock every test to a CI job forever.
Check actual gates and that expensive integration suites remain executable.
"""
from pathlib import Path
import re
import subprocess
from report_sccache_failures import summarize

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
    # All acceptance/seed CI jobs live in one workflow: no competing CI runs.
    assert not (ROOT / ".github/workflows/ci-deep.yml").exists()
    assert not (ROOT / ".github/workflows/rust-cache-prime.yml").exists()
    deep = workflow.split("  comprehensive:\n", 1)[1].split("  rust-cache-prime:\n", 1)[0]
    prime = workflow.split("  rust-cache-prime:\n", 1)[1]
    release = (ROOT / ".github/workflows/worker-image.yml").read_text(encoding="utf-8")
    legacy = (ROOT / "automation/ci_verify.sh").read_text(encoding="utf-8")
    assert workflow.startswith("name: ci\n")
    assert "  pull_request:\n" in workflow
    assert "  push:\n    branches: [main]\n" in workflow
    assert "  workflow_dispatch:\n" in workflow
    assert "  gate:\n    name: CI\n    if: always() && (github.event_name == 'pull_request' || github.event_name == 'push')\n" in workflow
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
    # A new PR restores cache objects from the trusted default branch. It does
    # not flood the service with cache writes in its PR-local merge ref.
    assert "SCCACHE_GHA_RW_MODE: READ_ONLY" in architecture
    assert "SCCACHE_ERROR_LOG:" in architecture
    assert "SCCACHE_ERROR_LOG: /tmp/sccache-pr-errors.log" in architecture
    assert "SCCACHE_LOG: sccache::server=debug" in architecture
    assert "runner.temp" not in architecture
    assert 'python3 automation/report_sccache_failures.py --log "$SCCACHE_ERROR_LOG"' in architecture
    assert "cache-mode: write" not in workflow
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
    assert "    if: github.event_name == 'workflow_dispatch'\n" in deep
    assert "  pull_request:\n" not in deep
    assert "types: [ready_for_review" not in workflow
    assert "github.event.pull_request.draft == false" not in workflow
    assert "ref: ${{ github.sha }}" in deep
    assert "bash automation/ci_verify.sh all" in deep
    assert "cargo test --workspace --locked" in deep
    assert "cargo test -p integration_harness --features e2e" in deep
    assert "--lib cms_revision_" in deep
    assert "--lib published_page_review_requires_exact_pending_revision_and_active_workflow" not in deep
    assert "--lib cms_publish_keeps_verified_links_structured_without_duplicate_body_sections" not in deep
    assert "bash automation/run_truth_certification_gate.sh" in legacy
    assert "python3 automation/smoke_manual_llm_queue.py" in legacy

    # Main-only compiler warm-up runs after an accepted main CI gate, never on
    # a PR or a manual deep dispatch, and skips non-Rust AND test-only pushes.
    # A changed production source alongside tests must still prime main.
    for forbidden in ("  pull_request_target:\n", "  workflow_run:\n"):
        assert forbidden not in workflow
    assert "    needs: gate\n" in prime
    assert "    if: github.event_name == 'push' && github.ref == 'refs/heads/main' && needs.gate.result == 'success'" in prime
    assert 'BEFORE_SHA: ${{ github.event.before }}' in prime
    assert 'git diff --name-only "$BEFORE_SHA" "$SOURCE_SHA"' in prime
    selector = (
        r"/^app\/rust\// && !/^app\/rust\/crates\/integration_harness\/src\/tests\// { needed=1 }"
        "\nEND { exit !needed }"
    )
    assert r"/^app\/rust\// && !/^app\/rust\/crates\/integration_harness\/src\/tests\// { needed=1 }" in prime
    for paths, expected in (
        (["docs/ci-fast-path.md"], False),
        ([".github/workflows/ci.yml"], False),
        (["app/rust/crates/integration_harness/src/tests/manual_llm_temporal_tests.rs"], False),
        (["app/rust/crates/integration_harness/src/tests/manual_llm_temporal_tests.rs",
          "app/rust/crates/seo_domain/src/cms_revision.rs"], True),
        (["app/rust/crates/integration_harness/src/containers.rs"], True),
        (["app/rust/Cargo.lock"], True),
    ):
        selected = subprocess.run(
            ["awk", selector], input="\n".join(paths) + "\n",
            text=True, capture_output=True, check=False,
        )
        assert (selected.returncode == 0) == expected, (paths, selected.stderr)
    assert "MAIN_CACHE_PRIME=SKIPPED no cached Rust compilation input changed" in prime
    assert prime.count("if: steps.cache_scope.outputs.needed == 'true'") >= 6
    assert "if: always() && steps.cache_scope.outputs.needed == 'true'" in prime
    assert "SCCACHE_GHA_RW_MODE: READ_WRITE" in prime
    assert 'CARGO_BUILD_JOBS: "2"' in prime
    assert "SCCACHE_ERROR_LOG: /tmp/sccache-main-errors.log" in prime
    assert "SCCACHE_LOG: sccache::server=debug" in prime
    assert "runner.temp" not in prime
    assert "SCCACHE_GHA_ENABLED: \"on\"" in prime
    assert "version: \"v0.16.0\"" in prime
    assert "cargo check --locked --package integration_harness" in prime
    assert "cargo test --locked --package integration_harness --no-run" in prime
    assert "cargo test -p integration_harness --features e2e --lib --no-run --locked" in prime
    assert "cargo build --locked -p temporal_worker -p outbox_worker" in prime
    assert "core.exportVariable('ACTIONS_RUNTIME_TOKEN', token)" in prime
    assert "cache-mode: write" not in prime
    assert 'python3 automation/report_sccache_failures.py --log "$SCCACHE_ERROR_LOG"' in prime

    # Only safe aggregates may leave the disposable sccache daemon log.
    classified = summarize([
        "sccache Error executing cache write: 429 Too Many Requests token=private-secret",
        "sccache Cache write error: 403 Forbidden url=https://private.example.test",
        "sccache Error executing cache write: Cannot write to read-only storage",
        "sccache Cache write error: unexpected backend failure token=private-secret",
        "sccache request succeeded token=private-secret",
    ])
    assert classified == {"write_failure_log_lines": 4, "classes": {
        "rate_limited_429": 1, "forbidden_403": 1, "read_only_skip": 1, "unclassified": 1,
    }}
    assert "private-secret" not in str(classified)


if __name__ == "__main__":
    check()
    print("FAST_CI_AND_OPT_IN_DEEP_CONTRACT=PASS")
