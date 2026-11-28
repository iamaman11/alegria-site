-- Opt-in LIVE DataForSEO owner budget/admission ledger (not deployed by this
-- migration alone). The V1 stub-only capability is intentionally untouched.
--
-- Independent authority chain, all separately authenticated SQL LOGINs:
--   user approval -> one-use grant issuer -> Temporal Start owner ->
--   V2 business/History issuers -> paid-effect issuer -> paid-effect worker.
-- The paid worker receives EXECUTE only on the exact routines below; it may
-- NEVER mint admissions or modify any ledger table directly.
--
-- A committed attempt precedes the provider POST. An interrupted POST is
-- UNKNOWN and MUST NOT be repeated. A durable POST task ID permits GET only.
-- No credentials, owner routes, GRANTs, or paid requests are created here.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE TABLE pipeline.dataforseo_paid_owner_admissions_v2 (
    admission_id UUID PRIMARY KEY,
    grant_request_id UUID NOT NULL UNIQUE
        REFERENCES pipeline.operation_dag_one_step_request_grants_v1(request_id),
    run_id UUID NOT NULL
        REFERENCES pipeline.operation_dag_business_runs_v1(run_id),
    tenant_id TEXT NOT NULL CHECK (length(tenant_id) BETWEEN 1 AND 128
        AND tenant_id=btrim(tenant_id)),
    context_key TEXT NOT NULL CHECK (length(context_key) BETWEEN 1 AND 512
        AND context_key=btrim(context_key)),
    operation_id TEXT NOT NULL DEFAULT 'dataforseo.google_organic.search@1'
        CHECK (operation_id='dataforseo.google_organic.search@1'),
    operation_instance TEXT NOT NULL CHECK (length(operation_instance) BETWEEN 1 AND 128
        AND operation_instance=btrim(operation_instance)),
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
    max_cost_usd_micros BIGINT NOT NULL
        CHECK (max_cost_usd_micros BETWEEN 1 AND 1000000000),
    provider_mode TEXT NOT NULL DEFAULT 'paid'
        CHECK (provider_mode='paid'),
    issued_by NAME NOT NULL DEFAULT session_user,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    UNIQUE (run_id,plan_hash,operation_instance),
    UNIQUE (child_workflow_run_id,operation_instance),
    CHECK (parent_workflow_id='alegria-operation-dag-v2-' || run_id::text),
    CHECK (child_workflow_id='alegria-owner-operation-v2-' || dispatch_execution_key),
    CHECK (command_sha256=encode(public.digest(command_bytes,'sha256'),'hex')),
    CHECK (expires_at>issued_at AND expires_at<=issued_at+interval '10 minutes'),
    CHECK (revoked_at IS NULL OR revoked_at>=issued_at)
);

