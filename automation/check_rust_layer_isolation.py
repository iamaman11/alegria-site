#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUST_CRATES = ROOT / "app" / "rust" / "crates"


def scan_rs(path: Path) -> list[Path]:
    return sorted(path.rglob("*.rs"))


def add_violation(violations: list[str], file_path: Path, message: str) -> None:
    violations.append(f"{file_path.relative_to(ROOT)}: {message}")


def main() -> int:
    violations: list[str] = []

    contracts_src = RUST_CRATES / "contracts" / "src"
    primitives_src = RUST_CRATES / "primitives" / "src"
    policies_src = RUST_CRATES / "policies" / "src"
    use_cases_src = RUST_CRATES / "use_cases" / "src"
    infra_src = RUST_CRATES / "infrastructure" / "src"

    for file_path in scan_rs(contracts_src):
        txt = file_path.read_text(encoding="utf-8")
        if re.search(r"\buse\s+(infrastructure|use_cases|policies|primitives)::", txt):
            add_violation(violations, file_path, "contracts layer must not depend on other layers")

    for file_path in scan_rs(primitives_src):
        txt = file_path.read_text(encoding="utf-8")
        if re.search(r"\buse\s+(infrastructure|use_cases|policies)::", txt):
            add_violation(violations, file_path, "primitives layer must not depend on infrastructure/use_cases/policies")
        if re.search(r"\buse\s+contracts::", txt):
            add_violation(violations, file_path, "primitives layer must not depend on contracts")

    for file_path in scan_rs(policies_src):
        txt = file_path.read_text(encoding="utf-8")
        if re.search(r"\buse\s+(infrastructure|use_cases)::", txt):
            add_violation(violations, file_path, "policies layer must not depend on infrastructure/use_cases")

    for file_path in scan_rs(use_cases_src):
        txt = file_path.read_text(encoding="utf-8")
        if re.search(r"\buse\s+use_cases::", txt):
            add_violation(violations, file_path, "use_cases must not depend on other use_cases crate")

    if infra_src.exists():
        allowed = {"lib.rs", "adapters"}
        for p in infra_src.iterdir():
            if p.is_file() and p.name not in allowed:
                add_violation(violations, p, "infrastructure root may contain only lib.rs (modules live in adapters/)")
            if p.is_dir() and p.name not in allowed:
                add_violation(violations, p, "unexpected infrastructure subdirectory")
        adapters_mod = infra_src / "adapters" / "mod.rs"
        if not adapters_mod.exists():
            add_violation(violations, infra_src / "adapters", "missing adapters/mod.rs")

    if violations:
        print("RUST_LAYER_ISOLATION: FAILED")
        for v in violations:
            print(f"- {v}")
        return 1

    print("RUST_LAYER_ISOLATION: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
