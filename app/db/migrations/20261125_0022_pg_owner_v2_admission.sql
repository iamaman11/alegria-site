-- Additive versioned governance for parallel-wave native Temporal DAG V2.
-- Historical V1 table, rows, SQL matcher and Workflow Histories remain frozen.
-- A new V2 parent Type/Task Queue/Workflow ID and server Run ID are mandatory.
BEGIN;
SET LOCAL lock_timeout = '5s';

-- Independent V2 business-run authority, not an SEO execution_runs alias.
-- Deployment grants INSERT/revocation only to a trusted tenant-authenticated
-- business issuer; DAG manifest/owner workers cannot create this authority.
CREATE TABLE pipeline.operation_dag_business_runs_v1 (
    run_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0),
    context_key TEXT NOT NULL CHECK (context_key = btrim(context_key) AND length(context_key) > 0),
    plan_hash TEXT NOT NULL CHECK (plan_hash ~ '^[0-9a-f]{64}$'),
    parent_workflow_id TEXT NOT NULL
        CHECK (parent_workflow_id = 'alegria-operation-dag-v2-' || run_id::text),
    issued_by NAME NOT NULL DEFAULT session_user,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    CHECK (expires_at > issued_at),
    CHECK (revoked_at IS NULL OR revoked_at >= issued_at)
);
REVOKE ALL ON pipeline.operation_dag_business_runs_v1 FROM PUBLIC;
ALTER TABLE pipeline.operation_dag_business_runs_v1 ENABLE ROW LEVEL SECURITY;
CREATE POLICY operation_dag_business_run_tenant_v1
    ON pipeline.operation_dag_business_runs_v1
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
CREATE FUNCTION pipeline.guard_operation_dag_business_run_v1()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    IF pg_catalog.current_setting('transaction_isolation') <> 'read committed' THEN
       RAISE EXCEPTION 'business authority requires READ COMMITTED' USING ERRCODE='AV001';
    END IF;
    IF TG_OP IN ('DELETE','TRUNCATE') THEN
       RAISE EXCEPTION 'generic business-run authority cannot be deleted' USING ERRCODE='AV001';
    END IF;
    IF TG_OP = 'UPDATE' THEN
       IF pipeline.current_session_tenant_v1() IS DISTINCT FROM OLD.tenant_id
          OR OLD.issued_by <> session_user
          OR OLD.revoked_at IS NOT NULL OR NEW.revoked_at IS NULL
          OR (to_jsonb(NEW)-'revoked_at') IS DISTINCT FROM
             (to_jsonb(OLD)-'revoked_at') THEN
          RAISE EXCEPTION 'business-run authority permits only issuer-owned revocation'
              USING ERRCODE='AV001';
       END IF;
       PERFORM pg_advisory_xact_lock(hashtextextended(
           'operation-dag-business|' || OLD.run_id::text,0));
       PERFORM pg_advisory_xact_lock(hashtextextended(
           'operation-dag-canonical|' || OLD.run_id::text,0));
       NEW.revoked_at := clock_timestamp();
       RETURN NEW;
    END IF;
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM NEW.tenant_id
       OR NEW.issued_by <> session_user OR NEW.revoked_at IS NOT NULL
       OR has_table_privilege(session_user,
          'pipeline.operation_dag_run_governance_v2','INSERT')
       OR EXISTS (SELECT 1 FROM pg_catalog.pg_roles r
          WHERE r.rolname=session_user
            AND (r.rolsuper OR r.rolbypassrls OR r.rolcreaterole
                 OR r.rolcreatedb OR r.rolreplication)) THEN
       RAISE EXCEPTION 'untrusted business-run issuer or tenant' USING ERRCODE='AV001';
    END IF;
    -- A tenant-bound LOGIN alone cannot reassign a context owned by another
    -- tenant. The authoritative context ownership registry is the root of
    -- business-scope identity, independent of old seo_site_build phases.
    IF NOT EXISTS (
       SELECT 1 FROM pipeline.tenant_context_ownership own
       WHERE own.tenant_id = NEW.tenant_id
         AND own.context_key = NEW.context_key
    ) THEN
       RAISE EXCEPTION 'business-run issuer does not own canonical context'
           USING ERRCODE='AV001';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended(
       'operation-dag-business|' || NEW.run_id::text,0));
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_operation_dag_business_run_v1() FROM PUBLIC;
CREATE TRIGGER guard_operation_dag_business_run_v1
    BEFORE INSERT OR UPDATE OR DELETE ON pipeline.operation_dag_business_runs_v1
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_operation_dag_business_run_v1();
CREATE TRIGGER guard_operation_dag_business_run_truncate_v1
    BEFORE TRUNCATE ON pipeline.operation_dag_business_runs_v1
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.guard_operation_dag_business_run_v1();

