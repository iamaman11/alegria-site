-- V6 experimental *second* external-effect owner: an immutable, tenant-scoped
-- Qdrant keyword projection with an explicit unknown-outcome barrier.
--
-- IMPORTANT: this does not authorize production use. Temporal parent/child
-- History must be attested independently by an operator before a real worker
-- role receives EXECUTE privileges. The values below are EXPECTED identities,
-- not cryptographic proof of a Temporal server History.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS pipeline.qdrant_projection_v1_admissions (
    admission_id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES pipeline.execution_runs(run_id),
    tenant_id TEXT NOT NULL CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0),
    context_key TEXT NOT NULL REFERENCES kb.visa_contexts(context_key),
    keyword_key TEXT NOT NULL CHECK (keyword_key = btrim(keyword_key) AND length(keyword_key) > 0),
    parent_workflow_id TEXT NOT NULL CHECK (length(btrim(parent_workflow_id)) > 0),
    parent_workflow_run_id TEXT NOT NULL CHECK (length(btrim(parent_workflow_run_id)) > 0),
    child_workflow_id TEXT NOT NULL CHECK (length(btrim(child_workflow_id)) > 0),
    child_workflow_run_id TEXT NOT NULL CHECK (length(btrim(child_workflow_run_id)) > 0),
    child_started_event_id BIGINT NOT NULL CHECK (child_started_event_id > 0),
    operation_instance TEXT NOT NULL CHECK (length(btrim(operation_instance)) > 0),
    plan_sha256 TEXT NOT NULL CHECK (plan_sha256 ~ '^[0-9a-f]{64}$'),
    command_sha256 TEXT NOT NULL CHECK (command_sha256 ~ '^[0-9a-f]{64}$'),
    -- The operator approves EXACT serialized bytes, never an editable URL or SQL.
    command_bytes BYTEA NOT NULL CHECK (octet_length(command_bytes) > 0),
    issued_by NAME NOT NULL DEFAULT session_user,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    UNIQUE (tenant_id, context_key, keyword_key),
    UNIQUE (run_id, plan_sha256, operation_instance),
    UNIQUE (child_workflow_run_id, operation_instance),
    CHECK (command_sha256 = encode(public.digest(command_bytes, 'sha256'), 'hex')),
    CHECK (expires_at > issued_at)
);

