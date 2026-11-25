-- Forward-only V7 security correction. Operator issuance is not enough:
-- a canonical Truth run can be terminal before a delayed provider Activity
-- reaches PostgreSQL. Serialize the FIRST Qdrant attempt with the same
-- advisory key held by the execution_runs status trigger (migration 0002).
-- An existing attempt remains an immutable unknown-outcome barrier even after
-- a terminal transition; it never authorizes a second provider RPC.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.begin_qdrant_projection_v1(
    p_admission_id UUID, p_command_bytes BYTEA
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_existing TEXT; v_receipt RECORD;
BEGIN
    -- A caller-chosen repeatable-read snapshot predating a terminal status
    -- must never be reused to approve fresh external effects.
    IF pg_catalog.current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'Qdrant first-start fence requires READ COMMITTED isolation'
            USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO a FROM pipeline.qdrant_projection_v1_admissions
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user
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
        SELECT * INTO v_receipt FROM pipeline.qdrant_projection_v1_receipts
            WHERE admission_id = p_admission_id;
        IF FOUND THEN
            IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
               OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
               OR v_receipt.point_id IS DISTINCT FROM p_admission_id
               OR v_receipt.exact_provider_readback IS DISTINCT FROM TRUE THEN
                RAISE EXCEPTION 'Qdrant immutable receipt identity is inconsistent'
                    USING ERRCODE = 'AV001';
            END IF;
            RETURN 'confirmed';
        END IF;
        RETURN 'uncertain'; -- no second provider RPC, even after run completion
    END IF;
    IF EXISTS (SELECT 1 FROM pipeline.qdrant_projection_v1_receipts
               WHERE admission_id = p_admission_id) THEN
        RAISE EXCEPTION 'Qdrant receipt exists without an admitted attempt'
            USING ERRCODE = 'AV001';
    END IF;

    -- The execution_runs BEFORE UPDATE/DELETE guard uses pg_try_advisory_xact_lock
    -- on this exact key. A status writer that wins first makes us wait and
    -- observe its terminal status; when this transaction wins first the writer
    -- fails closed with SQLSTATE 55P03. No user-supplied lock key is trusted.
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('truth-run-status|' || a.run_id::text, 0)
    );
    -- A revocation or expiry blocks a NEW provider RPC, never readback or
    -- confirmation of a prior immutable attempt. This permits safe recovery
    -- after a provider succeeded but PostgreSQL confirmation was interrupted.
    IF a.revoked_at IS NOT NULL OR a.expires_at <= clock_timestamp() THEN
        RAISE EXCEPTION 'Qdrant projection first start denied by revoked/expired admission'
            USING ERRCODE = 'AV001';
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
        RAISE EXCEPTION 'Qdrant projection first start denied by canonical run status'
            USING ERRCODE = 'AV001';
    END IF;
    INSERT INTO pipeline.qdrant_projection_v1_attempts (admission_id, tenant_id)
        VALUES (p_admission_id, a.tenant_id);
    RETURN 'started';
END;
$$;

COMMIT;
