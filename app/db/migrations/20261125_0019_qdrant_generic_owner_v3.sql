-- Opt-in native owner-neutral DAG: independent Qdrant V3 admission namespace.
-- Legacy V1/V2 histories and operator interfaces remain unchanged.
-- Operator credentials and execution privileges are provisioned separately
-- by a DBA. No production cutover or automatic grant is performed here.
BEGIN;
SET LOCAL lock_timeout = '5s';

-- A new V3-only UNIQUE would not stop an old V1/V2 operator racing with
-- V3 on the same provider business key. Exclusively lock issuance on both
-- old tables while atomically backfilling and installing reservation triggers.
-- Existing rows and immutable history remain unchanged. If an old V1/V2
-- conflict already exists, FAIL migration rather than silently selecting one.
LOCK TABLE pipeline.qdrant_projection_v1_admissions,
    pipeline.qdrant_projection_v2_admissions IN SHARE ROW EXCLUSIVE MODE;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM (
            SELECT tenant_id, context_key, keyword_key
            FROM pipeline.qdrant_projection_v1_admissions
            UNION ALL
            SELECT tenant_id, context_key, keyword_key
            FROM pipeline.qdrant_projection_v2_admissions
        ) old_business_keys
        GROUP BY tenant_id, context_key, keyword_key
        HAVING count(*) > 1
    ) THEN
        RAISE EXCEPTION
            'Qdrant V1/V2 contain conflicting immutable business-key admissions; V3 migration refused'
            USING ERRCODE = '23505';
    END IF;
    -- Every revision writes point_id = admission_id into the SAME Qdrant
    -- collection. Reusing a UUID for a different business key would silently
    -- overwrite that other point even if business keys were both unique.
    IF EXISTS (
        SELECT 1 FROM (
            SELECT admission_id
            FROM pipeline.qdrant_projection_v1_admissions
            UNION ALL
            SELECT admission_id
            FROM pipeline.qdrant_projection_v2_admissions
        ) old_point_ids
        GROUP BY admission_id
        HAVING count(*) > 1
    ) THEN
        RAISE EXCEPTION
            'Qdrant V1/V2 reuse a provider point UUID across versions; V3 migration refused'
            USING ERRCODE = '23505';
    END IF;
END;
$$;

CREATE TABLE pipeline.qdrant_projection_business_key_reservations_v3 (
    tenant_id TEXT NOT NULL,
    context_key TEXT NOT NULL,
    keyword_key TEXT NOT NULL,
    originating_revision SMALLINT NOT NULL
        CHECK (originating_revision IN (1,2,3)),
    admission_id UUID NOT NULL,
    reserved_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, context_key, keyword_key),
    -- One provider point UUID cannot belong to different business keys even
    -- if two admission revisions use the same random UUID.
    UNIQUE (admission_id)
);
REVOKE ALL ON pipeline.qdrant_projection_business_key_reservations_v3 FROM PUBLIC;
ALTER TABLE pipeline.qdrant_projection_business_key_reservations_v3 ENABLE ROW LEVEL SECURITY;
CREATE POLICY qdrant_business_reservation_v3_tenant
    ON pipeline.qdrant_projection_business_key_reservations_v3
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

INSERT INTO pipeline.qdrant_projection_business_key_reservations_v3
    (tenant_id,context_key,keyword_key,originating_revision,admission_id)
SELECT tenant_id,context_key,keyword_key,1,admission_id
    FROM pipeline.qdrant_projection_v1_admissions
UNION ALL
SELECT tenant_id,context_key,keyword_key,2,admission_id
    FROM pipeline.qdrant_projection_v2_admissions;

-- Migration self-test: every historical admission has exactly one immutable
-- reservation. Row counts are stable under the old-table issuance locks.
DO $$
DECLARE v_admissions BIGINT; v_reservations BIGINT;
BEGIN
    SELECT
        (SELECT count(*) FROM pipeline.qdrant_projection_v1_admissions)
        + (SELECT count(*) FROM pipeline.qdrant_projection_v2_admissions)
    INTO v_admissions;
    SELECT count(*) INTO v_reservations
        FROM pipeline.qdrant_projection_business_key_reservations_v3;
    IF v_admissions <> v_reservations THEN
        RAISE EXCEPTION 'Qdrant historical cross-version reservation mismatch'
            USING ERRCODE = 'AV001';
    END IF;
END;
$$;