-- After START, provider outcome is UNKNOWN until independent exact readback.
-- An uncertain row MUST NEVER be automatically sent a second provider RPC.
CREATE TABLE IF NOT EXISTS pipeline.qdrant_projection_v1_attempts (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.qdrant_projection_v1_admissions(admission_id),
    tenant_id TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'uncertain' CHECK (state IN ('uncertain', 'confirmed')),
    started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    confirmed_at TIMESTAMPTZ,
    CHECK ((state = 'uncertain' AND confirmed_at IS NULL)
        OR (state = 'confirmed' AND confirmed_at IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS pipeline.qdrant_projection_v1_receipts (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.qdrant_projection_v1_admissions(admission_id),
    tenant_id TEXT NOT NULL,
    command_sha256 TEXT NOT NULL CHECK (command_sha256 ~ '^[0-9a-f]{64}$'),
    point_id UUID NOT NULL UNIQUE,
    collection_name TEXT NOT NULL DEFAULT 'seo_keyword_clusters_4'
        CHECK (collection_name = 'seo_keyword_clusters_4'),
    -- Point was independently retrieved, including all 1024 floats and payload.
    exact_provider_readback BOOLEAN NOT NULL CHECK (exact_provider_readback),
    committed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

REVOKE ALL ON pipeline.qdrant_projection_v1_admissions,
    pipeline.qdrant_projection_v1_attempts,
    pipeline.qdrant_projection_v1_receipts FROM PUBLIC;

-- These are not application-insertable tenant claims. Only DBA-provisioned
-- PostgreSQL LOGINs can issue an admission for their canonical context/run.
CREATE OR REPLACE FUNCTION pipeline.guard_qdrant_projection_admission_v1()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE v_owner TEXT;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        RAISE EXCEPTION 'Qdrant effect admissions are append-only; no delete or rewrite'
            USING ERRCODE = 'AV001';
    END IF;
    v_owner := pipeline.current_session_tenant_v1();
    IF v_owner IS DISTINCT FROM NEW.tenant_id OR NEW.issued_by <> session_user THEN
        RAISE EXCEPTION 'Qdrant effect operator LOGIN is not provisioned for tenant'
            USING ERRCODE = 'AV001';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pipeline.tenant_context_ownership o
        JOIN pipeline.execution_runs r ON r.context_key = o.context_key
        WHERE o.context_key = NEW.context_key AND o.tenant_id = NEW.tenant_id
          AND r.run_id = NEW.run_id AND r.tenant_id = NEW.tenant_id
    ) THEN
        RAISE EXCEPTION 'Qdrant projection context/run has no authoritative tenant owner'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS qdrant_projection_admission_insert_v1
    ON pipeline.qdrant_projection_v1_admissions;
CREATE TRIGGER qdrant_projection_admission_insert_v1
    BEFORE INSERT ON pipeline.qdrant_projection_v1_admissions
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_qdrant_projection_admission_v1();
DROP TRIGGER IF EXISTS qdrant_projection_admission_immutable_v1
    ON pipeline.qdrant_projection_v1_admissions;
CREATE TRIGGER qdrant_projection_admission_immutable_v1
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_v1_admissions
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_qdrant_projection_admission_v1();

-- Operator and worker must use different PostgreSQL LOGINs (both DBA bound to
-- the same tenant). The app role is deliberately granted NOTHING here.
CREATE OR REPLACE FUNCTION pipeline.begin_qdrant_projection_v1(
    p_admission_id UUID, p_command_bytes BYTEA
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_existing TEXT;
BEGIN
    SELECT * INTO a FROM pipeline.qdrant_projection_v1_admissions
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user
       OR a.revoked_at IS NOT NULL OR a.expires_at <= clock_timestamp()
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR encode(public.digest(p_command_bytes, 'sha256'), 'hex') <> a.command_sha256
       OR NOT EXISTS (
            SELECT 1 FROM pipeline.execution_runs r
            JOIN pipeline.tenant_context_ownership o ON o.context_key = r.context_key
            WHERE r.run_id = a.run_id AND r.context_key = a.context_key
              AND r.tenant_id = a.tenant_id AND o.tenant_id = a.tenant_id
       )
    THEN
        RAISE EXCEPTION 'Qdrant projection admission denied or stale' USING ERRCODE = 'AV001';
    END IF;
    SELECT state INTO v_existing FROM pipeline.qdrant_projection_v1_attempts
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF FOUND THEN
        RETURN v_existing; -- uncertain means NEVER retry the RPC blindly
    END IF;
    INSERT INTO pipeline.qdrant_projection_v1_attempts (admission_id, tenant_id)
        VALUES (p_admission_id, a.tenant_id);
    RETURN 'started';
END;
$$;

CREATE OR REPLACE FUNCTION pipeline.confirm_qdrant_projection_v1(
    p_admission_id UUID, p_command_bytes BYTEA
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_attempt RECORD;
BEGIN
    SELECT * INTO a FROM pipeline.qdrant_projection_v1_admissions
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR encode(public.digest(p_command_bytes, 'sha256'), 'hex') <> a.command_sha256
    THEN
        RAISE EXCEPTION 'Qdrant projection confirmation denied' USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO v_attempt FROM pipeline.qdrant_projection_v1_attempts
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR v_attempt.tenant_id IS DISTINCT FROM a.tenant_id THEN
        RAISE EXCEPTION 'provider RPC was never admitted' USING ERRCODE = 'AV001';
    END IF;
    IF v_attempt.state = 'confirmed' THEN
        RETURN TRUE;
    END IF;
    INSERT INTO pipeline.qdrant_projection_v1_receipts (
        admission_id, tenant_id, command_sha256, point_id, exact_provider_readback
    ) VALUES (p_admission_id, a.tenant_id, a.command_sha256, p_admission_id, TRUE);
    UPDATE pipeline.qdrant_projection_v1_attempts
       SET state = 'confirmed', confirmed_at = clock_timestamp()
     WHERE admission_id = p_admission_id;
    RETURN TRUE;
END;
$$;

REVOKE ALL ON FUNCTION pipeline.begin_qdrant_projection_v1(UUID, BYTEA) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.confirm_qdrant_projection_v1(UUID, BYTEA) FROM PUBLIC;

-- Role-identity RLS, never GUC identity. Superuser/owner bypass remains an
-- explicit production NO-GO until a separately attested worker is provisioned.
ALTER TABLE pipeline.qdrant_projection_v1_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.qdrant_projection_v1_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.qdrant_projection_v1_receipts ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS qdrant_admission_tenant_v1 ON pipeline.qdrant_projection_v1_admissions;
CREATE POLICY qdrant_admission_tenant_v1 ON pipeline.qdrant_projection_v1_admissions
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS qdrant_attempt_tenant_v1 ON pipeline.qdrant_projection_v1_attempts;
CREATE POLICY qdrant_attempt_tenant_v1 ON pipeline.qdrant_projection_v1_attempts
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS qdrant_receipt_tenant_v1 ON pipeline.qdrant_projection_v1_receipts;
CREATE POLICY qdrant_receipt_tenant_v1 ON pipeline.qdrant_projection_v1_receipts
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

COMMIT;