CREATE TABLE pipeline.operation_dag_run_governance_v2 (
    parent_workflow_run_id TEXT PRIMARY KEY
        CHECK (parent_workflow_run_id = btrim(parent_workflow_run_id)
               AND length(parent_workflow_run_id) > 0),
    canonical_run_id UUID NOT NULL REFERENCES pipeline.operation_dag_business_runs_v1(run_id),
    tenant_id TEXT NOT NULL CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0),
    context_key TEXT NOT NULL,
    parent_workflow_id TEXT NOT NULL,
    parent_workflow_type TEXT NOT NULL
        CHECK (parent_workflow_type = 'AlegriaOperationDagV2Workflow'),
    parent_task_queue TEXT NOT NULL
        CHECK (parent_task_queue = 'alegria-operation-dag-v2'),
    plan_hash TEXT NOT NULL CHECK (plan_hash ~ '^[0-9a-f]{64}$'),
    parent_start_sha256 TEXT NOT NULL CHECK (parent_start_sha256 ~ '^[0-9a-f]{64}$'),
    issued_by NAME NOT NULL DEFAULT session_user,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    CHECK (parent_workflow_id =
        'alegria-operation-dag-v2-' || canonical_run_id::text),
    CHECK (expires_at > issued_at),
    CHECK (revoked_at IS NULL OR revoked_at >= issued_at),
    UNIQUE (parent_workflow_id,parent_workflow_run_id)
);
CREATE INDEX idx_operation_dag_run_governance_v2_canonical
    ON pipeline.operation_dag_run_governance_v2(canonical_run_id,expires_at);
REVOKE ALL ON pipeline.operation_dag_run_governance_v2 FROM PUBLIC;
ALTER TABLE pipeline.operation_dag_run_governance_v2 ENABLE ROW LEVEL SECURITY;
CREATE POLICY operation_dag_run_governance_v2_tenant
    ON pipeline.operation_dag_run_governance_v2
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