CREATE TABLE pipeline.dataforseo_paid_owner_attempts_v2 (
    admission_id UUID PRIMARY KEY
        REFERENCES pipeline.dataforseo_paid_owner_admissions_v2(admission_id),
    tenant_id TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE pipeline.dataforseo_paid_owner_posted_tasks_v2 (
    admission_id UUID PRIMARY KEY
        REFERENCES pipeline.dataforseo_paid_owner_admissions_v2(admission_id),
    tenant_id TEXT NOT NULL,
    provider_task_id TEXT NOT NULL
        CHECK (provider_task_id ~ '^[A-Za-z0-9_-]{1,128}$'),
    provider_post_response_bytes BYTEA NOT NULL
        CHECK (octet_length(provider_post_response_bytes) BETWEEN 1 AND 4194304),
    provider_post_sha256 TEXT NOT NULL
        CHECK (provider_post_sha256 ~ '^[0-9a-f]{64}$'),
    post_reported_cost_usd_micros BIGINT NOT NULL
        CHECK (post_reported_cost_usd_micros BETWEEN 0 AND 900000000000000000),
    posted_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CHECK (provider_post_sha256=encode(public.digest(provider_post_response_bytes,'sha256'),'hex'))
);

CREATE TABLE pipeline.dataforseo_paid_owner_receipts_v2 (
    admission_id UUID PRIMARY KEY
        REFERENCES pipeline.dataforseo_paid_owner_admissions_v2(admission_id),
    tenant_id TEXT NOT NULL,
    command_sha256 TEXT NOT NULL,
    dispatch_execution_key TEXT NOT NULL,
    provider_task_id TEXT NOT NULL,
    actual_cost_usd_micros BIGINT NOT NULL CHECK (actual_cost_usd_micros>=0),
    provider_result_raw_bytes BYTEA NOT NULL
        CHECK (octet_length(provider_result_raw_bytes) BETWEEN 1 AND 4194304),
    output_bytes BYTEA NOT NULL CHECK (octet_length(output_bytes) BETWEEN 1 AND 524288),
    receipt_bytes BYTEA NOT NULL CHECK (octet_length(receipt_bytes) BETWEEN 1 AND 524288),
    provider_result_sha256 TEXT NOT NULL CHECK (provider_result_sha256 ~ '^[0-9a-f]{64}$'),
    output_sha256 TEXT NOT NULL CHECK (output_sha256 ~ '^[0-9a-f]{64}$'),
    receipt_sha256 TEXT NOT NULL CHECK (receipt_sha256 ~ '^[0-9a-f]{64}$'),
    committed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CHECK (provider_result_sha256=encode(public.digest(provider_result_raw_bytes,'sha256'),'hex')),
    CHECK (output_sha256=encode(public.digest(output_bytes,'sha256'),'hex')),
    CHECK (receipt_sha256=encode(public.digest(receipt_bytes,'sha256'),'hex'))
);

REVOKE ALL ON pipeline.dataforseo_paid_owner_admissions_v2,
    pipeline.dataforseo_paid_owner_attempts_v2,
    pipeline.dataforseo_paid_owner_posted_tasks_v2,
    pipeline.dataforseo_paid_owner_receipts_v2 FROM PUBLIC;

CREATE FUNCTION pipeline.guard_dataforseo_paid_owner_admission_v2()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE
    g RECORD;
    approval RECORD;
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'paid DataForSEO admission requires READ COMMITTED' USING ERRCODE='AV001';
    END IF;
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'paid DataForSEO admission cannot be deleted' USING ERRCODE='AV001';
    END IF;
    IF TG_OP='UPDATE' THEN
        IF OLD.issued_by<>session_user
           OR pipeline.current_session_tenant_v1() IS DISTINCT FROM OLD.tenant_id
           OR OLD.revoked_at IS NOT NULL OR NEW.revoked_at IS NULL
           OR (to_jsonb(NEW)-'revoked_at') IS DISTINCT FROM (to_jsonb(OLD)-'revoked_at') THEN
            RAISE EXCEPTION 'paid DataForSEO only permits issuer-owned revocation'
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
       OR has_table_privilege(session_user,'pipeline.operation_dag_one_step_request_grants_v1','INSERT')
       OR EXISTS (SELECT 1 FROM pg_roles r WHERE r.rolname=session_user
                  AND (r.rolsuper OR r.rolbypassrls OR r.rolcreaterole
                       OR r.rolcreatedb OR r.rolreplication)) THEN
        RAISE EXCEPTION 'untrusted paid DataForSEO issuer or tenant' USING ERRCODE='AV001';
    END IF;

    PERFORM pg_advisory_xact_lock(hashtextextended('operation-dag-business|'||NEW.run_id::text,0));
    PERFORM pg_advisory_xact_lock(hashtextextended('operation-dag-canonical|'||NEW.run_id::text,0));

    SELECT * INTO g FROM pipeline.operation_dag_one_step_request_grants_v1
        WHERE request_id=NEW.grant_request_id AND run_id=NEW.run_id FOR SHARE;
    IF NOT FOUND OR g.tenant_id IS DISTINCT FROM NEW.tenant_id
       OR g.context_key IS DISTINCT FROM NEW.context_key
       OR g.operation_id<>'dataforseo.google_organic.search'
       OR g.operation_revision<>1
       OR g.plan_hash IS DISTINCT FROM NEW.plan_hash
       OR g.parent_workflow_id IS DISTINCT FROM NEW.parent_workflow_id
       OR g.state<>'start_confirmed' OR g.revoked_at IS NOT NULL
       OR g.expires_at<=clock_timestamp()
       OR g.issued_by=session_user OR g.reserved_by=session_user
       OR g.limits->>'currency_code'<>'USD'
       OR (g.limits->>'max_effect_dispatches')::numeric<>1
       OR (g.limits->>'max_provider_requests')::numeric<1
       OR (g.limits->>'max_charge_minor_units')::numeric * 10000
             < NEW.max_cost_usd_micros::numeric
       OR g.expected_start->'command_hashes'->>NEW.operation_instance
            IS DISTINCT FROM NEW.command_hash THEN
        RAISE EXCEPTION 'paid DataForSEO requires exact unused one-step USD grant'
            USING ERRCODE='AV001';
    END IF;
    SELECT * INTO approval FROM pipeline.operation_dag_one_step_request_approvals_v1
        WHERE authorization_event_id=g.authorization_event_id FOR SHARE;
    IF NOT FOUND OR approval.tenant_id IS DISTINCT FROM NEW.tenant_id
       OR approval.run_id IS DISTINCT FROM NEW.run_id
       OR approval.principal_id IS DISTINCT FROM g.principal_id
       OR approval.policy_id IS DISTINCT FROM g.policy_id
       OR approval.revoked_at IS NOT NULL
       OR approval.approved_by=session_user
       OR approval.expires_at<=clock_timestamp() THEN
        RAISE EXCEPTION 'paid DataForSEO user approval absent or invalid' USING ERRCODE='AV001';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pipeline.operation_dag_business_runs_v1 b
        WHERE b.run_id=NEW.run_id AND b.tenant_id=NEW.tenant_id
          AND b.context_key=NEW.context_key AND b.plan_hash=NEW.plan_hash
          AND b.revoked_at IS NULL AND b.expires_at>clock_timestamp()
    ) OR NOT pipeline.operation_dag_governance_matches_v2(
        NEW.run_id,NEW.tenant_id,NEW.context_key,
        NEW.parent_workflow_id,NEW.parent_workflow_run_id,
        NEW.plan_hash,NEW.parent_sha256,TRUE) THEN
        RAISE EXCEPTION 'paid DataForSEO requires independent live V2 History authority'
            USING ERRCODE='AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_dataforseo_paid_owner_admission_v2() FROM PUBLIC;
