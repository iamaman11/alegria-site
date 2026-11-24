#!/usr/bin/env python3
"""Small offline contract for fast PR CI and opt-in comprehensive CI.

The project is experimental: don't lock every test to a CI job forever.
Check actual gates and that expensive integration suites remain executable.
"""
from pathlib import Path
import ast
import re
import textwrap
from ci_impact import NON_WORKER_DELIVERY_INPUTS, SCHEMA_VERSION, main_cache_prime_required, worker_release_decision
from report_sccache_failures import summarize

ROOT = Path(__file__).resolve().parents[1]
OWNERS = (
    "architecture-and-rust", "product-truth",
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
    assert sorted(path.name for path in (ROOT / ".github/workflows").glob("*.yml")) == ["ci.yml"]
    deep = workflow.split("  comprehensive:\n", 1)[1].split("  rust-cache-prime:\n", 1)[0]
    prime = workflow.split("  rust-cache-prime:\n", 1)[1].split("  select-image:\n", 1)[0]
    release = workflow.split("  select-image:\n", 1)[1].split("  dependency-refresh:\n", 1)[0]
    refresh = workflow.split("  dependency-refresh:\n", 1)[1]
    legacy = (ROOT / "automation/ci_verify.sh").read_text(encoding="utf-8")
    assert workflow.startswith("name: ci\n")
    assert "  pull_request:\n" in workflow
    assert "  push:\n    branches: [main]\n" in workflow
    assert "  workflow_dispatch:\n" in workflow
    assert "  gate:\n    name: CI\n    if: always() && (github.event_name == 'pull_request' || github.event_name == 'push' || (github.event_name == 'workflow_dispatch' && inputs.operation == 'worker-image' && github.ref == 'refs/heads/main'))\n" in workflow
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
    assert "  select-image:\n" in workflow
    assert "  package:\n" in workflow
    assert "    needs: gate\n" in release
    assert "!cancelled() && needs.gate.result == 'success'" in release
    assert "!cancelled() && needs.gate.result == 'success' && needs.select-image.result == 'success'" in release
    assert "(github.event_name == 'workflow_dispatch' && inputs.operation == 'worker-image' && github.ref == 'refs/heads/main')" in release
    assert "python3 automation/ci_impact.py" in release
    assert "--head \"$SOURCE_SHA\"" in release
    assert "worker_release_decision(" in release
    assert "cache_export_required: ${{ steps.select.outputs.cache_export_required }}" in release
    assert "f.write('cache_export_required=' + str(decision['cache_export_required']).lower() + '\\n')" in release
    assert "    needs: [gate, select-image]" in release
    assert "needs.gate.result == 'success' && needs.select-image.result == 'success'" in release
    assert "needs.select-image.outputs.worker_required == 'true' && github.ref == 'refs/heads/main'" in release
    assert "      id-token: write\n      attestations: write\n      artifact-metadata: write\n" in release
    assert "worker-image-impact-${{ env.SOURCE_SHA }}" in release
    assert "SOURCE_SHA: ${{ github.sha }}" in release
    assert "file: ${{ github.workspace }}/app/rust/services/temporal/Dockerfile" in release
    assert "python3 automation/prepare_worker_build_context.py" in release
    assert "--source-sha \"$SOURCE_SHA\" --destination \"$RELEASE_CONTEXT\"" in release
    assert "context: ${{ runner.temp }}/alegria-worker-context" in release
    assert "cache-from: type=gha,scope=alegria-temporal-worker" in release
    assert "cache-to: ${{ needs.select-image.outputs.cache_export_required == 'true' && 'type=gha,scope=alegria-temporal-worker,mode=max" in release
    assert "test -f /app/worker-context.json" in release
    assert "env -u DATABASE_URL temporal_worker" in release
    assert "DATABASE_URL must be set" in release
    assert 'python3 -c "import json, os; assert json.load' in release
    worker_main = (ROOT / "app/rust/services/temporal/src/main.rs").read_text(encoding="utf-8")
    assert "DATABASE_URL must be set" in worker_main
    dockerfile = (ROOT / "app/rust/services/temporal/Dockerfile").read_text(encoding="utf-8")
    assert dockerfile.index("RUN cargo build --release --locked") < dockerfile.index("FROM node:")
    assert dockerfile.index("FROM node:") < dockerfile.index("COPY infra/cloudflare/seo-static")
    assert "by.alegria.source-head-sha=${{ env.SOURCE_SHA }}" in release
    assert "uses: actions/attest@v4" in release
    assert "temporal-worker-image-${{ env.SOURCE_SHA }}" in release
    assert "report['worker_release_decision']" in release
    assert "print('WORKER_IMAGE_REASON=' + decision['reason'])" in release
    selection_script = textwrap.dedent(
        release.split("          python3 - <<'PY'\n", 1)[1].split("\n          PY", 1)[0]
    )
    ast.parse(selection_script)
    assert "from ci_impact import SCHEMA_VERSION, changed_paths, worker_release_decision" in selection_script
    assert "changed = changed_paths('HEAD^1', source)" in selection_script
    assert "'--name-only'" not in selection_script
    prime_script = textwrap.dedent(
        prime.split("          python3 - <<'PY'\n", 1)[1].split("\n          PY", 1)[0]
    )
    ast.parse(prime_script)
    assert "from ci_impact import changed_paths, main_cache_prime_required" in prime_script
    assert "changed = changed_paths(os.environ['BEFORE_SHA'], os.environ['SOURCE_SHA'])" in prime_script
    assert "needed = main_cache_prime_required(changed)" in prime_script
    assert "git diff --name-only" not in prime
    assert "MAIN_CACHE_PRIME=" in prime_script
    for changed, skip in (
        ([".github/workflows/ci.yml", "infra/local/windows_staging.ps1"], True),
        (["docs/ci-fast-path.md", "automation/check_ci_workflow_contract.py"], True),
        (["automation/check_worker_dependency_skeleton.py"], True),
        ([".github/workflows/ci.yml", "automation/ci_impact.py", "automation/check_ci_impact.py"], True),
        (["app/rust/services/temporal/src/main.rs"], False),
        (["automation/deploy_seo_static_cloudflare.py"], False),
        ([".github/workflows/ci.yml", "app/rust/Cargo.lock"], False),
        ([".github/workflows/new-workflow.yml"], False),
    ):
        report = {
            "schema_version": SCHEMA_VERSION,
            "mandatory_full_ci_gate": True,
            "image_status": {"temporal-worker": "selected"},
            "selected_images": ["temporal-worker"],
            "changed_paths": sorted(set(changed)),
        }
        decision = worker_release_decision(report, changed, manual=False)
        assert (not decision["worker_required"]) == skip, changed
        assert (set(changed) <= NON_WORKER_DELIVERY_INPUTS) == skip, changed
    assert "  dependency-refresh:\n" in workflow
    assert "inputs.operation == 'dependency-refresh' && startsWith(github.ref, 'refs/heads/') && github.ref != 'refs/heads/main'" in refresh
    assert "    permissions:\n      contents: write\n" in refresh
    assert "git add app/rust/Cargo.lock" in refresh
    # A fresh runner must not waste ~3 minutes compiling the audit CLI itself.
    assert "cargo install cargo-audit" not in workflow
    assert "cargo-audit-x86_64-unknown-linux-gnu-v0.23.0.tgz" in workflow
    assert "b77993532b253fcbf550996d7716465ec3d734d4e74e7f87bfb65a19add0fb59" in workflow

    # The full workspace unit suite runs in the existing verify owner while
    # architecture runs Worker/E2E. Scoped edits retain Rust's narrow runner.
    architecture = workflow.split("  architecture-and-rust:\n", 1)[1].split("  product-truth:\n", 1)[0]
    product = workflow.split("  product-truth:\n", 1)[1].split("  rust-security:\n", 1)[0]
    verify = workflow.split("  verify:\n", 1)[1].split("  windows-operator-smoke:\n", 1)[0]
    assert "python3 automation/run_scoped_cargo_feedback.py" in architecture
    assert "Verify dependency-scoped Worker release context resolves locked Cargo" in architecture
    assert "python3 automation/check_worker_dependency_skeleton.py" in architecture
    assert 'dependency-skeleton/app/rust/Cargo.toml"' in architecture
    # A new PR restores cache objects from the trusted default branch. It does
    # not flood the service with cache writes in its PR-local merge ref.
    assert "SCCACHE_GHA_RW_MODE: READ_ONLY" in architecture
    assert "SCCACHE_ERROR_LOG:" in architecture
    assert "SCCACHE_ERROR_LOG: /tmp/sccache-pr-errors.log" in architecture
    assert "SCCACHE_LOG: sccache::server=debug" in architecture
    assert "runner.temp" not in architecture
    assert 'python3 automation/report_sccache_failures.py --log "$SCCACHE_ERROR_LOG"' in architecture
    assert "cargo_scope: ${{ steps.decide.outputs.cargo_scope }}" in workflow
    assert "report.get('scope') not in {'none', 'scoped', 'full'}" in workflow
    assert "not {'architecture-and-rust', 'verify'}.issubset(selected)" in workflow
    assert "f.write('cargo_scope=' + report['scope'] + '\\n')" in workflow
    assert "if [[ '${{ needs.impact.outputs.cargo_scope }}' == full ]]; then" in architecture
    assert "args+=(--delegate-workspace-unit-tests-to-verify)" in architecture
    assert 'python3 automation/run_scoped_cargo_feedback.py \\' in architecture
    assert "cache-mode: write" not in workflow
    assert "cargo test -p integration_harness --features e2e" not in architecture
    assert "  rust-contract-smokes:\n" not in workflow
    assert "cargo metadata --format-version 1 --no-deps --locked" not in verify
    assert "SCCACHE_GHA_RW_MODE: READ_ONLY" in verify
    assert "      RUSTC_WRAPPER: sccache\n" not in verify.split("    steps:\n", 1)[0]
    assert "uses: actions/cache@v4" not in verify
    assert "uses: actions/cache/save" not in verify
    assert verify.count("uses: actions/cache/restore@v4") == 1
    assert verify.count("run: cargo test --workspace --locked") == 1
    assert "cargo test --workspace --locked" not in architecture
    for step_name in (
        "Install native Rust test build dependencies",
        "Pin Rust for full workspace tests",
        "Reuse trusted main Rust compiler objects for full workspace tests",
        "Authorize full-scope Rust cache reads",
        "Restore shared Cargo dependency sources without a second writer",
        "Full workspace Rust unit tests (parallel with real E2E)",
    ):
        step = verify.split(f"      - name: {step_name}\n", 1)[1].split("\n      - name:", 1)[0]
        assert "        if: needs.impact.outputs.cargo_scope == 'full'" in step, step_name
        if step_name == "Full workspace Rust unit tests (parallel with real E2E)":
            assert "          RUSTC_WRAPPER: sccache\n" in step
    for step_name in ("Full-scope compiler cache statistics", "Classify full-scope Rust cache failures"):
        step = verify.split(f"      - name: {step_name}\n", 1)[1].split("\n      - name:", 1)[0]
        assert "        if: always() && needs.impact.outputs.cargo_scope == 'full'" in step, step_name
    assert "python3 automation/check_crawler_invariants.py" in fast
    assert "python3 automation/check_module_boundaries.py" not in architecture
    for phase, owner in (("contracts", verify), ("product", product), ("integration", verify)):
        invocation = f"bash automation/measure_ci_phase.sh {phase} bash automation/ci_fast_verify.sh {phase}"
        assert workflow.count(invocation) == 1, phase
        assert invocation in owner
        assert f"  {phase})" in fast
        assert f'"$PHASE" == "{phase}"' in legacy
    for block in (product, verify):
        assert "bash automation/run_truth_certification_gate.sh" not in block
        assert "python3 automation/smoke_manual_llm_queue.py" not in block
        assert "bash automation/ci_verify.sh" not in block

    # Deep acceptance is available explicitly, with real Rust-backed checks,
    # all 16 Truth fixtures and Temporal/manual E2E. It must never start
    # automatically when a PR is marked Ready or synchronized: exact-head
    # scoped suites already run in the required `ci` workflow.
    assert "    if: github.event_name == 'workflow_dispatch' && inputs.operation == 'comprehensive'\n" in deep
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
    assert "    if: ${{ !cancelled() && github.event_name == 'push' && github.ref == 'refs/heads/main' && needs.gate.result == 'success' }}" in prime
    assert 'BEFORE_SHA: ${{ github.event.before }}' in prime
    assert "changed_paths(os.environ['BEFORE_SHA'], os.environ['SOURCE_SHA'])" in prime
    assert "main_cache_prime_required(changed)" in prime
    assert "python3 automation/check_expert_core_activation_contract.py" in fast
    for paths, expected in (
        (["docs/ci-fast-path.md"], False),
        ([".github/workflows/ci.yml"], False),
        (["app/rust/crates/integration_harness/src/tests/manual_llm_temporal_tests.rs"], False),
        (["app/rust/crates/integration_harness/src/tests/manual_llm_temporal_tests.rs",
          "app/rust/crates/seo_domain/src/cms_revision.rs"], True),
        (["app/rust/crates/integration_harness/src/containers.rs"], True),
        (["app/rust/Cargo.lock"], True),
    ):
        assert main_cache_prime_required(paths) == expected, paths
    assert "MAIN_CACHE_PRIME=" in prime
    assert "SKIPPED no cached Rust compilation input changed" in prime
    assert prime.count("if: steps.cache_scope.outputs.needed == 'true'") >= 6
    assert "if: always() && steps.cache_scope.outputs.needed == 'true'" in prime
    assert "SCCACHE_GHA_RW_MODE: READ_WRITE" in prime
    assert 'CARGO_BUILD_JOBS: "3"' in prime
    assert "SCCACHE_ERROR_LOG: /tmp/sccache-main-errors.log" in prime
    assert "SCCACHE_LOG: sccache::server=debug" in prime
    assert "runner.temp" not in prime
    assert "SCCACHE_GHA_ENABLED: \"on\"" in prime
    assert "version: \"v0.16.0\"" in prime
    assert "python3 automation/ci_impact.py \\" in prime
    assert '--report automation/reports/main_cache_prime_impact.json' in prime
    assert 'python3 automation/run_main_cache_prime.py \\' in prime
    assert '--base-sha "$BEFORE_SHA"' in prime
    assert '--source-sha "$SOURCE_SHA"' in prime
    assert "cargo check --locked --package integration_harness" not in prime
    assert "cargo test --locked --package integration_harness --no-run" not in prime
    assert "cargo test -p integration_harness --features e2e --lib --no-run --locked" not in prime
    assert "cargo build --locked -p temporal_worker -p outbox_worker" not in prime
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
