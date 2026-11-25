-- Distinct, opt-in tenant governance for native owner-neutral DAG runs.
-- A canonical source Truth run (seo_site_build) and the REAL generic Temporal
-- parent are DIFFERENT executions. Never overwrite execution_runs.workflow_*
-- to impersonate the generic workflow. No issuance is made by this migration:
-- a separate privileged operator must first attest the exact Temporal server
-- WorkflowStarted History, immutable plan/route and parent input SHA256.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE TABLE pipeline.operation_dag_run_governance_v1 (
    -- A canonical business Truth run may have later separately admitted DAG
    -- executions, but never two active conflicting generic parents at once.
    parent_workflow_run_id TEXT PRIMARY KEY
        CHECK (parent_workflow_run_id = btrim(parent_workflow_run_id)
               AND length(parent_workflow_run_id) > 0),
    canonical_run_id UUID NOT NULL REFERENCES pipeline.execution_runs(run_id),
    tenant_id TEXT NOT NULL CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0),
    context_key TEXT NOT NULL REFERENCES kb.visa_contexts(context_key),
    parent_workflow_id TEXT NOT NULL,
    parent_workflow_type TEXT NOT NULL
        CHECK (parent_workflow_type = 'AlegriaOperationDagV1Workflow'),
    parent_task_queue TEXT NOT NULL
        CHECK (parent_task_queue = 'alegria-operation-dag-v1'),
    plan_hash TEXT NOT NULL CHECK (plan_hash ~ '^[0-9a-f]{64}$'),
    parent_start_sha256 TEXT NOT NULL
        CHECK (parent_start_sha256 ~ '^[0-9a-f]{64}$'),
    -- Pinned original site-build Temporal run ID stays separate from the
    -- admitted generic DAG parent Temporal Run ID.
    canonical_source_workflow_run_id TEXT NOT NULL
        CHECK (length(btrim(canonical_source_workflow_run_id)) > 0),
    issued_by NAME NOT NULL DEFAULT session_user,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    CHECK (parent_workflow_id =
        'alegria-operation-dag-v1-' || canonical_run_id::text),
    CHECK (canonical_source_workflow_run_id <> parent_workflow_run_id),
    CHECK (expires_at > issued_at),
    CHECK (revoked_at IS NULL OR revoked_at >= issued_at),
    UNIQUE (parent_workflow_id,parent_workflow_run_id)
);
CREATE INDEX idx_operation_dag_run_governance_v1_canonical
    ON pipeline.operation_dag_run_governance_v1(canonical_run_id,expires_at);
REVOKE ALL ON pipeline.operation_dag_run_governance_v1 FROM PUBLIC;
ALTER TABLE pipeline.operation_dag_run_governance_v1 ENABLE ROW LEVEL SECURITY;
CREATE POLICY operation_dag_run_governance_v1_tenant
    ON pipeline.operation_dag_run_governance_v1
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

-- Read-only narrow source identifier lookup for the separately credentialed
-- generic governance issuer. It need not receive table-wide SELECT on the
-- legacy site-build journal. The actual guarded admission INSERT must still
-- recheck the source under the canonical run status advisory lock.
CREATE FUNCTION pipeline.operation_dag_canonical_source_run_id_v1(
    p_run_id UUID,p_tenant_id TEXT,p_context_key TEXT
) RETURNS TEXT LANGUAGE SQL SECURITY DEFINER
SET search_path = pg_catalog AS $$
    SELECT er.workflow_run_id FROM pipeline.execution_runs er
    JOIN pipeline.tenant_context_ownership o
      ON o.context_key = er.context_key AND o.tenant_id = er.tenant_id
    WHERE er.run_id = p_run_id
      AND er.tenant_id = p_tenant_id AND er.context_key = p_context_key
      AND er.workflow_type = 'seo_site_build'
      AND er.status IN ('verifying','pending_hitl')
      AND pipeline.current_session_tenant_v1() = p_tenant_id
$$;
REVOKE ALL ON FUNCTION pipeline.operation_dag_canonical_source_run_id_v1(
    UUID,TEXT,TEXT
) FROM PUBLIC;

