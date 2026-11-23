#!/usr/bin/env python3
"""Report only aggregate sccache write-failure classes, never raw logs.

The daemon log can contain cache service URLs, runtime credentials or source
paths. It stays inside the disposable runner and is NEVER uploaded or printed.
"""

from __future__ import annotations

import argparse
import json
import os
import re
from collections import Counter
from pathlib import Path


WRITE_FAILURE = re.compile(
    r"cache write error|error executing cache write|failed to write (?:to )?cache|cache write failed", re.I,
)
CLASSIFIERS = (
    ("read_only_skip", re.compile(r"cannot write to read.only storage", re.I)),
    ("rate_limited_429", re.compile(r"\b429\b|too many requests|rate.limit", re.I)),
    ("unauthorized_401", re.compile(r"\b401\b|unauthoriz", re.I)),
    ("forbidden_403", re.compile(r"\b403\b|forbidden|permission denied", re.I)),
    ("quota_or_capacity", re.compile(r"\b507\b|quota|insufficient storage|cache full", re.I)),
    ("server_5xx", re.compile(r"\b5\d\d\b|internal server error|bad gateway", re.I)),
    ("timeout", re.compile(r"timed? out|deadline exceeded", re.I)),
)


def summarize(lines: list[str]) -> dict[str, object]:
    failures: Counter[str] = Counter()
    for line in lines:
        if not WRITE_FAILURE.search(line):
            continue
        category = next((name for name, pattern in CLASSIFIERS if pattern.search(line)), "unclassified")
        failures[category] += 1
    return {"write_failure_log_lines": sum(failures.values()),
            "classes": dict(sorted(failures.items()))}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", type=Path, required=True)
    args = parser.parse_args()
    # sccache 0.16.0's ReadOnlyStorage::put returns an error, so its stats
    # count each deliberately skipped write as a "Cache write error".
    mode = os.environ.get("SCCACHE_GHA_RW_MODE", "UNSET")
    try:
        # Never print file contents or an exception that could expose the path.
        with args.log.open(encoding="utf-8", errors="replace") as file:
            report = summarize(file.readlines())
        report["log_available"] = True
    except OSError:
        report = {"log_available": False, "write_failure_log_lines": 0, "classes": {}}
    report["cache_mode"] = mode if mode in {"READ_ONLY", "READ_WRITE"} else "UNSET"
    report["read_only_skip_counter_expected"] = mode == "READ_ONLY"
    print("SCCACHE_WRITE_DIAGNOSTIC=" + json.dumps(report, sort_keys=True))


if __name__ == "__main__":
    main()
