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
