-- Versioned, independently operator-attested Temporal parent/child provenance.
-- V3 rows remain valid. No caller-written workflow/run ID is authority.
BEGIN;
SET LOCAL lock_timeout = '5s';

ALTER TABLE pipeline.pg_effect_v3_admissions
    ADD COLUMN IF NOT EXISTS admission_schema_revision SMALLINT NOT NULL DEFAULT 3,
    ADD COLUMN IF NOT EXISTS v3_expected_execution_key TEXT,
    ADD COLUMN IF NOT EXISTS parent_workflow_id TEXT,
    ADD COLUMN IF NOT EXISTS parent_workflow_run_id TEXT,
    ADD COLUMN IF NOT EXISTS dispatch_execution_key TEXT,
    ADD COLUMN IF NOT EXISTS dispatch_input_hash TEXT,
    ADD COLUMN IF NOT EXISTS invocation_sha256 TEXT,
    ADD COLUMN IF NOT EXISTS bound_input_sha256 TEXT,
    ADD COLUMN IF NOT EXISTS owner_command_sha256 TEXT,
    ADD COLUMN IF NOT EXISTS temporal_attestation_sha256 TEXT,
    ADD COLUMN IF NOT EXISTS temporal_child_started_event_id BIGINT;

ALTER TABLE pipeline.pg_effect_v3_admissions
    DROP CONSTRAINT IF EXISTS pg_effect_v4_admission_revision_check,
    ADD CONSTRAINT pg_effect_v4_admission_revision_check
        CHECK (admission_schema_revision IN (3,4)),
    DROP CONSTRAINT IF EXISTS pg_effect_v3_expected_execution_key_shape,
    ADD CONSTRAINT pg_effect_v3_expected_execution_key_shape
        CHECK (v3_expected_execution_key IS NULL OR length(v3_expected_execution_key) = 64),
    DROP CONSTRAINT IF EXISTS pg_effect_v4_admission_shape,
    ADD CONSTRAINT pg_effect_v4_admission_shape CHECK (
        (admission_schema_revision = 3 AND
            parent_workflow_id IS NULL AND parent_workflow_run_id IS NULL AND
            dispatch_execution_key IS NULL AND dispatch_input_hash IS NULL AND
            invocation_sha256 IS NULL AND bound_input_sha256 IS NULL AND
            owner_command_sha256 IS NULL AND temporal_attestation_sha256 IS NULL AND
            temporal_child_started_event_id IS NULL)
        OR (admission_schema_revision = 4 AND
            num_nonnulls(parent_workflow_id, parent_workflow_run_id,
                dispatch_execution_key, dispatch_input_hash,
                invocation_sha256, bound_input_sha256, owner_command_sha256,
                temporal_attestation_sha256, temporal_child_started_event_id) = 9 AND
            length(trim(parent_workflow_id)) > 0 AND
            length(trim(parent_workflow_run_id)) > 0 AND
            length(dispatch_execution_key) = 64 AND
            length(dispatch_input_hash) = 64 AND
            length(invocation_sha256) = 64 AND
            length(bound_input_sha256) = 64 AND
            length(owner_command_sha256) = 64 AND
            length(temporal_attestation_sha256) = 64 AND
            temporal_child_started_event_id > 0)
    );

CREATE UNIQUE INDEX IF NOT EXISTS uq_pg_effect_v4_parent_instance
    ON pipeline.pg_effect_v3_admissions(run_id, plan_hash, operation_instance)
    WHERE admission_schema_revision = 4;

ALTER TABLE pipeline.pg_effect_v3_receipts
    ADD COLUMN IF NOT EXISTS admission_schema_revision SMALLINT NOT NULL DEFAULT 3;
ALTER TABLE pipeline.pg_effect_v3_receipts
    DROP CONSTRAINT IF EXISTS pg_effect_v4_receipt_revision_check,
    ADD CONSTRAINT pg_effect_v4_receipt_revision_check
        CHECK (admission_schema_revision IN (3,4));

-- Preserve append-only semantics for every V4 attestation field, including
-- Temporal server evidence and exact dispatch+invocation hashes.
CREATE OR REPLACE FUNCTION pipeline.guard_pg_effect_v3_admission_mutation()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'TRUNCATE' THEN
        RAISE EXCEPTION 'V3 operator admissions cannot be truncated'
            USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'V3 operator admissions cannot be deleted'
            USING ERRCODE = '23514';
    END IF;
    IF ROW(NEW.admission_id, NEW.run_id, NEW.tenant_id, NEW.context_key,
           NEW.admission_schema_revision,
           NEW.v3_expected_execution_key,
           NEW.workflow_id, NEW.workflow_execution_run_id, NEW.operation_instance,
           NEW.plan_hash, NEW.operation_id, NEW.operation_revision,
           NEW.contract_fingerprint, NEW.authorized_effect, NEW.candidate_set_hash,
           NEW.issued_by, NEW.issued_at, NEW.expires_at,
           NEW.parent_workflow_id, NEW.parent_workflow_run_id,
           NEW.dispatch_execution_key, NEW.dispatch_input_hash,
           NEW.invocation_sha256, NEW.bound_input_sha256,
           NEW.owner_command_sha256, NEW.temporal_attestation_sha256,
           NEW.temporal_child_started_event_id)
       IS DISTINCT FROM
       ROW(OLD.admission_id, OLD.run_id, OLD.tenant_id, OLD.context_key,
           OLD.admission_schema_revision,
           OLD.v3_expected_execution_key,
           OLD.workflow_id, OLD.workflow_execution_run_id, OLD.operation_instance,
           OLD.plan_hash, OLD.operation_id, OLD.operation_revision,
           OLD.contract_fingerprint, OLD.authorized_effect, OLD.candidate_set_hash,
           OLD.issued_by, OLD.issued_at, OLD.expires_at,
           OLD.parent_workflow_id, OLD.parent_workflow_run_id,
           OLD.dispatch_execution_key, OLD.dispatch_input_hash,
           OLD.invocation_sha256, OLD.bound_input_sha256,
           OLD.owner_command_sha256, OLD.temporal_attestation_sha256,
           OLD.temporal_child_started_event_id)
       OR (OLD.revoked_at IS NOT NULL
           AND NEW.revoked_at IS DISTINCT FROM OLD.revoked_at)
    THEN
        RAISE EXCEPTION 'V3/V4 admission authority and revocation are immutable'
            USING ERRCODE = '23514';
    END IF;
    IF NOT pg_try_advisory_xact_lock(
        hashtextextended('pg-v3-admission|' || OLD.admission_id::text, 0)
    ) THEN
        RAISE EXCEPTION 'admission effect in flight; retry revocation'
            USING ERRCODE = '55P03';
    END IF;
    RETURN NEW;
END;
$$;

COMMIT;