CREATE TRIGGER guard_dataforseo_paid_owner_admission_v2
    BEFORE INSERT OR UPDATE OR DELETE ON pipeline.dataforseo_paid_owner_admissions_v2
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_dataforseo_paid_owner_admission_v2();

-- Never permit a Worker to fabricate its own POST evidence by direct DML,
-- even if a future deployment accidentally grants table INSERT. Only an
-- SECURITY DEFINER function running as the table owner may append rows.
CREATE FUNCTION pipeline.guard_dataforseo_paid_owner_append_v2()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF TG_OP<>'INSERT' OR current_user <> pg_get_userbyid(
         (SELECT relowner FROM pg_class WHERE oid=TG_RELID)) THEN
        RAISE EXCEPTION 'paid DataForSEO effect ledger is append-only through owner functions'
            USING ERRCODE='AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_dataforseo_paid_owner_append_v2() FROM PUBLIC;
CREATE TRIGGER guard_dataforseo_paid_attempt_v2
    BEFORE INSERT OR UPDATE OR DELETE ON pipeline.dataforseo_paid_owner_attempts_v2
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_dataforseo_paid_owner_append_v2();
CREATE TRIGGER guard_dataforseo_paid_posted_v2
    BEFORE INSERT OR UPDATE OR DELETE ON pipeline.dataforseo_paid_owner_posted_tasks_v2
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_dataforseo_paid_owner_append_v2();
CREATE TRIGGER guard_dataforseo_paid_receipt_v2
    BEFORE INSERT OR UPDATE OR DELETE ON pipeline.dataforseo_paid_owner_receipts_v2
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_dataforseo_paid_owner_append_v2();
CREATE TRIGGER guard_dataforseo_paid_admission_truncate_v2
    BEFORE TRUNCATE ON pipeline.dataforseo_paid_owner_admissions_v2
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1();
CREATE TRIGGER guard_dataforseo_paid_attempt_truncate_v2
    BEFORE TRUNCATE ON pipeline.dataforseo_paid_owner_attempts_v2
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1();
CREATE TRIGGER guard_dataforseo_paid_posted_truncate_v2
    BEFORE TRUNCATE ON pipeline.dataforseo_paid_owner_posted_tasks_v2
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1();
CREATE TRIGGER guard_dataforseo_paid_receipt_truncate_v2
    BEFORE TRUNCATE ON pipeline.dataforseo_paid_owner_receipts_v2
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_dataforseo_owner_ledger_rewrite_v1();

