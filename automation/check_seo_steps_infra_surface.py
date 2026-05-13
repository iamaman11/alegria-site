#!/usr/bin/env python3
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1]
SEO_STEPS = ROOT / "app" / "rust" / "crates" / "seo_steps" / "src"

ALLOWED_FILES = set()


def main() -> int:
    offenders = {}
    for path in SEO_STEPS.glob("*.rs"):
        text = path.read_text(encoding="utf-8")
        matches = [
            line.strip()
            for line in text.splitlines()
            if re.search(r"\binfrastructure::", line)
        ]
        if matches:
            offenders[path.name] = matches

    bad = sorted(set(offenders) - ALLOWED_FILES)
    if bad:
        for filename in bad:
            print(f"FAIL unexpected infrastructure import in seo_steps/{filename}")
            for line in offenders[filename]:
                print(f"  {line}")
        return 1

    if offenders:
        print("OK seo_steps infra surface frozen: " + ", ".join(sorted(offenders.keys())))
    else:
        print("OK seo_steps infra surface frozen: no infrastructure imports")
    return 0


if __name__ == "__main__":
    sys.exit(main())
