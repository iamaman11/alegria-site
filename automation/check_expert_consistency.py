#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


@dataclass
class Finding:
    level: str
    code: str
    message: str


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def run_check(cmd: list[str], cwd: Path) -> tuple[bool, str]:
    p = subprocess.run(cmd, cwd=str(cwd), check=False, text=True, capture_output=True)
    out = (p.stdout or "") + (p.stderr or "")
    return p.returncode == 0, out.strip()


def has_req(requirements_text: str, package_prefix: str) -> bool:
    for raw_line in requirements_text.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line.lower().startswith(package_prefix.lower()):
            return True
    return False


def resolve_run_file(filename: str) -> Path:
    """Resolve run artifact in flat docs layout first, legacy nested layout second."""
    flat = ROOT / "docs" / f"run_gemini3_global_186_20260320_top10__{filename}"
    if flat.exists():
        return flat
    legacy = ROOT / "docs" / "run_gemini3_global_186_20260320_top10" / filename
    return legacy


def main() -> int:
    parser = argparse.ArgumentParser(description="Expert consistency checks for Alegria automation/runtime.")
    parser.add_argument("--run-boundary", action="store_true", help="Run automation/check_python_rust_boundary.sh")
    parser.add_argument("--run-invariants", action="store_true", help="Run automation/check_end_to_end_invariants.py")
    parser.add_argument("--run-rust-adapters", action="store_true", help="Run automation/check_rust_adapter_boundaries.py")
    parser.add_argument("--run-json-boundary", action="store_true", help="Run automation/check_json_boundary_policy.py")
    parser.add_argument("--report-json", type=str, default="automation/reports/consistency_report.json")
    args = parser.parse_args()

    findings: list[Finding] = []

    required_files = [
        ROOT / "automation" / "ci_verify.sh",
        ROOT / "automation" / "contract_verify.sh",
        ROOT / "automation" / "check_python_rust_boundary.sh",
        ROOT / "automation" / "check_rust_adapter_boundaries.py",
        ROOT / "automation" / "check_rust_layer_isolation.py",
        ROOT / "automation" / "check_json_boundary_policy.py",
        ROOT / "app" / "rust" / "Cargo.toml",
        ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "mod.rs",
        ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_adapter.rs",
        ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "neo4rs_adapter.rs",
        ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "qdrant_client_adapter.rs",
        ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "tonic_adapter.rs",
        resolve_run_file("results.jsonl"),
        resolve_run_file("enrichment_resolve.jsonl"),
        resolve_run_file("enrichment_resolve_rendered.jsonl"),
    ]

    for path in required_files:
        if not path.exists():
            findings.append(Finding("error", "F001", f"missing required file: {path}"))

    cargo_path = ROOT / "app" / "rust" / "Cargo.toml"
    reqs_path = ROOT / "requirements.txt"
    cargo_text = read_text(cargo_path) if cargo_path.exists() else ""
    reqs_text = read_text(reqs_path) if reqs_path.exists() else ""

    if cargo_text:
        if "[workspace]" not in cargo_text:
            findings.append(Finding("error", "R001", "app/rust/Cargo.toml is not a Rust workspace"))
        for dep in ["sqlx", "neo4rs", "qdrant-client", "reqwest", "tonic"]:
            if not re.search(rf"^{re.escape(dep)}\s*=", cargo_text, re.MULTILINE):
                findings.append(Finding("warn", "R002", f"workspace dependency not found: {dep}"))

    # Root Python runtime should stay minimal in Rust-first mode.
    forbidden_runtime = [
        "psycopg",
        "psycopg-pool",
        "voyageai",
        "qdrant-client",
        "neo4j",
        "temporalio",
    ]
    for pkg in forbidden_runtime:
        if has_req(reqs_text, pkg):
            findings.append(Finding("error", "D001", f"forbidden runtime dependency in requirements.txt: {pkg}"))

    command_results: list[dict[str, str | bool]] = []

    if args.run_boundary:
        ok, out = run_check(["bash", "automation/check_python_rust_boundary.sh"], ROOT)
        command_results.append({"cmd": "check_python_rust_boundary.sh", "ok": ok, "output": out})
        if not ok:
            findings.append(Finding("error", "C001", "boundary check failed"))

    if args.run_invariants:
        ok, out = run_check(["python3", "automation/check_end_to_end_invariants.py"], ROOT)
        command_results.append({"cmd": "check_end_to_end_invariants.py", "ok": ok, "output": out})
        if not ok:
            findings.append(Finding("error", "C002", "invariants check failed"))

    if args.run_rust_adapters:
        ok, out = run_check(["python3", "automation/check_rust_adapter_boundaries.py"], ROOT)
        command_results.append({"cmd": "check_rust_adapter_boundaries.py", "ok": ok, "output": out})
        if not ok:
            findings.append(Finding("error", "C003", "rust adapter boundary check failed"))

    if args.run_json_boundary:
        ok, out = run_check(["python3", "automation/check_json_boundary_policy.py"], ROOT)
        command_results.append({"cmd": "check_json_boundary_policy.py", "ok": ok, "output": out})
        if not ok:
            findings.append(Finding("error", "C004", "json boundary policy check failed"))

    status = "failed" if any(f.level == "error" for f in findings) else "ok"
    report = {
        "status": status,
        "project_root": str(ROOT),
        "checks_run": {
            "run_boundary": args.run_boundary,
            "run_invariants": args.run_invariants,
            "run_rust_adapters": args.run_rust_adapters,
            "run_json_boundary": args.run_json_boundary,
        },
        "findings": [f.__dict__ for f in findings],
        "commands": command_results,
    }

    report_path = ROOT / args.report_json
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")

    print(f"CONSISTENCY: {status.upper()}")
    print(f"report: {report_path}")
    for f in findings:
        print(f"[{f.level}] {f.code}: {f.message}")
    for c in command_results:
        print(f"command {c['cmd']}: {'OK' if c['ok'] else 'FAIL'}")

    return 0 if status == "ok" else 1


if __name__ == "__main__":
    sys.exit(main())
