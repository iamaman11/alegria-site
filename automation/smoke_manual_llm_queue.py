#!/usr/bin/env python3
"""Exercise the actual manual LLM inbox against a disposable PostgreSQL 18 container."""
from __future__ import annotations

import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    command = [
        "cargo",
        "test",
        "-p",
        "integration_harness",
        "--features",
        "e2e",
        "manual_llm_queue_persists_validates_replays_and_recovers",
    ]
    result = subprocess.run(
        command,
        cwd=ROOT / "app" / "rust",
        check=False,
    )
    print(
        "MANUAL_LLM_QUEUE_E2E: " + ("OK" if result.returncode == 0 else "FAILED"),
        flush=True,
    )
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
