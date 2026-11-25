-- Opt-in V6 mixed-owner DAG: a NEW Qdrant V2 admission namespace. V1 is
-- frozen for History replay and cannot be relabelled as a checked DAG effect.
-- Operator credentials and execution privileges are provisioned separately
-- by a DBA. No production cutover or automatic grant is performed here.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE TABLE pipeline.qdrant_projection_v2_admissions (
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
    plan_hash TEXT NOT NULL CHECK (plan_hash ~ '^[0-9a-f]{64}$'),
    dispatch_execution_key TEXT NOT NULL CHECK (dispatch_execution_key ~ '^[0-9a-f]{64}$'),
    parent_sha256 TEXT NOT NULL CHECK (parent_sha256 ~ '^[0-9a-f]{64}$'),
    child_sha256 TEXT NOT NULL CHECK (child_sha256 ~ '^[0-9a-f]{64}$'),
    command_sha256 TEXT NOT NULL CHECK (command_sha256 ~ '^[0-9a-f]{64}$'),
    command_bytes BYTEA NOT NULL CHECK (octet_length(command_bytes) > 0 AND octet_length(command_bytes) <= 2 * 1024 * 1024),
    issued_by NAME NOT NULL DEFAULT session_user,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    -- A canonical keyword currently has one immutable projection identity;
    -- future update semantics need their own versioned owner contract.
    UNIQUE (tenant_id, context_key, keyword_key),
    UNIQUE (run_id, plan_hash, operation_instance),
    UNIQUE (child_workflow_run_id, operation_instance),
    CHECK (command_sha256 = encode(public.digest(command_bytes, 'sha256'), 'hex')),
    CHECK (expires_at > issued_at)
);