CREATE FUNCTION pipeline.guard_operation_dag_run_governance_v2()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE v_source RECORD;
BEGIN
    IF pg_catalog.current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'V2 governance issuance/revocation requires READ COMMITTED'
            USING ERRCODE = 'AV001';
    END IF;
    IF TG_OP IN ('DELETE','TRUNCATE') THEN
        RAISE EXCEPTION 'V2 governance cannot be deleted/truncated'
            USING ERRCODE = 'AV001';
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF pipeline.current_session_tenant_v1() IS DISTINCT FROM OLD.tenant_id
           OR OLD.issued_by <> session_user
           OR OLD.revoked_at IS NOT NULL OR NEW.revoked_at IS NULL
           OR (to_jsonb(NEW) - 'revoked_at') IS DISTINCT FROM
              (to_jsonb(OLD) - 'revoked_at') THEN
            RAISE EXCEPTION 'V2 governance permits only issuer-owned monotonic revocation'
                USING ERRCODE = 'AV001';
        END IF;
        PERFORM pg_advisory_xact_lock(
            hashtextextended('operation-dag-business|' || OLD.canonical_run_id::text,0));
        PERFORM pg_advisory_xact_lock(
            hashtextextended('operation-dag-canonical|' || OLD.canonical_run_id::text,0));
        NEW.revoked_at := clock_timestamp();
        RETURN NEW;
    END IF;
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM NEW.tenant_id
       OR NEW.issued_by <> session_user
       OR NEW.revoked_at IS NOT NULL
       OR has_table_privilege(session_user,
          'pipeline.operation_dag_business_runs_v1','INSERT')
       OR has_table_privilege(session_user,
          'pipeline.operation_dag_business_runs_v1','UPDATE')
       OR NEW.parent_workflow_id IS DISTINCT FROM
          'alegria-operation-dag-v2-' || NEW.canonical_run_id::text
       OR NEW.parent_workflow_type <> 'AlegriaOperationDagV2Workflow'
       OR NEW.parent_task_queue <> 'alegria-operation-dag-v2' THEN
        RAISE EXCEPTION 'untrusted V2 DAG governance issuer/identity'
            USING ERRCODE = 'AV001';
    END IF;
    PERFORM pg_advisory_xact_lock(
        hashtextextended('operation-dag-business|' || NEW.canonical_run_id::text,0));
    PERFORM pg_advisory_xact_lock(
        hashtextextended('operation-dag-canonical|' || NEW.canonical_run_id::text,0));
    SELECT br.context_key,br.tenant_id,br.plan_hash,br.parent_workflow_id,
           br.expires_at,br.revoked_at
    INTO v_source
    FROM pipeline.operation_dag_business_runs_v1 br
    WHERE br.run_id = NEW.canonical_run_id;
    IF NOT FOUND
       OR v_source.context_key IS DISTINCT FROM NEW.context_key
       OR v_source.tenant_id IS DISTINCT FROM NEW.tenant_id
       OR v_source.revoked_at IS NOT NULL
       OR v_source.expires_at <= clock_timestamp()
       OR v_source.plan_hash IS DISTINCT FROM NEW.plan_hash
       OR v_source.parent_workflow_id IS DISTINCT FROM NEW.parent_workflow_id
       OR NOT EXISTS (
           SELECT 1 FROM pipeline.tenant_context_ownership own
           WHERE own.tenant_id = NEW.tenant_id
             AND own.context_key = NEW.context_key
       )
       OR EXISTS (
          SELECT 1 FROM pipeline.operation_dag_run_governance_v1 active
          WHERE active.canonical_run_id = NEW.canonical_run_id
            AND active.revoked_at IS NULL
            AND active.expires_at > clock_timestamp()
       ) OR EXISTS (
          SELECT 1 FROM pipeline.operation_dag_run_governance_v2 active
          WHERE active.canonical_run_id = NEW.canonical_run_id
            AND active.revoked_at IS NULL
            AND active.expires_at > clock_timestamp()
       ) THEN
        RAISE EXCEPTION 'V2 run denied by independent business authority or active DAG'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_operation_dag_run_governance_v2() FROM PUBLIC;
CREATE TRIGGER guard_operation_dag_run_governance_v2
    BEFORE INSERT OR UPDATE OR DELETE ON pipeline.operation_dag_run_governance_v2
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_operation_dag_run_governance_v2();
CREATE TRIGGER guard_operation_dag_run_governance_truncate_v2
    BEFORE TRUNCATE ON pipeline.operation_dag_run_governance_v2
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.guard_operation_dag_run_governance_v2();

