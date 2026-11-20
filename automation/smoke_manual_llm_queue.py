#!/usr/bin/env python3
"""Exercise the actual manual LLM inbox against a disposable PostgreSQL 18 container."""
from __future__ import annotations

import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RUST_ROOT = ROOT / "app" / "rust"


def prepare_worker_binaries() -> tuple[dict[str, str], int]:
    """Build real Workers before cargo test holds the shared target lock.

    The harness itself may not invoke Cargo against its parent's target dir:
    that would deadlock. Its existing binary overrides let this driver reuse
    the already cached workspace target and keep runtime process isolation.
    """
    env = os.environ.copy()
    temporal_key = "INTEGRATION_TEMPORAL_WORKER_BIN"
    outbox_key = "INTEGRATION_OUTBOX_WORKER_BIN"
    if env.get(temporal_key) and env.get(outbox_key):
        print("CI_SHARED_WORKERS_BUILD source=provided", flush=True)
        return env, 0

    target = Path(env.get("CARGO_TARGET_DIR", "target"))
    if not target.is_absolute():
        target = RUST_ROOT / target
    binary_dir = target.resolve() / "debug"
    suffix = ".exe" if os.name == "nt" else ""
    binaries = {
        temporal_key: binary_dir / f"temporal_worker{suffix}",
        outbox_key: binary_dir / f"outbox_worker{suffix}",
    }
    command = [
        "cargo", "build", "--locked", "-p", "temporal_worker", "-p", "outbox_worker",
        "--bin", "temporal_worker", "--bin", "outbox_worker",
    ]
    started = time.perf_counter_ns()
    result = subprocess.run(command, cwd=RUST_ROOT, check=False)
    print(
        f"CI_SHARED_WORKERS_BUILD elapsed_ms={(time.perf_counter_ns() - started) // 1_000_000} exit_code={result.returncode}",
        flush=True,
    )
    if result.returncode:
        return env, result.returncode
    for key, path in binaries.items():
        if not path.is_file():
            print(f"CI_SHARED_WORKERS_BUILD missing_binary={path.name}", file=sys.stderr, flush=True)
            return env, 1
        env.setdefault(key, str(path))
    return env, 0


def main() -> int:
    # Real disposable PostgreSQL state transitions and negative submission cases.
    # Then exercise the actual canonical Temporal workflow, not a mock runner.
    worker_env, worker_status = prepare_worker_binaries()
    if worker_status:
        return worker_status
    for test_name in (
        "manual_llm_queue_persists_validates_replays_and_recovers",
        "manual_truth_inbox_refuses_fabricated_evidence_and_replays_atomically",
        "canonical_temporal_manual_editorial_resumes_after_worker_restart",
    ):
        command = [
            "cargo", "test", "-p", "integration_harness", "--features", "e2e", test_name,
        ]
        if test_name == "canonical_temporal_manual_editorial_resumes_after_worker_restart":
            # Show bounded stage timings without hiding the real Temporal run.
            command.extend(["--", "--nocapture"])
        started = time.perf_counter_ns()
        result = subprocess.run(
            command,
            cwd=RUST_ROOT,
            check=False,
            env=worker_env,
        )
        print(
            f"MANUAL_LLM_E2E_TIME test={test_name} elapsed_ms={(time.perf_counter_ns() - started) // 1_000_000} exit_code={result.returncode}",
            flush=True,
        )
        if result.returncode:
            print(f"MANUAL_LLM_E2E: FAILED test={test_name}", flush=True)
            return result.returncode

    print("MANUAL_LLM_E2E: OK", flush=True)
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
