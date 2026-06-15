#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build.rs"
ACTIVITIES = ROOT / "app/rust/services/temporal/src/activities/mod.rs"
ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs"
STARTER = ROOT / "app/rust/services/temporal/src/bin/temporal_starter.rs"


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "mod.rs" and "activities" in path.parts:
        registry_file = path.parent / "registry" / "mod_registry_impl.rs"
        if registry_file.exists():
            text += "\n" + registry_file.read_text(encoding="utf-8")
    elif path.name == "sqlx_seo_adapter.rs":
        sub_dir = path.parent / "sqlx_seo_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "temporal_starter.rs":
        sub_dir = path.parent / "temporal_starter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def main() -> int:
    failures: list[str] = []
    workflow = WORKFLOW.read_text(encoding="utf-8")
    activities = read_file_resolved(ACTIVITIES)
    adapter = read_file_resolved(ADAPTER)
    starter = read_file_resolved(STARTER)

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
