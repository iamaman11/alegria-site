-- Forward-only V6 correction: an immutable provider-attempt barrier is never
-- reopened by a DELETE/UPDATE, even when an executor LOGIN is overgranted.
-- Successful confirmation is DERIVED from the append-only receipt, not from
-- a mutable attempts.state bit. Historical confirmed attempts are preserved.
-- Admission revocation is a monotonic issuer-only NULL -> server-time change;
-- it denies a *new* begin, never claims to cancel an already-started RPC.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.guard_qdrant_projection_admission_v1()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE v_owner TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'Qdrant effect admissions cannot be deleted'
            USING ERRCODE = 'AV001';
    END IF;
    v_owner := pipeline.current_session_tenant_v1();
    IF TG_OP = 'UPDATE' THEN
        IF v_owner IS DISTINCT FROM OLD.tenant_id OR OLD.issued_by <> session_user
           OR OLD.revoked_at IS NOT NULL OR NEW.revoked_at IS NULL
           OR (to_jsonb(NEW) - 'revoked_at') IS DISTINCT FROM
              (to_jsonb(OLD) - 'revoked_at') THEN
            RAISE EXCEPTION 'Qdrant admission allows only issuer-owned monotonic revocation'
                USING ERRCODE = 'AV001';
        END IF;
        -- Never trust a caller-selected timestamp, including a future date.
        NEW.revoked_at := clock_timestamp();
        RETURN NEW;
    END IF;
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

CREATE OR REPLACE FUNCTION pipeline.deny_qdrant_projection_ledger_rewrite_v1()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'Qdrant provider attempt and receipt ledgers are append-only'
        USING ERRCODE = '23514';
END;
$$;
REVOKE ALL ON FUNCTION pipeline.deny_qdrant_projection_ledger_rewrite_v1() FROM PUBLIC;
DROP TRIGGER IF EXISTS deny_qdrant_projection_attempt_rewrite_v1
    ON pipeline.qdrant_projection_v1_attempts;
CREATE TRIGGER deny_qdrant_projection_attempt_rewrite_v1
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_v1_attempts
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_qdrant_projection_ledger_rewrite_v1();
DROP TRIGGER IF EXISTS deny_qdrant_projection_receipt_rewrite_v1
    ON pipeline.qdrant_projection_v1_receipts;
CREATE TRIGGER deny_qdrant_projection_receipt_rewrite_v1
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_v1_receipts
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_qdrant_projection_ledger_rewrite_v1();

CREATE OR REPLACE FUNCTION pipeline.begin_qdrant_projection_v1(
    p_admission_id UUID, p_command_bytes BYTEA
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_existing TEXT; v_receipt RECORD;
BEGIN
    -- Row lock serializes a first start against issuer revocation. If begin
    -- wins, a later revocation cannot recall an in-flight provider RPC.
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
        RETURN 'uncertain'; -- no second provider RPC, even for legacy state
    END IF;
    IF EXISTS (SELECT 1 FROM pipeline.qdrant_projection_v1_receipts
               WHERE admission_id = p_admission_id) THEN
        RAISE EXCEPTION 'Qdrant receipt exists without an admitted attempt'
            USING ERRCODE = 'AV001';
    END IF;
    INSERT INTO pipeline.qdrant_projection_v1_attempts (admission_id, tenant_id)
        VALUES (p_admission_id, a.tenant_id);
    RETURN 'started';
END;
$$;

CREATE OR REPLACE FUNCTION pipeline.confirm_qdrant_projection_v1(
    p_admission_id UUID, p_command_bytes BYTEA
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_attempt RECORD; v_receipt RECORD;
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
    -- Revocation after the durable start cannot undo Qdrant I/O. Reconcile
    -- and record the observed result even if the operator has since revoked.
    SELECT * INTO v_attempt FROM pipeline.qdrant_projection_v1_attempts
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR v_attempt.tenant_id IS DISTINCT FROM a.tenant_id THEN
        RAISE EXCEPTION 'provider RPC was never admitted' USING ERRCODE = 'AV001';
    END IF;
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
        RETURN TRUE;
    END IF;
    INSERT INTO pipeline.qdrant_projection_v1_receipts (
        admission_id, tenant_id, command_sha256, point_id, exact_provider_readback
    ) VALUES (p_admission_id, a.tenant_id, a.command_sha256, p_admission_id, TRUE);
    -- Do not mutate the uncertainty barrier. Durable confirmation is exactly
    -- the existence of this append-only receipt, including historical rows.
    RETURN TRUE;
END;
$$;

COMMIT;
