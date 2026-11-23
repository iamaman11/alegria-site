#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ACTIVITIES_DIR = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities"
ACTIVITIES_MOD = ACTIVITIES_DIR / "mod.rs"
ACTIVITIES_RUNTIME = ACTIVITIES_DIR / "runtime.rs"
ACTIVE_EXTRACTION_CONTRACT = (
    ACTIVITIES_DIR / "operations" / "contracts" / "preamble.rs"
)
ACTIVE_CANDIDATE_VALIDATION = (
    ACTIVITIES_DIR / "operations" / "truth_extraction_ops" / "candidate_validation.rs"
)
PURE_RUST_LAYERS = [
    ROOT / "app" / "rust" / "crates" / "seo_application" / "src" / "knowledge.rs",
    ROOT / "app" / "rust" / "crates" / "seo_application" / "src" / "hitl.rs",
    ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "operations.rs",
]


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "mod.rs" and "activities" in path.parts:
        registry_file = path.parent / "registry" / "mod_registry_impl.rs"
        if registry_file.exists():
            text += "\n" + registry_file.read_text(encoding="utf-8")
    elif path.name == "operations.rs" and "activities" in path.parts:
        sub_dir = path.parent / "operations"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    failures: list[str] = []

    runtime_text = ACTIVITIES_RUNTIME.read_text(encoding="utf-8")
    required_runtime = [
        "use primitives::errors::{DomainError, ErrorClass};",
        "pub(crate) fn activity_error_from_domain(err: &DomainError) -> ActivityError",
        "Fut: Future<Output = Result<O, DomainError>>",
    ]
    for needle in required_runtime:
        if needle not in runtime_text:
            failures.append(
                f"missing `{needle}` in {ACTIVITIES_RUNTIME.relative_to(ROOT)}"
            )

    for path, error_variant in [
        (ACTIVE_EXTRACTION_CONTRACT, "DomainError::ContractViolation"),
        (ACTIVE_CANDIDATE_VALIDATION, "DomainError::ValidationFailure"),
    ]:
        if error_variant not in path.read_text(encoding="utf-8"):
            failures.append(f"missing `{error_variant}` in {path.relative_to(ROOT)}")

    mod_text = read_file_resolved(ACTIVITIES_MOD)
    required_mod = [
        "Self::activity_error_from_domain(&err)",
        "self.execute_step(",
    ]
    for needle in required_mod:
        if needle not in mod_text:
            failures.append(f"missing `{needle}` in {ACTIVITIES_MOD.relative_to(ROOT)}")

    forbidden = [
        "Output = anyhow::Result",
        "anyhow!(",
        "ActivityError::NonRetryable(\n                anyhow",
    ]
    for needle in forbidden:
        if needle in runtime_text:
            failures.append(
                f"forbidden `{needle}` in {ACTIVITIES_RUNTIME.relative_to(ROOT)}"
            )
        if needle in mod_text:
            failures.append(
                f"forbidden `{needle}` in {ACTIVITIES_MOD.relative_to(ROOT)}"
            )

    if failures:
        print("DOMAIN_ERROR_USAGE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    required_uc_needles = [
        "DomainError",
        "Result<",
    ]
    forbidden_uc_needles = [
        "use anyhow::Result;",
    ]
    for path in PURE_RUST_LAYERS:
        text = read_file_resolved(path)
        for needle in required_uc_needles:
            if needle not in text:
                failures.append(f"missing `{needle}` in {path.relative_to(ROOT)}")
        for needle in forbidden_uc_needles:
            if needle in text:
                failures.append(f"forbidden `{needle}` in {path.relative_to(ROOT)}")

    if failures:
        print("DOMAIN_ERROR_USAGE: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("DOMAIN_ERROR_USAGE: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
