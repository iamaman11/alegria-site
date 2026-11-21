#!/usr/bin/env python3
"""Report sccache GHA write failure *categories*, never raw backend logs.

sccache v0.16.0 logs write failures at DEBUG (src/server.rs). The report
cannot assume that every failed cache write is an HTTP 429: a missing or
incomplete debug log remains explicitly unattributed.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OWNER = "product-truth"
CATEGORIES = ("http_429", "http_403", "http_5xx", "quota", "timeout", "other")
MARKER = "Error executing cache write:"


def classify(message: str) -> str:
    if re.search(r"(?<!\d)429(?!\d)|too many requests|rate[ -]?limit", message, re.IGNORECASE):
        return "http_429"
    if re.search(r"(?<!\d)403(?!\d)|forbidden", message, re.IGNORECASE):
        return "http_403"
    if re.search(r"(?<!\d)5\d\d(?!\d)", message):
        return "http_5xx"
    if re.search(r"quota|storage limit|insufficient storage", message, re.IGNORECASE):
        return "quota"
    if re.search(r"timed? out|timeout|deadline exceeded", message, re.IGNORECASE):
        return "timeout"
    return "other"


def classify_log(path: Path) -> tuple[bool, dict[str, int]]:
    counts = dict.fromkeys(CATEGORIES, 0)
    if not path.is_file():
        return False, counts
    # Streaming only; never print or upload raw log lines (they may contain
    # backend URLs or request metadata). Unrecognized events stay 'other'.
    with path.open(encoding="utf-8", errors="replace") as stream:
        for line in stream:
            if MARKER in line:
                counts[classify(line.split(MARKER, 1)[1])] += 1
    return True, counts


def nonnegative_counter(stats: dict[str, object], key: str) -> int:
    value = stats.get(key)
    if type(value) is not int or value < 0:
        raise ValueError(f"missing/non-integer sccache counter: {key}")
    return value


def make_report(stats: dict[str, object], log_path: Path, source_sha: str) -> dict[str, object]:
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise ValueError("missing exact PR source SHA")
    errors = nonnegative_counter(stats, "cache_write_errors")
    writes = nonnegative_counter(stats, "cache_writes")
    present, categories = classify_log(log_path)
    logged = sum(categories.values())
    return {
        "schema_version": 1,
        "source_sha": source_sha,
        "owner": OWNER,
        "sccache_write_errors": errors,
        "sccache_successful_writes": writes,
        "debug_log_present": present,
        "logged_write_error_events": logged,
        "unattributed_write_errors": max(errors - logged, 0),
        "logged_events_exceed_stats": logged > errors,
        "write_error_categories": categories,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--job", choices=(OWNER,), required=True)
    args = parser.parse_args()
    log_name = os.environ.get("SCCACHE_ERROR_LOG")
    if not log_name:
        raise SystemExit("SCCACHE_ERROR_LOG must be set for the Product/Truth diagnosis")
    result = subprocess.run(
        ["sccache", "--show-stats", "--stats-format=json"],
        capture_output=True, text=True, check=True,
    )
    stats = json.loads(result.stdout)
    if not isinstance(stats, dict):
        raise ValueError("sccache JSON statistics must be an object")
    if isinstance(stats.get("stats"), dict):
        stats = stats["stats"]
    report = make_report(stats, Path(log_name), os.environ.get("CI_SOURCE_SHA", ""))
    target = ROOT / "automation/reports" / f"sccache_diagnostic_{args.job}.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(report, sort_keys=True) + "\n", encoding="utf-8")
    print("CI_SCCACHE_WRITE_DIAGNOSTIC " + json.dumps(report, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
