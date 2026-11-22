#!/usr/bin/env python3
"""One-command native Rust feedback for local edits, without a Docker rebuild.

Run from anywhere: python3 automation/dev_rust_feedback.py [--check-only]
The default compares HEAD to the *working tree*, including untracked files;
--base HEAD~1 also covers committed edits in the last commit. It is NOT CI.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import time
from pathlib import Path

from cargo_feedback import run_cargo
from ci_impact import (
    ROOT,
    SCHEMA_VERSION,
    WORKSPACE,
    analyze,
    changed_worktree_paths,
    load_cargo_metadata,
    workspace_graph,
)


PACKAGE_NAME = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\Z")


def commands_for_report(report: dict, *, check_only: bool) -> list[list[str]]:
    if report.get("schema_version") != SCHEMA_VERSION or report.get("mandatory_full_ci_gate") is not True:
        raise ValueError("invalid impact report: full CI acceptance must remain mandatory")
    direct, affected = report.get("direct_crates"), report.get("affected_crates")
    if not isinstance(direct, list) or not isinstance(affected, list):
        raise ValueError("invalid impact package lists")
    if any(not isinstance(p, str) or not PACKAGE_NAME.fullmatch(p) for p in direct + affected):
        raise ValueError("unsafe Cargo package name in impact report")
    if not set(direct).issubset(affected):
        raise ValueError("affected closure omitted a directly modified crate")

    if report.get("analysis") == "full":
        # Global/unknown changes require the whole workspace. `cargo test`
        # already compiles its targets; a preceding full `cargo check` would
        # compile the dependency graph again for another profile.
        return [["cargo", "check", "--workspace", "--locked"]] if check_only else [
            ["cargo", "test", "--workspace", "--locked"]
        ]
    if report.get("analysis") not in {"scoped", "none"}:
        raise ValueError("unknown impact scope")
    if not affected:
        return []
    command = ["cargo", "check", "--locked"]
    for package in affected:
        command.extend(["--package", package])
    commands = [command]
    if not check_only and direct:
        tests = ["cargo", "test", "--locked"]
        for package in direct:
            tests.extend(["--package", package])
        commands.append(tests)
    return commands


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="HEAD", help="Base commit; default HEAD + local edits")
    parser.add_argument("--check-only", action="store_true", help="Run fmt and affected check, skip tests")
    args = parser.parse_args()

    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    # Validate the user-supplied base ref *before* asking Git to diff it.
    subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", "--end-of-options", f"{args.base}^{{commit}}"],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        check=True,
    )
    paths = changed_worktree_paths(args.base)
    metadata = load_cargo_metadata()
    owners, reverse = workspace_graph(metadata)
    report = analyze(paths, owners, reverse, base=args.base, head=head)
    report["local_worktree"] = True
    report["exact_source_sha"] = False  # HEAD does not cover uncommitted edits.
    report["deferred_external_suites"] = {
        name: "local feedback runs Cargo only; use exact-source CI for disposable Docker/provider integration"
        for name in report["selected_suites"]
    }
    report_path = Path(subprocess.check_output(
        ["git", "rev-parse", "--git-path", "alegria-dev-impact.json"],
        cwd=ROOT,
        text=True,
    ).strip())
    if not report_path.is_absolute():
        report_path = ROOT / report_path
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"DEV_IMPACT={report['analysis']} changed={len(paths)} affected={','.join(report['affected_crates']) or '-'}", flush=True)
    print(f"DEV_SELECTED_SUITES={','.join(report['selected_suites']) or '-'} (deferred to CI)", flush=True)
    print(f"DEV_SELECTED_IMAGES={','.join(report['selected_images']) or '-'} (not built locally)", flush=True)
    for reason in report["full_scope_reasons"]:
        print(f"  FULL_SCOPE: {reason}", flush=True)

    fmt_started = time.monotonic()
    subprocess.run(["cargo", "fmt", "--all", "--", "--check"], cwd=WORKSPACE, check=True)
    print(f"DEV_RUST_FMT_SECONDS={time.monotonic() - fmt_started:.3f}", flush=True)
    measurements = []
    for command in commands_for_report(report, check_only=args.check_only):
        print("DEV_RUST_STEP=" + " ".join(command), flush=True)
        measurements.append(run_cargo(command, cwd=WORKSPACE))
    report_path.with_name("alegria-dev-cargo-timings.json").write_text(
        json.dumps({"impact": report["analysis"], "measurements": measurements}, indent=2) + "\n",
        encoding="utf-8",
    )
    print("DEV_RUST_FEEDBACK=PASS (local only; exact-HEAD CI Gate and Windows acceptance still required)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
