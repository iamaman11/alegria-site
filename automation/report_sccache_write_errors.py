#!/usr/bin/env python3
"""Report sccache write-error categories without exposing raw CI server logs.

The GHA cache backend counts failed writes but does not include their HTTP
status in --show-stats. A short-lived server debug log is inspected on the
runner and never uploaded; only aggregate error categories are printed.
"""

from __future__ import annotations

from collections import Counter
from pathlib import Path
import re
import sys


WRITE_ERROR = "Error executing cache write:"
HTTP_STATUS = re.compile(r"\b(?:HTTP(?:\s+status)?|status(?:\s+code)?)[\s:=()\-]*([45]\d\d)\b", re.I)
STATUS_CODES = ("400", "401", "403", "404", "409", "413", "429", "500", "502", "503", "504", "507")


def classify(error: str) -> str:
    match = HTTP_STATUS.search(error)
    if match:
        return f"http_{match.group(1)}"
    for code in STATUS_CODES:
        if re.search(rf"(?<![0-9A-Za-z]){code}(?![0-9A-Za-z])", error):
            return f"http_{code}"
    lower = error.lower()
    if "too many requests" in lower or "rate limit" in lower:
        return "rate_limited"
    if "permission" in lower or "forbidden" in lower or "unauthorized" in lower:
        return "permission"
    if "quota" in lower or "capacity" in lower or "storage limit" in lower:
        return "capacity"
    if "timed out" in lower or "connection" in lower or "network" in lower:
        return "network"
    return "unknown"


def summarize(path: Path) -> Counter[str]:
    counts: Counter[str] = Counter()
    if not path.is_file():
        return counts
    with path.open(encoding="utf-8", errors="replace") as log:
        for line in log:
            if WRITE_ERROR in line:
                counts[classify(line.split(WRITE_ERROR, 1)[1])] += 1
    return counts


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: report_sccache_write_errors.py <sccache-server-log>", file=sys.stderr)
        return 2
    path = Path(sys.argv[1])
    if not path.is_file():
        print("SCCACHE_WRITE_DIAGNOSTIC=log_unavailable")
        return 0
    counts = summarize(path)
    print(f"SCCACHE_WRITE_DIAGNOSTIC=observed total={sum(counts.values())}")
    for category, count in sorted(counts.items()):
        print(f"SCCACHE_WRITE_FAILURE category={category} count={count}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
