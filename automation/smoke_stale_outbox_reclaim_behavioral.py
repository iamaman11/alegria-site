#!/usr/bin/env python3
from __future__ import annotations

import uuid

from _behavioral_utils import DATABASE_URL, ROOT, psql, random_key, run


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
             retry_count, next_retry_at, worker_id, locked_at, locked_until, created_at, updated_at)
            VALUES
            ('{event_id}'::uuid, 'runtime_test', '{aggregate_key}', 'neo4j', 'RuleInstanceUpserted', 'processing',
             'alegria.outbox.neo4j_rule_upserted.v1', 1, '{idempotency_key}', E'\\\\x01'::bytea, '{payload_hash}',
             0, now() - interval '1 minute', 'stale-worker', now() - interval '100 years', now() - interval '100 years',
             now() - interval '100 years', now() - interval '100 years')
            ON CONFLICT (event_id) DO NOTHING;
            """
        )
        # target-system reconcile is routed through cli_tools to keep the test isolated from the live backlog
        run(
            [
                "cargo",
                "run",
                "-q",
                "--manifest-path",
                "app/rust/services/cli_tools/Cargo.toml",
                "--",
                "reconcile-target-system",
                "--target-system",
                "neo4j",
                "--batch-limit",
                "1",
            ],
            cwd=ROOT,
            env={"DATABASE_URL": DATABASE_URL},
        )
        row = psql(
            f"""
            SELECT status || '|' || COALESCE(last_error, '')
            FROM system.sync_outbox
            WHERE event_id = '{event_id}'::uuid;
            """
        )
        if not row.startswith("pending|"):
            print(f"expected pending after reconcile, got `{row}`")
            return 1
        print("SMOKE_STALE_OUTBOX_RECLAIM_BEHAVIORAL: OK")
        return 0
    finally:
        psql(f"DELETE FROM system.sync_outbox WHERE event_id = '{event_id}'::uuid;")


if __name__ == "__main__":
    raise SystemExit(main())
