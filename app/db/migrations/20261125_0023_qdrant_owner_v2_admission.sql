-- Independent V2 Temporal DAG Qdrant effect authority. Historical Qdrant
-- V1/V2/V3 admissions and provider receipts remain untouched. V4 ties its
-- durable first-attempt/receipt ledger to generic business governance, not to
-- the old seo_site_build execution_runs or kb.visa_contexts domain schema.
BEGIN;
SET LOCAL lock_timeout = '5s';

-- All four revisions still upsert the SAME provider collection. A distinct
-- V4 ledger must honor existing immutable business-key/point reservations.
DO $$
DECLARE v_constraint NAME;
BEGIN
    -- PostgreSQL truncates auto-generated CHECK names at 63 bytes. Resolve
    -- the one exact source-column constraint from pg_catalog, not a guessed
    -- identifier or an unbounded table rewrite.
    SELECT c.conname INTO STRICT v_constraint
    FROM pg_catalog.pg_constraint c
    WHERE c.conrelid = 'pipeline.qdrant_projection_business_key_reservations_v3'::regclass
      AND c.contype = 'c'
      AND pg_catalog.pg_get_constraintdef(c.oid) LIKE '%originating_revision%'
      AND pg_catalog.pg_get_constraintdef(c.oid) LIKE '%3%';
    EXECUTE pg_catalog.format(
        'ALTER TABLE pipeline.qdrant_projection_business_key_reservations_v3 DROP CONSTRAINT %I',
        v_constraint);
EXCEPTION WHEN NO_DATA_FOUND OR TOO_MANY_ROWS THEN
    RAISE EXCEPTION 'cross-version Qdrant reservation revision CHECK is not uniquely identifiable'
        USING ERRCODE='AV001';
END;
$$;
ALTER TABLE pipeline.qdrant_projection_business_key_reservations_v3
    ADD CONSTRAINT qdrant_business_key_reservation_revision_v4
    CHECK (originating_revision IN (1,2,3,4));

CREATE OR REPLACE FUNCTION pipeline.reserve_qdrant_business_key_v3()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE v_revision SMALLINT;
BEGIN
    v_revision := CASE TG_TABLE_NAME
        WHEN 'qdrant_projection_v1_admissions' THEN 1
        WHEN 'qdrant_projection_v2_admissions' THEN 2
        WHEN 'qdrant_projection_v3_admissions' THEN 3
        WHEN 'qdrant_projection_v4_admissions' THEN 4
        ELSE NULL END;
    IF v_revision IS NULL
       OR pipeline.current_session_tenant_v1() IS DISTINCT FROM NEW.tenant_id
       OR NEW.issued_by <> session_user THEN
        RAISE EXCEPTION 'unauthorized cross-version Qdrant business-key reservation'
            USING ERRCODE = 'AV001';
    END IF;
    INSERT INTO pipeline.qdrant_projection_business_key_reservations_v3
        (tenant_id,context_key,keyword_key,originating_revision,admission_id)
    VALUES (NEW.tenant_id,NEW.context_key,NEW.keyword_key,v_revision,NEW.admission_id);
    RETURN NEW;
END;
$$;

