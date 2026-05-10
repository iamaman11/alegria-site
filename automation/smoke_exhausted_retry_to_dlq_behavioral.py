#!/usr/bin/env python3
from __future__ import annotations

import uuid

from _behavioral_utils import DATABASE_URL, cargo_run, psql, random_key


def main() -> int:
    event_id = str(uuid.uuid4())
    aggregate_key = random_key("agg")
    idempotency_key = random_key("idem")
    payload_hash = random_key("hash")
    try:
        psql(
            f"""
            INSERT INTO system.sync_outbox
            (event_id, aggregate_type, aggregate_key, target_system, event_type, status,
             payload_type, schema_version, idempotency_key, payload_bytes, payload_hash,
             retry_count, next_retry_at, created_at, updated_at)
            VALUES
            ('{event_id}'::uuid, 'runtime_test', '{aggregate_key}', 'qdrant', 'UnsupportedEvent', 'pending',
             'alegria.sync.v1.UnsupportedEvent', 1, '{idempotency_key}', E'\\\\x01'::bytea, '{payload_hash}',
             9, now() - interval '1 minute', now() - interval '30 days', now() - interval '30 days')
            ON CONFLICT (event_id) DO NOTHING;
            """
        )
        cargo_run(
            "services/outbox_worker/Cargo.toml",
            env={
                "DATABASE_URL": DATABASE_URL,
                "OUTBOX_MAX_CYCLES": "1",
                "OUTBOX_WORKER_ID": "behavioral-retry-dlq",
                "OUTBOX_ONLY_EVENT_ID": event_id,
            },
        )
        row = psql(
            f"""
            SELECT status || '|' || retry_count::text
            FROM system.sync_outbox
            WHERE event_id = '{event_id}'::uuid;
            """
        )
        dlq = psql(
            f"""
            SELECT step_name
            FROM system.dead_letter_queue
            WHERE workflow_id = '{event_id}'
            ORDER BY dlq_id DESC
            LIMIT 1;
            """
        )
        if not row.startswith("failed|10"):
            print(f"expected failed|10, got `{row}`")
            return 1
        if dlq != "sync_outbox_dispatch":
            print(f"expected sync_outbox_dispatch DLQ, got `{dlq}`")
            return 1
        print("SMOKE_EXHAUSTED_RETRY_TO_DLQ_BEHAVIORAL: OK")
        return 0
    finally:
        psql(f"DELETE FROM system.dead_letter_queue WHERE workflow_id = '{event_id}';")
        psql(f"DELETE FROM system.sync_outbox WHERE event_id = '{event_id}'::uuid;")


if __name__ == "__main__":
    raise SystemExit(main())
