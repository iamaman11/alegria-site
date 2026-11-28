BEGIN;
SET LOCAL lock_timeout = '5s';

-- The exact top-level transaction is part of the verified-rule intent identity.
ALTER TABLE system.sync_outbox ADD COLUMN IF NOT EXISTS staged_xid xid8;

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                   WHERE conrelid='kb.qdrant_points'::regclass
                     AND conname='qdrant_points_verified_rule_entity_type_check') THEN
        ALTER TABLE kb.qdrant_points
            ADD CONSTRAINT qdrant_points_verified_rule_entity_type_check
            CHECK (collection_name <> 'verified_rules_4' OR entity_type='rule_instance') NOT VALID;
    END IF;
END $$;

CREATE OR REPLACE FUNCTION system.guard_verified_rule_outbox_identity() RETURNS trigger AS $$
BEGIN
    IF ROW(OLD.run_id, OLD.aggregate_type, OLD.aggregate_key, OLD.target_system,
           OLD.event_type, OLD.payload_type, OLD.schema_version, OLD.idempotency_key,
           OLD.payload_bytes, OLD.payload_hash, OLD.projection_revision, OLD.staged_xid)
       IS DISTINCT FROM
       ROW(NEW.run_id, NEW.aggregate_type, NEW.aggregate_key, NEW.target_system,
           NEW.event_type, NEW.payload_type, NEW.schema_version, NEW.idempotency_key,
           NEW.payload_bytes, NEW.payload_hash, NEW.projection_revision, NEW.staged_xid) THEN
        RAISE EXCEPTION 'verified-rule Outbox identity and revision are immutable'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- A caller cannot claim another transaction's outbox intent by supplying an
-- arbitrary xid. PostgreSQL stamps the *top-level* current transaction itself.
CREATE OR REPLACE FUNCTION system.stamp_verified_rule_outbox_xid() RETURNS trigger AS $$
BEGIN
    IF NEW.target_system='qdrant' AND NEW.aggregate_type='rule_instance' THEN
        NEW.staged_xid := pg_current_xact_id();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_verified_rule_outbox_xid ON system.sync_outbox;
CREATE TRIGGER trg_verified_rule_outbox_xid
    BEFORE INSERT ON system.sync_outbox
    FOR EACH ROW EXECUTE FUNCTION system.stamp_verified_rule_outbox_xid();

-- A new Truth transaction must not erase its older causal receipts or reset
-- its revision authority through ordinary DELETE. History retention is part
-- of the fence; deliberate operator repair is a separate reviewed action.
CREATE OR REPLACE FUNCTION system.reject_verified_rule_history_delete() RETURNS trigger AS $$
BEGIN
    IF TG_TABLE_NAME='sync_outbox' THEN
        IF OLD.target_system='qdrant' AND OLD.aggregate_type='rule_instance' THEN
            RAISE EXCEPTION 'verified-rule Outbox history cannot be deleted'
                USING ERRCODE='23514';
        END IF;
    ELSE
        RAISE EXCEPTION 'verified-rule revision head cannot be deleted'
            USING ERRCODE='23514';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_verified_rule_outbox_delete ON system.sync_outbox;
CREATE TRIGGER trg_verified_rule_outbox_delete
    BEFORE DELETE ON system.sync_outbox
    FOR EACH ROW EXECUTE FUNCTION system.reject_verified_rule_history_delete();
DROP TRIGGER IF EXISTS trg_verified_rule_revision_head_delete ON system.verified_rule_projection_heads;
CREATE TRIGGER trg_verified_rule_revision_head_delete
    BEFORE DELETE ON system.verified_rule_projection_heads
    FOR EACH ROW EXECUTE FUNCTION system.reject_verified_rule_history_delete();

-- NOT VALID quarantines unprovable historical receipts. New verified-rule
-- events are always stamped; old rows cannot be repaired by rewriting them.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                   WHERE conrelid='system.sync_outbox'::regclass
                     AND conname='sync_outbox_verified_rule_xid_check') THEN
        ALTER TABLE system.sync_outbox
            ADD CONSTRAINT sync_outbox_verified_rule_xid_check
            CHECK (NOT (target_system='qdrant' AND aggregate_type='rule_instance')
                   OR staged_xid IS NOT NULL) NOT VALID;
    END IF;
END $$;

-- Commit-time guard: a projected identity's final SQL state must have a
-- matching, latest-revision Qdrant command staged by THIS SAME transaction.
-- This protects low-level SQL/SQLx calls, including writes that bypass Rust
-- helpers, while remaining compatible with intentionally unprojected Truth.
CREATE OR REPLACE FUNCTION system.assert_verified_rule_tx_intent(
    rule_key TEXT, require_projection BOOLEAN
) RETURNS void AS $$
DECLARE
    latest BIGINT;
    point_exists BOOLEAN;
    expected_command TEXT;
