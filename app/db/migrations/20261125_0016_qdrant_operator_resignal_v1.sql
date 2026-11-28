-- A committed operator admission can outlive its Temporal wakeup Signal.
-- Recovering the wakeup must independently re-attest both server Histories,
-- then prove that the EXISTING immutable row is exactly the one issued by
-- this tenant-bound operator. No admission INSERT/UPDATE and no provider RPC.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.verify_qdrant_operator_resignal_v1(
    p_admission_id UUID,
    p_run_id UUID,
    p_tenant_id TEXT,
    p_context_key TEXT,
    p_keyword_key TEXT,
    p_parent_workflow_id TEXT,
    p_parent_workflow_run_id TEXT,
    p_child_workflow_id TEXT,
    p_child_workflow_run_id TEXT,
    p_child_started_event_id BIGINT,
    p_parent_input_bytes BYTEA,
    p_command_bytes BYTEA
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE v_actor RECORD;
BEGIN
    -- A recovery verifier performs NO effect: there is no reason to let an
    -- operator with EXECUTE lock arbitrary canonical runs. The later SQL
    -- provider START has its own serialized run-status fence (0014).
    SELECT r.rolsuper, r.rolbypassrls INTO v_actor
      FROM pg_roles r WHERE r.rolname = session_user;
    IF NOT FOUND OR v_actor.rolsuper OR v_actor.rolbypassrls
       OR pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
       OR octet_length(p_parent_input_bytes) > 2 * 1024 * 1024
       OR octet_length(p_command_bytes) > 2 * 1024 * 1024
       OR NOT has_table_privilege(session_user,
             'pipeline.qdrant_projection_v1_admissions', 'INSERT')
       OR NOT EXISTS (
           SELECT 1 FROM pipeline.qdrant_projection_v1_admissions a
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
             AND a.operation_instance = 'keyword-' || p_admission_id::text
             AND a.plan_sha256 = encode(public.digest(p_parent_input_bytes,'sha256'),'hex')
             AND a.command_sha256 = encode(public.digest(p_command_bytes,'sha256'),'hex')
             AND a.command_bytes = p_command_bytes
             AND a.revoked_at IS NULL AND a.expires_at > clock_timestamp()
             AND er.tenant_id = p_tenant_id AND er.context_key = p_context_key
             AND er.workflow_run_id = p_parent_workflow_run_id
             AND er.workflow_type = 'seo_site_build'
             AND er.status IN ('verifying','pending_hitl')
       )
    THEN
        RAISE EXCEPTION 'Qdrant operator re-signal denied: no exact live immutable admission'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN TRUE;
END;
$$;

-- Explicit DBA grant to a dedicated operator LOGIN is required. Never grant
-- the operator SELECT on admissions, or grant this helper to PUBLIC.
REVOKE ALL ON FUNCTION pipeline.verify_qdrant_operator_resignal_v1(
    UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BIGINT,BYTEA,BYTEA
) FROM PUBLIC;
COMMIT;
