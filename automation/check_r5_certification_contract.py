#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
RUNNER = ROOT / "automation" / "e2e_supersite_certification.py"
NIGHTLY = ROOT / "automation" / "nightly_r5_production_gate.sh"
RESTORE = ROOT / "infra" / "backups" / "restore_drill.sh"
GATE = ROOT / "automation" / "temporal_production_gate.sh"
PLAN = ROOT / "docs" / "SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md"


def main() -> int:
    failures: list[str] = []
    for path in [RUNNER, NIGHTLY, RESTORE, GATE]:
        if not path.exists():
            failures.append(f"missing file: {path.relative_to(ROOT)}")
        elif path.suffix in {".sh", ".py"} and path.name != "nightly_r5_production_gate.sh":
            if path.suffix == ".sh" and not path.stat().st_mode & 0o111:
                failures.append(f"script is not executable: {path.relative_to(ROOT)}")

    runner = RUNNER.read_text(encoding="utf-8")
    shell_runner = (ROOT / "automation" / "e2e_supersite_certification.sh").read_text(encoding="utf-8")
    nightly = NIGHTLY.read_text(encoding="utf-8")
    plan = PLAN.read_text(encoding="utf-8")

    for needle in [
        "temporal_production_gate.sh",
        "restore_drill.sh",
        "fresh_scope",
        "scope_expansion",
        "factual_change_rebuild",
        "docs/runs",
        "PENDING_CREDENTIALS",
    ]:
        if needle not in runner:
            failures.append(f"runner missing `{needle}`")

    for needle in ["python3 automation/e2e_supersite_certification.py"]:
        if needle not in shell_runner:
            failures.append(f"shell runner missing `{needle}`")

    for needle in [
        "e2e_supersite_certification.sh",
    ]:
        if needle not in nightly:
            failures.append(f"nightly wrapper missing `{needle}`")

    for needle in [
        "automation/e2e_supersite_certification.sh",
        "nightly production gate",
        "three live certification scenarios",
        "immutable evidence",
    ]:
        if needle not in plan:
            failures.append(f"plan missing `{needle}`")

    if failures:
        print("R5_CERTIFICATION_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("R5_CERTIFICATION_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
