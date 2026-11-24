-- Explicit opt-in upgrade for the standalone PostgreSQL Truth-review V3
-- effect owner. This migration is NOT a deployment switch: production starts
-- no new worker until role provisioning, admission issuance and tests pass.
-- `app/db/schema.sql` carries the equivalent fresh-install definitions.

CREATE TABLE IF NOT EXISTS pipeline.pg_effect_v3_admissions (
    admission_id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id                 UUID NOT NULL REFERENCES pipeline.execution_runs(run_id),
    tenant_id              TEXT NOT NULL CHECK (length(trim(tenant_id)) > 0),
    context_key            TEXT NOT NULL CHECK (length(trim(context_key)) > 0),
    workflow_id            TEXT NOT NULL CHECK (length(trim(workflow_id)) > 0),
    workflow_execution_run_id TEXT NOT NULL CHECK (length(trim(workflow_execution_run_id)) > 0),
    operation_instance     TEXT NOT NULL CHECK (length(trim(operation_instance)) > 0),
    plan_hash              TEXT NOT NULL CHECK (length(plan_hash) = 64),
    operation_id           TEXT NOT NULL DEFAULT 'pg.truth.reconcile_candidate_reviews'
        CHECK (operation_id = 'pg.truth.reconcile_candidate_reviews'),
    operation_revision     INTEGER NOT NULL DEFAULT 1 CHECK (operation_revision = 1),
    contract_fingerprint   TEXT NOT NULL CHECK (length(contract_fingerprint) = 64),
    authorized_effect      TEXT NOT NULL DEFAULT 'pg.truth.review_queue.reconcile'
        CHECK (authorized_effect = 'pg.truth.review_queue.reconcile'),
    candidate_set_hash     TEXT NOT NULL CHECK (length(candidate_set_hash) = 64),
    issued_by              TEXT NOT NULL CHECK (length(trim(issued_by)) > 0 AND issued_by <> 'seo_system'),
    issued_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at             TIMESTAMPTZ NOT NULL,
    revoked_at             TIMESTAMPTZ,
    UNIQUE (workflow_execution_run_id, operation_instance)
);

CREATE TABLE IF NOT EXISTS pipeline.pg_effect_v3_receipts (
    receipt_id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    admission_id           UUID NOT NULL UNIQUE REFERENCES pipeline.pg_effect_v3_admissions(admission_id),
    execution_key          TEXT NOT NULL UNIQUE CHECK (length(execution_key) = 64),
    operation_id           TEXT NOT NULL CHECK (operation_id = 'pg.truth.reconcile_candidate_reviews'),
    operation_revision     INTEGER NOT NULL CHECK (operation_revision = 1),
    run_id                 UUID NOT NULL REFERENCES pipeline.execution_runs(run_id),
    context_key            TEXT NOT NULL,
    candidate_set_hash     TEXT NOT NULL,
    created_review_tasks   BIGINT NOT NULL CHECK (created_review_tasks >= 0),
    open_review_tasks      BIGINT NOT NULL CHECK (open_review_tasks >= 0),
    committed_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

REVOKE INSERT, UPDATE, DELETE, TRUNCATE ON pipeline.pg_effect_v3_admissions FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE, TRUNCATE ON pipeline.pg_effect_v3_receipts FROM PUBLIC;

-- Same append-only, serialized operator-admission fence as fresh schema.sql.
-- No Activity or untrusted DAG can mutate admission authority.
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
           NEW.workflow_id, NEW.workflow_execution_run_id, NEW.operation_instance,
           NEW.plan_hash, NEW.operation_id, NEW.operation_revision,
           NEW.contract_fingerprint, NEW.authorized_effect, NEW.candidate_set_hash,
           NEW.issued_by, NEW.issued_at, NEW.expires_at)
       IS DISTINCT FROM
       ROW(OLD.admission_id, OLD.run_id, OLD.tenant_id, OLD.context_key,
           OLD.workflow_id, OLD.workflow_execution_run_id, OLD.operation_instance,
           OLD.plan_hash, OLD.operation_id, OLD.operation_revision,
           OLD.contract_fingerprint, OLD.authorized_effect, OLD.candidate_set_hash,
           OLD.issued_by, OLD.issued_at, OLD.expires_at)
       OR (OLD.revoked_at IS NOT NULL
           AND NEW.revoked_at IS DISTINCT FROM OLD.revoked_at)
    THEN
        RAISE EXCEPTION 'V3 admission authority and completed revocation are immutable'
            USING ERRCODE = '23514';
    END IF;

    IF NOT pg_try_advisory_xact_lock(
        hashtextextended('pg-v3-admission|' || OLD.admission_id::text, 0)
    ) THEN
        RAISE EXCEPTION 'V3 admission effect in flight; retry revocation'
            USING ERRCODE = '55P03';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS guard_pg_effect_v3_admission_update_delete
    ON pipeline.pg_effect_v3_admissions;
CREATE TRIGGER guard_pg_effect_v3_admission_update_delete
BEFORE UPDATE OR DELETE ON pipeline.pg_effect_v3_admissions
FOR EACH ROW EXECUTE FUNCTION pipeline.guard_pg_effect_v3_admission_mutation();

DROP TRIGGER IF EXISTS guard_pg_effect_v3_admission_truncate
    ON pipeline.pg_effect_v3_admissions;
CREATE TRIGGER guard_pg_effect_v3_admission_truncate
BEFORE TRUNCATE ON pipeline.pg_effect_v3_admissions
FOR EACH STATEMENT EXECUTE FUNCTION pipeline.guard_pg_effect_v3_admission_mutation();
