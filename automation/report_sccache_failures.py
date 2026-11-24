#!/usr/bin/env python3
"""Report cache effectiveness and write failures without exposing daemon logs.

The daemon log can contain cache service URLs, runtime credentials or source
paths. It stays inside the disposable runner and is NEVER uploaded or printed.
The native sccache counters, not the log's number of messages, are authoritative
for whether compiler objects actually reached the shared cache.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
from collections import Counter
from pathlib import Path
from typing import Iterable


WRITE_FAILURE = re.compile(
    r"cache write error|error executing cache write|failed to write (?:to )?cache|cache write failed", re.I,
)
CLASSIFIERS = (
    ("rate_limited_429", re.compile(r"\b429\b|too many requests|rate.limit", re.I)),
    ("unauthorized_401", re.compile(r"\b401\b|unauthoriz", re.I)),
    ("forbidden_403", re.compile(r"\b403\b|forbidden|permission denied", re.I)),
    ("quota_or_capacity", re.compile(r"\b507\b|quota|insufficient storage|cache full", re.I)),
    ("server_5xx", re.compile(r"\b5\d\d\b|internal server error|bad gateway", re.I)),
    ("timeout", re.compile(r"timed? out|deadline exceeded", re.I)),
    ("read_only_skip", re.compile(r"cannot write to read.only storage", re.I)),
)


def summarize(lines: Iterable[str]) -> dict[str, object]:
    failures: Counter[str] = Counter()
    for line in lines:
        if not WRITE_FAILURE.search(line):
            continue
        category = next((name for name, pattern in CLASSIFIERS if pattern.search(line)), "unclassified")
        failures[category] += 1
    return {"write_failure_log_lines": sum(failures.values()),
            "classes": dict(sorted(failures.items()))}


def nonnegative_counter(stats: dict[str, object], key: str) -> int:
    value = stats.get(key)
    if type(value) is not int or value < 0:
        raise ValueError(f"invalid sccache counter: {key}")
    return value


def rust_counter(stats: dict[str, object], key: str) -> int:
    value = stats.get(key)
    if not isinstance(value, dict):
        raise ValueError(f"invalid sccache counter family: {key}")
    counts = value.get("counts")
    if not isinstance(counts, dict):
        raise ValueError(f"invalid sccache counts: {key}")
    rust = counts.get("Rust", 0)
    if type(rust) is not int or rust < 0:
        raise ValueError(f"invalid sccache Rust counter: {key}")
    return rust


def make_report(
    raw_stats: dict[str, object],
    log_path: Path,
    source_sha: str,
    cache_mode: str,
) -> dict[str, object]:
    """Match classified log events to native counters; fail closed in metrics.

    Cache write failures are performance degradation, not a product/test
    failure: the CI job remains valid, but the cache is explicitly DEGRADED.
    A PR's deliberately read-only write skips are not backend failures.
    """
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise ValueError("missing exact source SHA for cache diagnosis")
    if cache_mode not in {"READ_ONLY", "READ_WRITE"}:
        raise ValueError("invalid sccache storage mode")
    stats = raw_stats.get("stats", raw_stats)
    if not isinstance(stats, dict):
        raise ValueError("invalid sccache JSON statistics")

    successful_writes = nonnegative_counter(stats, "cache_writes")
    failed_writes = nonnegative_counter(stats, "cache_write_errors")
    rust_hits = rust_counter(stats, "cache_hits")
    rust_misses = rust_counter(stats, "cache_misses")
    try:
        with log_path.open(encoding="utf-8", errors="replace") as stream:
            events = summarize(stream)
        log_available = True
    except OSError:
        events = summarize(())
        log_available = False

    classes = events["classes"]
    assert isinstance(classes, dict)
    classified = int(events["write_failure_log_lines"])
    read_only_lines = int(classes.get("read_only_skip", 0)) if cache_mode == "READ_ONLY" else 0
    unexpected_log_events = classified - read_only_lines
    unmatched = max(failed_writes - classified, 0)
    # A read-only skip is a real debug message, but might never increment
    # cache_write_errors. Compare only unexpected log events to the native
    # counter; never flag an intentional PR skip as provider degradation.
    mismatched = unexpected_log_events > failed_writes
    expected_skips = min(
        failed_writes,
        read_only_lines,
    )
    unexpected_failures = failed_writes - expected_skips
    if unexpected_log_events or unexpected_failures or mismatched:
        health = "DEGRADED"
    elif cache_mode == "READ_ONLY":
        health = "EXPECTED_READ_ONLY"
    else:
        health = "OK"

    attempts = successful_writes + failed_writes
    decisions = rust_hits + rust_misses
    return {
        "schema_version": 1,
        "source_sha": source_sha,
        "cache_mode": cache_mode,
        "health": health,
        "cache_writes": successful_writes,
        "cache_write_errors": failed_writes,
        "cache_write_attempts": attempts,
        "successful_write_percent": round(100 * successful_writes / attempts, 1)
        if attempts else None,
        "rust_cache_hits": rust_hits,
        "rust_cache_misses": rust_misses,
        "rust_hit_percent": round(100 * rust_hits / decisions, 1)
        if decisions else None,
        "log_available": log_available,
        "write_failure_log_lines": classified,
        "classes": classes,
        "unattributed_write_errors": unmatched,
        "logged_events_exceed_stats": mismatched,
        "unexpected_write_failure_log_lines": unexpected_log_events,
        "read_only_skip_log_lines": read_only_lines,
        "expected_read_only_skips": expected_skips,
        "unexpected_write_errors": unexpected_failures,
    }


def format_summary(report: dict[str, object]) -> str:
    """A one-line, secret-free CI signal alongside the machine-readable JSON."""
    hit_rate = report["rust_hit_percent"]
    hit_display = "n/a" if hit_rate is None else f"{hit_rate}%"
    classes = report["classes"]
    assert isinstance(classes, dict)
    return (
        f"health={report['health']} source_sha={report['source_sha']} "
        f"mode={report['cache_mode']} rust_hit_rate={hit_display} "
        f"cache_writes={report['cache_writes']} "
        f"cache_write_errors={report['cache_write_errors']} "
        f"http_429={classes.get('rate_limited_429', 0)} "
        f"unattributed={report['unattributed_write_errors']}"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", type=Path, required=True)
    args = parser.parse_args()
    # Reading native JSON stats costs milliseconds and allows us to distinguish
    # 40 HTTP 429 events from 1055 successful writes, instead of reporting a
    # meaningless PASS from a nonempty debug log. Never print raw subprocess
    # stderr or daemon lines: both can contain backend URLs/runtime metadata.
    result = subprocess.run(
        ["sccache", "--show-stats", "--stats-format=json"],
        capture_output=True, text=True, check=True,
    )
    raw_stats = json.loads(result.stdout)
    if not isinstance(raw_stats, dict):
        raise ValueError("invalid native sccache JSON output")
    # The PR runner reports the reviewed source head; trusted main uses its
    # exact merge SHA. Never infer provenance from the disposable log content.
    report = make_report(
        raw_stats,
        args.log,
        os.environ.get("CI_SOURCE_SHA") or os.environ.get("GITHUB_SHA", ""),
        os.environ.get("SCCACHE_GHA_RW_MODE", "UNSET"),
    )
    print("SCCACHE_WRITE_DIAGNOSTIC=" + json.dumps(report, sort_keys=True))
    print("SCCACHE_CACHE_STATUS=" + str(report["health"]))
    print("SCCACHE_CACHE_SUMMARY=" + format_summary(report))


if __name__ == "__main__":
    main()
