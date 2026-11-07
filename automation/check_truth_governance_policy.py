#!/usr/bin/env python3
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app" / "db" / "schema.sql"
POLICY_ENGINE = ROOT / "app" / "rust" / "crates" / "policies" / "src" / "truth_governance.rs"

# Keep the runtime gate independent of docs/ and the GitHub Issues API.
# Separate documentation parity remains mandatory until M2 certification.
RAW_CRAWL = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "raw_crawl_adapter.rs"
RAW_CRAWL_SPLIT_DIR = (
    ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "raw_crawl_adapter"
)
TEMPORAL_OPS = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "operations.rs"
TEMPORAL_OPS_SPLIT_DIR = (
    ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "operations"
)
CI_VERIFY = ROOT / "automation" / "ci_verify.sh"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def read_module_surface(facade: Path, split_dir: Path) -> str:
    text = read(facade)
    if split_dir.exists():
        for path in sorted(split_dir.rglob("*.rs")):
            text += "\n" + read(path)
    return text


def main() -> int:
    failures: list[str] = []
    schema = read(SCHEMA)
    for needle in [
        "authority_class",
        "independence_group_key",
        "freshness_ttl_days",
        "override_eligible",
    ]:
        if needle not in schema:
            failures.append(f"schema missing governance field: {needle}")

    if not POLICY_ENGINE.exists():
        failures.append("missing truth_governance policy engine")
    else:
        engine = read(POLICY_ENGINE)
        for needle in [
            "non_independent_corroboration",
            "weak_source_corroboration",
            "authority_override@1",
            "override_not_allowed_for_authority_class",
            "freshness_block; freshness_class=",
        ]:
            if needle not in engine:
                failures.append(f"policy engine missing marker: {needle}")

        for test_name in [
            "independent_sources_verify",
            "non_independent_same_group_needs_hitl",
            "authoritative_single_source_override_verifies",
            "weak_source_corroboration_does_not_verify",
            "stale_candidate_is_blocked_before_verification",
            "override_is_denied_for_non_authoritative_source_class",
        ]:
            if f"fn {test_name}(" not in engine:
                failures.append(f"policy engine missing behavioral regression: {test_name}")

    for text, label in [
        (read_module_surface(RAW_CRAWL, RAW_CRAWL_SPLIT_DIR), "raw crawl adapter"),
        (
            read_module_surface(TEMPORAL_OPS, TEMPORAL_OPS_SPLIT_DIR),
            "temporal truth adjudication",
        ),
    ]:
        if "adjudicate_truth_candidates_with_governance" not in text:
            failures.append(f"{label} is not wired to governance adjudication")

    ci_verify = read(CI_VERIFY)
    if "python3 automation/check_truth_governance_policy.py" not in ci_verify:
        failures.append("ci_verify.sh does not include truth governance policy check")

    if "python3 automation/check_truth_governance_documentation.py" not in ci_verify:
        failures.append("ci_verify.sh does not include legacy documentation parity check")

    if failures:
        print("TRUTH_GOVERNANCE_POLICY: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("TRUTH_GOVERNANCE_POLICY: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
