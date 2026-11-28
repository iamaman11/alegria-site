"""Report the real Rust workspace artifacts compiled by a scoped Cargo command.

Cargo JSON is consumed as a stream so diagnostics and test output remain visible.
This only observes compilation: it never chooses tests or changes acceptance.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Mapping, TextIO


def workspace_artifact(record: dict, workspace: Path) -> str | None:
    if record.get("reason") != "compiler-artifact":
        return None
    target = record.get("target")
    if not isinstance(target, dict) or not isinstance(target.get("src_path"), str):
        return None
    if not isinstance(target.get("name"), str):
        return None
    source = Path(target["src_path"]).resolve()
    if not source.is_relative_to(workspace.resolve()):
        return None
    return target["name"]


def run_cargo(
    command: list[str], *, cwd: Path, output: TextIO = sys.stdout,
    env: Mapping[str, str] | None = None,
) -> dict:
    if len(command) < 2 or command[0] != "cargo" or (
        command[1] not in {"check", "test"}
        and not (command[1] == "build" and "--locked" in command and "--release" not in command)
    ):
        raise ValueError("only scoped Cargo check/test or locked debug build may be instrumented")

    traced = [*command[:2], "--message-format=json-render-diagnostics", *command[2:]]
    started = time.monotonic()
    rebuilt: set[str] = set()
    fresh: set[str] = set()
    test_executables: list[dict[str, str]] = []
    compiler_seconds: float | None = None
    process = subprocess.Popen(
        traced, cwd=cwd, stdout=subprocess.PIPE, stderr=None,
        text=True, errors="replace", bufsize=1,
        env={**os.environ, **env} if env is not None else None,
    )
    assert process.stdout is not None
    for line in process.stdout:
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            output.write(line)
            continue
        if not isinstance(record, dict) or not isinstance(record.get("reason"), str):
            output.write(line)
            continue
        artifact = workspace_artifact(record, cwd)
        if artifact is not None:
            (fresh if record.get("fresh") is True else rebuilt).add(artifact)
            executable = record.get("executable")
            target = record.get("target")
            if (isinstance(executable, str) and executable
                    and isinstance(target, dict)
                    and isinstance(target.get("kind"), list)
                    and "lib" in target["kind"]):
                test_executables.append({"target": artifact, "path": executable})
        elif record["reason"] == "compiler-message":
            message = record.get("message")
            rendered = message.get("rendered") if isinstance(message, dict) else None
            if isinstance(rendered, str):
                output.write(rendered)
        elif record["reason"] == "build-finished":
            compiler_seconds = time.monotonic() - started

    exit_code = process.wait()
    total = time.monotonic() - started
    seen = bool(rebuilt or fresh)
    result = {
        "command": command,
        "exit_code": exit_code,
        "elapsed_seconds": round(total, 3),
        "compiler_seconds": round(compiler_seconds, 3) if compiler_seconds is not None else None,
        "post_compiler_seconds": (
            round(max(total - compiler_seconds, 0), 3)
            if compiler_seconds is not None else None
        ),
        "rebuilt_workspace_targets": sorted(rebuilt),
        "fresh_workspace_targets": sorted(fresh - rebuilt),
        "test_executables": test_executables,
        "artifact_evidence": "observed" if seen else "unavailable",
    }
    output.write("CARGO_FEEDBACK_JSON=" + json.dumps(result, sort_keys=True) + "\n")
    output.flush()
    if exit_code:
        raise subprocess.CalledProcessError(exit_code, traced)
    return result
