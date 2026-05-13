#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
FILES = [
    ROOT / "automation" / "OPS_CANONICALS.md",
    ROOT / "automation" / "LAYER_MAP.md",
    ROOT / "automation" / "INTEGRITY_RUNBOOK.md",
    ROOT / "automation" / "NEW_OPERATION_PLAYBOOK.md",
    ROOT / "docs" / "V5_SEO_SUPERSITE_IMPLEMENTATION_ROADMAP.md",
    ROOT / "docs" / "OPS_RUNTIME_RUNBOOK.md",
]

FORBIDDEN_SNIPPETS = [
    "use_cases = policy + orchestration",
    "seo_steps::hitl_queue",
]


def main() -> int:
    failures: list[str] = []
    for path in FILES:
        text = path.read_text(encoding="utf-8")
        for snippet in FORBIDDEN_SNIPPETS:
            if snippet in text:
                failures.append(f"{path.relative_to(ROOT)} still contains `{snippet}`")

    if failures:
        print("SEO_ARCHITECTURE_WORDING: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SEO_ARCHITECTURE_WORDING: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