-- Immutable reservations deliberately survive admission revocation, expiry,
-- committed receipts and retries. Reprojection of a keyword needs a future
-- authoritative source_revision plus stable point identity, not an ad hoc
-- DELETE of this tombstone-like fence.
CREATE FUNCTION pipeline.deny_qdrant_business_key_reservation_rewrite_v3()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'Qdrant cross-version business-key reservation is immutable'
        USING ERRCODE = '23514';
END;
$$;
REVOKE ALL ON FUNCTION pipeline.deny_qdrant_business_key_reservation_rewrite_v3() FROM PUBLIC;
CREATE TRIGGER deny_qdrant_business_key_reservation_update_v3
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_business_key_reservations_v3
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_qdrant_business_key_reservation_rewrite_v3();
CREATE TRIGGER deny_qdrant_business_key_reservation_truncate_v3
    BEFORE TRUNCATE ON pipeline.qdrant_projection_business_key_reservations_v3
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_qdrant_business_key_reservation_rewrite_v3();

CREATE FUNCTION pipeline.reserve_qdrant_business_key_v3()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE v_revision SMALLINT;
BEGIN
    v_revision := CASE TG_TABLE_NAME
        WHEN 'qdrant_projection_v1_admissions' THEN 1
        WHEN 'qdrant_projection_v2_admissions' THEN 2
        WHEN 'qdrant_projection_v3_admissions' THEN 3
        ELSE NULL
    END;
    IF v_revision IS NULL
       OR pipeline.current_session_tenant_v1() IS DISTINCT FROM NEW.tenant_id
       OR NEW.issued_by <> session_user THEN
        RAISE EXCEPTION 'unauthorized Qdrant cross-version reservation'
            USING ERRCODE = 'AV001';
    END IF;
    -- PRIMARY KEY serializes concurrent issuers even across revisions.
    -- No ON CONFLICT and no actor-selected revision can overwrite a point.
    INSERT INTO pipeline.qdrant_projection_business_key_reservations_v3
        (tenant_id,context_key,keyword_key,originating_revision,admission_id)
    VALUES (NEW.tenant_id,NEW.context_key,NEW.keyword_key,v_revision,NEW.admission_id);
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.reserve_qdrant_business_key_v3() FROM PUBLIC;
CREATE TRIGGER reserve_qdrant_v1_business_key_v3
    BEFORE INSERT ON pipeline.qdrant_projection_v1_admissions
    FOR EACH ROW EXECUTE FUNCTION pipeline.reserve_qdrant_business_key_v3();
CREATE TRIGGER reserve_qdrant_v2_business_key_v3
    BEFORE INSERT ON pipeline.qdrant_projection_v2_admissions
    FOR EACH ROW EXECUTE FUNCTION pipeline.reserve_qdrant_business_key_v3();

CREATE TABLE pipeline.qdrant_projection_v3_admissions (
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
    -- Also enforced across V1/V2/V3 by an append-only global reservation.
    UNIQUE (tenant_id, context_key, keyword_key),
    UNIQUE (run_id, plan_hash, operation_instance),
    UNIQUE (child_workflow_run_id, operation_instance),
    CHECK (command_sha256 = encode(public.digest(command_bytes, 'sha256'), 'hex')),
    CHECK (expires_at > issued_at)
);