-- Additional V1 insertion guard: a V1 issuer must not create a conflicting
-- live parent after a V2 parent was already admitted. The historical V1
-- trigger remains unchanged and still checks all of its original conditions.
CREATE FUNCTION pipeline.guard_v1_run_against_active_v2()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    PERFORM pg_advisory_xact_lock(
        hashtextextended('truth-run-status|' || NEW.canonical_run_id::text,0));
    PERFORM pg_advisory_xact_lock(
        hashtextextended('operation-dag-canonical|' || NEW.canonical_run_id::text,0));
    IF EXISTS (SELECT 1 FROM pipeline.operation_dag_run_governance_v2 active
       WHERE active.canonical_run_id = NEW.canonical_run_id
         AND active.revoked_at IS NULL AND active.expires_at > clock_timestamp()) THEN
       RAISE EXCEPTION 'V1 cannot start while V2 governance is active'
           USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_v1_run_against_active_v2() FROM PUBLIC;
CREATE TRIGGER guard_v1_run_against_active_v2
    BEFORE INSERT ON pipeline.operation_dag_run_governance_v1
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_v1_run_against_active_v2();

CREATE FUNCTION pipeline.operation_dag_governance_matches_v2(
    p_canonical_run_id UUID,p_tenant_id TEXT,p_context_key TEXT,
    p_parent_workflow_id TEXT,p_parent_workflow_run_id TEXT,
    p_plan_hash TEXT,p_parent_start_sha256 TEXT,p_require_active BOOLEAN
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
       OR (p_require_active AND
           pg_catalog.current_setting('transaction_isolation') <> 'read committed')
       OR p_require_active IS NULL
       OR p_parent_workflow_id IS DISTINCT FROM
          'alegria-operation-dag-v2-' || p_canonical_run_id::text
       OR p_parent_start_sha256 IS NULL
       OR p_parent_start_sha256 !~ '^[0-9a-f]{64}$' THEN
        RETURN FALSE;
    END IF;
    RETURN EXISTS (
        SELECT 1 FROM pipeline.operation_dag_run_governance_v2 g
        JOIN pipeline.operation_dag_business_runs_v1 br
          ON br.run_id = g.canonical_run_id
        JOIN pipeline.tenant_context_ownership own
          ON own.tenant_id = br.tenant_id
         AND own.context_key = br.context_key
        WHERE g.canonical_run_id = p_canonical_run_id
          AND g.tenant_id = p_tenant_id AND g.context_key = p_context_key
          AND g.parent_workflow_id = p_parent_workflow_id
          AND g.parent_workflow_run_id = p_parent_workflow_run_id
          AND g.parent_workflow_type = 'AlegriaOperationDagV2Workflow'
          AND g.parent_task_queue = 'alegria-operation-dag-v2'
          AND g.plan_hash = p_plan_hash
          AND g.parent_start_sha256 = p_parent_start_sha256
          AND br.context_key = p_context_key AND br.tenant_id = p_tenant_id
          AND br.plan_hash = p_plan_hash
          AND br.parent_workflow_id = p_parent_workflow_id
          AND (NOT p_require_active OR
            (g.revoked_at IS NULL AND g.expires_at > clock_timestamp()
             AND br.revoked_at IS NULL AND br.expires_at > clock_timestamp()))
    );
END;
$$;
REVOKE ALL ON FUNCTION pipeline.operation_dag_governance_matches_v2(
    UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BOOLEAN
) FROM PUBLIC;

-- Backward-compatible version marker for PG-owned projection admission.
-- No historical V1 receipt or admission row is rewritten.
ALTER TABLE pipeline.pg_vector_projection_admissions_v1
    ADD COLUMN parent_protocol_revision SMALLINT NOT NULL DEFAULT 1
    CHECK(parent_protocol_revision IN (1,2));

-- A new, separately named SQL capability. Do NOT relax or replace the V1
-- consumer: old receipts/Workflow Histories still use the V1 governance seal.
CREATE FUNCTION pipeline.consume_pg_vector_projection_owner_v2(
    p_admission_id UUID,p_run_id UUID,p_tenant_id TEXT,
    p_context_key TEXT,p_keyword_key TEXT,
    p_child_workflow_id TEXT,p_child_workflow_run_id TEXT,
    p_operation_instance TEXT,p_plan_hash TEXT,
    p_contract_fingerprint TEXT,p_execution_key TEXT,
    p_parent_workflow_id TEXT,p_parent_workflow_run_id TEXT,
    p_invocation_bytes BYTEA
) RETURNS TABLE(receipt_id UUID,source_hash TEXT,vector JSONB)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    a RECORD;
    s RECORD;
    v_source_hash TEXT;
    prior RECORD;
BEGIN
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
       OR p_parent_workflow_id IS DISTINCT FROM
          'alegria-operation-dag-v2-' || p_run_id::text
       OR p_child_workflow_id IS DISTINCT FROM
          'alegria-owner-operation-v2-' || p_execution_key
       OR p_execution_key !~ '^[0-9a-f]{64}$'
       OR p_plan_hash !~ '^[0-9a-f]{64}$'
       OR p_contract_fingerprint !~ '^[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'V2 projection tenant/identity/hash denied'
            USING ERRCODE='AV001';
    END IF;
    -- Root revocation uses business then canonical lock; this owner follows
    -- the same order so a source read cannot race admission revocation.
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'operation-dag-business|' || p_run_id::text,0));
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'operation-dag-canonical|' || p_run_id::text,0));
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'pg-vector-admission|' || p_admission_id::text,0));
    SELECT * INTO a FROM pipeline.pg_vector_projection_admissions_v1
    WHERE admission_id=p_admission_id
      AND parent_protocol_revision=2
      AND run_id=p_run_id AND tenant_id=p_tenant_id
      AND context_key=p_context_key AND keyword_key=p_keyword_key
      AND child_workflow_id=p_child_workflow_id
      AND child_workflow_run_id=p_child_workflow_run_id
      AND operation_instance=p_operation_instance
      AND plan_hash=p_plan_hash
      AND contract_fingerprint=p_contract_fingerprint
      AND dispatch_execution_key=p_execution_key
      AND parent_workflow_id=p_parent_workflow_id
      AND parent_workflow_run_id=p_parent_workflow_run_id
      AND invocation_sha256=encode(public.digest(p_invocation_bytes,'sha256'),'hex');
    IF NOT FOUND OR NOT pipeline.operation_dag_governance_matches_v2(
        a.run_id,a.tenant_id,a.context_key,
        a.parent_workflow_id,a.parent_workflow_run_id,
        a.plan_hash,a.generic_parent_start_sha256,FALSE) THEN
        RAISE EXCEPTION 'V2 producer admission or exact server History denied'
            USING ERRCODE='AV001';
    END IF;
    SELECT * INTO prior FROM pipeline.operation_artifact_receipts_v1 r
    WHERE r.admission_id=p_admission_id;
    IF FOUND THEN
        RETURN QUERY SELECT prior.receipt_id,prior.source_hash,prior.projection->'vector';
        RETURN;
    END IF;
    IF a.revoked_at IS NOT NULL OR a.expires_at <= clock_timestamp()
       OR NOT pipeline.operation_dag_governance_matches_v2(
          a.run_id,a.tenant_id,a.context_key,
          a.parent_workflow_id,a.parent_workflow_run_id,
          a.plan_hash,a.generic_parent_start_sha256,TRUE) THEN
        RAISE EXCEPTION 'V2 producer source admission/revocation fence denied'
            USING ERRCODE='AV001';
    END IF;
    SELECT * INTO s FROM pipeline.vector_projection_sources_v1
    WHERE run_id=p_run_id AND tenant_id=p_tenant_id
      AND context_key=p_context_key AND keyword_key=p_keyword_key
    FOR SHARE;
    IF NOT FOUND OR EXISTS (
       SELECT 1 FROM jsonb_array_elements(s.vector) e
       WHERE jsonb_typeof(e.value)<>'number') THEN
        RAISE EXCEPTION 'V2 source precomputed vector absent/invalid'
            USING ERRCODE='AV001';
    END IF;
    v_source_hash:=encode(public.digest(convert_to(
        p_tenant_id || '|' || p_run_id::text || '|' ||
        p_context_key || '|' || p_keyword_key || '|' || s.vector::text,
        'UTF8'),'sha256'),'hex');
    IF v_source_hash <> a.source_hash THEN
        RAISE EXCEPTION 'V2 producer source changed since operator admission'
            USING ERRCODE='AV001';
    END IF;
    INSERT INTO pipeline.operation_artifact_receipts_v1
        (admission_id,run_id,tenant_id,context_key,keyword_key,plan_hash,
         operation_instance,dispatch_execution_key,parent_workflow_id,
         parent_workflow_run_id,child_workflow_id,child_workflow_run_id,
         source_hash,projection)
    VALUES (a.admission_id,a.run_id,a.tenant_id,a.context_key,a.keyword_key,
        a.plan_hash,a.operation_instance,a.dispatch_execution_key,
        a.parent_workflow_id,a.parent_workflow_run_id,a.child_workflow_id,
        a.child_workflow_run_id,v_source_hash,
        jsonb_build_object('tenant_id',p_tenant_id,'context_key',p_context_key,
        'keyword_key',p_keyword_key,'vector',s.vector));
    RETURN QUERY SELECT r.receipt_id,r.source_hash,r.projection->'vector'
    FROM pipeline.operation_artifact_receipts_v1 r
    WHERE r.admission_id=p_admission_id;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.consume_pg_vector_projection_owner_v2(
    UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BYTEA
) FROM PUBLIC;

