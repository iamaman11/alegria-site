#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
RUST_ROOT = ROOT / "app" / "rust"
CANONICAL_IDENTITY = "crates/seo_domain/src/identity.rs"
CANONICAL_REBUILD = "crates/seo_domain/src/rebuild.rs"


def rust_files():
    for path in RUST_ROOT.rglob("*.rs"):
        if "target" not in path.parts:
            yield path


def main() -> int:
    allowlist_defs = []
    classify_defs = []
    priority_defs = []
    for path in rust_files():
        rel = path.relative_to(RUST_ROOT).as_posix()
        text = path.read_text(encoding="utf-8")
        if "ALLOWED_APPLICANT_PROFILES" in text:
            allowlist_defs.append(rel)
        if "fn classify_trigger(" in text:
            classify_defs.append(rel)
        if "fn trigger_priority(" in text:
            priority_defs.append(rel)

    errors = []
    if allowlist_defs != [CANONICAL_IDENTITY]:
        errors.append(
            "ALLOWED_APPLICANT_PROFILES must be defined only in "
            f"{CANONICAL_IDENTITY}; found {allowlist_defs}"
        )
    if classify_defs != [CANONICAL_REBUILD]:
        errors.append(
            f"classify_trigger must be defined only in {CANONICAL_REBUILD}; found {classify_defs}"
        )
    if priority_defs != [CANONICAL_REBUILD]:
        errors.append(
            f"trigger_priority must be defined only in {CANONICAL_REBUILD}; found {priority_defs}"
        )

    if errors:
        for error in errors:
            print(f"FAIL {error}")
        return 1

    print("OK seo canonical domain ownership")
    return 0


if __name__ == "__main__":
    sys.exit(main())