CREATE TABLE pipeline.qdrant_projection_v3_attempts (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.qdrant_projection_v3_admissions(admission_id),
    tenant_id TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE pipeline.qdrant_projection_v3_receipts (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.qdrant_projection_v3_admissions(admission_id),
    tenant_id TEXT NOT NULL,
    command_sha256 TEXT NOT NULL CHECK (command_sha256 ~ '^[0-9a-f]{64}$'),
    dispatch_execution_key TEXT NOT NULL CHECK (dispatch_execution_key ~ '^[0-9a-f]{64}$'),
    point_id UUID NOT NULL UNIQUE,
    collection_name TEXT NOT NULL DEFAULT 'seo_keyword_clusters_4'
        CHECK (collection_name = 'seo_keyword_clusters_4'),
    exact_provider_readback BOOLEAN NOT NULL CHECK (exact_provider_readback),
    committed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

REVOKE ALL ON pipeline.qdrant_projection_v3_admissions,
    pipeline.qdrant_projection_v3_attempts,
    pipeline.qdrant_projection_v3_receipts FROM PUBLIC;

CREATE FUNCTION pipeline.guard_qdrant_projection_v3_admission()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE v_owner TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'V3 Qdrant admission cannot be deleted'
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
            RAISE EXCEPTION 'V3 Qdrant admission allows only issuer-owned monotonic revocation'
                USING ERRCODE = 'AV001';
        END IF;
        NEW.revoked_at := clock_timestamp();
        RETURN NEW;
    END IF;
    IF v_owner IS DISTINCT FROM NEW.tenant_id
       OR NEW.issued_by <> session_user
       OR NEW.child_workflow_id <> 'alegria-owner-operation-v1-' || NEW.dispatch_execution_key
       OR NOT EXISTS (
           SELECT 1 FROM pipeline.tenant_context_ownership o
           JOIN pipeline.execution_runs r ON r.context_key = o.context_key
           WHERE o.context_key = NEW.context_key AND o.tenant_id = NEW.tenant_id
             AND r.run_id = NEW.run_id AND r.tenant_id = NEW.tenant_id
       ) THEN
        RAISE EXCEPTION 'V3 Qdrant operator/tenant/child identity not admitted'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_qdrant_projection_v3_admission() FROM PUBLIC;
CREATE TRIGGER guard_qdrant_projection_v3_admission
    BEFORE INSERT OR UPDATE OR DELETE ON pipeline.qdrant_projection_v3_admissions
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_qdrant_projection_v3_admission();
CREATE TRIGGER reserve_qdrant_v3_business_key_v3
    BEFORE INSERT ON pipeline.qdrant_projection_v3_admissions
    FOR EACH ROW EXECUTE FUNCTION pipeline.reserve_qdrant_business_key_v3();

CREATE FUNCTION pipeline.deny_qdrant_projection_v3_ledger_rewrite()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'Qdrant V3 attempt and receipt ledgers are append-only'
        USING ERRCODE = '23514';
END;
$$;
REVOKE ALL ON FUNCTION pipeline.deny_qdrant_projection_v3_ledger_rewrite() FROM PUBLIC;
CREATE TRIGGER deny_qdrant_projection_v3_attempt_rewrite
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_v3_attempts
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_qdrant_projection_v3_ledger_rewrite();
CREATE TRIGGER deny_qdrant_projection_v3_receipt_rewrite
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_v3_receipts
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_qdrant_projection_v3_ledger_rewrite();

CREATE FUNCTION pipeline.begin_qdrant_projection_v3(
    p_admission_id UUID, p_command_bytes BYTEA, p_dispatch_execution_key TEXT
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_attempt RECORD; v_receipt RECORD;
BEGIN
    -- Never approve a fresh effect against a stale repeatable-read snapshot.
    IF pg_catalog.current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'Qdrant V3 first-start requires READ COMMITTED isolation'
            USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO a FROM pipeline.qdrant_projection_v3_admissions
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
        RAISE EXCEPTION 'V3 checked Qdrant admission denied or stale' USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO v_attempt FROM pipeline.qdrant_projection_v3_attempts
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF FOUND THEN
        SELECT * INTO v_receipt FROM pipeline.qdrant_projection_v3_receipts
            WHERE admission_id = p_admission_id;
        IF FOUND THEN
            IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
               OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
               OR v_receipt.dispatch_execution_key IS DISTINCT FROM a.dispatch_execution_key
               OR v_receipt.point_id IS DISTINCT FROM p_admission_id
               OR v_receipt.exact_provider_readback IS DISTINCT FROM TRUE THEN
                RAISE EXCEPTION 'V3 immutable Qdrant receipt identity is inconsistent'
                    USING ERRCODE = 'AV001';
            END IF;
            RETURN 'confirmed';
        END IF;
        RETURN 'uncertain'; -- UNKNOWN RPC outcome; no second upsert.
    END IF;
    IF EXISTS (
        SELECT 1 FROM pipeline.qdrant_projection_v3_receipts
        WHERE admission_id = p_admission_id
    ) THEN
        RAISE EXCEPTION 'V3 receipt without immutable attempt' USING ERRCODE = 'AV001';
    END IF;
    -- Exactly the same advisory key as the execution_runs status trigger.
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('truth-run-status|' || a.run_id::text, 0)
    );
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('operation-dag-canonical|' || a.run_id::text, 0)
    );
    IF a.revoked_at IS NOT NULL OR a.expires_at <= clock_timestamp() THEN
        RAISE EXCEPTION 'V3 Qdrant first start revoked/expired' USING ERRCODE = 'AV001';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pipeline.execution_runs r
        JOIN pipeline.tenant_context_ownership o ON o.context_key = r.context_key
        WHERE r.run_id = a.run_id AND r.context_key = a.context_key
          AND r.tenant_id = a.tenant_id AND o.tenant_id = a.tenant_id
          AND r.workflow_type = 'seo_site_build'
          AND r.status IN ('verifying', 'pending_hitl')
          AND pipeline.operation_dag_governance_matches_v1(
              a.run_id,a.tenant_id,a.context_key,
              a.parent_workflow_id,a.parent_workflow_run_id,
              a.plan_hash,a.parent_sha256,TRUE)
        FOR SHARE OF r
    ) THEN
        RAISE EXCEPTION 'V3 Qdrant first start denied by canonical run status'
            USING ERRCODE = 'AV001';
    END IF;
    INSERT INTO pipeline.qdrant_projection_v3_attempts (admission_id, tenant_id)
        VALUES (p_admission_id, a.tenant_id);
    RETURN 'started';
