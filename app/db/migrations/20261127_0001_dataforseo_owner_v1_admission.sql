-- Independent, tenant-scoped Temporal DAG V2 admission for DataForSEO.
-- Revision 1 intentionally authorizes STUB effects only: no paid HTTP call
-- can be made by an owner using this database capability. The separately
-- credentialed operator must attest reciprocal SERVER parent/child Histories,
-- the full ValidateOnly wave and exact typed command BEFORE INSERT.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE TABLE pipeline.dataforseo_owner_admissions_v1 (
    admission_id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES pipeline.operation_dag_business_runs_v1(run_id),
    tenant_id TEXT NOT NULL CHECK (length(tenant_id)>0 AND tenant_id=btrim(tenant_id)),
    context_key TEXT NOT NULL CHECK (length(context_key)>0 AND context_key=btrim(context_key)),
    operation_id TEXT NOT NULL DEFAULT 'dataforseo.google_organic.search@1'
        CHECK (operation_id='dataforseo.google_organic.search@1'),
    operation_instance TEXT NOT NULL CHECK (length(operation_instance)>0),
    plan_hash TEXT NOT NULL CHECK (plan_hash ~ '^[0-9a-f]{64}$'),
    dispatch_execution_key TEXT NOT NULL UNIQUE
        CHECK (dispatch_execution_key ~ '^[0-9a-f]{64}$'),
    parent_workflow_id TEXT NOT NULL,
    parent_workflow_run_id TEXT NOT NULL CHECK (length(parent_workflow_run_id)>0),
    child_workflow_id TEXT NOT NULL,
    child_workflow_run_id TEXT NOT NULL CHECK (length(child_workflow_run_id)>0),
    child_started_event_id BIGINT NOT NULL CHECK (child_started_event_id>0),
    parent_sha256 TEXT NOT NULL CHECK (parent_sha256 ~ '^[0-9a-f]{64}$'),
    child_sha256 TEXT NOT NULL CHECK (child_sha256 ~ '^[0-9a-f]{64}$'),
    command_sha256 TEXT NOT NULL CHECK (command_sha256 ~ '^[0-9a-f]{64}$'),
    command_hash TEXT NOT NULL CHECK (command_hash ~ '^[0-9a-f]{64}$'),
    input_hash TEXT NOT NULL CHECK (input_hash ~ '^[0-9a-f]{64}$'),
    command_bytes BYTEA NOT NULL CHECK (octet_length(command_bytes) BETWEEN 1 AND 524288),
    max_cost_usd_micros BIGINT NOT NULL CHECK (max_cost_usd_micros BETWEEN 0 AND 1000000000),
    provider_mode TEXT NOT NULL DEFAULT 'stub' CHECK (provider_mode='stub'),
    issued_by NAME NOT NULL DEFAULT session_user,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    UNIQUE (run_id,plan_hash,operation_instance),
    UNIQUE (child_workflow_run_id,operation_instance),
    CHECK (parent_workflow_id='alegria-operation-dag-v2-' || run_id::text),
    CHECK (child_workflow_id='alegria-owner-operation-v2-' || dispatch_execution_key),
    CHECK (command_sha256=encode(public.digest(command_bytes,'sha256'),'hex')),
    CHECK (expires_at>issued_at),
    CHECK (revoked_at IS NULL OR revoked_at>=issued_at)
);