CREATE FUNCTION pipeline.lookup_dataforseo_paid_owner_admission_v2(
    p_tenant_id TEXT,p_run_id UUID,p_plan_hash TEXT,p_instance TEXT,
    p_parent_workflow_id TEXT,p_parent_run_id TEXT,
    p_child_workflow_id TEXT,p_child_run_id TEXT,
    p_dispatch_key TEXT,p_command_hash TEXT,p_input_hash TEXT,
    p_max_cost_micros BIGINT
) RETURNS UUID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_id UUID;
BEGIN
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
       OR has_table_privilege(session_user,'pipeline.dataforseo_paid_owner_admissions_v2','INSERT') THEN
        RAISE EXCEPTION 'paid DataForSEO owner identity/tenant denied' USING ERRCODE='AV001';
    END IF;
    SELECT a.admission_id INTO v_id FROM pipeline.dataforseo_paid_owner_admissions_v2 a
    WHERE a.tenant_id=p_tenant_id AND a.run_id=p_run_id
      AND a.plan_hash=p_plan_hash AND a.operation_instance=p_instance
      AND a.parent_workflow_id=p_parent_workflow_id
      AND a.parent_workflow_run_id=p_parent_run_id
      AND a.child_workflow_id=p_child_workflow_id
      AND a.child_workflow_run_id=p_child_run_id
      AND a.dispatch_execution_key=p_dispatch_key
      AND a.command_hash=p_command_hash AND a.input_hash=p_input_hash
      AND a.max_cost_usd_micros=p_max_cost_micros AND a.provider_mode='paid'
      AND a.issued_by<>session_user;
    RETURN v_id; -- absent admission is NOT authority to send a paid POST
END;
$$;

