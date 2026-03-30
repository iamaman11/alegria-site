#!/usr/bin/env python3
from __future__ import annotations

from _behavioral_utils import DATABASE_URL, TEMPORAL_URL, cargo_run, psql, random_key


def main() -> int:
    run_id = random_key("11111111")[:8]  # placeholder overridden below
    import uuid
    run_id = str(uuid.uuid4())
    workflow_run_id = random_key("wf")
    try:
        psql(
            f"""
            INSERT INTO pipeline.execution_runs
            (run_id, workflow_run_id, workflow_type, context_key, status, created_at, updated_at)
            VALUES
            ('{run_id}'::uuid, '{workflow_run_id}', 'extract_facts', 'ctx.behavioral.pending_hitl', 'pending_hitl',
             now() - interval '30 days', now() - interval '30 days')
            ON CONFLICT (run_id) DO NOTHING;
            """
        )
        cargo_run(
            "services/reconcile/Cargo.toml",
            env={
                "DATABASE_URL": DATABASE_URL,
                "TEMPORAL_URL": TEMPORAL_URL,
                "RECONCILE_BATCH_LIMIT": "10",
                "RECONCILE_DRY_RUN": "0",
            },
        )
        row = psql(
            f"""
            SELECT status || '|' ||
                   (SELECT count(*)::text FROM system.dead_letter_queue WHERE run_id = '{run_id}'::uuid)
            FROM pipeline.execution_runs
            WHERE run_id = '{run_id}'::uuid;
            """
        )
        if row != "pending_hitl|0":
            print(f"expected pending_hitl|0 after reconcile, got `{row}`")
            return 1
        print("SMOKE_PENDING_HITL_NOT_FAILURE_BEHAVIORAL: OK")
        return 0
    finally:
        psql(f"DELETE FROM system.dead_letter_queue WHERE run_id = '{run_id}'::uuid;")
        psql(f"DELETE FROM pipeline.execution_runs WHERE run_id = '{run_id}'::uuid;")


if __name__ == "__main__":
    raise SystemExit(main())