-- A committed attempt marker precedes any provider effect. On an ambiguous
-- crash it is NEVER deleted/retried. A confirmed receipt permits byte-exact
-- replay of the previously observed OwnerResult, but never another POST.
CREATE TABLE pipeline.dataforseo_owner_attempts_v1 (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.dataforseo_owner_admissions_v1(admission_id),
    tenant_id TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
-- A successful POST may return a task identifier before the GET/readback is
-- available. This row is independently durable, immutable and permits GET
-- resume only. Absence after an attempt means uncertain, never re-POST.
CREATE TABLE pipeline.dataforseo_owner_posted_tasks_v1 (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.dataforseo_owner_admissions_v1(admission_id),
    tenant_id TEXT NOT NULL,
    provider_task_id TEXT NOT NULL CHECK (provider_task_id ~ '^stub-serp-[0-9]{1,8}$'),
    posted_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE pipeline.dataforseo_owner_receipts_v1 (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.dataforseo_owner_admissions_v1(admission_id),
    tenant_id TEXT NOT NULL,
    command_sha256 TEXT NOT NULL,
    dispatch_execution_key TEXT NOT NULL,
    provider_task_id TEXT NOT NULL CHECK (length(provider_task_id)>0),
    actual_cost_usd_micros BIGINT NOT NULL CHECK (actual_cost_usd_micros>=0),
    result_bytes BYTEA NOT NULL CHECK (octet_length(result_bytes) BETWEEN 1 AND 524288),
    receipt_bytes BYTEA NOT NULL CHECK (octet_length(receipt_bytes) BETWEEN 1 AND 524288),
    result_sha256 TEXT NOT NULL CHECK (result_sha256 ~ '^[0-9a-f]{64}$'),
    receipt_sha256 TEXT NOT NULL CHECK (receipt_sha256 ~ '^[0-9a-f]{64}$'),
    committed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CHECK (result_sha256=encode(public.digest(result_bytes,'sha256'),'hex')),
    CHECK (receipt_sha256=encode(public.digest(receipt_bytes,'sha256'),'hex'))
);
REVOKE ALL ON pipeline.dataforseo_owner_admissions_v1,
    pipeline.dataforseo_owner_attempts_v1,
    pipeline.dataforseo_owner_posted_tasks_v1,
    pipeline.dataforseo_owner_receipts_v1 FROM PUBLIC;

CREATE FUNCTION pipeline.guard_dataforseo_owner_admission_v1()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'DataForSEO owner admission requires READ COMMITTED' USING ERRCODE='AV001';
    END IF;
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'DataForSEO owner admission cannot be deleted' USING ERRCODE='AV001';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF OLD.issued_by<>session_user
           OR pipeline.current_session_tenant_v1() IS DISTINCT FROM OLD.tenant_id
           OR OLD.revoked_at IS NOT NULL OR NEW.revoked_at IS NULL
           OR (to_jsonb(NEW)-'revoked_at') IS DISTINCT FROM (to_jsonb(OLD)-'revoked_at') THEN
            RAISE EXCEPTION 'DataForSEO owner only permits issuer-owned revocation'
                USING ERRCODE='AV001';
        END IF;
        PERFORM pg_advisory_xact_lock(hashtextextended('operation-dag-business|'||OLD.run_id::text,0));
        PERFORM pg_advisory_xact_lock(hashtextextended('operation-dag-canonical|'||OLD.run_id::text,0));
        NEW.revoked_at:=clock_timestamp();
        RETURN NEW;
    END IF;
    IF NEW.issued_by<>session_user
       OR pipeline.current_session_tenant_v1() IS DISTINCT FROM NEW.tenant_id
       OR NEW.revoked_at IS NOT NULL
       OR has_table_privilege(session_user,'pipeline.operation_dag_business_runs_v1','INSERT')
       OR has_table_privilege(session_user,'pipeline.operation_dag_run_governance_v2','INSERT')
       OR EXISTS (SELECT 1 FROM pg_roles r WHERE r.rolname=session_user
                  AND (r.rolsuper OR r.rolbypassrls OR r.rolcreaterole
                       OR r.rolcreatedb OR r.rolreplication)) THEN
        RAISE EXCEPTION 'untrusted DataForSEO operator issuer or tenant' USING ERRCODE='AV001';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('operation-dag-business|'||NEW.run_id::text,0));
    PERFORM pg_advisory_xact_lock(hashtextextended('operation-dag-canonical|'||NEW.run_id::text,0));
    IF NOT EXISTS (
        SELECT 1 FROM pipeline.operation_dag_business_runs_v1 b
        WHERE b.run_id=NEW.run_id AND b.tenant_id=NEW.tenant_id
          AND b.context_key=NEW.context_key AND b.plan_hash=NEW.plan_hash
          AND b.revoked_at IS NULL AND b.expires_at>clock_timestamp()
    ) OR NOT pipeline.operation_dag_governance_matches_v2(
        NEW.run_id,NEW.tenant_id,NEW.context_key,
        NEW.parent_workflow_id,NEW.parent_workflow_run_id,
        NEW.plan_hash,NEW.parent_sha256,TRUE) THEN
        RAISE EXCEPTION 'DataForSEO owner requires active independent V2 business/History authority'
            USING ERRCODE='AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_dataforseo_owner_admission_v1() FROM PUBLIC;
CREATE TRIGGER guard_dataforseo_owner_admission_v1
    BEFORE INSERT OR UPDATE OR DELETE ON pipeline.dataforseo_owner_admissions_v1
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_dataforseo_owner_admission_v1();

CREATE FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'DataForSEO owner attempts and receipts are immutable' USING ERRCODE='23514';
END;
$$;
REVOKE ALL ON FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1() FROM PUBLIC;
CREATE TRIGGER deny_dataforseo_attempt_rewrite_v1 BEFORE UPDATE OR DELETE
    ON pipeline.dataforseo_owner_attempts_v1 FOR EACH ROW
    EXECUTE FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1();
CREATE TRIGGER deny_dataforseo_receipt_rewrite_v1 BEFORE UPDATE OR DELETE
    ON pipeline.dataforseo_owner_receipts_v1 FOR EACH ROW
    EXECUTE FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1();
CREATE TRIGGER deny_dataforseo_posted_rewrite_v1 BEFORE UPDATE OR DELETE
    ON pipeline.dataforseo_owner_posted_tasks_v1 FOR EACH ROW
    EXECUTE FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1();
CREATE TRIGGER deny_dataforseo_admission_truncate_v1 BEFORE TRUNCATE
    ON pipeline.dataforseo_owner_admissions_v1 FOR EACH STATEMENT
    EXECUTE FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1();
CREATE TRIGGER deny_dataforseo_attempt_truncate_v1 BEFORE TRUNCATE
    ON pipeline.dataforseo_owner_attempts_v1 FOR EACH STATEMENT
    EXECUTE FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1();
CREATE TRIGGER deny_dataforseo_posted_truncate_v1 BEFORE TRUNCATE
    ON pipeline.dataforseo_owner_posted_tasks_v1 FOR EACH STATEMENT
    EXECUTE FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1();
CREATE TRIGGER deny_dataforseo_receipt_truncate_v1 BEFORE TRUNCATE
    ON pipeline.dataforseo_owner_receipts_v1 FOR EACH STATEMENT
    EXECUTE FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1();

-- The Worker has only EXECUTE and no SELECT on admission, attempt or receipt
-- tables. Return an admission locator ONLY for the entire exact V2 identity.
CREATE FUNCTION pipeline.lookup_dataforseo_owner_admission_v1(
    p_tenant_id TEXT,p_run_id UUID,p_plan_hash TEXT,p_instance TEXT,
    p_parent_workflow_id TEXT,p_parent_run_id TEXT,
    p_child_workflow_id TEXT,p_child_run_id TEXT,
    p_dispatch_key TEXT,p_command_hash TEXT,p_input_hash TEXT,
    p_max_cost_micros BIGINT
) RETURNS UUID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_id UUID;
BEGIN
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id THEN
        RAISE EXCEPTION 'DataForSEO admission lookup tenant denied' USING ERRCODE='AV001';
    END IF;
    SELECT a.admission_id INTO v_id FROM pipeline.dataforseo_owner_admissions_v1 a
    WHERE a.tenant_id=p_tenant_id AND a.run_id=p_run_id
      AND a.plan_hash=p_plan_hash AND a.operation_instance=p_instance
      AND a.parent_workflow_id=p_parent_workflow_id
      AND a.parent_workflow_run_id=p_parent_run_id
      AND a.child_workflow_id=p_child_workflow_id
      AND a.child_workflow_run_id=p_child_run_id
      AND a.dispatch_execution_key=p_dispatch_key
      AND a.command_hash=p_command_hash AND a.input_hash=p_input_hash
      AND a.max_cost_usd_micros=p_max_cost_micros AND a.provider_mode='stub'
      AND a.issued_by<>session_user;
    IF v_id IS NULL THEN
        -- A legitimate owner may be signaled before the independent operator
        -- issues admission. Let it keep waiting rather than fail the Workflow.
        RETURN NULL;
    END IF;
    RETURN v_id;
END;
$$;

CREATE FUNCTION pipeline.begin_dataforseo_owner_v1(
    p_admission_id UUID,p_command_bytes BYTEA,p_dispatch_key TEXT,p_max_cost_micros BIGINT
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE a RECORD;v_run_id UUID;v_attempt RECORD;v_posted RECORD;v_receipt RECORD;
BEGIN
    IF current_setting('transaction_isolation')<>'read committed' THEN
        RAISE EXCEPTION 'DataForSEO first attempt requires READ COMMITTED' USING ERRCODE='AV001';
    END IF;
    -- Resolve immutable locator before global revocation lock sequence.
    SELECT run_id INTO v_run_id FROM pipeline.dataforseo_owner_admissions_v1
        WHERE admission_id=p_admission_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'DataForSEO admission absent' USING ERRCODE='AV001';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('operation-dag-business|'||v_run_id::text,0));
    PERFORM pg_advisory_xact_lock(hashtextextended('operation-dag-canonical|'||v_run_id::text,0));
    SELECT * INTO a FROM pipeline.dataforseo_owner_admissions_v1
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.run_id IS DISTINCT FROM v_run_id
       OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by=session_user OR a.provider_mode<>'stub'
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR a.dispatch_execution_key IS DISTINCT FROM p_dispatch_key
       OR a.max_cost_usd_micros IS DISTINCT FROM p_max_cost_micros
       OR a.command_sha256<>encode(public.digest(p_command_bytes,'sha256'),'hex') THEN
        RAISE EXCEPTION 'DataForSEO checked first-attempt denied' USING ERRCODE='AV001';
    END IF;
    SELECT * INTO v_attempt FROM pipeline.dataforseo_owner_attempts_v1
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF FOUND THEN
        SELECT * INTO v_receipt FROM pipeline.dataforseo_owner_receipts_v1
            WHERE admission_id=p_admission_id;
        IF FOUND THEN
            IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
               OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
               OR v_receipt.dispatch_execution_key IS DISTINCT FROM a.dispatch_execution_key
               OR v_receipt.actual_cost_usd_micros>a.max_cost_usd_micros
               OR NOT EXISTS (
                    SELECT 1 FROM pipeline.dataforseo_owner_posted_tasks_v1 pt
                    WHERE pt.admission_id=a.admission_id AND pt.tenant_id=a.tenant_id
                      AND pt.provider_task_id=v_receipt.provider_task_id) THEN
                RAISE EXCEPTION 'DataForSEO confirmed receipt mismatch' USING ERRCODE='AV001';
            END IF;
            RETURN 'confirmed';
        END IF;
        SELECT * INTO v_posted FROM pipeline.dataforseo_owner_posted_tasks_v1
            WHERE admission_id=p_admission_id;
        IF FOUND THEN
            IF v_posted.tenant_id IS DISTINCT FROM a.tenant_id
               OR v_posted.provider_task_id !~ '^stub-serp-[0-9]{1,8}$' THEN
                RAISE EXCEPTION 'DataForSEO durable task marker mismatch' USING ERRCODE='AV001';
            END IF;
            RETURN 'posted';
        END IF;
        RETURN 'uncertain';
    END IF;
    IF EXISTS (SELECT 1 FROM pipeline.dataforseo_owner_receipts_v1
               WHERE admission_id=p_admission_id) THEN
        RAISE EXCEPTION 'DataForSEO receipt without attempt' USING ERRCODE='AV001';
    END IF;
    IF a.revoked_at IS NOT NULL OR a.expires_at<=clock_timestamp()
       OR NOT pipeline.operation_dag_governance_matches_v2(
            a.run_id,a.tenant_id,a.context_key,a.parent_workflow_id,
            a.parent_workflow_run_id,a.plan_hash,a.parent_sha256,TRUE) THEN
        RAISE EXCEPTION 'DataForSEO first-start has no active V2 governance'
            USING ERRCODE='AV001';
    END IF;
    INSERT INTO pipeline.dataforseo_owner_attempts_v1(admission_id,tenant_id)
        VALUES(p_admission_id,a.tenant_id);
    RETURN 'started';
END;
$$;

CREATE FUNCTION pipeline.record_stub_dataforseo_task_v1(
    p_admission_id UUID,p_command_bytes BYTEA,p_dispatch_key TEXT,p_provider_task_id TEXT
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE a RECORD;v_attempt RECORD;v_posted RECORD;
BEGIN
    SELECT * INTO a FROM pipeline.dataforseo_owner_admissions_v1
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by=session_user OR a.provider_mode<>'stub'
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR a.dispatch_execution_key IS DISTINCT FROM p_dispatch_key
       OR a.command_sha256<>encode(public.digest(p_command_bytes,'sha256'),'hex')
       OR p_provider_task_id !~ '^stub-serp-[0-9]{1,8}$' THEN
        RAISE EXCEPTION 'DataForSEO stub task marker denied' USING ERRCODE='AV001';
    END IF;
    SELECT * INTO v_attempt FROM pipeline.dataforseo_owner_attempts_v1
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF NOT FOUND OR v_attempt.tenant_id IS DISTINCT FROM a.tenant_id THEN
        RAISE EXCEPTION 'DataForSEO task marker requires committed first attempt' USING ERRCODE='AV001';
    END IF;
    SELECT * INTO v_posted FROM pipeline.dataforseo_owner_posted_tasks_v1
        WHERE admission_id=p_admission_id;
    IF FOUND THEN
        IF v_posted.tenant_id IS DISTINCT FROM a.tenant_id
           OR v_posted.provider_task_id IS DISTINCT FROM p_provider_task_id THEN
            RAISE EXCEPTION 'DataForSEO immutable task marker mismatch' USING ERRCODE='AV001';
        END IF;
        RETURN TRUE;
    END IF;
    INSERT INTO pipeline.dataforseo_owner_posted_tasks_v1(
        admission_id,tenant_id,provider_task_id)
    VALUES(p_admission_id,a.tenant_id,p_provider_task_id);
    RETURN TRUE;
END;
$$;

CREATE FUNCTION pipeline.lookup_stub_dataforseo_task_v1(
    p_admission_id UUID,p_command_bytes BYTEA,p_dispatch_key TEXT
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_task TEXT;
BEGIN
    SELECT pt.provider_task_id INTO v_task
    FROM pipeline.dataforseo_owner_posted_tasks_v1 pt
    JOIN pipeline.dataforseo_owner_admissions_v1 a ON a.admission_id=pt.admission_id
    JOIN pipeline.dataforseo_owner_attempts_v1 at ON at.admission_id=a.admission_id
    WHERE a.admission_id=p_admission_id
      AND a.tenant_id=pipeline.current_session_tenant_v1()
      AND at.tenant_id=a.tenant_id AND pt.tenant_id=a.tenant_id
      AND a.issued_by<>session_user AND a.provider_mode='stub'
      AND a.command_bytes=p_command_bytes
      AND a.command_sha256=encode(public.digest(p_command_bytes,'sha256'),'hex')
      AND a.dispatch_execution_key=p_dispatch_key;
    IF v_task IS NULL THEN
        RAISE EXCEPTION 'DataForSEO exact posted task unavailable' USING ERRCODE='AV001';
    END IF;
    RETURN v_task;
END;
$$;

CREATE FUNCTION pipeline.confirm_dataforseo_owner_v1(
    p_admission_id UUID,p_command_bytes BYTEA,p_dispatch_key TEXT,
    p_provider_task_id TEXT,p_result_bytes BYTEA,p_receipt_bytes BYTEA,
    p_actual_cost_micros BIGINT
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE a RECORD;v_attempt RECORD;v_posted RECORD;v_receipt RECORD;
BEGIN
    SELECT * INTO a FROM pipeline.dataforseo_owner_admissions_v1
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by=session_user OR a.provider_mode<>'stub'
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR a.dispatch_execution_key IS DISTINCT FROM p_dispatch_key
       OR a.command_sha256<>encode(public.digest(p_command_bytes,'sha256'),'hex')
       OR p_provider_task_id !~ '^stub-serp-[0-9]{1,8}$'
       OR p_actual_cost_micros IS NULL OR p_actual_cost_micros<>0
       OR p_actual_cost_micros>a.max_cost_usd_micros
       OR octet_length(p_result_bytes) NOT BETWEEN 1 AND 524288
       OR octet_length(p_receipt_bytes) NOT BETWEEN 1 AND 524288 THEN
        RAISE EXCEPTION 'DataForSEO stub confirmation denied' USING ERRCODE='AV001';
    END IF;
    SELECT * INTO v_attempt FROM pipeline.dataforseo_owner_attempts_v1
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF NOT FOUND OR v_attempt.tenant_id IS DISTINCT FROM a.tenant_id THEN
        RAISE EXCEPTION 'DataForSEO effect attempt absent' USING ERRCODE='AV001';
    END IF;
    SELECT * INTO v_posted FROM pipeline.dataforseo_owner_posted_tasks_v1
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF NOT FOUND OR v_posted.tenant_id IS DISTINCT FROM a.tenant_id
       OR v_posted.provider_task_id IS DISTINCT FROM p_provider_task_id THEN
        RAISE EXCEPTION 'DataForSEO exact durable task marker absent' USING ERRCODE='AV001';
    END IF;
    SELECT * INTO v_receipt FROM pipeline.dataforseo_owner_receipts_v1
        WHERE admission_id=p_admission_id;
    IF FOUND THEN
        IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
           OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
           OR v_receipt.dispatch_execution_key IS DISTINCT FROM a.dispatch_execution_key
           OR v_receipt.provider_task_id IS DISTINCT FROM p_provider_task_id
           OR v_receipt.result_bytes IS DISTINCT FROM p_result_bytes
           OR v_receipt.receipt_bytes IS DISTINCT FROM p_receipt_bytes
           OR v_receipt.actual_cost_usd_micros IS DISTINCT FROM p_actual_cost_micros THEN
            RAISE EXCEPTION 'DataForSEO immutable receipt differs from confirmation'
                USING ERRCODE='AV001';
        END IF;
        RETURN TRUE;
    END IF;
    INSERT INTO pipeline.dataforseo_owner_receipts_v1 (
        admission_id,tenant_id,command_sha256,dispatch_execution_key,
        provider_task_id,actual_cost_usd_micros,result_bytes,receipt_bytes,
        result_sha256,receipt_sha256)
    VALUES (p_admission_id,a.tenant_id,a.command_sha256,a.dispatch_execution_key,
        p_provider_task_id,p_actual_cost_micros,p_result_bytes,p_receipt_bytes,
        encode(public.digest(p_result_bytes,'sha256'),'hex'),
        encode(public.digest(p_receipt_bytes,'sha256'),'hex'));
    RETURN TRUE;
END;
$$;

CREATE FUNCTION pipeline.read_confirmed_dataforseo_owner_receipt_v1(
    p_admission_id UUID,p_command_bytes BYTEA,p_dispatch_key TEXT
) RETURNS BYTEA LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_receipt BYTEA;
BEGIN
    SELECT r.receipt_bytes INTO v_receipt
    FROM pipeline.dataforseo_owner_receipts_v1 r
    JOIN pipeline.dataforseo_owner_admissions_v1 a ON a.admission_id=r.admission_id
    JOIN pipeline.dataforseo_owner_attempts_v1 at ON at.admission_id=a.admission_id
    JOIN pipeline.dataforseo_owner_posted_tasks_v1 pt ON pt.admission_id=a.admission_id
    WHERE a.admission_id=p_admission_id
      AND a.tenant_id=pipeline.current_session_tenant_v1()
      AND at.tenant_id=a.tenant_id AND r.tenant_id=a.tenant_id
      AND pt.tenant_id=a.tenant_id AND pt.provider_task_id=r.provider_task_id
      AND a.issued_by<>session_user AND a.provider_mode='stub'
      AND a.command_bytes=p_command_bytes
      AND a.command_sha256=encode(public.digest(p_command_bytes,'sha256'),'hex')
      AND a.dispatch_execution_key=p_dispatch_key
      AND r.command_sha256=a.command_sha256
      AND r.dispatch_execution_key=a.dispatch_execution_key
      AND r.provider_task_id ~ '^stub-serp-[0-9]{1,8}$'
      AND r.actual_cost_usd_micros=0
      AND r.receipt_sha256=encode(public.digest(r.receipt_bytes,'sha256'),'hex');
    IF v_receipt IS NULL THEN
        RAISE EXCEPTION 'DataForSEO exact confirmed receipt unavailable' USING ERRCODE='AV001';
    END IF;
    RETURN v_receipt;
END;
$$;

REVOKE ALL ON FUNCTION pipeline.lookup_dataforseo_owner_admission_v1(
    TEXT,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BIGINT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.begin_dataforseo_owner_v1(UUID,BYTEA,TEXT,BIGINT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.record_stub_dataforseo_task_v1(UUID,BYTEA,TEXT,TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.lookup_stub_dataforseo_task_v1(UUID,BYTEA,TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.confirm_dataforseo_owner_v1(
    UUID,BYTEA,TEXT,TEXT,BYTEA,BYTEA,BIGINT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.read_confirmed_dataforseo_owner_receipt_v1(
    UUID,BYTEA,TEXT) FROM PUBLIC;

ALTER TABLE pipeline.dataforseo_owner_admissions_v1 ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.dataforseo_owner_attempts_v1 ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.dataforseo_owner_posted_tasks_v1 ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.dataforseo_owner_receipts_v1 ENABLE ROW LEVEL SECURITY;
CREATE POLICY dataforseo_owner_admission_tenant_v1 ON pipeline.dataforseo_owner_admissions_v1
    USING (tenant_id=pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id=pipeline.current_session_tenant_v1());
CREATE POLICY dataforseo_owner_attempt_tenant_v1 ON pipeline.dataforseo_owner_attempts_v1
    USING (tenant_id=pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id=pipeline.current_session_tenant_v1());
CREATE POLICY dataforseo_owner_posted_tenant_v1 ON pipeline.dataforseo_owner_posted_tasks_v1
    USING (tenant_id=pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id=pipeline.current_session_tenant_v1());
CREATE POLICY dataforseo_owner_receipt_tenant_v1 ON pipeline.dataforseo_owner_receipts_v1
    USING (tenant_id=pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id=pipeline.current_session_tenant_v1());

-- No automatic GRANTs. Deploy operator needs INSERT/UPDATE on admissions;
-- effect Worker needs EXECUTE on lookup/begin/record-task/lookup-task/confirm/
-- read-confirmed ONLY. No arbitrary paid provider permission is granted.
-- Independent V2 business-run/governance issuers remain separate principals.
COMMIT;
