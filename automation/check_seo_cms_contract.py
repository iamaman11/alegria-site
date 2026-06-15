#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app" / "db" / "schema.sql"
SYNC_PROTO = ROOT / "app" / "contracts" / "proto" / "sync.proto"
TEMPORAL_PROTO = ROOT / "app" / "contracts" / "proto" / "temporal_payloads.proto"
CMS_ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_seo_cms_adapter.rs"
ACTIVITIES = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "activities" / "mod.rs"
WORKFLOW = ROOT / "app" / "rust" / "services" / "temporal" / "src" / "workflows" / "seo_site_build.rs"
OUTBOX_WORKER = ROOT / "app" / "rust" / "services" / "outbox_worker" / "src" / "materialize.rs"
PAYLOAD_STORE = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "proto_runtime_payload_store.rs"

REQUIRED_TABLES = [
    "site.cms_pages",
    "site.cms_page_revisions",
    "site.cms_publish_events",
    "site.seo_hitl_tasks",
]

REQUIRED_EVENTS = [
    "seo_page_review_requested",
    "seo_page_approved",
    "seo_page_publish_blocked",
    "seo_page_published",
    "seo_page_deprecated",
    "seo_page_rollback_requested",
    "seo_page_rolled_back",
    "seo_page_rebuild_requested",
    "seo_page_canonical_changed",
]

REQUIRED_BLOCKERS = [
    "failing_draft_qa_verdict",
    "missing_traceability_manifest",
    "active_cannibalization_conflict",
    "required_link_obligations_unsatisfied",
    "canonical_url_conflict",
]


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "mod.rs" and "activities" in path.parts:
        registry_file = path.parent / "registry" / "mod_registry_impl.rs"
        if registry_file.exists():
            text += "\n" + registry_file.read_text(encoding="utf-8")
    elif path.name == "sqlx_seo_cms_adapter.rs":
        sub_dir = path.parent / "sqlx_seo_cms_adapter"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "proto_runtime_payload_store.rs":
        sub_dir = path.parent / "proto_runtime_payload_store"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    elif path.name == "materialize.rs" and "outbox_worker" in path.parts:
        delegate_path = path.parents[3] / "crates" / "infrastructure" / "src" / "adapters" / "projection_materialize_adapter.rs"
        if delegate_path.exists():
            text += "\n" + delegate_path.read_text(encoding="utf-8")
    return text


def main() -> int:
    schema = SCHEMA.read_text(encoding="utf-8")
    sync_proto = SYNC_PROTO.read_text(encoding="utf-8")
    temporal_proto = TEMPORAL_PROTO.read_text(encoding="utf-8")
    cms_adapter = read_file_resolved(CMS_ADAPTER) if CMS_ADAPTER.exists() else ""
    activities = read_file_resolved(ACTIVITIES)
    workflow = WORKFLOW.read_text(encoding="utf-8")
    outbox_worker = read_file_resolved(OUTBOX_WORKER)
    payload_store = read_file_resolved(PAYLOAD_STORE)
    failures: list[str] = []

    for table in REQUIRED_TABLES:
        if f"CREATE TABLE IF NOT EXISTS {table}" not in schema:
            failures.append(f"schema missing `{table}`")
    for event in REQUIRED_EVENTS:
        if event not in schema:
            failures.append(f"schema missing CMS event `{event}`")
        if event not in outbox_worker:
            failures.append(f"outbox worker does not accept CMS event `{event}`")

    for needle in [
        "message SeoCmsEventPayload",
        "message CmsPublishInputPayload",
        "message CmsPublishOutputPayload",
    ]:
        haystack = sync_proto + "\n" + temporal_proto
        if needle not in haystack:
            failures.append(f"proto missing `{needle}`")

    for blocker in REQUIRED_BLOCKERS:
        if blocker not in cms_adapter:
            failures.append(f"CMS adapter missing publish blocker `{blocker}`")

    for needle in [
        "persist_cms_publish_output",
        "site.cms_page_revisions",
        "site.cms_pages",
        "site.cms_publish_events",
        "site.seo_hitl_tasks",
        "SeoCmsEventPayload",
    ]:
        if needle not in cms_adapter:
            failures.append(f"CMS adapter missing `{needle}`")

    if "run_cms_publish_step" not in activities or '"cms_publish"' not in activities:
        failures.append("Temporal activity surface missing ledger-backed cms_publish step")
    if "run_cms_publish_step" not in workflow:
        failures.append("SeoSiteBuildWorkflow does not call cms_publish step")
    if "CmsPublishInputPayload" not in payload_store or "CmsPublishOutputPayload" not in payload_store:
        failures.append("runtime payload store missing CMS publish payloads")
    if "target_system == \"cms\"" not in outbox_worker or "SeoCmsEventPayload" not in outbox_worker:
        failures.append("outbox worker missing typed CMS dispatch")

    if failures:
        print("SEO_CMS_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_CMS_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
