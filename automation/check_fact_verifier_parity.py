#!/usr/bin/env python3
from __future__ import annotations

import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    report_path = ROOT / "automation" / "reports" / "fact_verifier_parity.rust.json"
    report_path.parent.mkdir(parents=True, exist_ok=True)

    cmd = [
        "cargo",
        "run",
        "-q",
        "--manifest-path",
        "app/rust/services/cli_tools/Cargo.toml",
        "--",
        "check-fact-verifier-parity",
        "--root",
        ".",
        "--report-json",
        str(report_path),
    ]

    p = subprocess.run(cmd, cwd=str(ROOT), check=False)
    if p.returncode != 0:
        print("FACT_VERIFIER_PARITY: FAILED")
        print(f"report: {report_path}")
        return p.returncode

    print("FACT_VERIFIER_PARITY: OK")
    print(f"report: {report_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
