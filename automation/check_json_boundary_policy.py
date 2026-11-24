#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / "app" / "rust"

FORBIDDEN_PATTERNS = {
    "serde_json::Value": re.compile(r"\bserde_json::Value\b"),
    "json! macro": re.compile(r"\bjson!\s*\("),
    "serde_json::to_value": re.compile(r"\bserde_json::to_value\b"),
    "serde_json::from_value": re.compile(r"\bserde_json::from_value\b"),
    "serde_json::from_slice": re.compile(r"\bserde_json::from_slice\b"),
    "serde_json::from_str": re.compile(r"\bserde_json::from_str\b"),
}


def is_allowed_json_boundary(path: Path) -> bool:
    rel = str(path.relative_to(ROOT))
    if rel.startswith("app/rust/crates/runtime_models/src/"):
        return True
    if rel.startswith("app/rust/crates/infrastructure/src/adapters/"):
        return True
    # Cross-provider runtime wiring remains an adapter boundary after moving
    # from infrastructure; domains, steps and the crate root stay deny-by-default.
    if rel.startswith("app/rust/crates/runtime_composition/src/adapters/"):
        return True
    # Transport-only Qdrant boundary, extracted verbatim from infrastructure.
    # Wire JSON remains legal here, never in the SEO domain or step crates.
    if rel.startswith("app/rust/crates/vector_qdrant/src/"):
        return True
    if rel.startswith("app/rust/crates/integration_harness/src/"):
        return True
    if rel in {
        "app/rust/crates/seo_ports/src/lib.rs",
        # The provider-free protocol's tests verify a strict serialized plan
        # boundary; production domain/operation modules remain deny-by-default.
        "app/rust/crates/operation_protocol/src/tests.rs",
        "app/rust/crates/seo_application/src/rebuild_detect.rs",
        "app/rust/crates/seo_domain/src/applicability.rs",
        "app/rust/crates/seo_domain/src/rebuild.rs",
        "app/rust/services/temporal/src/activities/mod.rs",
    }:
        return True
    if rel.startswith("app/rust/crates/primitives/src/") and path.name.endswith("_json.rs"):
        return True
    if rel.startswith("app/rust/crates/telemetry/src/"):
        return True
    if rel.startswith("app/rust/services/outbox_worker/src/"):
        return True
    if rel.startswith("app/rust/services/gsc_sync/src/"):
        return True
    if rel.startswith("app/rust/services/cli_tools/src/"):
        return True
    # The CLI is a wire boundary; other runtime code must use typed inputs.
    if rel.startswith("app/rust/services/temporal/src/bin/temporal_starter/"):
        return True
    if rel in {
        "app/rust/services/temporal/src/bin/whole_page_semantic_report.rs",
        # The preamble and registry decode registered external activity JSON.
        "app/rust/services/temporal/src/activities/operations/contracts/preamble.rs",
        "app/rust/services/temporal/src/activities/registry/mod_registry_impl.rs",
        # The activity compatibility tests exercise the registered wire form.
        "app/rust/services/temporal/src/activities/operations/truth_write_and_fixture_tests.rs",
        "app/rust/services/temporal/src/activities/operations/truth_projection_helpers.rs",
    }:
        return True
    return False


def main() -> int:
    violations: list[str] = []
    for rs in sorted(RUST.rglob("*.rs")):
        if "target" in rs.parts:
            continue
        text = rs.read_text(encoding="utf-8")
        hits = [label for label, pattern in FORBIDDEN_PATTERNS.items() if pattern.search(text)]
        if hits and not is_allowed_json_boundary(rs):
            violations.append(f"{rs.relative_to(ROOT)}: forbidden JSON boundary usage {', '.join(hits)}")

    if violations:
        print("JSON_BOUNDARY_POLICY: FAILED")
        for violation in violations:
            print(f"- {violation}")
        print("action: JSON boundary is deny-by-default; keep it only in adapters, *_json.rs, and explicit raw/debug service boundaries")
        return 1

    print("JSON_BOUNDARY_POLICY: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