CREATE FUNCTION pipeline.operation_artifact_receipt_matches_v2(
    p_receipt_id UUID,p_tenant_id TEXT,p_run_id UUID,
    p_context_key TEXT,p_keyword_key TEXT,p_plan_hash TEXT,
    p_operation_instance TEXT,p_execution_key TEXT,
    p_parent_workflow_id TEXT,p_parent_workflow_run_id TEXT,
    p_child_workflow_id TEXT,p_child_workflow_run_id TEXT,
    p_source_hash TEXT,p_projection JSONB
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
       OR p_parent_workflow_id IS DISTINCT FROM
          'alegria-operation-dag-v2-' || p_run_id::text
       OR p_child_workflow_id IS DISTINCT FROM
          'alegria-owner-operation-v2-' || p_execution_key THEN
        RETURN FALSE;
    END IF;
    RETURN EXISTS (
        SELECT 1 FROM pipeline.operation_artifact_receipts_v1 r
        JOIN pipeline.pg_vector_projection_admissions_v1 a
          ON a.admission_id=r.admission_id
        WHERE r.receipt_id=p_receipt_id
          AND a.parent_protocol_revision=2
          AND r.tenant_id=p_tenant_id AND r.run_id=p_run_id
          AND r.context_key=p_context_key AND r.keyword_key=p_keyword_key
          AND r.plan_hash=p_plan_hash
          AND r.operation_instance=p_operation_instance
          AND r.dispatch_execution_key=p_execution_key
          AND r.parent_workflow_id=p_parent_workflow_id
          AND r.parent_workflow_run_id=p_parent_workflow_run_id
          AND r.child_workflow_id=p_child_workflow_id
          AND r.child_workflow_run_id=p_child_workflow_run_id
          AND r.source_hash=p_source_hash
          AND r.projection=p_projection
          AND a.source_hash=r.source_hash AND a.tenant_id=r.tenant_id
          AND a.dispatch_execution_key=r.dispatch_execution_key
          AND a.revoked_at IS NULL AND a.expires_at>clock_timestamp()
          AND pipeline.operation_dag_governance_matches_v2(
             a.run_id,a.tenant_id,a.context_key,
             a.parent_workflow_id,a.parent_workflow_run_id,
             a.plan_hash,a.generic_parent_start_sha256,TRUE)
    );
END;
$$;
REVOKE ALL ON FUNCTION pipeline.operation_artifact_receipt_matches_v2(
    UUID,TEXT,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,JSONB
) FROM PUBLIC;

COMMIT;
