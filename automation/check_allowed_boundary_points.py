#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / "app" / "rust"

FORBIDDEN_PATTERNS = (
    re.compile(r"\bserde_json::Value\b"),
    re.compile(r"\bjson!\s*\("),
    re.compile(r"\bserde_json::to_value\b"),
    re.compile(r"\bserde_json::from_value\b"),
)


def allowed(path: Path) -> bool:
    rel = path.relative_to(ROOT)
    rel_s = str(rel)
    if "target/" in rel_s:
        return True
    if rel_s.startswith("app/rust/crates/runtime_models/src/"):
        return True
    if rel_s.startswith("app/rust/crates/infrastructure/src/adapters/"):
        return True
    if rel_s.startswith("app/rust/crates/primitives/src/") and path.name.endswith("_json.rs"):
        return True
    if rel_s.startswith("app/rust/services/outbox_worker/src/"):
        return True
    if rel_s.startswith("app/rust/services/gsc_sync/src/"):
        return True
    if rel_s.startswith("app/rust/services/cli_tools/src/"):
        return True
    if rel_s.startswith("app/rust/crates/telemetry/src/"):
        return True
    return False


def main() -> int:
    violations: list[str] = []
    for file_path in sorted(RUST.rglob("*.rs")):
        text = file_path.read_text(encoding="utf-8")
        if any(pattern.search(text) for pattern in FORBIDDEN_PATTERNS) and not allowed(file_path):
            violations.append(str(file_path.relative_to(ROOT)))

    if violations:
        print("ALLOWED_BOUNDARY_POINTS: FAILED")
        for violation in violations:
            print(f"- {violation}")
        return 1

    print("ALLOWED_BOUNDARY_POINTS: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
