#!/usr/bin/env python3
"""Small offline contract for fast PR CI and opt-in comprehensive CI.

The project is experimental: don't lock every test to a CI job forever.
Check actual gates and that expensive integration suites remain executable.
"""
from pathlib import Path
import ast
import re
import textwrap
import tomllib
from ci_impact import (
    CI_PR_OWNERS, E2E_ARCH_SHARDS, E2E_VERIFY_SHARDS, E2E_RUNNER_PARTITIONS,
    E2E_PG_SHARDS, E2E_OUTBOX_SHARDS, NON_WORKER_DELIVERY_INPUTS,
    SCHEMA_VERSION, main_cache_prime_required, worker_release_decision,
)
from report_sccache_failures import summarize
from report_ci_critical_path import COMPILER_STEPS, E2E_PATHS, LIVE_E2E_STEPS

ROOT = Path(__file__).resolve().parents[1]
OWNERS = (
    "architecture-and-rust", "rust-e2e-graph-harness", "rust-e2e-qdrant-harness",
    "rust-e2e-qdrant-temporal-harness",
    "rust-e2e-pg-app-harness", "rust-e2e-temporal-harness", "rust-e2e-truth-harness",
    "rust-e2e-pg-harness", "rust-e2e-outbox-harness", "rust-e2e-legacy-binaries",
    "rust-e2e-v3-binaries", "rust-e2e-qdrant-effects-binaries",
    "rust-graph-e2e", "rust-qdrant-e2e", "rust-qdrant-temporal-e2e",
    "rust-qdrant-temporal-legacy-e2e", "rust-pg-app-e2e",
    "rust-temporal-legacy-e2e", "rust-temporal-v3-e2e", "rust-pg-e2e",
    "rust-outbox-e2e", "rust-truth-e2e", "product-truth",
    "rust-security", "cloudflare-static-build",
    "stack-temporal-postgres18", "stack-storage-graph", "verify",
    "windows-operator-smoke",
)
assert set(OWNERS) == set(CI_PR_OWNERS) and len(OWNERS) == len(CI_PR_OWNERS)