CREATE TABLE pipeline.qdrant_projection_v4_admissions (
    admission_id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES pipeline.operation_dag_business_runs_v1(run_id),
    tenant_id TEXT NOT NULL CHECK (length(tenant_id) > 0 AND tenant_id = btrim(tenant_id)),
    context_key TEXT NOT NULL CHECK (length(context_key) > 0 AND context_key = btrim(context_key)),
    keyword_key TEXT NOT NULL CHECK (length(keyword_key) > 0 AND keyword_key = btrim(keyword_key)),
    parent_workflow_id TEXT NOT NULL,
    parent_workflow_run_id TEXT NOT NULL,
    child_workflow_id TEXT NOT NULL,
    child_workflow_run_id TEXT NOT NULL,
    child_started_event_id BIGINT NOT NULL CHECK (child_started_event_id > 0),
    operation_instance TEXT NOT NULL CHECK (length(operation_instance) > 0),
    plan_hash TEXT NOT NULL CHECK (plan_hash ~ '^[0-9a-f]{64}$'),
    dispatch_execution_key TEXT NOT NULL CHECK (dispatch_execution_key ~ '^[0-9a-f]{64}$'),
    parent_sha256 TEXT NOT NULL CHECK (parent_sha256 ~ '^[0-9a-f]{64}$'),
    child_sha256 TEXT NOT NULL CHECK (child_sha256 ~ '^[0-9a-f]{64}$'),
    command_sha256 TEXT NOT NULL CHECK (command_sha256 ~ '^[0-9a-f]{64}$'),
    command_bytes BYTEA NOT NULL CHECK (octet_length(command_bytes) BETWEEN 1 AND 2097152),
    issued_by NAME NOT NULL DEFAULT session_user,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    UNIQUE (tenant_id, context_key, keyword_key),
    UNIQUE (run_id,plan_hash,operation_instance),
    UNIQUE (child_workflow_run_id,operation_instance),
    CHECK (parent_workflow_id = 'alegria-operation-dag-v2-' || run_id::text),
    CHECK (child_workflow_id = 'alegria-owner-operation-v2-' || dispatch_execution_key),
    CHECK (command_sha256 = encode(public.digest(command_bytes,'sha256'),'hex')),
    CHECK (expires_at > issued_at)
);
CREATE TABLE pipeline.qdrant_projection_v4_attempts (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.qdrant_projection_v4_admissions(admission_id),
    tenant_id TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE pipeline.qdrant_projection_v4_receipts (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.qdrant_projection_v4_admissions(admission_id),
    tenant_id TEXT NOT NULL,
    command_sha256 TEXT NOT NULL,
    dispatch_execution_key TEXT NOT NULL,
    point_id UUID NOT NULL UNIQUE,
    collection_name TEXT NOT NULL DEFAULT 'seo_keyword_clusters_4'
        CHECK (collection_name = 'seo_keyword_clusters_4'),
    exact_provider_readback BOOLEAN NOT NULL CHECK (exact_provider_readback),
    committed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
REVOKE ALL ON pipeline.qdrant_projection_v4_admissions,
    pipeline.qdrant_projection_v4_attempts,
    pipeline.qdrant_projection_v4_receipts FROM PUBLIC;

CREATE FUNCTION pipeline.guard_qdrant_projection_v4_admission()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'V4 Qdrant admission cannot be deleted' USING ERRCODE = 'AV001';
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF OLD.issued_by <> session_user
           OR pipeline.current_session_tenant_v1() IS DISTINCT FROM OLD.tenant_id
           OR OLD.revoked_at IS NOT NULL OR NEW.revoked_at IS NULL
           OR (to_jsonb(NEW)-'revoked_at') IS DISTINCT FROM (to_jsonb(OLD)-'revoked_at') THEN
            RAISE EXCEPTION 'V4 Qdrant only permits issuer-owned monotonic revocation'
                USING ERRCODE = 'AV001';
        END IF;
        NEW.revoked_at := clock_timestamp();
        RETURN NEW;
    END IF;
    IF NEW.issued_by <> session_user
       OR pipeline.current_session_tenant_v1() IS DISTINCT FROM NEW.tenant_id
       OR NOT EXISTS (
           SELECT 1 FROM pipeline.operation_dag_business_runs_v1 b
           WHERE b.run_id = NEW.run_id AND b.tenant_id = NEW.tenant_id
             AND b.context_key = NEW.context_key
             AND b.plan_hash = NEW.plan_hash
             AND b.revoked_at IS NULL AND b.expires_at > clock_timestamp()
       ) THEN
        RAISE EXCEPTION 'V4 Qdrant foreign or inactive canonical business run'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_qdrant_projection_v4_admission() FROM PUBLIC;
CREATE TRIGGER guard_qdrant_projection_v4_admission
    BEFORE INSERT OR UPDATE OR DELETE ON pipeline.qdrant_projection_v4_admissions
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_qdrant_projection_v4_admission();
CREATE TRIGGER reserve_qdrant_v4_business_key
    BEFORE INSERT ON pipeline.qdrant_projection_v4_admissions
    FOR EACH ROW EXECUTE FUNCTION pipeline.reserve_qdrant_business_key_v3();

CREATE FUNCTION pipeline.deny_qdrant_projection_v4_ledger_rewrite()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'Qdrant V4 provider attempt and receipt are immutable'
        USING ERRCODE = '23514';
END;
$$;
REVOKE ALL ON FUNCTION pipeline.deny_qdrant_projection_v4_ledger_rewrite() FROM PUBLIC;
CREATE TRIGGER deny_qdrant_v4_attempt_rewrite
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_v4_attempts
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_qdrant_projection_v4_ledger_rewrite();
CREATE TRIGGER deny_qdrant_v4_receipt_rewrite
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_v4_receipts
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_qdrant_projection_v4_ledger_rewrite();
CREATE TRIGGER deny_qdrant_v4_admission_truncate
    BEFORE TRUNCATE ON pipeline.qdrant_projection_v4_admissions
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_qdrant_projection_v4_ledger_rewrite();
CREATE TRIGGER deny_qdrant_v4_attempt_truncate
    BEFORE TRUNCATE ON pipeline.qdrant_projection_v4_attempts
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_qdrant_projection_v4_ledger_rewrite();
CREATE TRIGGER deny_qdrant_v4_receipt_truncate
    BEFORE TRUNCATE ON pipeline.qdrant_projection_v4_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_qdrant_projection_v4_ledger_rewrite();

CREATE FUNCTION pipeline.begin_qdrant_projection_v4(
    p_admission_id UUID,p_command_bytes BYTEA,p_dispatch_execution_key TEXT
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE a RECORD;v_attempt RECORD;v_receipt RECORD;v_run_id UUID;
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'V4 first-attempt requires READ COMMITTED' USING ERRCODE = 'AV001';
    END IF;
    -- Resolve only immutable lock locator first; do NOT take an admission row
    -- lock before the global business→canonical revocation lock sequence.
    SELECT run_id INTO v_run_id FROM pipeline.qdrant_projection_v4_admissions
        WHERE admission_id=p_admission_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'V4 admission absent' USING ERRCODE='AV001';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('operation-dag-business|'||v_run_id::text,0));
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('operation-dag-canonical|'||v_run_id::text,0));
    SELECT * INTO a FROM pipeline.qdrant_projection_v4_admissions
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.run_id IS DISTINCT FROM v_run_id
       OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR a.dispatch_execution_key IS DISTINCT FROM p_dispatch_execution_key
       OR a.command_sha256 <> encode(public.digest(p_command_bytes,'sha256'),'hex') THEN
        RAISE EXCEPTION 'V4 checked Qdrant admission denied' USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO v_attempt FROM pipeline.qdrant_projection_v4_attempts
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF FOUND THEN
        SELECT * INTO v_receipt FROM pipeline.qdrant_projection_v4_receipts
            WHERE admission_id=p_admission_id;
        IF FOUND THEN
            IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
               OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
               OR v_receipt.dispatch_execution_key IS DISTINCT FROM a.dispatch_execution_key
               OR v_receipt.point_id IS DISTINCT FROM p_admission_id
               OR v_receipt.exact_provider_readback IS DISTINCT FROM TRUE THEN
                RAISE EXCEPTION 'V4 confirmed Qdrant receipt mismatch' USING ERRCODE = 'AV001';
            END IF;
            RETURN 'confirmed';
        END IF;
        RETURN 'uncertain';
    END IF;
    IF EXISTS (SELECT 1 FROM pipeline.qdrant_projection_v4_receipts
               WHERE admission_id=p_admission_id) THEN
        RAISE EXCEPTION 'V4 receipt without attempt' USING ERRCODE = 'AV001';
    END IF;
    -- Locks were acquired before the mutable admission read, so revocation
    -- cannot race the durable first-attempt marker.
    IF a.revoked_at IS NOT NULL OR a.expires_at <= clock_timestamp()
       OR NOT pipeline.operation_dag_governance_matches_v2(
             a.run_id,a.tenant_id,a.context_key,a.parent_workflow_id,
             a.parent_workflow_run_id,a.plan_hash,a.parent_sha256,TRUE) THEN
        RAISE EXCEPTION 'V4 first-start lacks active independent V2 governance'
            USING ERRCODE = 'AV001';
    END IF;
    INSERT INTO pipeline.qdrant_projection_v4_attempts(admission_id,tenant_id)
        VALUES(p_admission_id,a.tenant_id);
    RETURN 'started';
END;
$$;

CREATE FUNCTION pipeline.confirm_qdrant_projection_v4(
    p_admission_id UUID,p_command_bytes BYTEA,p_dispatch_execution_key TEXT
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE a RECORD;v_attempt RECORD;v_receipt RECORD;
BEGIN
    SELECT * INTO a FROM pipeline.qdrant_projection_v4_admissions
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR a.dispatch_execution_key IS DISTINCT FROM p_dispatch_execution_key
       OR a.command_sha256 <> encode(public.digest(p_command_bytes,'sha256'),'hex') THEN
        RAISE EXCEPTION 'V4 Qdrant confirmation denied' USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO v_attempt FROM pipeline.qdrant_projection_v4_attempts
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF NOT FOUND OR v_attempt.tenant_id IS DISTINCT FROM a.tenant_id THEN
        RAISE EXCEPTION 'V4 Qdrant provider attempt absent' USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO v_receipt FROM pipeline.qdrant_projection_v4_receipts
        WHERE admission_id=p_admission_id;
    IF FOUND THEN
        IF v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
           OR v_receipt.dispatch_execution_key IS DISTINCT FROM a.dispatch_execution_key
           OR v_receipt.point_id IS DISTINCT FROM p_admission_id
           OR v_receipt.exact_provider_readback IS DISTINCT FROM TRUE THEN
            RAISE EXCEPTION 'V4 immutable receipt inconsistent' USING ERRCODE = 'AV001';
        END IF;
        RETURN TRUE;
    END IF;
    INSERT INTO pipeline.qdrant_projection_v4_receipts(
        admission_id,tenant_id,command_sha256,dispatch_execution_key,
        point_id,exact_provider_readback)
    VALUES(p_admission_id,a.tenant_id,a.command_sha256,a.dispatch_execution_key,
        p_admission_id,TRUE);
    RETURN TRUE;
END;
$$;

CREATE FUNCTION pipeline.verify_qdrant_operator_resignal_v4(
    p_admission_id UUID,p_run_id UUID,p_tenant_id TEXT,
    p_context_key TEXT,p_keyword_key TEXT,
    p_parent_workflow_id TEXT,p_parent_workflow_run_id TEXT,
    p_child_workflow_id TEXT,p_child_workflow_run_id TEXT,
    p_child_started_event_id BIGINT,
    p_parent_bytes BYTEA,p_child_bytes BYTEA,p_command_bytes BYTEA,
    p_dispatch_execution_key TEXT
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
       OR octet_length(p_parent_bytes)>2097152
       OR octet_length(p_child_bytes)>2097152
       OR octet_length(p_command_bytes)>2097152
       OR NOT has_table_privilege(session_user,'pipeline.qdrant_projection_v4_admissions','INSERT')
       OR NOT EXISTS(
           SELECT 1 FROM pipeline.qdrant_projection_v4_admissions a
           WHERE a.admission_id=p_admission_id AND a.issued_by=session_user
             AND a.run_id=p_run_id AND a.tenant_id=p_tenant_id
             AND a.context_key=p_context_key AND a.keyword_key=p_keyword_key
             AND a.parent_workflow_id=p_parent_workflow_id
             AND a.parent_workflow_run_id=p_parent_workflow_run_id
             AND a.child_workflow_id=p_child_workflow_id
             AND a.child_workflow_run_id=p_child_workflow_run_id
             AND a.child_started_event_id=p_child_started_event_id
             AND a.parent_sha256=encode(public.digest(p_parent_bytes,'sha256'),'hex')
             AND a.child_sha256=encode(public.digest(p_child_bytes,'sha256'),'hex')
             AND a.command_bytes=p_command_bytes
             AND a.command_sha256=encode(public.digest(p_command_bytes,'sha256'),'hex')
             AND a.dispatch_execution_key=p_dispatch_execution_key
             AND a.revoked_at IS NULL AND a.expires_at>clock_timestamp()
             AND pipeline.operation_dag_governance_matches_v2(
                 a.run_id,a.tenant_id,a.context_key,a.parent_workflow_id,
                 a.parent_workflow_run_id,a.plan_hash,a.parent_sha256,TRUE)
       ) THEN
        RAISE EXCEPTION 'V4 Qdrant operator resignal denied' USING ERRCODE='AV001';
    END IF;
    RETURN TRUE;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.begin_qdrant_projection_v4(UUID,BYTEA,TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.confirm_qdrant_projection_v4(UUID,BYTEA,TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.verify_qdrant_operator_resignal_v4(
    UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BIGINT,BYTEA,BYTEA,BYTEA,TEXT
) FROM PUBLIC;

ALTER TABLE pipeline.qdrant_projection_v4_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.qdrant_projection_v4_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.qdrant_projection_v4_receipts ENABLE ROW LEVEL SECURITY;
CREATE POLICY qdrant_v4_admission_tenant ON pipeline.qdrant_projection_v4_admissions
    USING (tenant_id=pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id=pipeline.current_session_tenant_v1());
CREATE POLICY qdrant_v4_attempt_tenant ON pipeline.qdrant_projection_v4_attempts
    USING (tenant_id=pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id=pipeline.current_session_tenant_v1());
CREATE POLICY qdrant_v4_receipt_tenant ON pipeline.qdrant_projection_v4_receipts
    USING (tenant_id=pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id=pipeline.current_session_tenant_v1());

COMMIT;