CREATE FUNCTION pipeline.begin_dataforseo_paid_owner_v2(
    p_admission_id UUID,p_command_bytes BYTEA,p_dispatch_key TEXT,
    p_max_cost_micros BIGINT
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE a RECORD;g RECORD;v_run_id UUID;v_attempt RECORD;v_posted RECORD;v_receipt RECORD;
BEGIN
    IF current_setting('transaction_isolation')<>'read committed'
       OR has_table_privilege(session_user,'pipeline.dataforseo_paid_owner_admissions_v2','INSERT') THEN
        RAISE EXCEPTION 'paid DataForSEO first-attempt identity denied' USING ERRCODE='AV001';
    END IF;
    SELECT run_id INTO v_run_id FROM pipeline.dataforseo_paid_owner_admissions_v2
        WHERE admission_id=p_admission_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'paid DataForSEO admission absent' USING ERRCODE='AV001';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('operation-dag-business|'||v_run_id::text,0));
    PERFORM pg_advisory_xact_lock(hashtextextended('operation-dag-canonical|'||v_run_id::text,0));
    SELECT * INTO a FROM pipeline.dataforseo_paid_owner_admissions_v2
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.run_id IS DISTINCT FROM v_run_id
       OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by=session_user OR a.provider_mode<>'paid'
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR a.dispatch_execution_key IS DISTINCT FROM p_dispatch_key
       OR a.max_cost_usd_micros IS DISTINCT FROM p_max_cost_micros
       OR a.command_sha256<>encode(public.digest(p_command_bytes,'sha256'),'hex') THEN
        RAISE EXCEPTION 'paid DataForSEO exact command denied' USING ERRCODE='AV001';
    END IF;
    SELECT * INTO v_attempt FROM pipeline.dataforseo_paid_owner_attempts_v2
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF FOUND THEN
        SELECT * INTO v_receipt FROM pipeline.dataforseo_paid_owner_receipts_v2
            WHERE admission_id=p_admission_id;
        IF FOUND THEN
            IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
               OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
               OR v_receipt.dispatch_execution_key IS DISTINCT FROM a.dispatch_execution_key
               OR v_receipt.actual_cost_usd_micros>a.max_cost_usd_micros
               OR NOT EXISTS (SELECT 1 FROM pipeline.dataforseo_paid_owner_posted_tasks_v2 pt
                   WHERE pt.admission_id=a.admission_id AND pt.tenant_id=a.tenant_id
                     AND pt.provider_task_id=v_receipt.provider_task_id) THEN
                RAISE EXCEPTION 'paid DataForSEO confirmed result mismatch' USING ERRCODE='AV001';
            END IF;
            RETURN 'confirmed';
        END IF;
        SELECT * INTO v_posted FROM pipeline.dataforseo_paid_owner_posted_tasks_v2
            WHERE admission_id=p_admission_id;
        IF FOUND THEN
            IF v_posted.tenant_id IS DISTINCT FROM a.tenant_id THEN
                RAISE EXCEPTION 'paid DataForSEO posted task mismatch' USING ERRCODE='AV001';
            END IF;
            RETURN 'posted'; -- GET/readback only, NEVER repeat POST
        END IF;
        RETURN 'uncertain'; -- network may have charged; NEVER repeat POST
    END IF;
    IF EXISTS (SELECT 1 FROM pipeline.dataforseo_paid_owner_receipts_v2
               WHERE admission_id=p_admission_id) THEN
        RAISE EXCEPTION 'paid receipt without first attempt' USING ERRCODE='AV001';
    END IF;
    SELECT * INTO g FROM pipeline.operation_dag_one_step_request_grants_v1
        WHERE request_id=a.grant_request_id FOR SHARE;
    IF NOT FOUND OR g.run_id IS DISTINCT FROM a.run_id
       OR g.tenant_id IS DISTINCT FROM a.tenant_id
       OR g.state<>'start_confirmed' OR g.revoked_at IS NOT NULL
       OR g.expires_at<=clock_timestamp() OR a.revoked_at IS NOT NULL
       OR a.expires_at<=clock_timestamp()
       OR NOT pipeline.operation_dag_governance_matches_v2(
            a.run_id,a.tenant_id,a.context_key,a.parent_workflow_id,
            a.parent_workflow_run_id,a.plan_hash,a.parent_sha256,TRUE) THEN
        RAISE EXCEPTION 'paid DataForSEO first POST lacks active grant and History'
            USING ERRCODE='AV001';
    END IF;
    INSERT INTO pipeline.dataforseo_paid_owner_attempts_v2(admission_id,tenant_id)
        VALUES(a.admission_id,a.tenant_id);
    RETURN 'started'; -- COMMIT required BEFORE provider POST
END;
$$;