BEGIN
    IF rule_key IS NULL OR btrim(rule_key)='' THEN
        RAISE EXCEPTION 'verified-rule projection identity is missing'
            USING ERRCODE='23514';
    END IF;
    SELECT current_revision INTO latest
      FROM system.verified_rule_projection_heads WHERE aggregate_key=rule_key;
    SELECT EXISTS (
        SELECT 1 FROM kb.qdrant_points
        WHERE collection_name='verified_rules_4'
          AND entity_type='rule_instance' AND entity_key=rule_key
    ) INTO point_exists;
    IF NOT require_projection AND latest IS NULL AND NOT point_exists THEN
        RETURN;
    END IF;
    IF latest IS NULL THEN
        RAISE EXCEPTION 'verified-rule projection % has no revision head', rule_key
            USING ERRCODE='23514';
    END IF;
    expected_command := CASE WHEN point_exists
        THEN 'QdrantUpsertCommand' ELSE 'QdrantDeleteCommand' END;
    IF NOT EXISTS (
        SELECT 1 FROM system.sync_outbox
        WHERE target_system='qdrant' AND aggregate_type='rule_instance'
          AND aggregate_key=rule_key AND projection_revision=latest
          AND staged_xid=pg_current_xact_id()
          AND event_type=expected_command
    ) THEN
        RAISE EXCEPTION 'verified-rule projection % requires same-transaction % at revision %',
            rule_key, expected_command, latest USING ERRCODE='23514';
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION system.guard_verified_rule_registry_commit() RETURNS trigger AS $$
BEGIN
    IF TG_OP='UPDATE' THEN
        IF (to_jsonb(OLD)-'updated_at')=(to_jsonb(NEW)-'updated_at') THEN
            RETURN NULL;
        END IF;
    END IF;
    IF TG_OP='UPDATE' OR TG_OP='DELETE' THEN
        IF OLD.collection_name='verified_rules_4'
           AND OLD.entity_type='rule_instance' THEN
            PERFORM system.assert_verified_rule_tx_intent(OLD.entity_key, TRUE);
        END IF;
    END IF;
    IF TG_OP='INSERT' THEN
        IF NEW.collection_name='verified_rules_4' AND NEW.entity_type='rule_instance' THEN
            PERFORM system.assert_verified_rule_tx_intent(NEW.entity_key, TRUE);
        END IF;
    ELSIF TG_OP='UPDATE' THEN
        IF NEW.collection_name='verified_rules_4' AND NEW.entity_type='rule_instance'
           AND (OLD.entity_key IS DISTINCT FROM NEW.entity_key
                OR OLD.collection_name IS DISTINCT FROM NEW.collection_name
                OR OLD.entity_type IS DISTINCT FROM NEW.entity_type) THEN
            PERFORM system.assert_verified_rule_tx_intent(NEW.entity_key, TRUE);
        END IF;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_verified_rule_registry_commit ON kb.qdrant_points;
CREATE CONSTRAINT TRIGGER trg_verified_rule_registry_commit
    AFTER INSERT OR UPDATE OR DELETE ON kb.qdrant_points
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION system.guard_verified_rule_registry_commit();

CREATE OR REPLACE FUNCTION system.guard_verified_rule_truth_commit() RETURNS trigger AS $$
BEGIN
    IF TG_OP='UPDATE' THEN
        IF (to_jsonb(OLD)-'updated_at')=(to_jsonb(NEW)-'updated_at') THEN
            RETURN NULL;
        END IF;
    END IF;
    IF TG_OP='UPDATE' OR TG_OP='DELETE' THEN
        PERFORM system.assert_verified_rule_tx_intent(OLD.rule_instance_id, FALSE);
    END IF;
    IF TG_OP='INSERT' THEN
        PERFORM system.assert_verified_rule_tx_intent(NEW.rule_instance_id, FALSE);
    ELSIF TG_OP='UPDATE' AND OLD.rule_instance_id IS DISTINCT FROM NEW.rule_instance_id THEN
        PERFORM system.assert_verified_rule_tx_intent(NEW.rule_instance_id, FALSE);
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_verified_rule_truth_commit ON verified.rule_instances;
CREATE CONSTRAINT TRIGGER trg_verified_rule_truth_commit
    AFTER INSERT OR UPDATE OR DELETE ON verified.rule_instances
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION system.guard_verified_rule_truth_commit();

COMMIT;
