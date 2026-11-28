BEGIN;
SET LOCAL lock_timeout = '5s';

-- Every newly staged qdrant/rule_instance effect has a producer-owned causal
-- revision, allocated under its Truth advisory lock in the same transaction.
-- Historical events have NO recoverable causal order: never backfill from
-- timestamps, UUIDv4, run IDs or the sequence of this migration.
ALTER TABLE system.sync_outbox
    ADD COLUMN IF NOT EXISTS projection_revision BIGINT;

CREATE TABLE IF NOT EXISTS system.verified_rule_projection_heads (
    aggregate_key TEXT PRIMARY KEY CHECK (aggregate_key <> '' AND btrim(aggregate_key)=aggregate_key),
    current_revision BIGINT NOT NULL CHECK (current_revision > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_sync_outbox_verified_rule_revision
    ON system.sync_outbox(aggregate_key, projection_revision)
    WHERE target_system='qdrant' AND aggregate_type='rule_instance'
      AND projection_revision IS NOT NULL;

-- Protect causal history from accidental revision/identity rewrites by any
-- future generic Outbox status updater. No automatic historical rebaseline.
CREATE OR REPLACE FUNCTION system.guard_verified_rule_outbox_identity() RETURNS trigger AS $$
BEGIN
    IF ROW(OLD.run_id, OLD.aggregate_type, OLD.aggregate_key, OLD.target_system,
           OLD.event_type, OLD.payload_type, OLD.schema_version, OLD.idempotency_key,
           OLD.payload_bytes, OLD.payload_hash, OLD.projection_revision)
       IS DISTINCT FROM
       ROW(NEW.run_id, NEW.aggregate_type, NEW.aggregate_key, NEW.target_system,
           NEW.event_type, NEW.payload_type, NEW.schema_version, NEW.idempotency_key,
           NEW.payload_bytes, NEW.payload_hash, NEW.projection_revision) THEN
        RAISE EXCEPTION 'verified-rule Outbox identity and revision are immutable'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_verified_rule_outbox_identity ON system.sync_outbox;
CREATE TRIGGER trg_verified_rule_outbox_identity
    BEFORE UPDATE ON system.sync_outbox
    FOR EACH ROW
    WHEN ((OLD.target_system='qdrant' AND OLD.aggregate_type='rule_instance')
       OR (NEW.target_system='qdrant' AND NEW.aggregate_type='rule_instance'))
    EXECUTE FUNCTION system.guard_verified_rule_outbox_identity();

-- NOT VALID is intentional. Existing legacy rows remain genuinely NULL and
-- visible to diagnosis; NEW inserts and updates of old unversioned rows are
-- rejected by PostgreSQL (including attempted lease claims by old Workers).
-- ANY historical unversioned point identity (even done) is fenced in Rust:
-- a pre-cutover worker may have left an unobservable late provider RPC.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                   WHERE conrelid='system.sync_outbox'::regclass
                     AND conname='sync_outbox_verified_rule_revision_check') THEN
        ALTER TABLE system.sync_outbox
            ADD CONSTRAINT sync_outbox_verified_rule_revision_check
            CHECK (NOT (target_system='qdrant' AND aggregate_type='rule_instance')
                   OR (projection_revision IS NOT NULL AND projection_revision > 0)) NOT VALID;
    END IF;
END $$;

COMMIT;