def check() -> None:
    workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    toolchain = tomllib.loads((ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"]
    version = toolchain["channel"]
    # Cargo's test objects are keyed by rustc version. A moving `stable`
    # invalidated every trusted-main sccache object between CI #450 and #689.
    # Pin the actual compiler in both developer Cargo and every CI job; a
    # deliberate version bump requires a new accepted-main cache prime.
    assert re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version)
    assert toolchain["profile"] == "minimal" and "rustfmt" in toolchain["components"]
    rust_action = "dtolnay/rust-toolchain@df2a88e9fe4098bad09cb42c3dec7187f15861fc"
    action_lines = workflow.splitlines()
    action_indices = [i for i, line in enumerate(action_lines) if "dtolnay/rust-toolchain@" in line]
    assert len(action_indices) >= 10
    for index in action_indices:
        line = action_lines[index]
        assert rust_action in line
        indent = line[:len(line) - len(line.lstrip())]
        with_indent = indent + ("  " if line.strip().startswith("- uses:") else "")
        assert action_lines[index + 1] == with_indent + "with:"
        assert action_lines[index + 2] == with_indent + f'  toolchain: "{version}"'
    assert "dtolnay/rust-toolchain@stable" not in workflow
    fast = (ROOT / "automation/ci_fast_verify.sh").read_text(encoding="utf-8")
    v3_topology_guard = "python3 infra/temporal/check_v3_evaluation_topology.py"
    assert fast.count(v3_topology_guard) == 1
    history_transition_gate = "python3 automation/test_temporal_history_transition.py"
    assert fast.count(history_transition_gate) == 1
    assert fast.index(history_transition_gate) < fast.index("  product)")
    assert fast.index(v3_topology_guard) < fast.index("  product)")
    # All acceptance/seed CI jobs live in one workflow: no competing CI runs.
    assert not (ROOT / ".github/workflows/ci-deep.yml").exists()
    assert not (ROOT / ".github/workflows/rust-cache-prime.yml").exists()
    assert sorted(path.name for path in (ROOT / ".github/workflows").glob("*.yml")) == ["ci.yml"]
    deep = workflow.split("  comprehensive:\n", 1)[1].split("  rust-cache-prime:\n", 1)[0]
    prime = workflow.split("  rust-cache-prime:\n", 1)[1].split("  select-image:\n", 1)[0]
    release = workflow.split("  select-image:\n", 1)[1].split("  dependency-refresh:\n", 1)[0]
    refresh = workflow.split("  dependency-refresh:\n", 1)[1]
    legacy = (ROOT / "automation/ci_verify.sh").read_text(encoding="utf-8")
    assert legacy.count(v3_topology_guard) == 1
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
    impact = workflow.split("  impact:\n", 1)[1].split("  architecture-and-rust:\n", 1)[0]
    assert impact.index("id: decide") < impact.index(
        "Reject Rust format and SDK boundary violations before compilation"
    ) < impact.index("Preserve selected and not-applicable evidence")
    assert "if: steps.decide.outputs.cargo_scope != 'none'" in impact
    assert "(cd app/rust && cargo fmt --all -- --check)" in impact
    assert "python3 automation/check_rust_adapter_boundaries.py" in impact
    # Every costly owner needs impact=success, so these cheap failures stop
    # its compiler from consuming minutes before the first actionable error.
    for owner in OWNERS:
        section = workflow.split(f"  {owner}:\n", 1)[1]
        assert "needs: impact\n" in section or "needs: [impact," in section
    assert "Report measured PR compiler-to-E2E critical paths" in gate
    assert "python3 automation/report_ci_critical_path.py" in gate
    assert "CI_SOURCE_SHA: ${{ github.event.pull_request.head.sha }}" in gate
    assert "CI_CHECKOUT_SHA: ${{ github.sha }}" in gate
    assert "CI_RUN_ATTEMPT: ${{ github.run_attempt }}" in gate
    assert fast.count("python3 automation/test_report_ci_critical_path.py") == 1
    for _, (display_name, prerequisite_jobs) in E2E_PATHS.items():
        owner = display_name.partition(" (")[0]
        section = re.split(r"(?m)^  [a-z][a-z0-9_-]*:$",
                           workflow.split(f"  {owner}:\n", 1)[1], maxsplit=1)[0]
        assert f"    needs: [impact, {', '.join(prerequisite_jobs)}]" in section
    for owner, compiler_step in COMPILER_STEPS.items():
        section = re.split(r"(?m)^  [a-z][a-z0-9_-]*:$",
                           workflow.split(f"  {owner}:\n", 1)[1], maxsplit=1)[0]
        assert f"- name: {compiler_step}" in section
    for route, step in LIVE_E2E_STEPS.items():
        owner = E2E_PATHS[route][0].split(" (")[0]
        section = re.split(r"(?m)^  [a-z][a-z0-9_-]*:$",
                           workflow.split(f"  {owner}:\n", 1)[1], maxsplit=1)[0]
        assert f"- name: {step}" in section
    for owner in OWNERS:
        assert f"  {owner}:\n" in workflow, owner
        assert f"      - {owner}\n" in gate, owner
        assert f"contains(fromJSON(needs.impact.outputs.selected_ci_jobs), '{owner}')" in workflow, owner
    assert len(re.findall(r"(?m)^    if: github.event_name == 'pull_request' && needs\.impact\.result == 'success' && contains\(", workflow)) == len(OWNERS)
    # The real Outbox quarantine fence must be a required PR architecture
    # regression, not merely an opt-in comprehensive/deep check.
    architecture = workflow.split("  architecture-and-rust:\n", 1)[1].split("  rust-e2e-graph-harness:\n", 1)[0]
    assert "python3 automation/check_reliability_contracts.py" in architecture
    for inventory_gate in (
        "python3 automation/check_temporal_activity_owner_inventory.py",
        "python3 automation/test_temporal_activity_owner_inventory.py",
    ):
        # Both the architecture owner and the existing fast `verify` phase
        # check the exact legacy SDK registration inventory. No new CI job.
        assert architecture.count(inventory_gate) == 1
        assert fast.count(inventory_gate) == 1
    assert "python3 automation/test_causal_revision_contract.py" in architecture
    assert "python3 automation/test_neo4j_schema_contract.py" in architecture
    assert "python3 automation/test_operation_freshness_contract.py" in architecture

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
    architecture = workflow.split("  architecture-and-rust:\n", 1)[1].split("  rust-e2e-graph-harness:\n", 1)[0]
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
    for name in ("graph", "qdrant", "pg_app", "temporal_legacy", "temporal_v3", "truth"):
        assert f"{name}_e2e_shards: ${{{{ steps.decide.outputs.{name}_e2e_shards }}}}" in workflow
    assert "pg_e2e_shards: ${{ steps.decide.outputs.pg_e2e_shards }}" in workflow
    assert "report.get('scope') not in {'none', 'scoped', 'full'}" in workflow
    assert "not {'architecture-and-rust', 'verify'}.issubset(selected)" in workflow
    assert "f.write('cargo_scope=' + report['scope'] + '\\n')" in workflow
    assert "if: needs.impact.outputs.cargo_scope != 'full'" in architecture
    assert "--delegate-workspace-unit-tests-to-verify" not in architecture
    assert "run: python3 automation/run_scoped_cargo_feedback.py --report automation/reports/ci_impact.json" in architecture
    assert 'git fetch --depth=1 origin "refs/pull/${CI_PR_NUMBER}/head"' in architecture
    assert 'test "$(git rev-parse FETCH_HEAD)" = "${CI_SOURCE_SHA}"' in architecture
    def job_block(name: str, next_job: str) -> str:
        return workflow.split(f"  {name}:\n", 1)[1].split(f"  {next_job}:\n", 1)[0]

    compiler_owners = {
        "graph": "graph_harness",
        "qdrant": "qdrant_harness",
        "qdrant-temporal": "qdrant_temporal_harness",
        "pg-app": "pg_app_harness",
        "temporal": "temporal_harness",
    }
    for name, kind in compiler_owners.items():
        job = job_block(f"rust-e2e-{name}-harness", {
            "graph": "rust-e2e-qdrant-harness",
            "qdrant": "rust-e2e-qdrant-temporal-harness",
            "qdrant-temporal": "rust-e2e-pg-app-harness",
            "pg-app": "rust-e2e-temporal-harness",
            "temporal": "rust-e2e-truth-harness",
        }[name])
        assert f"--compile-owner-harness {kind}" in job
        assert f"--owner-harness-bundle-dir automation/reports/rust-ci-artifacts/{kind}" in job
        assert f"name: rust-e2e-{name}-harness-${{{{ github.sha }}}}" in job
        assert "ref: ${{ github.sha }}" in job
        assert "uses: actions/cache/restore@v4" in job
        assert "SCCACHE_GHA_RW_MODE: READ_ONLY" in job
        if name == "temporal":
            assert "CARGO_BUILD_JOBS:" not in job
            assert "--compile-owner-harness temporal_harness" in job
        assert "--compile-harness-only" not in job

    truth_compiler = job_block("rust-e2e-truth-harness", "rust-e2e-pg-harness")
    assert "--compile-harness-only" in truth_compiler
    assert "--harness-bundle-dir automation/reports/rust-ci-artifacts/harness" in truth_compiler
    assert "name: rust-e2e-truth-harness-${{ github.sha }}" in truth_compiler
    assert "contains(fromJSON(needs.impact.outputs.selected_ci_jobs), 'rust-e2e-truth-harness')" in truth_compiler
    pg_compiler = job_block("rust-e2e-pg-harness", "rust-e2e-outbox-harness")
    outbox_compiler = job_block("rust-e2e-outbox-harness", "rust-pg-physical-ab")
    assert "--compile-pg-harness-only" in pg_compiler
    assert "--pg-harness-bundle-dir automation/reports/rust-ci-artifacts/pg_harness" in pg_compiler
    assert "--compile-outbox-harness-only" in outbox_compiler
    assert "--outbox-harness-bundle-dir automation/reports/rust-ci-artifacts/outbox_harness" in outbox_compiler
    assert "name: rust-e2e-pg-harness-${{ github.sha }}" in pg_compiler
    assert "name: rust-e2e-outbox-harness-${{ github.sha }}" in outbox_compiler
    assert workflow.count("--compile-harness-only") == 1
    assert workflow.count("--compile-pg-harness-only") == 1
    assert workflow.count("--compile-outbox-harness-only") == 1
    assert workflow.count("--compile-owner-harness") == len(compiler_owners)

    optional_pg_ab = job_block("rust-pg-physical-ab", "rust-e2e-legacy-binaries")
    legacy_job = job_block("rust-e2e-legacy-binaries", "rust-e2e-v3-binaries")
    v3_job = job_block("rust-e2e-v3-binaries", "rust-e2e-qdrant-effects-binaries")
    qdrant_effects_job = job_block("rust-e2e-qdrant-effects-binaries", "rust-graph-e2e")
    assert "github.event_name == 'workflow_dispatch' && inputs.operation == 'physical-study'" in optional_pg_ab
    assert "continue-on-error: true" not in optional_pg_ab
    assert "CI_SOURCE_SHA: ${{ inputs.expected_source_sha }}" in optional_pg_ab
    assert "ref: ${{ inputs.expected_source_sha }}" in optional_pg_ab
    assert "fetch-depth: 0" in optional_pg_ab
    assert "const expected = process.env.EXPECTED_SOURCE_SHA" in optional_pg_ab
    assert "actual !== expected" in optional_pg_ab
    assert "head_sha: expected, event: 'pull_request'" in optional_pg_ab
    assert "fullRunMarkers.every(name => completedJobs.has(name))" in optional_pg_ab
    assert "'rust-qdrant-temporal-legacy-e2e (temporal_qdrant)'" in optional_pg_ab
    assert "'CI', 'impact', 'verify'" in optional_pg_ab
    assert "result.total_count !== result.jobs.length" in optional_pg_ab
    # GH Actions job.head_sha identifies the *trigger* head, not the source
    # checked out by actions/checkout on a synthetic PR merge. A comparison
    # to the same expected head commit is a tautology, never provenance proof.
    assert "impactJobs[0].head_sha" not in optional_pg_ab
    assert "checkoutCommit.tree.sha" not in optional_pg_ab
    assert "PHYSICAL_STUDY_GREEN_SOURCE=PASS" not in optional_pg_ab
    assert "PHYSICAL_STUDY_GREEN_PR_RUN=FOUND" in optional_pg_ab
    assert "historical_checkout_tree=UNVERIFIED" in optional_pg_ab
    assert "--compiler-owners > automation/reports/physical_owner_ab.json" in optional_pg_ab
    assert "--compiler > automation/reports/physical_coldwarm_ab.json" in optional_pg_ab
    assert "RUSTC_WRAPPER=sccache python3 automation/measure_owner_rebuild.py" in optional_pg_ab
    assert "python3 automation/summarize_owner_build_evidence.py" in optional_pg_ab
    assert '--study "$STUDY" --source-sha "$STUDY_SOURCE_SHA" --report "$report"' in optional_pg_ab
    assert "name: Publish exact four-owner metrics as auditable text" in optional_pg_ab
    assert "inputs.physical_study || 'shared'" in workflow.split("jobs:\n", 1)[0]
    assert "--output automation/reports/pg_v3_harness_physical_ab.json" in optional_pg_ab
    assert "--report automation/reports/pg_v3_harness_physical_ab.json" in optional_pg_ab
    assert "- legacy-worker-split" in workflow
    assert "python3 automation/measure_legacy_worker_split.py" in optional_pg_ab
    assert "--output automation/reports/legacy_worker_split_physical.json" in optional_pg_ab
    assert "automation/reports/legacy_worker_split_physical.json" in optional_pg_ab
    assert "expected_source_sha" in optional_pg_ab
    assert "if: always()" in optional_pg_ab
    assert "rust-pg-physical-ab" not in gate and "rust-pg-physical-ab" not in OWNERS
    for opt_in_probe in ("physical-ab-probe", "physical-ab-coldwarm", "owner-build-probe"):
        assert opt_in_probe not in architecture, "physical experiments must not stretch the required PR gate"
    assert "- physical-study" in workflow
    assert "expected_source_sha:" in workflow and "physical_study:" in workflow
    # The expensive default-vs-legacy rustc A/B remains a manual exact-green
    # SHA experiment in the ONE workflow, never a mandatory PR job.
    assert "- dag-parent-ab" in workflow
    assert "python3 automation/measure_generic_temporal_rebuild.py" in optional_pg_ab
    assert "automation/reports/generic_parent_rebuild_ab.json" in optional_pg_ab
    assert "cargo build --locked -p temporal_worker -p outbox_worker -p temporal_orchestrator" in legacy_job
    assert "--features temporal_orchestrator/legacy-qdrant-v1,temporal_orchestrator/legacy-workflows" in legacy_job
    assert "cargo build --locked --no-default-features -p temporal_orchestrator -p temporal_pg_effects" in v3_job
    # Reuse existing concurrent compiler owners: ONE Cargo invocation builds
    # canonical Worker/Outbox and historical Temporal with shared features;
    # generic DAG+PG runs separately without waiting for a second Cargo pass.
    assert legacy_job.count('cargo build --locked') == 1
    assert 'legacy-qdrant-v1,legacy-workflows' not in v3_job
    assert legacy_job.index('--features temporal_orchestrator/legacy-qdrant-v1,temporal_orchestrator/legacy-workflows') < legacy_job.index(
        '--kind legacy --source-sha "$GITHUB_SHA"'
    ) < legacy_job.index('--kind legacy_qdrant_parent --source-sha "$GITHUB_SHA"')
    assert "CI_TEMPORAL_GENERIC_REPLAY=PASS historical=refused" in v3_job
    assert "CI_TEMPORAL_LEGACY_REPLAY=PASS modes=pure-v3,v4,v5,qdrant-v1" in legacy_job
    assert 'name: rust-e2e-legacy-qdrant-parent-${{ github.sha }}' in legacy_job
    assert 'name: rust-e2e-legacy-qdrant-parent-${{ github.sha }}' not in v3_job
    for historical_replay in (
        "--replay-history", "--replay-effect-history", "--replay-v5-effect-history",
        "--replay-qdrant-effect-history",
    ):
        assert historical_replay in v3_job
        assert historical_replay in legacy_job
    assert 'grep -F \'requires legacy-\'' in v3_job
    assert "grep -F 'read actual Temporal history'" in legacy_job
    assert "cargo build --locked -p temporal_qdrant_effects --bin temporal_qdrant_effects" in qdrant_effects_job
    assert '--kind qdrant_effects --source-sha "$GITHUB_SHA"' in qdrant_effects_job
    for block in (truth_compiler, pg_compiler, outbox_compiler, legacy_job, v3_job, qdrant_effects_job):
        assert "uses: actions/cache/restore@v4" in block
        assert "SCCACHE_GHA_RW_MODE: READ_ONLY" in block
        assert "uses: actions/upload-artifact@v7" in block
        assert "cargo test --workspace --locked" not in block

    runner_names = (
        "rust-graph-e2e", "rust-qdrant-e2e", "rust-qdrant-temporal-e2e",
        "rust-qdrant-temporal-legacy-e2e", "rust-pg-app-e2e",
        "rust-temporal-legacy-e2e", "rust-temporal-v3-e2e",
        "rust-pg-e2e", "rust-outbox-e2e", "rust-truth-e2e",
    )
    runner_following = (*runner_names[1:], "product-truth")
    runner_blocks = {
        name: job_block(name, following)
        for name, following in zip(runner_names, runner_following)
    }
    assert set(runner_blocks) == set(E2E_RUNNER_PARTITIONS)
    required_bundles = {
        "rust-graph-e2e": ("rust-e2e-graph-harness", "graph_harness"),
        "rust-qdrant-e2e": ("rust-e2e-qdrant-harness", "qdrant_harness"),
        "rust-qdrant-temporal-e2e": ("rust-e2e-qdrant-temporal-harness", "qdrant_temporal_harness"),
        "rust-qdrant-temporal-legacy-e2e": ("rust-e2e-qdrant-temporal-harness", "qdrant_temporal_harness"),
        "rust-pg-app-e2e": ("rust-e2e-pg-app-harness", "pg_app_harness"),
        "rust-temporal-legacy-e2e": ("rust-e2e-temporal-harness", "temporal_harness"),
        "rust-temporal-v3-e2e": ("rust-e2e-temporal-harness", "temporal_harness"),
        "rust-pg-e2e": ("rust-e2e-pg-harness", "pg_harness"),
        "rust-outbox-e2e": ("rust-e2e-outbox-harness", "outbox_harness"),
        "rust-truth-e2e": ("rust-e2e-truth-harness", "harness"),
    }
    for owner, (compiler, kind) in required_bundles.items():
        runner = runner_blocks[owner]
        assert f"name: {owner} (${{{{ matrix.shard }}}})" in runner
        assert f"needs.{compiler}.result == 'success'" in runner
        assert f"name: {compiler}-${{{{ github.sha }}}}" in runner
        if owner == "rust-qdrant-temporal-legacy-e2e":
            assert f'for kind in {kind} qdrant_effects legacy_qdrant_parent; do' in runner
            assert '--kind "$kind" --source-sha "$GITHUB_SHA"' in runner
        else:
            assert f'--kind {kind} --source-sha "$GITHUB_SHA"' in runner
        assert "name: ci-owner-impact" in runner
        assert "--execute-precompiled-e2e --e2e-shard '${{ matrix.shard }}'" in runner
        assert "fail-fast: false" in runner
        assert "continue-on-error:" not in runner
        if owner not in {"rust-pg-e2e", "rust-outbox-e2e"}:
            assert "--suite-job auto" in runner
    for owner in ("rust-graph-e2e", "rust-qdrant-e2e", "rust-pg-app-e2e"):
        runner = runner_blocks[owner]
        assert "rust-e2e-legacy-" not in runner and "rust-e2e-v3-" not in runner
        assert "rust-e2e-truth-harness" not in runner
    for owner, worker in (
        ("rust-temporal-legacy-e2e", "legacy"),
        ("rust-outbox-e2e", "legacy"),
        ("rust-truth-e2e", "legacy"),
    ):
        runner = runner_blocks[owner]
        assert f'--kind {worker} --source-sha "$GITHUB_SHA"' in runner
        assert f"name: rust-e2e-{worker}-${{{{ github.sha }}}}" in runner
    assert '--kind v3 --source-sha "$GITHUB_SHA"' in runner_blocks["rust-pg-e2e"]
    for historical_runner in ("rust-temporal-v3-e2e", "rust-pg-e2e"):
        runner = runner_blocks[historical_runner]
        assert "needs.rust-e2e-legacy-binaries.result == 'success'" in runner
        assert 'name: rust-e2e-legacy-qdrant-parent-${{ github.sha }}' in runner
        assert '--kind legacy_qdrant_parent --source-sha "$GITHUB_SHA"' in runner
    pure_runner = runner_blocks["rust-temporal-v3-e2e"]
    assert 'name: rust-e2e-v3-${{ github.sha }}' not in pure_runner
    assert '--kind v3 --source-sha "$GITHUB_SHA"' not in pure_runner
    assert 'name: rust-e2e-v3-${{ github.sha }}' in runner_blocks["rust-pg-e2e"]
    qdrant_runner = runner_blocks["rust-qdrant-temporal-e2e"]
    assert (
        "needs: [impact, architecture-and-rust, rust-e2e-qdrant-temporal-harness, "
        "rust-e2e-qdrant-effects-binaries, rust-e2e-v3-binaries]"
    ) in qdrant_runner
    assert "needs.rust-e2e-qdrant-effects-binaries.result == 'success'" in qdrant_runner
    assert "needs.rust-e2e-v3-binaries.result == 'success'" in qdrant_runner
    assert "rust-e2e-legacy-binaries" not in qdrant_runner
    legacy_qdrant_runner = runner_blocks["rust-qdrant-temporal-legacy-e2e"]
    assert (
        "needs: [impact, architecture-and-rust, rust-e2e-qdrant-temporal-harness, "
        "rust-e2e-qdrant-effects-binaries, rust-e2e-legacy-binaries]"
    ) in legacy_qdrant_runner
    assert "needs.rust-e2e-legacy-binaries.result == 'success'" in legacy_qdrant_runner
    assert "needs.rust-e2e-v3-binaries" not in legacy_qdrant_runner
    assert "needs.rust-e2e-qdrant-effects-binaries.result == 'success'" in legacy_qdrant_runner
    # A Qdrant/Temporal runner used to fetch all seven independent artifact
    # bundles even when one live shard uses only two. GitHub artifact download
    # stalls then dominated the PR critical path. Every shard must continue
    # installing the exact source-bound binaries it actually executes.
    mandatory_shards = (
        "temporal_mixed_v6", "temporal_mixed_v7",
        "temporal_mixed_v2", "temporal_dataforseo", "temporal_acquisition",
        "temporal_seo_steps", "temporal_truth", "temporal_semantic",
    )
    assert E2E_RUNNER_PARTITIONS["rust-qdrant-temporal-e2e"] == mandatory_shards
    assert E2E_RUNNER_PARTITIONS["rust-qdrant-temporal-legacy-e2e"] == ("temporal_qdrant",)
    download_gates = {
        "qdrant-effects": 'contains(fromJSON(\'["temporal_mixed_v6","temporal_mixed_v7","temporal_mixed_v2"]\'), matrix.shard)',
        "dataforseo-effects": "matrix.shard == 'temporal_dataforseo'",
        "acquisition-pure": "matrix.shard == 'temporal_acquisition'",
        "one-step-request-issuer": "matrix.shard == 'temporal_acquisition'",
        "seo-steps-pure": 'contains(fromJSON(\'["temporal_seo_steps","temporal_acquisition"]\'), matrix.shard)',
        "truth-pure": 'contains(fromJSON(\'["temporal_truth","temporal_acquisition"]\'), matrix.shard)',
        "semantic-pure": "matrix.shard == 'temporal_semantic'",
    }
    assert (
        "- uses: actions/download-artifact@v7\n"
        "        with:\n"
        "          name: rust-e2e-v3-${{ github.sha }}"
    ) in qdrant_runner
    for artifact, condition in download_gates.items():
        assert (
            f"- uses: actions/download-artifact@v7\n"
            f"        if: {condition}\n"
            f"        with:\n"
            f"          name: rust-e2e-{artifact}-${{{{ github.sha }}}}"
        ) in qdrant_runner, artifact
    # The separately compiled DataForSEO provider artifact is published once
    # from an already-required parallel owner. No second Cargo pass may be
    # appended to the generic parent/PG binary chain on the slowest PR path.
    issuer_build = 'cargo build --locked -p temporal_dataforseo -p temporal_acquisition_pure -p temporal_seo_steps_pure -p temporal_truth_pure -p temporal_semantic_pure -p acquisition_source_issuer -p one_step_request_issuer --bins'
    assert issuer_build in architecture
    assert '--kind dataforseo_effects --source-sha "$GITHUB_SHA"' in architecture
    assert 'name: rust-e2e-dataforseo-effects-${{ github.sha }}' in architecture
    assert issuer_build not in v3_job
    assert '--kind dataforseo_effects --source-sha "$GITHUB_SHA"' not in v3_job
    assert 'name: rust-e2e-dataforseo-effects-${{ github.sha }}' not in v3_job
    assert 'needs.architecture-and-rust.result == \'success\'' in qdrant_runner
    assert '--kind acquisition_pure --source-sha "$GITHUB_SHA"' in architecture
    assert 'name: rust-e2e-acquisition-pure-${{ github.sha }}' in architecture
    for kind, artifact in (("seo_steps_pure", "seo-steps-pure"), ("truth_pure", "truth-pure"),
                           ("semantic_pure", "semantic-pure")):
        assert f'--kind {kind} --source-sha "$GITHUB_SHA"' in architecture
        assert f'name: rust-e2e-{artifact}-${{{{ github.sha }}}}' in architecture
    # Separate SQL identity boundary, separately compiled and digest-checked;
    # never smuggle grant issuance into a Temporal effect-owner binary.
    assert '--kind one_step_request_issuer --source-sha "$GITHUB_SHA"' in architecture
    assert 'name: rust-e2e-one-step-request-issuer-${{ github.sha }}' in architecture
    expected_installs = (
        "temporal_mixed_v6|temporal_mixed_v7|temporal_mixed_v2) kinds=(qdrant_effects v3)",
        "temporal_dataforseo) kinds=(v3 dataforseo_effects)",
        "temporal_acquisition) kinds=(v3 acquisition_pure one_step_request_issuer seo_steps_pure truth_pure)",
        "temporal_seo_steps) kinds=(v3 seo_steps_pure)",
        "temporal_truth) kinds=(v3 truth_pure)",
        "temporal_semantic) kinds=(v3 semantic_pure)",
    )
    for entry in expected_installs:
        assert entry in qdrant_runner, entry
    assert '*) echo "Unknown mandatory Temporal shard" >&2; exit 1' in qdrant_runner
    assert 'for kind in "${kinds[@]}"; do' in qdrant_runner
    assert '--kind "$kind" --source-sha "$GITHUB_SHA"' in qdrant_runner
    assert qdrant_runner.count('python3 automation/rust_ci_artifacts.py install') == 2
    for bundle in ("qdrant-temporal-harness", "qdrant-effects", "legacy-qdrant-parent"):
        assert f'name: rust-e2e-{bundle}-${{{{ github.sha }}}}' in legacy_qdrant_runner
    assert 'for kind in qdrant_temporal_harness qdrant_effects legacy_qdrant_parent; do' in legacy_qdrant_runner
    assert '--kind "$kind" --source-sha "$GITHUB_SHA"' in legacy_qdrant_runner
    assert 'sudo install -d -o root -g root -m 0755 /etc/alegria' in legacy_qdrant_runner
    assert "name: Prove live Temporal history and durable PostgreSQL/Qdrant receipt" in legacy_qdrant_runner
    assert 'name: rust-e2e-qdrant-effects-${{ github.sha }}' not in runner_blocks['rust-qdrant-e2e']
    assert "if: matrix.shard == 'pg_temporal'" in runner_blocks["rust-pg-e2e"]
    assert "if: matrix.shard == 'verify_d'" in runner_blocks["rust-pg-e2e"]
    # verify_d now executes a real native operator CLI: both PG shards must
    # authenticate and install the independently compiled PG V3 binary, never
    # silently run with only the test executable or rebuild on the runner.
    pg_runner = runner_blocks["rust-pg-e2e"]
    assert pg_runner.count("name: rust-e2e-v3-${{ github.sha }}") == 1
    assert pg_runner.count('--kind v3 --source-sha "$GITHUB_SHA"') == 1
    assert pg_runner.count("python3 automation/rust_ci_artifacts.py install") == 3
    assert "if: matrix.shard == 'pg_temporal'\n        with:\n          name: rust-e2e-v3-" not in pg_runner
    assert "E2E_RUNNER_PARTITIONS" in workflow
    for name in ("graph", "qdrant", "qdrant_temporal", "pg_app", "temporal_legacy", "temporal_v3", "pg", "outbox", "truth"):
        assert f"{name}_e2e_shards" in workflow
    assert len(E2E_ARCH_SHARDS) == 15 and len(E2E_VERIFY_SHARDS) == 9
    assert set(E2E_PG_SHARDS) == {"pg_temporal", "verify_d"}
    assert set(E2E_OUTBOX_SHARDS) == {"outbox"}
    assert set().union(*(set(shards) for shards in E2E_RUNNER_PARTITIONS.values())) == set(E2E_ARCH_SHARDS) | set(E2E_VERIFY_SHARDS)
    assert "if any(not routes[owner] for owner in routes if owner != 'rust-truth-e2e')" in workflow
    assert "truth_required = 'rust-truth-e2e' in selected" in workflow
    assert "if truth_required != bool(routes['rust-truth-e2e']):" in workflow
    assert "CI_IMPACT=FAIL conditional Truth certification owner mismatch" in workflow
    assert 'if "truth_certification" not in suites:' in (ROOT / "automation/ci_impact.py").read_text(encoding="utf-8")
    assert '"rust-e2e-truth-harness"' in gate
    assert '"rust-e2e-qdrant-harness"' in gate
    assert '"rust-e2e-qdrant-temporal-harness"' in gate
    assert '"rust-e2e-qdrant-effects-binaries"' in gate
    assert 'matrix_owners = {owner: set(shards) for owner, shards in E2E_RUNNER_PARTITIONS.items()}' in gate
    assert "did not pass all selected E2E shards" in gate
    assert "      - name: Checkout exact CI plan owner registry\n" in gate
    checkout = gate.split("      - name: Checkout exact CI plan owner registry\n", 1)[1].split("      - name:", 1)[0]
    assert "uses: actions/checkout@v7" in checkout and "ref: ${{ github.sha }}" in checkout
    assert "if: github.event_name == 'pull_request'" not in checkout
    assert "--execute-precompiled-e2e" not in architecture
    assert "--execute-precompiled-e2e" not in verify
    # Architecture's real Temporal and graph container suites must not compete
    # on the same runner: CI #492 proved no speed gain and a 40s startup delay.
    assert "--parallel-architecture-e2e" not in architecture
    assert 'run: python3 automation/run_scoped_cargo_feedback.py --report automation/reports/ci_impact.json' in architecture
    assert "cache-mode: write" not in workflow
    assert "cargo test -p integration_harness --features e2e" not in architecture
    assert "  rust-contract-smokes:\n" not in workflow
    assert "cargo metadata --format-version 1 --no-deps --locked" not in verify
    assert "SCCACHE_GHA_RW_MODE: READ_ONLY" in verify
    assert "      RUSTC_WRAPPER: sccache\n" not in verify.split("    steps:\n", 1)[0]
    assert "uses: actions/cache@v4" not in verify
    assert "uses: actions/cache/save" not in verify
    assert verify.count("uses: actions/cache/restore@v4") == 1
    assert verify.count("cargo test --workspace --locked") == 1
    assert "cargo test --workspace --locked" not in architecture
    for step_name in ("Fast source contracts and policy", "Fast integration and consistency checks"):
        step = verify.split(f"      - name: {step_name}\n", 1)[1].split("\n      - name:", 1)[0]
        assert "if: needs.impact.outputs.cargo_scope != 'full'" in step
    for step_name in (
        "Install native Rust test build dependencies",
        "Pin Rust for full workspace tests",
        "Reuse trusted main Rust compiler objects for full workspace tests",
        "Authorize full-scope Rust cache reads",
        "Restore shared Cargo dependency sources without a second writer",
        "Full workspace Rust tests and both mandatory policy phases",
    ):
        step = verify.split(f"      - name: {step_name}\n", 1)[1].split("\n      - name:", 1)[0]
        assert "        if: needs.impact.outputs.cargo_scope == 'full'" in step, step_name
        if step_name == "Full workspace Rust tests and both mandatory policy phases":
            assert "          RUSTC_WRAPPER: sccache\n" in step
            assert "bash automation/measure_ci_phase.sh contracts bash automation/ci_fast_verify.sh contracts" in step
            assert "bash automation/measure_ci_phase.sh integration bash automation/ci_fast_verify.sh integration" in step
            assert 'wait "$policy_pid"' in step and 'wait "$rust_pid"' in step
            assert 'if [[ "$policy_result" -ne 0 || "$rust_result" -ne 0 ]]' in step
    for step_name in ("Full-scope compiler cache statistics", "Classify full-scope Rust cache failures"):
        step = verify.split(f"      - name: {step_name}\n", 1)[1].split("\n      - name:", 1)[0]
        assert "        if: always() && needs.impact.outputs.cargo_scope == 'full' && steps.verify_sccache.outcome == 'success'" in step, step_name
    assert "python3 automation/check_crawler_invariants.py" in fast
    assert "python3 automation/check_module_boundaries.py" not in architecture
    for phase, owner in (("contracts", verify), ("product", product), ("integration", verify)):
        invocation = f"bash automation/measure_ci_phase.sh {phase} bash automation/ci_fast_verify.sh {phase}"
        # Full-scope source policies overlap the Rust workspace compilation;
        # narrow scopes retain their own source-only steps. Both paths execute
        # the *same* phase once, never skip it.
        assert workflow.count(invocation) == (1 if phase == "product" else 2), phase
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
