#!/usr/bin/env python3
"""Publish precise four-owner Cargo evidence from opt-in GitHub Actions artifacts.

The GitHub REST artifact endpoint returns a ZIP that many read-only consumers
cannot inspect. The job therefore also emits a small, exact-SHA JSON summary
into its *text logs* and GitHub step summary. This never treats a successful
compiler invocation as proof of production runtime isolation or CI <=180s.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import sys
from typing import Any


OWNERS = frozenset({
    "postgres_store", "graph_neo4j", "vector_qdrant", "temporal_orchestrator",
})
STUDIES = {
    "compiler-owners": ("FOUR_OWNER_ACTUAL_RUSTC_AB_CAPTURED", "four_owner_probes"),
    "owner-rebuild": ("EVIDENCE_CAPTURED", "probes"),
}


def require_number(value: Any, context: str) -> float:
    if type(value) not in (int, float) or not 0 <= value < float("inf"):
        raise ValueError(f"{context}: missing, non-finite or negative elapsed seconds")
    return round(float(value), 3)


def require_names(value: Any, context: str) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(name, str) for name in value):
        raise ValueError(f"{context}: expected exact list of binary names")
    if len(value) != len(set(value)):
        raise ValueError(f"{context}: duplicate binary names")
    return sorted(value)


def summarize(report: dict[str, Any], study: str, expected_sha: str) -> dict[str, Any]:
    if study not in STUDIES:
        raise ValueError(f"unsupported owner study: {study}")
    status, owner_field = STUDIES[study]
    if not re.fullmatch(r"[a-f0-9]{40}", expected_sha):
        raise ValueError("expected SHA must be an exact 40-character commit")
    if report.get("status") != status or report.get("source_sha") != expected_sha:
        raise ValueError("physical evidence status or source SHA does not match accepted input")
    if study == "compiler-owners" and report.get("actual_rustc_evidence") is not True:
        raise ValueError("compiler-owner study did not capture actual Rust compiler evidence")
    entries = (report.get("compiler", {}).get(owner_field) if study == "compiler-owners"
               else report.get(owner_field))
    if not isinstance(entries, list) or len(entries) != len(OWNERS):
        raise ValueError("physical evidence requires exactly four owner probes")
    result = []
    for entry in entries:
        if not isinstance(entry, dict) or entry.get("owner") not in OWNERS:
            raise ValueError("physical evidence contains an unknown owner")
        build = entry if study == "compiler-owners" else entry.get("build")
        if not isinstance(build, dict):
            raise ValueError("physical evidence is missing an owner build")
        elapsed = require_number(build.get("elapsed_seconds"), entry["owner"])
        relinked = require_names(
            entry.get("cargo_non_fresh_bin_targets") if study == "compiler-owners"
            else entry.get("relinked_binaries"), entry["owner"],
        )
        written = require_names(
            entry.get("binary_files_written"), entry["owner"],
        )
        result.append({
            "owner": entry["owner"], "elapsed_seconds": elapsed,
            "relinked_bins": relinked, "binary_files_written": written,
        })
    if {item["owner"] for item in result} != OWNERS:
        raise ValueError("physical evidence contains duplicate or missing owner probes")
    return {
        "schema_version": 1, "source_sha": expected_sha, "study": study,
        "status": "FOUR_OWNER_COMPILER_EVIDENCE_CAPTURED_NOT_PRODUCTION_ACCEPTANCE",
        "owners": sorted(result, key=lambda item: item["owner"]),
        "scope": report.get("measurement_scope", "four owner rebuild probes"),
    }


def markdown(summary: dict[str, Any]) -> str:
    lines = [
        "### Four-owner Rust compiler experiment",
        "",
        f"Exact source: `{summary['source_sha']}`; mode: `{summary['study']}`.",
        "This is compiler evidence, **not** proof of tenant isolation, runtime parity or CI <=180s.",
        "",
        "| Owner-only source edit | Cargo wall seconds | Binary relinks | Files written |",
        "| --- | ---: | --- | --- |",
    ]
    for owner in summary["owners"]:
        relinks = ", ".join(owner["relinked_bins"]) or "none"
        written = ", ".join(owner["binary_files_written"]) or "none"
        lines.append(f"| `{owner['owner']}` | {owner['elapsed_seconds']:.3f} | "
                     f"{relinks} | {written} |")
    lines.extend(["", f"Scope: {summary['scope']}", ""])
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--study", choices=sorted(STUDIES), required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--report", type=Path, required=True)
    options = parser.parse_args()
    try:
        report = json.loads(options.report.read_text(encoding="utf-8"))
        if not isinstance(report, dict):
            raise ValueError("owner evidence must be a JSON object")
        summary = summarize(report, options.study, options.source_sha)
    except (ValueError, OSError, json.JSONDecodeError) as exc:
        print(f"FOUR_OWNER_EVIDENCE=REJECTED reason={exc}", file=sys.stderr)
        return 2
    print("FOUR_OWNER_EVIDENCE=" + json.dumps(summary, sort_keys=True), flush=True)
    github_summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if github_summary:
        with Path(github_summary).open("a", encoding="utf-8") as stream:
            stream.write(markdown(summary))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
