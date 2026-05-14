#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build.rs"
ACTIVITIES = ROOT / "app/rust/services/temporal/src/activities/mod.rs"
ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs"
STARTER = ROOT / "app/rust/services/temporal/src/bin/temporal_starter.rs"


def main() -> int:
    failures: list[str] = []
    workflow = WORKFLOW.read_text(encoding="utf-8")
    activities = ACTIVITIES.read_text(encoding="utf-8")
    adapter = ADAPTER.read_text(encoding="utf-8")
    starter = STARTER.read_text(encoding="utf-8")

    for needle in [
        "load_verified_support_bundle",
        "SeoPhaseKey::LoadVerifiedSupportBundle",
    ]:
        if needle not in workflow:
            failures.append(f"workflow missing `{needle}`")
    for needle in [
        "pub async fn load_verified_support_bundle",
    ]:
        if needle not in activities:
            failures.append(f"activities missing `{needle}`")
    for needle in [
        "pub async fn load_verified_support_bundle(",
        "verified support bundle is empty",
        '"verified_support_bundle"',
    ]:
        if needle not in adapter:
            failures.append(f"adapter missing `{needle}`")
    if "register_site_build_input(" not in starter:
        failures.append("starter should remain thin and use shared registration path")

    if failures:
        print("SUPPORT_BUNDLE_REQUIRED: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SUPPORT_BUNDLE_REQUIRED: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
