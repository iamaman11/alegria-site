#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]

FILES = {
    "drafting": ROOT / "app/rust/crates/seo_application/src/drafting.rs",
    "ports": ROOT / "app/rust/crates/seo_ports/src/lib.rs",
    "ports_adapter": ROOT / "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter.rs",
    "seo_adapter": ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs",
    "proto_runtime": ROOT / "app/rust/crates/infrastructure/src/adapters/proto_runtime_payload_store.rs",
    "source_projection": ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_source_projection_adapter.rs",
    "raw_crawl": ROOT / "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter.rs",
    "cms": ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter.rs",
}

REQUIRED = {
    "ports": [
        "async fn persist_draft_assemble_output(",
        "run_id: &str,",
    ],
    "drafting": [
        "repo.persist_draft_assemble_output(&input.run_id, &output)",
    ],
    "ports_adapter": [
        "sqlx_seo_adapter::persist_draft_assemble_output(self.pool, run_id, output).await",
    ],
    "seo_adapter": [
        "pub async fn persist_draft_assemble_output(",
        "run_id: &str,",
        "non_empty(run_id, \"run_id\")?;",
        "emit_projection_events_for_run(pool, run_id, projection_events).await?;",
        "persist_draft_assemble_output(pool, &input.run_id, &output_as_assemble).await",
        "if event.run_id.trim().is_empty() {",
        "event.run_id = run_id.to_string();",
    ],
    "source_projection": [
        "pub async fn persist_from_pipeline_state(",
        "run_id: &str,",
        "event.run_id = run_id.to_string();",
    ],
    "proto_runtime": [
        "pub(crate) fn neo4j_rule_upserted(",
        "run_id: String::new(),",
    ],
    "raw_crawl": [
        "pub async fn emit_raw_section_qdrant_events(",
        "run_id: &str,",
        "run_id: run_id.to_string(),",
    ],
    "cms": [
        "fn cms_event_outbox(event: &SeoCmsEventPayload, run_id: &str) -> OutboxEnvelope {",
        "run_id: run_id.to_string(),",
        "cms_event_outbox(&cms_event, &input.run_id)",
    ],
}

ALLOWED_EMPTY_RUN_ID_FILES = {
    ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs",
    ROOT / "app/rust/crates/infrastructure/src/adapters/proto_runtime_payload_store.rs",
}

ALLOWED_OUTBOX_EMIT_CALLS = {
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs": [
        "let _ = outbox_emit_many(pool, &events).await?;",
    ],
    "app/rust/crates/infrastructure/src/adapters/sqlx_source_projection_adapter.rs": [
        "let outbox_count = outbox_emit_many(pool, &outbox_events).await? as usize;",
    ],
    "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter.rs": [
        "let emitted = outbox_emit_many(pool, &events).await?;",
    ],
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter.rs": [
        "let emitted = outbox_emit_many(pool, &[cms_event_outbox(&cms_event, &input.run_id)]).await?;",
        "let _ = outbox_emit_many(pool, &[cms_event_outbox(&cms_event, &input.run_id)]).await?;",
    ],
}


def main() -> int:
    failures: list[str] = []
    for key, path in FILES.items():
        text = path.read_text(encoding="utf-8")
        for needle in REQUIRED.get(key, []):
            if needle not in text:
                failures.append(f"{path.relative_to(ROOT)} missing `{needle}`")

    for path in ROOT.glob("app/rust/crates/infrastructure/src/adapters/*.rs"):
        text = path.read_text(encoding="utf-8")
        if "run_id: String::new()," in text and path not in ALLOWED_EMPTY_RUN_ID_FILES:
            failures.append(
                f"{path.relative_to(ROOT)} contains unsanctioned `run_id: String::new(),`"
            )
        if "outbox_emit_many(" not in text:
            continue
        allowed_calls = ALLOWED_OUTBOX_EMIT_CALLS.get(str(path.relative_to(ROOT)), [])
        for line in [
            line.strip()
            for line in text.splitlines()
            if "outbox_emit_many(" in line and "fn outbox_emit_many(" not in line
        ]:
            if line not in allowed_calls:
                failures.append(
                    f"{path.relative_to(ROOT)} contains unsanctioned outbox emit site `{line}`"
                )

    if failures:
        print("RUN_ID_EVENT_STAMPING: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("RUN_ID_EVENT_STAMPING: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
