#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "app" / "rust" / "services" / "cli_tools" / "src" / "main.rs"
REGISTRY = ROOT / "docs" / "V6_Support_Process_Registry.md"
OPS = ROOT / "docs" / "OPS_RUNTIME_RUNBOOK.md"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def main() -> int:
    failures: list[str] = []
    cli_files = [CLI]
    cli_dir = CLI.parent / "cli"
    if cli_dir.is_dir():
        cli_files.extend(cli_dir.glob("*.rs"))
    cli = "\n".join(read(f) for f in cli_files)
    registry = read(REGISTRY)
    ops = read(OPS)

    for needle in [
        "SeoReleaseRestoreGate",
        "seo_release_restore_gate(",
        "release_and_restore_gate",
        "\"automation/check_temporal_build_id_policy.py\"",
        "\"automation/check_seo_rollout_compat_contract.py\"",
        "\"automation/check_backup_restore_layout.py\"",
        "\"automation/ci_verify.sh\"",
        "\"automation/temporal_production_gate.sh\"",
        "\"infra/backups/restore_drill.sh\"",
        "\"run_ci_verify\": run_ci_verify",
        "\"run_temporal_gate\": run_temporal_gate",
        "\"run_restore_drill\": run_restore_drill",
        "write_report(root, report_json, &payload)",
    ]:
        if needle not in cli:
            failures.append(f"cli_tools missing `{needle}`")

    for needle in [
        "SeoReleaseRestoreGate",
        "--report-json",
        "release_and_restore_gate",
    ]:
        if needle not in registry:
            failures.append(f"support registry missing `{needle}`")
        if needle not in ops:
            failures.append(f"ops runbook missing `{needle}`")

    if failures:
        print("RELEASE_RESTORE_GATE_SURFACE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("RELEASE_RESTORE_GATE_SURFACE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