CREATE FUNCTION pipeline.guard_operation_dag_run_governance_v1()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE v_source RECORD;
BEGIN
    -- Under repeatable-read an issuer can retain a snapshot predating a
    -- concurrent committed admission even after obtaining our advisory key.
    -- READ COMMITTED gives the post-lock SELECT a fresh serialization view.
    IF pg_catalog.current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'native DAG governance issuance/revocation requires READ COMMITTED'
            USING ERRCODE = 'AV001';
    END IF;
    IF TG_OP IN ('DELETE','TRUNCATE') THEN
        RAISE EXCEPTION 'operator DAG run governance cannot be deleted or truncated'
            USING ERRCODE = 'AV001';
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF pipeline.current_session_tenant_v1() IS DISTINCT FROM OLD.tenant_id
           OR OLD.issued_by <> session_user
           OR OLD.revoked_at IS NOT NULL
           OR NEW.revoked_at IS NULL
           OR (to_jsonb(NEW) - 'revoked_at') IS DISTINCT FROM
              (to_jsonb(OLD) - 'revoked_at') THEN
            RAISE EXCEPTION
                'generic run governance allows only issuer-owned monotonic revocation'
                USING ERRCODE = 'AV001';
        END IF;
        -- Same lock ORDER as issuance and first effect, ensuring a revocation
        -- cannot interleave between an owner checking active governance and
        -- its irreversible transaction / Qdrant first-start attempt.
        PERFORM pg_advisory_xact_lock(
            hashtextextended('truth-run-status|' || OLD.canonical_run_id::text, 0));
        PERFORM pg_advisory_xact_lock(
            hashtextextended('operation-dag-canonical|' || OLD.canonical_run_id::text, 0));
        NEW.revoked_at := clock_timestamp();
        RETURN NEW;
    END IF;
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM NEW.tenant_id
       OR NEW.issued_by <> session_user
       OR NEW.revoked_at IS NOT NULL
       OR NEW.parent_workflow_id IS DISTINCT FROM
          'alegria-operation-dag-v1-' || NEW.canonical_run_id::text
       OR NEW.parent_workflow_type <> 'AlegriaOperationDagV1Workflow'
       OR NEW.parent_task_queue <> 'alegria-operation-dag-v1' THEN
        RAISE EXCEPTION 'untrusted native DAG governance issuer/identity'
            USING ERRCODE = 'AV001';
    END IF;
    -- Truth-run status changes use the same existing advisory key.
    PERFORM pg_advisory_xact_lock(
        hashtextextended('truth-run-status|' || NEW.canonical_run_id::text, 0));
    PERFORM pg_advisory_xact_lock(
        hashtextextended('operation-dag-canonical|' || NEW.canonical_run_id::text, 0));
    SELECT er.context_key,er.tenant_id,er.workflow_type,er.status,
           er.workflow_run_id
    INTO v_source
    FROM pipeline.execution_runs er
    JOIN pipeline.tenant_context_ownership o
      ON o.context_key = er.context_key AND o.tenant_id = er.tenant_id
    WHERE er.run_id = NEW.canonical_run_id
    FOR SHARE OF er;
    IF NOT FOUND
       OR v_source.context_key IS DISTINCT FROM NEW.context_key
       OR v_source.tenant_id IS DISTINCT FROM NEW.tenant_id
       OR v_source.workflow_type IS DISTINCT FROM 'seo_site_build'
       OR v_source.status NOT IN ('verifying','pending_hitl')
       OR v_source.workflow_run_id IS DISTINCT FROM NEW.canonical_source_workflow_run_id
       OR EXISTS (
           SELECT 1 FROM pipeline.operation_dag_run_governance_v1 active
           WHERE active.canonical_run_id = NEW.canonical_run_id
             AND active.revoked_at IS NULL
             AND active.expires_at > clock_timestamp()
       ) THEN
        RAISE EXCEPTION
            'generic DAG admission denied by authoritative tenant/source/status or active DAG'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_operation_dag_run_governance_v1() FROM PUBLIC;