CREATE FUNCTION pipeline.record_dataforseo_paid_task_v2(
    p_admission_id UUID,p_command_bytes BYTEA,p_dispatch_key TEXT,
    p_provider_task_id TEXT,p_post_raw_bytes BYTEA,p_post_reported_cost_micros BIGINT
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE a RECORD;v_posted RECORD;
BEGIN
    SELECT * INTO a FROM pipeline.dataforseo_paid_owner_admissions_v2
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by=session_user OR a.provider_mode<>'paid'
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR a.dispatch_execution_key IS DISTINCT FROM p_dispatch_key
       OR a.command_sha256<>encode(public.digest(p_command_bytes,'sha256'),'hex')
       OR p_provider_task_id !~ '^[A-Za-z0-9_-]{1,128}$'
       OR octet_length(p_post_raw_bytes) NOT BETWEEN 1 AND 4194304
       OR p_post_reported_cost_micros IS NULL
       OR p_post_reported_cost_micros NOT BETWEEN 0 AND 900000000000000000
       OR NOT EXISTS (SELECT 1 FROM pipeline.dataforseo_paid_owner_attempts_v2 at
           WHERE at.admission_id=a.admission_id AND at.tenant_id=a.tenant_id) THEN
        RAISE EXCEPTION 'paid DataForSEO POST task evidence denied' USING ERRCODE='AV001';
    END IF;
    SELECT * INTO v_posted FROM pipeline.dataforseo_paid_owner_posted_tasks_v2
        WHERE admission_id=a.admission_id;
    IF FOUND THEN
        IF v_posted.tenant_id IS DISTINCT FROM a.tenant_id
           OR v_posted.provider_task_id IS DISTINCT FROM p_provider_task_id
           OR v_posted.provider_post_response_bytes IS DISTINCT FROM p_post_raw_bytes
           OR v_posted.post_reported_cost_usd_micros IS DISTINCT FROM p_post_reported_cost_micros THEN
            RAISE EXCEPTION 'paid DataForSEO immutable POST evidence conflict'
                USING ERRCODE='AV001';
        END IF;
        RETURN TRUE;
    END IF;
    INSERT INTO pipeline.dataforseo_paid_owner_posted_tasks_v2 (
        admission_id,tenant_id,provider_task_id,provider_post_response_bytes,
        provider_post_sha256,post_reported_cost_usd_micros)
    VALUES (a.admission_id,a.tenant_id,p_provider_task_id,p_post_raw_bytes,
        encode(public.digest(p_post_raw_bytes,'sha256'),'hex'),p_post_reported_cost_micros);
    RETURN TRUE;
END;
$$;

CREATE FUNCTION pipeline.read_dataforseo_paid_task_v2(
    p_admission_id UUID,p_command_bytes BYTEA,p_dispatch_key TEXT
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_task TEXT;
BEGIN
    SELECT pt.provider_task_id INTO v_task
    FROM pipeline.dataforseo_paid_owner_posted_tasks_v2 pt
    JOIN pipeline.dataforseo_paid_owner_admissions_v2 a ON a.admission_id=pt.admission_id
    JOIN pipeline.dataforseo_paid_owner_attempts_v2 at ON at.admission_id=a.admission_id
    WHERE a.admission_id=p_admission_id
      AND a.tenant_id=pipeline.current_session_tenant_v1()
      AND pt.tenant_id=a.tenant_id AND at.tenant_id=a.tenant_id
      AND a.issued_by<>session_user AND a.provider_mode='paid'
      AND a.command_bytes=p_command_bytes
      AND a.command_sha256=encode(public.digest(p_command_bytes,'sha256'),'hex')
      AND a.dispatch_execution_key=p_dispatch_key;
    IF v_task IS NULL THEN
        RAISE EXCEPTION 'paid DataForSEO exact durable task unavailable' USING ERRCODE='AV001';
    END IF;
    RETURN v_task;
END;
$$;

CREATE FUNCTION pipeline.confirm_dataforseo_paid_owner_v2(
    p_admission_id UUID,p_command_bytes BYTEA,p_dispatch_key TEXT,
    p_provider_task_id TEXT,p_provider_result_raw_bytes BYTEA,
    p_output_bytes BYTEA,p_receipt_bytes BYTEA,p_actual_cost_micros BIGINT
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE a RECORD;v_posted RECORD;v_receipt RECORD;
BEGIN
    SELECT * INTO a FROM pipeline.dataforseo_paid_owner_admissions_v2
        WHERE admission_id=p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by=session_user OR a.provider_mode<>'paid'
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR a.dispatch_execution_key IS DISTINCT FROM p_dispatch_key
       OR a.command_sha256<>encode(public.digest(p_command_bytes,'sha256'),'hex')
       OR p_actual_cost_micros IS NULL
       OR p_actual_cost_micros NOT BETWEEN 0 AND a.max_cost_usd_micros
       OR octet_length(p_provider_result_raw_bytes) NOT BETWEEN 1 AND 4194304
       OR octet_length(p_output_bytes) NOT BETWEEN 1 AND 524288
       OR octet_length(p_receipt_bytes) NOT BETWEEN 1 AND 524288
       OR NOT EXISTS (SELECT 1 FROM pipeline.dataforseo_paid_owner_attempts_v2 at
           WHERE at.admission_id=a.admission_id AND at.tenant_id=a.tenant_id) THEN
        RAISE EXCEPTION 'paid DataForSEO receipt admission/budget denied' USING ERRCODE='AV001';
    END IF;
    SELECT * INTO v_posted FROM pipeline.dataforseo_paid_owner_posted_tasks_v2
        WHERE admission_id=a.admission_id FOR UPDATE;
    IF NOT FOUND OR v_posted.tenant_id IS DISTINCT FROM a.tenant_id
       OR v_posted.provider_task_id IS DISTINCT FROM p_provider_task_id
       OR v_posted.post_reported_cost_usd_micros>a.max_cost_usd_micros THEN
        RAISE EXCEPTION 'paid DataForSEO POST task or spend cap mismatch' USING ERRCODE='AV001';
    END IF;
    SELECT * INTO v_receipt FROM pipeline.dataforseo_paid_owner_receipts_v2
        WHERE admission_id=a.admission_id;
    IF FOUND THEN
        IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
           OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
           OR v_receipt.dispatch_execution_key IS DISTINCT FROM a.dispatch_execution_key
           OR v_receipt.provider_task_id IS DISTINCT FROM p_provider_task_id
           OR v_receipt.provider_result_raw_bytes IS DISTINCT FROM p_provider_result_raw_bytes
           OR v_receipt.output_bytes IS DISTINCT FROM p_output_bytes
           OR v_receipt.receipt_bytes IS DISTINCT FROM p_receipt_bytes
           OR v_receipt.actual_cost_usd_micros IS DISTINCT FROM p_actual_cost_micros THEN
            RAISE EXCEPTION 'paid DataForSEO confirmed receipt mutation denied'
                USING ERRCODE='AV001';
        END IF;
        RETURN TRUE;
    END IF;
    INSERT INTO pipeline.dataforseo_paid_owner_receipts_v2 (
        admission_id,tenant_id,command_sha256,dispatch_execution_key,
        provider_task_id,actual_cost_usd_micros,provider_result_raw_bytes,
        output_bytes,receipt_bytes,provider_result_sha256,output_sha256,receipt_sha256)
    VALUES(a.admission_id,a.tenant_id,a.command_sha256,a.dispatch_execution_key,
        p_provider_task_id,p_actual_cost_micros,p_provider_result_raw_bytes,
        p_output_bytes,p_receipt_bytes,
        encode(public.digest(p_provider_result_raw_bytes,'sha256'),'hex'),
        encode(public.digest(p_output_bytes,'sha256'),'hex'),
        encode(public.digest(p_receipt_bytes,'sha256'),'hex'));
    RETURN TRUE;
END;
$$;

CREATE FUNCTION pipeline.read_confirmed_dataforseo_paid_owner_receipt_v2(
    p_admission_id UUID,p_command_bytes BYTEA,p_dispatch_key TEXT
) RETURNS BYTEA LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_receipt BYTEA;
BEGIN
    SELECT r.receipt_bytes INTO v_receipt
    FROM pipeline.dataforseo_paid_owner_receipts_v2 r
    JOIN pipeline.dataforseo_paid_owner_admissions_v2 a ON a.admission_id=r.admission_id
    JOIN pipeline.dataforseo_paid_owner_attempts_v2 at ON at.admission_id=a.admission_id
    JOIN pipeline.dataforseo_paid_owner_posted_tasks_v2 pt ON pt.admission_id=a.admission_id
    WHERE a.admission_id=p_admission_id
      AND a.tenant_id=pipeline.current_session_tenant_v1()
      AND at.tenant_id=a.tenant_id AND r.tenant_id=a.tenant_id
      AND pt.tenant_id=a.tenant_id AND pt.provider_task_id=r.provider_task_id
      AND a.issued_by<>session_user AND a.provider_mode='paid'
      AND a.command_bytes=p_command_bytes
      AND a.command_sha256=encode(public.digest(p_command_bytes,'sha256'),'hex')
      AND a.dispatch_execution_key=p_dispatch_key
      AND r.command_sha256=a.command_sha256
      AND r.dispatch_execution_key=a.dispatch_execution_key
      AND r.actual_cost_usd_micros<=a.max_cost_usd_micros
      AND r.receipt_sha256=encode(public.digest(r.receipt_bytes,'sha256'),'hex');
    IF v_receipt IS NULL THEN
        RAISE EXCEPTION 'paid DataForSEO exact confirmed receipt unavailable'
            USING ERRCODE='AV001';
    END IF;
    RETURN v_receipt;
END;
$$;

REVOKE ALL ON FUNCTION pipeline.lookup_dataforseo_paid_owner_admission_v2(
    TEXT,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BIGINT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.begin_dataforseo_paid_owner_v2(UUID,BYTEA,TEXT,BIGINT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.record_dataforseo_paid_task_v2(
    UUID,BYTEA,TEXT,TEXT,BYTEA,BIGINT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.read_dataforseo_paid_task_v2(UUID,BYTEA,TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.confirm_dataforseo_paid_owner_v2(
    UUID,BYTEA,TEXT,TEXT,BYTEA,BYTEA,BYTEA,BIGINT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.read_confirmed_dataforseo_paid_owner_receipt_v2(
    UUID,BYTEA,TEXT) FROM PUBLIC;

ALTER TABLE pipeline.dataforseo_paid_owner_admissions_v2 ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.dataforseo_paid_owner_attempts_v2 ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.dataforseo_paid_owner_posted_tasks_v2 ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.dataforseo_paid_owner_receipts_v2 ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.dataforseo_paid_owner_admissions_v2 FORCE ROW LEVEL SECURITY;
ALTER TABLE pipeline.dataforseo_paid_owner_attempts_v2 FORCE ROW LEVEL SECURITY;
ALTER TABLE pipeline.dataforseo_paid_owner_posted_tasks_v2 FORCE ROW LEVEL SECURITY;
ALTER TABLE pipeline.dataforseo_paid_owner_receipts_v2 FORCE ROW LEVEL SECURITY;
CREATE POLICY dataforseo_paid_admission_tenant_v2 ON pipeline.dataforseo_paid_owner_admissions_v2
    USING (tenant_id=pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id=pipeline.current_session_tenant_v1());
CREATE POLICY dataforseo_paid_attempt_tenant_v2 ON pipeline.dataforseo_paid_owner_attempts_v2
    USING (tenant_id=pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id=pipeline.current_session_tenant_v1());
CREATE POLICY dataforseo_paid_posted_tenant_v2 ON pipeline.dataforseo_paid_owner_posted_tasks_v2
    USING (tenant_id=pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id=pipeline.current_session_tenant_v1());
CREATE POLICY dataforseo_paid_receipt_tenant_v2 ON pipeline.dataforseo_paid_owner_receipts_v2
    USING (tenant_id=pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id=pipeline.current_session_tenant_v1());

-- DBA must grant exactly INSERT on admissions to a separate paid issuer and
-- EXECUTE on the six functions above to the worker. Never GRANT SELECT/INSERT
-- on attempts, posted tasks or receipts to any runtime role.
COMMIT;