END;
$$;

CREATE FUNCTION pipeline.confirm_qdrant_projection_v3(
    p_admission_id UUID, p_command_bytes BYTEA, p_dispatch_execution_key TEXT
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_attempt RECORD; v_receipt RECORD;
BEGIN
    SELECT * INTO a FROM pipeline.qdrant_projection_v3_admissions
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR a.dispatch_execution_key IS DISTINCT FROM p_dispatch_execution_key
       OR encode(public.digest(p_command_bytes, 'sha256'), 'hex') <> a.command_sha256
    THEN
        RAISE EXCEPTION 'V3 Qdrant confirmation denied' USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO v_attempt FROM pipeline.qdrant_projection_v3_attempts
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR v_attempt.tenant_id IS DISTINCT FROM a.tenant_id THEN
        RAISE EXCEPTION 'V3 provider RPC was never admitted' USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO v_receipt FROM pipeline.qdrant_projection_v3_receipts
        WHERE admission_id = p_admission_id;
    IF FOUND THEN
        IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
           OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
           OR v_receipt.dispatch_execution_key IS DISTINCT FROM a.dispatch_execution_key
           OR v_receipt.point_id IS DISTINCT FROM p_admission_id
           OR v_receipt.exact_provider_readback IS DISTINCT FROM TRUE THEN
            RAISE EXCEPTION 'V3 immutable Qdrant receipt identity inconsistent'
                USING ERRCODE = 'AV001';
        END IF;
        RETURN TRUE;
    END IF;
    -- The provider readback was completed independently before this call.
    -- Never mark a mutable attempt confirmed or retry the provider here.
    INSERT INTO pipeline.qdrant_projection_v3_receipts (
        admission_id, tenant_id, command_sha256, dispatch_execution_key,
        point_id, exact_provider_readback
    ) VALUES (
        p_admission_id, a.tenant_id, a.command_sha256,
        a.dispatch_execution_key, p_admission_id, TRUE
    );
    RETURN TRUE;
END;
$$;

CREATE FUNCTION pipeline.verify_qdrant_operator_resignal_v3(
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
           'pipeline.qdrant_projection_v3_admissions', 'INSERT')
       OR NOT EXISTS (
           SELECT 1 FROM pipeline.qdrant_projection_v3_admissions a
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
             AND er.workflow_type = 'seo_site_build'
             AND er.status IN ('verifying','pending_hitl')
             AND pipeline.operation_dag_governance_matches_v1(
                 a.run_id,a.tenant_id,a.context_key,
                 a.parent_workflow_id,a.parent_workflow_run_id,
                 a.plan_hash,a.parent_sha256,TRUE)
       ) THEN
        RAISE EXCEPTION 'V3 Qdrant operator resignal denied'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN TRUE;
END;
$$;

REVOKE ALL ON FUNCTION pipeline.begin_qdrant_projection_v3(UUID,BYTEA,TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.confirm_qdrant_projection_v3(UUID,BYTEA,TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.verify_qdrant_operator_resignal_v3(
    UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BIGINT,BYTEA,BYTEA,BYTEA,TEXT
) FROM PUBLIC;

ALTER TABLE pipeline.qdrant_projection_v3_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.qdrant_projection_v3_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.qdrant_projection_v3_receipts ENABLE ROW LEVEL SECURITY;
CREATE POLICY qdrant_v3_admission_tenant ON pipeline.qdrant_projection_v3_admissions
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
CREATE POLICY qdrant_v3_attempt_tenant ON pipeline.qdrant_projection_v3_attempts
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
CREATE POLICY qdrant_v3_receipt_tenant ON pipeline.qdrant_projection_v3_receipts
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

COMMIT;