CREATE TABLE pipeline.qdrant_projection_v2_attempts (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.qdrant_projection_v2_admissions(admission_id),
    tenant_id TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE pipeline.qdrant_projection_v2_receipts (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.qdrant_projection_v2_admissions(admission_id),
    tenant_id TEXT NOT NULL,
    command_sha256 TEXT NOT NULL CHECK (command_sha256 ~ '^[0-9a-f]{64}$'),
    dispatch_execution_key TEXT NOT NULL CHECK (dispatch_execution_key ~ '^[0-9a-f]{64}$'),
    point_id UUID NOT NULL UNIQUE,
    collection_name TEXT NOT NULL DEFAULT 'seo_keyword_clusters_4'
        CHECK (collection_name = 'seo_keyword_clusters_4'),
    exact_provider_readback BOOLEAN NOT NULL CHECK (exact_provider_readback),
    committed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

REVOKE ALL ON pipeline.qdrant_projection_v2_admissions,
    pipeline.qdrant_projection_v2_attempts,
    pipeline.qdrant_projection_v2_receipts FROM PUBLIC;

CREATE FUNCTION pipeline.guard_qdrant_projection_v2_admission()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE v_owner TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'V2 Qdrant admission cannot be deleted'
            USING ERRCODE = 'AV001';
    END IF;
    v_owner := pipeline.current_session_tenant_v1();
    IF TG_OP = 'UPDATE' THEN
        IF v_owner IS DISTINCT FROM OLD.tenant_id
           OR OLD.issued_by <> session_user
           OR OLD.revoked_at IS NOT NULL
           OR NEW.revoked_at IS NULL
           OR (to_jsonb(NEW) - 'revoked_at') IS DISTINCT FROM
              (to_jsonb(OLD) - 'revoked_at') THEN
            RAISE EXCEPTION 'V2 Qdrant admission allows only issuer-owned monotonic revocation'
                USING ERRCODE = 'AV001';
        END IF;
        NEW.revoked_at := clock_timestamp();
        RETURN NEW;
    END IF;
    IF v_owner IS DISTINCT FROM NEW.tenant_id
       OR NEW.issued_by <> session_user
       OR NEW.child_workflow_id <> 'alegria-qdrant-effect-v2-' || NEW.admission_id::text
       OR NOT EXISTS (
           SELECT 1 FROM pipeline.tenant_context_ownership o
           JOIN pipeline.execution_runs r ON r.context_key = o.context_key
           WHERE o.context_key = NEW.context_key AND o.tenant_id = NEW.tenant_id
             AND r.run_id = NEW.run_id AND r.tenant_id = NEW.tenant_id
       ) THEN
        RAISE EXCEPTION 'V2 Qdrant operator/tenant/child identity not admitted'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_qdrant_projection_v2_admission() FROM PUBLIC;
CREATE TRIGGER guard_qdrant_projection_v2_admission
    BEFORE INSERT OR UPDATE OR DELETE ON pipeline.qdrant_projection_v2_admissions
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_qdrant_projection_v2_admission();

CREATE FUNCTION pipeline.deny_qdrant_projection_v2_ledger_rewrite()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'Qdrant V2 attempt and receipt ledgers are append-only'
        USING ERRCODE = '23514';
END;
$$;
REVOKE ALL ON FUNCTION pipeline.deny_qdrant_projection_v2_ledger_rewrite() FROM PUBLIC;
CREATE TRIGGER deny_qdrant_projection_v2_attempt_rewrite
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_v2_attempts
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_qdrant_projection_v2_ledger_rewrite();
CREATE TRIGGER deny_qdrant_projection_v2_receipt_rewrite
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_v2_receipts
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_qdrant_projection_v2_ledger_rewrite();

CREATE FUNCTION pipeline.begin_qdrant_projection_v2(
    p_admission_id UUID, p_command_bytes BYTEA, p_dispatch_execution_key TEXT
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_attempt RECORD; v_receipt RECORD;
BEGIN
    -- Never approve a fresh effect against a stale repeatable-read snapshot.
    IF pg_catalog.current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'Qdrant V2 first-start requires READ COMMITTED isolation'
            USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO a FROM pipeline.qdrant_projection_v2_admissions
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR a.dispatch_execution_key IS DISTINCT FROM p_dispatch_execution_key
       OR encode(public.digest(p_command_bytes, 'sha256'), 'hex') <> a.command_sha256
       OR NOT EXISTS (
            SELECT 1 FROM pipeline.execution_runs r
            JOIN pipeline.tenant_context_ownership o ON o.context_key = r.context_key
            WHERE r.run_id = a.run_id AND r.context_key = a.context_key
              AND r.tenant_id = a.tenant_id AND o.tenant_id = a.tenant_id
       ) THEN
        RAISE EXCEPTION 'V2 checked Qdrant admission denied or stale' USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO v_attempt FROM pipeline.qdrant_projection_v2_attempts
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF FOUND THEN
        SELECT * INTO v_receipt FROM pipeline.qdrant_projection_v2_receipts
            WHERE admission_id = p_admission_id;
        IF FOUND THEN
            IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
               OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
               OR v_receipt.dispatch_execution_key IS DISTINCT FROM a.dispatch_execution_key
               OR v_receipt.point_id IS DISTINCT FROM p_admission_id
               OR v_receipt.exact_provider_readback IS DISTINCT FROM TRUE THEN
                RAISE EXCEPTION 'V2 immutable Qdrant receipt identity is inconsistent'
                    USING ERRCODE = 'AV001';
            END IF;
            RETURN 'confirmed';
        END IF;
        RETURN 'uncertain'; -- UNKNOWN RPC outcome; no second upsert.
    END IF;
    IF EXISTS (
        SELECT 1 FROM pipeline.qdrant_projection_v2_receipts
        WHERE admission_id = p_admission_id
    ) THEN
        RAISE EXCEPTION 'V2 receipt without immutable attempt' USING ERRCODE = 'AV001';
    END IF;
    -- Exactly the same advisory key as the execution_runs status trigger.
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('truth-run-status|' || a.run_id::text, 0)
    );
    IF a.revoked_at IS NOT NULL OR a.expires_at <= clock_timestamp() THEN
        RAISE EXCEPTION 'V2 Qdrant first start revoked/expired' USING ERRCODE = 'AV001';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pipeline.execution_runs r
        JOIN pipeline.tenant_context_ownership o ON o.context_key = r.context_key
        WHERE r.run_id = a.run_id AND r.context_key = a.context_key
          AND r.tenant_id = a.tenant_id AND o.tenant_id = a.tenant_id
          AND r.workflow_run_id = a.parent_workflow_run_id
          AND r.workflow_type = 'seo_site_build'
          AND r.status IN ('verifying', 'pending_hitl')
        FOR SHARE OF r
    ) THEN
        RAISE EXCEPTION 'V2 Qdrant first start denied by canonical run status'
            USING ERRCODE = 'AV001';
    END IF;
    INSERT INTO pipeline.qdrant_projection_v2_attempts (admission_id, tenant_id)
        VALUES (p_admission_id, a.tenant_id);
    RETURN 'started';
END;
$$;

CREATE FUNCTION pipeline.confirm_qdrant_projection_v2(
    p_admission_id UUID, p_command_bytes BYTEA, p_dispatch_execution_key TEXT
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_attempt RECORD; v_receipt RECORD;
BEGIN
    SELECT * INTO a FROM pipeline.qdrant_projection_v2_admissions
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR a.dispatch_execution_key IS DISTINCT FROM p_dispatch_execution_key
       OR encode(public.digest(p_command_bytes, 'sha256'), 'hex') <> a.command_sha256
    THEN
        RAISE EXCEPTION 'V2 Qdrant confirmation denied' USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO v_attempt FROM pipeline.qdrant_projection_v2_attempts
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR v_attempt.tenant_id IS DISTINCT FROM a.tenant_id THEN
        RAISE EXCEPTION 'V2 provider RPC was never admitted' USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO v_receipt FROM pipeline.qdrant_projection_v2_receipts
        WHERE admission_id = p_admission_id;
    IF FOUND THEN
        IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
           OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
           OR v_receipt.dispatch_execution_key IS DISTINCT FROM a.dispatch_execution_key
           OR v_receipt.point_id IS DISTINCT FROM p_admission_id
           OR v_receipt.exact_provider_readback IS DISTINCT FROM TRUE THEN
            RAISE EXCEPTION 'V2 immutable Qdrant receipt identity inconsistent'
                USING ERRCODE = 'AV001';
        END IF;
        RETURN TRUE;
    END IF;
    -- The provider readback was completed independently before this call.
    -- Never mark a mutable attempt confirmed or retry the provider here.
    INSERT INTO pipeline.qdrant_projection_v2_receipts (
        admission_id, tenant_id, command_sha256, dispatch_execution_key,
        point_id, exact_provider_readback
    ) VALUES (
        p_admission_id, a.tenant_id, a.command_sha256,
        a.dispatch_execution_key, p_admission_id, TRUE
    );
    RETURN TRUE;
END;
$$;

CREATE FUNCTION pipeline.verify_qdrant_operator_resignal_v2(
    p_admission_id UUID, p_run_id UUID, p_tenant_id TEXT,
    p_context_key TEXT, p_keyword_key TEXT,
    p_parent_workflow_id TEXT, p_parent_workflow_run_id TEXT,
    p_child_workflow_id TEXT, p_child_workflow_run_id TEXT,
    p_child_started_event_id BIGINT,
    p_parent_bytes BYTEA, p_child_bytes BYTEA, p_command_bytes BYTEA,
    p_dispatch_execution_key TEXT
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE v_actor RECORD;
BEGIN
    SELECT r.rolsuper, r.rolbypassrls INTO v_actor
        FROM pg_roles r WHERE r.rolname = session_user;
    IF NOT FOUND OR v_actor.rolsuper OR v_actor.rolbypassrls
       OR pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
       OR octet_length(p_parent_bytes) > 2 * 1024 * 1024
       OR octet_length(p_child_bytes) > 2 * 1024 * 1024
       OR octet_length(p_command_bytes) > 2 * 1024 * 1024
       OR NOT has_table_privilege(session_user,
           'pipeline.qdrant_projection_v2_admissions', 'INSERT')
       OR NOT EXISTS (
           SELECT 1 FROM pipeline.qdrant_projection_v2_admissions a
           JOIN pipeline.execution_runs er ON er.run_id = a.run_id
           JOIN pipeline.tenant_context_ownership o
             ON o.context_key = a.context_key AND o.tenant_id = a.tenant_id
           WHERE a.admission_id = p_admission_id
             AND a.issued_by = session_user
             AND a.run_id = p_run_id
             AND a.tenant_id = p_tenant_id
             AND a.context_key = p_context_key
             AND a.keyword_key = p_keyword_key
             AND a.parent_workflow_id = p_parent_workflow_id
             AND a.parent_workflow_run_id = p_parent_workflow_run_id
             AND a.child_workflow_id = p_child_workflow_id
             AND a.child_workflow_run_id = p_child_workflow_run_id
             AND a.child_started_event_id = p_child_started_event_id
             AND a.parent_sha256 = encode(public.digest(p_parent_bytes,'sha256'),'hex')
             AND a.child_sha256 = encode(public.digest(p_child_bytes,'sha256'),'hex')
             AND a.command_sha256 = encode(public.digest(p_command_bytes,'sha256'),'hex')
             AND a.command_bytes = p_command_bytes
             AND a.dispatch_execution_key = p_dispatch_execution_key
             AND a.revoked_at IS NULL AND a.expires_at > clock_timestamp()
             AND er.tenant_id = p_tenant_id AND er.context_key = p_context_key
             AND er.workflow_run_id = p_parent_workflow_run_id
             AND er.workflow_type = 'seo_site_build'
             AND er.status IN ('verifying','pending_hitl')
       ) THEN
        RAISE EXCEPTION 'V2 Qdrant operator resignal denied'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN TRUE;
END;
$$;

REVOKE ALL ON FUNCTION pipeline.begin_qdrant_projection_v2(UUID,BYTEA,TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.confirm_qdrant_projection_v2(UUID,BYTEA,TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.verify_qdrant_operator_resignal_v2(
    UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BIGINT,BYTEA,BYTEA,BYTEA,TEXT
) FROM PUBLIC;

ALTER TABLE pipeline.qdrant_projection_v2_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.qdrant_projection_v2_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.qdrant_projection_v2_receipts ENABLE ROW LEVEL SECURITY;
CREATE POLICY qdrant_v2_admission_tenant ON pipeline.qdrant_projection_v2_admissions
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
CREATE POLICY qdrant_v2_attempt_tenant ON pipeline.qdrant_projection_v2_attempts
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
CREATE POLICY qdrant_v2_receipt_tenant ON pipeline.qdrant_projection_v2_receipts
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

COMMIT;
