ALTER TABLE system.sync_outbox
    ADD COLUMN IF NOT EXISTS run_id TEXT NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS idx_system_sync_outbox_target_run_status
    ON system.sync_outbox(target_system, run_id, status);
