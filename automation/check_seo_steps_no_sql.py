#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SEO_STEPS = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src"

FORBIDDEN = {
    "sqlx::query": re.compile(r"\bsqlx::query(_as|_scalar)?\b"),
    "sqlx::Row": re.compile(r"\bsqlx::Row\b"),
}


def main() -> int:
    violations: list[str] = []
    for file_path in sorted(SEO_STEPS.rglob("*.rs")):
        text = file_path.read_text(encoding="utf-8")
        for label, pattern in FORBIDDEN.items():
            if pattern.search(text):
                violations.append(f"{file_path.relative_to(ROOT)}: forbidden {label}")

    if violations:
        print("SEO_STEPS_NO_SQL: FAILED")
        for violation in violations:
            print(f"- {violation}")
        return 1

    print("SEO_STEPS_NO_SQL: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
