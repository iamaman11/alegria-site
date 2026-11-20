#!/usr/bin/env python3
"""Check single CI ownership and fail-closed aggregate, without a second test catalog."""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]


def check() -> None:
    wf = ROOT / ".github/workflows/ci.yml"
    script = ROOT / "automation/ci_verify.sh"
    deploy = ROOT / "infra/local/windows_staging.ps1"
    assert wf.is_file(), "single ci.yml must exist"
    assert not (wf.parent / "quality-gate.yml").exists(), "duplicate Quality Gate"
    assert not (wf.parent / "automation-verify.yml").exists(), "duplicate Automation Verify"
    source = wf.read_text(encoding="utf-8")
    shell = script.read_text(encoding="utf-8")
    stage = deploy.read_text(encoding="utf-8")
    assert source.startswith("name: ci\n")
    assert "  pull_request:\n" in source
    assert "  push:\n    branches: [main]\n" in source
    assert "  gate:\n    name: CI\n    if: always()\n" in source
    assert "toJSON(needs)" in source and 'result != "success"' in source
    required = (
        "architecture-and-rust", "rust-security", "cloudflare-static-build",
        "temporal-worker-docker", "stack-temporal-postgres18",
        "stack-storage-graph", "verify", "windows-operator-smoke",
    )
    gate = source.split("  gate:\n", 1)[1]
    for job in required:
        assert f"      - {job}\n" in gate, f"not included in final CI gate: {job}"
    assert len(re.findall(r"(?m)^    if: github.event_name == 'pull_request'$", source)) == len(required), (
        "every full owner job must run on PR only; main must not repeat heavy builds"
    )
    worker = source.split("  temporal-worker-docker:\n", 1)[1].split("  stack-temporal-postgres18:\n", 1)[0]
    assert "ref: ${{ github.event.pull_request.head.sha }}" in worker, (
        "Docker image source must be the exact tested PR HEAD, never the synthetic merge"
    )
    assert "name: temporal-worker-image-${{ github.event.pull_request.head.sha }}" in worker
    assert "uses: actions/attest@v4" in worker
    assert "subject-path: ${{ runner.temp }}/temporal-worker-artifact/temporal-worker.tar.gz" in worker
    assert "id-token: write" in worker and "attestations: write" in worker
    assert worker.index("uses: actions/attest@v4") < worker.index("uses: actions/upload-artifact@v7")
    assert "        if: github.event_name == 'push'" in source
    assert "commits/{merge_sha}/pulls" in source
    assert "actions/runs?head_sha={source_sha}" in source
    assert "job[\"conclusion\"]" in source
    for phase in ("contracts", "product", "integration"):
        assert source.count(f"bash automation/ci_verify.sh {phase}") == 1
        assert f'"$PHASE" == "{phase}"' in shell
        assert source.count(f"bash automation/measure_ci_phase.sh {phase} ") == 1, (
            f"mandatory {phase} must produce an independent timing and preserve its exit code"
        )
    architecture = source.split("  architecture-and-rust:\n", 1)[1].split("  rust-security:\n", 1)[0]
    verify = source.split("  verify:\n", 1)[1].split("  windows-operator-smoke:\n", 1)[0]
    assert "bash automation/ci_verify.sh product" in architecture
    assert "bash automation/ci_verify.sh product" not in verify
    assert "bash automation/ci_verify.sh contracts" in verify
    assert "bash automation/ci_verify.sh integration" in verify
    # These owner jobs execute concurrently on separate ephemeral GitHub VMs,
    # each with its own database. They must not share a database/port or use
    # one job's mutable state as a hidden input to the other.
    for owner in (architecture, verify):
        assert "runs-on: ubuntu-latest" in owner
        assert "    services:\n      postgres:" in owner
        assert "DATABASE_URL: postgres://alegria_ci:alegria_ci_only@127.0.0.1:5432/alegria_ci" in owner
        assert "    needs:" not in owner
    for owner in (architecture, verify):
        assert "CARGO_INCREMENTAL: \"0\"" in owner
        assert "RUSTC_WRAPPER: sccache" in owner
        assert "SCCACHE_GHA_ENABLED: \"true\"" in owner
        assert "uses: mozilla-actions/sccache-action@v0.0.11" in owner
        assert "sccache --show-stats" in owner
    assert "name: ci-phase-product" in architecture
    assert "name: ci-phase-verify" in verify
    assert "bash automation/audit.sh fast" not in source
    assert "bash automation/audit.sh full" not in source
    assert "foreach ($name in @('ci'))" in stage
    assert "foreach ($name in @('quality-gate', 'automation-verify'))" not in stage
    # An executable Python check must have exactly one Linux owner. The
    # Windows job runs only native Windows contract/smoke checks.
    checks = re.findall(r"(?m)^python3 (automation/[\w/.-]+\.py)", shell)
    checks += re.findall(r"(?m)^          python3 (automation/[\w/.-]+\.py)", source)
    assert len(checks) == len(set(checks)), "repeated Linux Python script: " + repr(
        sorted({x for x in checks if checks.count(x) > 1})
    )
    assert "python automation/check_windows_staging_rollback.py" in source
    assert "pwsh -NoProfile -NonInteractive -File automation/smoke_windows_staging_reconcile.ps1" in source
    assert "python automation/smoke_e2e_first_site.py" not in source

if __name__ == "__main__":
    check()
    print("SINGLE_CI_CONTRACT=PASS")
