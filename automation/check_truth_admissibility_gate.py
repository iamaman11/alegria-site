#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SEO_RUNTIME = ROOT / "app/rust/crates/seo_application/src/seo_runtime.rs"
DRAFTING = ROOT / "app/rust/crates/seo_application/src/drafting.rs"


def main() -> int:
    seo_runtime = SEO_RUNTIME.read_text(encoding="utf-8")
    drafting = DRAFTING.read_text(encoding="utf-8")
    failures: list[str] = []

    for needle in [
        "pub fn truth_admissibility_gate(",
        "truth_admissibility_gate failed",
        "support row",
    ]:
        if needle not in seo_runtime:
            failures.append(f"seo_runtime missing `{needle}`")

    if "truth_admissibility_gate(" not in drafting:
        failures.append("drafting.rs does not enforce truth_admissibility_gate before draft_assemble")

    if failures:
        print("TRUTH_ADMISSIBILITY_GATE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("TRUTH_ADMISSIBILITY_GATE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