CREATE TRIGGER guard_operation_dag_run_governance_v1
    BEFORE INSERT OR UPDATE OR DELETE ON pipeline.operation_dag_run_governance_v1
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_operation_dag_run_governance_v1();
CREATE TRIGGER guard_operation_dag_run_governance_truncate_v1
    BEFORE TRUNCATE ON pipeline.operation_dag_run_governance_v1
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.guard_operation_dag_run_governance_v1();

-- EXECUTE-only tenant-scoped proof for individual owner operators, and for
-- the independent SQL SECURITY DEFINER first-start capabilities. No SELECT
-- on the platform governance table is granted to effect workers.
-- p_require_active=FALSE is reserved for exact immutable committed receipt
-- replay; a NEW provider RPC/PG effect MUST use TRUE under run-status lock.
CREATE FUNCTION pipeline.operation_dag_governance_matches_v1(
    p_canonical_run_id UUID,
    p_tenant_id TEXT,
    p_context_key TEXT,
    p_parent_workflow_id TEXT,
    p_parent_workflow_run_id TEXT,
    p_plan_hash TEXT,
    p_parent_start_sha256 TEXT,
    p_require_active BOOLEAN
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE v_allowed BOOLEAN;
BEGIN
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
       OR (p_require_active AND
           pg_catalog.current_setting('transaction_isolation') <> 'read committed')
       OR p_require_active IS NULL
       OR p_parent_workflow_id IS DISTINCT FROM
          'alegria-operation-dag-v1-' || p_canonical_run_id::text
       OR p_parent_start_sha256 IS NULL
       OR p_parent_start_sha256 !~ '^[0-9a-f]{64}$' THEN
        RETURN FALSE;
    END IF;
    SELECT EXISTS (
        SELECT 1 FROM pipeline.operation_dag_run_governance_v1 g
        JOIN pipeline.execution_runs er ON er.run_id = g.canonical_run_id
        JOIN pipeline.tenant_context_ownership o
          ON o.context_key = er.context_key AND o.tenant_id = er.tenant_id
        WHERE g.canonical_run_id = p_canonical_run_id
          AND g.tenant_id = p_tenant_id AND g.context_key = p_context_key
          AND g.parent_workflow_id = p_parent_workflow_id
          AND g.parent_workflow_run_id = p_parent_workflow_run_id
          AND g.parent_workflow_type = 'AlegriaOperationDagV1Workflow'
          AND g.parent_task_queue = 'alegria-operation-dag-v1'
          AND g.plan_hash = p_plan_hash
          AND g.parent_start_sha256 = p_parent_start_sha256
          AND er.context_key = p_context_key AND er.tenant_id = p_tenant_id
          AND er.workflow_type = 'seo_site_build'
          AND er.workflow_run_id = g.canonical_source_workflow_run_id
          AND (NOT p_require_active OR
               (g.revoked_at IS NULL AND g.expires_at > clock_timestamp()
                AND er.status IN ('verifying','pending_hitl')))
    ) INTO v_allowed;
    RETURN v_allowed;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.operation_dag_governance_matches_v1(
    UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BOOLEAN
) FROM PUBLIC;

-- DEPLOYMENT GATE (documentation only; no automatic privilege grants):
--  1. Provision a dedicated tenant-bound governance LOGIN with INSERT on
--     pipeline.operation_dag_run_governance_v1 and EXECUTE on
--     operation_dag_canonical_source_run_id_v1; no INSERT privilege on PG
--     or Qdrant owner admission tables, no SUPERUSER/BYPASSRLS/SET ROLE.
--  2. Give each separate PG/Qdrant operator LOGIN EXECUTE on
--     operation_dag_governance_matches_v1, never SELECT/INSERT on this table.
--  3. Give restricted PG Worker only EXECUTE on
--     consume_pg_truth_review_owner_v1 and restricted Qdrant Worker only
--     EXECUTE on begin/confirm_qdrant_projection_v3. V3 resignal EXECUTE
--     belongs only to the Qdrant operator, not the worker.
--  4. Test transactional issuer first-start/revocation/tenant/RLS races in
--     a disposable database BEFORE enabling any generic owner route.
-- Existing V3/V4 PG and V1/V2 Qdrant grants are intentionally untouched.

COMMIT;
