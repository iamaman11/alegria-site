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
    assert source.count("    if: github.event_name == 'pull_request'") == len(required), (
        "every full owner job must run on PR only; main must not repeat heavy builds"
    )
    assert "        if: github.event_name == 'push'" in source
    assert "commits/{merge_sha}/pulls" in source
    assert "actions/runs?head_sha={source_sha}" in source
    assert "job[\"conclusion\"]" in source
    for phase in ("contracts", "product", "integration"):
        assert source.count(f"bash automation/ci_verify.sh {phase}") == 1
        assert f'"$PHASE" == "{phase}"' in shell
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
