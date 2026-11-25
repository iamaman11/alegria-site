-- Independent PostgreSQL producer of versioned, PRECOMPUTED semantic vectors.
-- This is an opt-in business source/receipt path, not a generic arbitrary SQL
-- execution interface and not an old Truth/SEO History reinterpretation.
-- A downstream owner receives projection bytes through Temporal Child results,
-- and its privileged operator may verify ONLY this neutral immutable receipt
-- capability; no downstream Worker/owner requires PostgreSQL source access.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE TABLE pipeline.vector_projection_sources_v1 (
    run_id UUID NOT NULL,
    tenant_id TEXT NOT NULL CHECK (length(trim(tenant_id)) > 0),
    context_key TEXT NOT NULL CHECK (length(trim(context_key)) > 0),
    keyword_key TEXT NOT NULL CHECK (length(trim(keyword_key)) > 0),
    vector JSONB NOT NULL CHECK (
        jsonb_typeof(vector) = 'array'
        AND jsonb_array_length(vector) = 1024
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(run_id, tenant_id, context_key, keyword_key)
);
CREATE FUNCTION pipeline.guard_vector_projection_source_immutable_v1()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'precomputed vector projection source is immutable (publish a new run)'
        USING ERRCODE = '23514';
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_vector_projection_source_immutable_v1() FROM PUBLIC;
CREATE TRIGGER guard_vector_projection_source_immutable_v1
    BEFORE UPDATE OR DELETE ON pipeline.vector_projection_sources_v1
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_vector_projection_source_immutable_v1();
CREATE TRIGGER guard_vector_projection_source_truncate_v1
    BEFORE TRUNCATE ON pipeline.vector_projection_sources_v1
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.guard_vector_projection_source_immutable_v1();
ALTER TABLE pipeline.vector_projection_sources_v1 ENABLE ROW LEVEL SECURITY;
CREATE POLICY vector_projection_source_authenticated_tenant_v1
    ON pipeline.vector_projection_sources_v1 FOR SELECT
    USING (tenant_id = pipeline.current_session_tenant_v1());
CREATE POLICY vector_projection_source_authorized_tenant_insert_v1
    ON pipeline.vector_projection_sources_v1 FOR INSERT
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

-- A PG-only issuer first verifies reciprocal server Temporal Histories,
-- immutable DAG start, exact checked dispatch and the authoritative source.
-- The worker has NO INSERT/UPDATE/DELETE access to this admission table.
CREATE TABLE pipeline.pg_vector_projection_admissions_v1 (
    admission_id UUID PRIMARY KEY,
    run_id UUID NOT NULL,
    tenant_id TEXT NOT NULL,
    context_key TEXT NOT NULL,
    keyword_key TEXT NOT NULL,
    operation_instance TEXT NOT NULL,
    plan_hash TEXT NOT NULL CHECK(plan_hash ~ '^[0-9a-f]{64}$'),
    contract_fingerprint TEXT NOT NULL CHECK(contract_fingerprint ~ '^[0-9a-f]{64}$'),
    dispatch_execution_key TEXT NOT NULL CHECK(dispatch_execution_key ~ '^[0-9a-f]{64}$'),
    parent_workflow_id TEXT NOT NULL,
    parent_workflow_run_id TEXT NOT NULL,
    child_workflow_id TEXT NOT NULL,
    child_workflow_run_id TEXT NOT NULL,
    invocation_sha256 TEXT NOT NULL CHECK(invocation_sha256 ~ '^[0-9a-f]{64}$'),
    generic_parent_start_sha256 TEXT NOT NULL CHECK(generic_parent_start_sha256 ~ '^[0-9a-f]{64}$'),
    source_hash TEXT NOT NULL CHECK(source_hash ~ '^[0-9a-f]{64}$'),
    issued_by TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT uq_pg_vector_projection_instance_v1
        UNIQUE (run_id,plan_hash,operation_instance),
    CONSTRAINT uq_pg_vector_projection_child_v1
        UNIQUE (child_workflow_id,child_workflow_run_id),
    CONSTRAINT fk_pg_vector_projection_source_v1
        FOREIGN KEY (run_id,tenant_id,context_key,keyword_key)
        REFERENCES pipeline.vector_projection_sources_v1
            (run_id,tenant_id,context_key,keyword_key)
);
CREATE FUNCTION pipeline.guard_vector_projection_admission_immutable_v1()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF (to_jsonb(NEW) - 'revoked_at') IS DISTINCT FROM
       (to_jsonb(OLD) - 'revoked_at')
       OR (OLD.revoked_at IS NOT NULL AND NEW.revoked_at IS DISTINCT FROM OLD.revoked_at) THEN
        RAISE EXCEPTION 'PG vector projection admission is immutable except monotonic revocation'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_vector_projection_admission_immutable_v1() FROM PUBLIC;
CREATE TRIGGER guard_vector_projection_admission_immutable_v1
    BEFORE UPDATE ON pipeline.pg_vector_projection_admissions_v1
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_vector_projection_admission_immutable_v1();
ALTER TABLE pipeline.pg_vector_projection_admissions_v1 ENABLE ROW LEVEL SECURITY;
CREATE POLICY pg_vector_projection_admission_tenant_v1
    ON pipeline.pg_vector_projection_admissions_v1 FOR SELECT
    USING (tenant_id = pipeline.current_session_tenant_v1());
CREATE POLICY pg_vector_projection_admission_issuer_v1
    ON pipeline.pg_vector_projection_admissions_v1 FOR INSERT
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

-- Immutable cross-owner evidence is intentionally NEUTRAL. The domain source
-- row is never shared with another owner. Only a narrowly scoped Boolean
-- verifier is exposed to a separately privileged downstream OPERATOR.
CREATE TABLE pipeline.operation_artifact_receipts_v1 (
    receipt_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    admission_id UUID NOT NULL UNIQUE
        REFERENCES pipeline.pg_vector_projection_admissions_v1(admission_id),
    run_id UUID NOT NULL,
    tenant_id TEXT NOT NULL,
    context_key TEXT NOT NULL,
    keyword_key TEXT NOT NULL,
    plan_hash TEXT NOT NULL,
    operation_instance TEXT NOT NULL,
    dispatch_execution_key TEXT NOT NULL,
    parent_workflow_id TEXT NOT NULL,
    parent_workflow_run_id TEXT NOT NULL,
    child_workflow_id TEXT NOT NULL,
    child_workflow_run_id TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    projection JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE FUNCTION pipeline.guard_operation_artifact_receipt_immutable_v1()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'cross-owner artifact receipt cannot be updated/deleted'
        USING ERRCODE = '23514';
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_operation_artifact_receipt_immutable_v1() FROM PUBLIC;
CREATE TRIGGER guard_operation_artifact_receipt_immutable_v1
    BEFORE UPDATE OR DELETE ON pipeline.operation_artifact_receipts_v1
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_operation_artifact_receipt_immutable_v1();
CREATE TRIGGER guard_operation_artifact_receipt_truncate_v1
    BEFORE TRUNCATE ON pipeline.operation_artifact_receipts_v1
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.guard_operation_artifact_receipt_immutable_v1();
ALTER TABLE pipeline.operation_artifact_receipts_v1 ENABLE ROW LEVEL SECURITY;
CREATE POLICY operation_artifact_receipt_tenant_v1
    ON pipeline.operation_artifact_receipts_v1 FOR SELECT
    USING (tenant_id = pipeline.current_session_tenant_v1());

CREATE FUNCTION pipeline.consume_pg_vector_projection_owner_v1(
    p_admission_id UUID, p_run_id UUID, p_tenant_id TEXT,
    p_context_key TEXT, p_keyword_key TEXT,
    p_child_workflow_id TEXT, p_child_workflow_run_id TEXT,
    p_operation_instance TEXT, p_plan_hash TEXT,
    p_contract_fingerprint TEXT, p_execution_key TEXT,
    p_parent_workflow_id TEXT, p_parent_workflow_run_id TEXT,
    p_invocation_bytes BYTEA
) RETURNS TABLE(receipt_id UUID, source_hash TEXT, vector JSONB)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    a RECORD;
    s RECORD;
    v_source_hash TEXT;
    v_receipt RECORD;
BEGIN
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
       AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles
                       WHERE rolname = session_user AND rolsuper) THEN
        RAISE EXCEPTION 'projection owner authenticated tenant mismatch'
            USING ERRCODE = 'AV001';
    END IF;
    IF p_execution_key !~ '^[0-9a-f]{64}$'
       OR p_plan_hash !~ '^[0-9a-f]{64}$'
       OR p_contract_fingerprint !~ '^[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'invalid projection dispatch/hash'
            USING ERRCODE = 'AV001';
    END IF;
    -- Governance issuer uses status then canonical lock order. Source is
    -- append-only, so no independent mutable snapshot can be substituted.
    PERFORM pg_advisory_xact_lock(hashtextextended('truth-run-status|' || p_run_id::text, 0));
    PERFORM pg_advisory_xact_lock(hashtextextended('operation-dag-canonical|' || p_run_id::text, 0));
    PERFORM pg_advisory_xact_lock(hashtextextended('pg-vector-admission|' || p_admission_id::text, 0));

    SELECT * INTO a FROM pipeline.pg_vector_projection_admissions_v1
    WHERE admission_id = p_admission_id
      AND run_id = p_run_id AND tenant_id = p_tenant_id
      AND context_key = p_context_key AND keyword_key = p_keyword_key
      AND child_workflow_id = p_child_workflow_id
      AND child_workflow_run_id = p_child_workflow_run_id
      AND operation_instance = p_operation_instance
      AND plan_hash = p_plan_hash
      AND contract_fingerprint = p_contract_fingerprint
      AND dispatch_execution_key = p_execution_key
      AND parent_workflow_id = p_parent_workflow_id
      AND parent_workflow_run_id = p_parent_workflow_run_id
      AND invocation_sha256 = encode(public.digest(p_invocation_bytes, 'sha256'), 'hex');
    IF NOT FOUND THEN
        RAISE EXCEPTION 'independent PG projection operator admission denied'
            USING ERRCODE = 'AV001';
    END IF;
    IF NOT pipeline.operation_dag_governance_matches_v1(
        a.run_id,a.tenant_id,a.context_key,
        a.parent_workflow_id,a.parent_workflow_run_id,
        a.plan_hash,a.generic_parent_start_sha256,FALSE) THEN
        RAISE EXCEPTION 'projection generic parent server History/governance mismatch'
            USING ERRCODE = 'AV001';
    END IF;

    SELECT * INTO v_receipt FROM pipeline.operation_artifact_receipts_v1 r
    WHERE r.admission_id = p_admission_id;
    IF FOUND THEN
        RETURN QUERY SELECT v_receipt.receipt_id, v_receipt.source_hash,
                            v_receipt.projection->'vector';
        RETURN;
    END IF;
    IF a.revoked_at IS NOT NULL OR a.expires_at <= clock_timestamp()
       OR NOT pipeline.operation_dag_governance_matches_v1(
           a.run_id,a.tenant_id,a.context_key,
           a.parent_workflow_id,a.parent_workflow_run_id,
           a.plan_hash,a.generic_parent_start_sha256,TRUE) THEN
        RAISE EXCEPTION 'projection owner admission expired/revoked or governance inactive'
            USING ERRCODE = 'AV001';
    END IF;

    SELECT * INTO s FROM pipeline.vector_projection_sources_v1
    WHERE run_id = p_run_id AND tenant_id = p_tenant_id
      AND context_key = p_context_key AND keyword_key = p_keyword_key
    FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'authoritative precomputed vector is absent'
            USING ERRCODE = 'AV001';
    END IF;
    IF EXISTS (SELECT 1 FROM jsonb_array_elements(s.vector) e
               WHERE jsonb_typeof(e.value) <> 'number') THEN
        RAISE EXCEPTION 'precomputed vector contains nonnumeric values'
            USING ERRCODE = 'AV001';
    END IF;
    v_source_hash := encode(public.digest(convert_to(
        p_tenant_id || '|' || p_run_id::text || '|' || p_context_key || '|' ||
        p_keyword_key || '|' || s.vector::text, 'UTF8'), 'sha256'), 'hex');
    IF v_source_hash <> a.source_hash THEN
        RAISE EXCEPTION 'precomputed vector differs from operator admitted source'
            USING ERRCODE = 'AV001';
    END IF;

    INSERT INTO pipeline.operation_artifact_receipts_v1
        (admission_id,run_id,tenant_id,context_key,keyword_key,plan_hash,
         operation_instance,dispatch_execution_key,parent_workflow_id,
         parent_workflow_run_id,child_workflow_id,child_workflow_run_id,
         source_hash,projection)
    VALUES
        (a.admission_id,a.run_id,a.tenant_id,a.context_key,a.keyword_key,
         a.plan_hash,a.operation_instance,a.dispatch_execution_key,
         a.parent_workflow_id,a.parent_workflow_run_id,a.child_workflow_id,
         a.child_workflow_run_id,v_source_hash,
         jsonb_build_object('tenant_id',p_tenant_id,
            'context_key',p_context_key,'keyword_key',p_keyword_key,'vector',s.vector));
    RETURN QUERY SELECT r.receipt_id,r.source_hash,r.projection->'vector'
    FROM pipeline.operation_artifact_receipts_v1 r
    WHERE r.admission_id = p_admission_id;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.consume_pg_vector_projection_owner_v1(
    UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BYTEA
) FROM PUBLIC;

-- Trusted neutral proof API. It leaks no source rows and is callable ONLY by
-- an independently granted operator role, never by a Qdrant Activity Worker.
CREATE FUNCTION pipeline.operation_artifact_receipt_matches_v1(
    p_receipt_id UUID,p_tenant_id TEXT,p_run_id UUID,
    p_context_key TEXT,p_keyword_key TEXT,p_plan_hash TEXT,
    p_operation_instance TEXT,p_execution_key TEXT,
    p_parent_workflow_id TEXT,p_parent_workflow_run_id TEXT,
    p_child_workflow_id TEXT,p_child_workflow_run_id TEXT,
    p_source_hash TEXT,p_projection JSONB
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
       AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles
                       WHERE rolname = session_user AND rolsuper) THEN
        RETURN FALSE;
    END IF;
    RETURN EXISTS (
        SELECT 1 FROM pipeline.operation_artifact_receipts_v1 r
        JOIN pipeline.pg_vector_projection_admissions_v1 a
            ON a.admission_id = r.admission_id
        WHERE r.receipt_id = p_receipt_id
          AND r.tenant_id = p_tenant_id AND r.run_id = p_run_id
          AND r.context_key = p_context_key AND r.keyword_key = p_keyword_key
          AND r.plan_hash = p_plan_hash
          AND r.operation_instance = p_operation_instance
          AND r.dispatch_execution_key = p_execution_key
          AND r.parent_workflow_id = p_parent_workflow_id
          AND r.parent_workflow_run_id = p_parent_workflow_run_id
          AND r.child_workflow_id = p_child_workflow_id
          AND r.child_workflow_run_id = p_child_workflow_run_id
          AND r.source_hash = p_source_hash
          AND r.projection = p_projection
          AND a.source_hash = r.source_hash
          AND a.tenant_id = r.tenant_id
          AND a.dispatch_execution_key = r.dispatch_execution_key
          AND a.revoked_at IS NULL
          AND a.expires_at > clock_timestamp()
          AND pipeline.operation_dag_governance_matches_v1(
                a.run_id,a.tenant_id,a.context_key,
                a.parent_workflow_id,a.parent_workflow_run_id,
                a.plan_hash,a.generic_parent_start_sha256,TRUE)
    );
END;
$$;
REVOKE ALL ON FUNCTION pipeline.operation_artifact_receipt_matches_v1(
    UUID,TEXT,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,JSONB
) FROM PUBLIC;

COMMIT;
