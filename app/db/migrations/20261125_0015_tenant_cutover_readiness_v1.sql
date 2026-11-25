-- V8 production tenant cutover PRECONDITIONS ONLY. This migration does not
-- assign historical tenant IDs, grant production credentials, FORCE RLS,
-- transfer ownership, change Task Queue routing, or authorize deployment.
-- A zero-quarantine/forced-policy result remains a candidate for independent
-- production history, connection-identity and real cross-tenant acceptance.
BEGIN;
SET LOCAL lock_timeout = '5s';

-- This SECURITY DEFINER helper reads every tenant's raw evidence and run
-- state. PUBLIC could previously query any known tenant/context/run triple
-- and learn a cross-tenant readiness bit despite ordinary table RLS.
-- Runtime admission/receipt triggers call it as their own privileged definer;
-- a newly provisioned operator must receive a *specific* DBA-reviewed grant.
REVOKE ALL ON FUNCTION pipeline.tenant_truth_scope_ready_v1(TEXT,TEXT,UUID) FROM PUBLIC;

-- PostgreSQL checks a function's EXECUTE privilege when attaching a trigger,
-- not on each firing. A user with TEMP-table CREATE and USAGE on these schemas
-- could otherwise attach a privileged trigger to attacker-chosen NEW values,
-- receive an RLS-bypassing tenant_id, or invoke an owner-only mutation path.
-- All of these functions are trigger-only; their already installed triggers
-- continue to fire under the table's own DML permissions after REVOKE.
REVOKE ALL ON FUNCTION pipeline.guard_execution_run_truth_context() FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.guard_verified_truth_step_context() FROM PUBLIC;
REVOKE ALL ON FUNCTION site.guard_truth_review_hitl_context() FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.enforce_pg_effect_admission_tenant_v1() FROM PUBLIC;
REVOKE ALL ON FUNCTION site.bind_new_truth_review_tenant_v1() FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.enforce_pg_effect_receipt_tenant_v1() FROM PUBLIC;
REVOKE ALL ON FUNCTION kb.guard_truth_source_authority_writer_v1() FROM PUBLIC;
REVOKE ALL ON FUNCTION raw.stamp_ingress_page_tenant_v1() FROM PUBLIC;
REVOKE ALL ON FUNCTION raw.stamp_ingress_section_tenant_v1() FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.stamp_ingress_run_tenant_v1() FROM PUBLIC;
REVOKE ALL ON FUNCTION extracted.stamp_ingress_candidate_tenant_v1() FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.guard_qdrant_projection_admission_v1() FROM PUBLIC;

-- Detect a forgotten SECURITY DEFINER trigger or an unsafe explicit role or
-- PUBLIC grant while installing this forward-only migration. Do not auto-grant
-- private schema functions to worker/operator logins.
DO $$
DECLARE v_exposed_function TEXT;
BEGIN
    SELECT n.nspname || '.' || p.proname INTO v_exposed_function
    FROM pg_catalog.pg_proc p
    JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
    CROSS JOIN LATERAL pg_catalog.aclexplode(
        COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) grant_acl
    WHERE n.nspname IN ('pipeline','raw','extracted','site','kb')
      AND p.prorettype='pg_catalog.trigger'::regtype AND p.prosecdef
      AND grant_acl.grantee<>p.proowner AND grant_acl.privilege_type='EXECUTE'
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'SECURITY DEFINER trigger has non-owner EXECUTE grant: %',
            v_exposed_function USING ERRCODE='AV001';
    END IF;
END;
$$;

-- Even a deliberately granted operator must not turn the definer into an
-- oracle about a different tenant. A table owner's privilege does not make
-- the calling LOGIN an authority; only the DBA-owned identity ledger does.
-- The privilege check uses session_user (not SET ROLE/current_user or GUCs),
-- and keeps an explicit independent-superuser path for DBA diagnostics.
CREATE OR REPLACE FUNCTION pipeline.tenant_truth_scope_ready_v1(
    p_tenant TEXT, p_context TEXT, p_run UUID
) RETURNS BOOLEAN LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog AS $$
    SELECT (
        COALESCE((SELECT r.rolsuper FROM pg_catalog.pg_roles r
                  WHERE r.rolname = session_user), FALSE)
        OR (p_tenant IS NOT NULL AND
            p_tenant = pipeline.current_session_tenant_v1())
    )
    AND EXISTS (
        SELECT 1 FROM pipeline.tenant_context_ownership o
        JOIN pipeline.execution_runs er ON er.context_key = o.context_key
        WHERE o.context_key = p_context AND o.tenant_id = p_tenant
          AND er.run_id = p_run AND er.tenant_id = o.tenant_id
    )
    AND EXISTS (
        SELECT 1 FROM extracted.rule_candidates c
        WHERE c.context_key = p_context AND c.tenant_id = p_tenant
    )
    AND NOT EXISTS (
        SELECT 1 FROM extracted.rule_candidates c
        LEFT JOIN raw.sections source_section ON source_section.id = c.raw_section_id
        LEFT JOIN raw.pages source_page ON source_page.id = source_section.page_id
        LEFT JOIN raw.sections evidence_section ON evidence_section.id = c.evidence_section_id
        LEFT JOIN raw.pages evidence_page ON evidence_page.id = evidence_section.page_id
        WHERE c.context_key = p_context AND (
            c.tenant_id IS DISTINCT FROM p_tenant
            OR source_section.tenant_id IS DISTINCT FROM p_tenant
            OR source_page.tenant_id IS DISTINCT FROM p_tenant
            OR evidence_section.tenant_id IS DISTINCT FROM p_tenant
            OR evidence_page.tenant_id IS DISTINCT FROM p_tenant
        )
    )
    AND NOT EXISTS (
        SELECT 1 FROM site.seo_hitl_tasks t
        WHERE t.task_type = 'truth_candidate_review' AND t.scope_signature = p_context
          AND t.tenant_id IS DISTINCT FROM p_tenant
    );
$$;

CREATE OR REPLACE FUNCTION pipeline.assert_tenant_force_rls_preflight_v1()
RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE
    v_bad_relation TEXT;
    v_bad_role NAME;
    v_count BIGINT;
    v_audit_owner RECORD;
BEGIN
    -- A tenant login must not be able to enumerate the global quarantine or
    -- certify its own policy and credential privileges.
    IF NOT COALESCE((SELECT r.rolsuper OR r.rolbypassrls FROM pg_catalog.pg_roles r
                     WHERE r.rolname=session_user), false) THEN
        RAISE EXCEPTION 'FORCE RLS cutover preflight requires an independent DBA session'
            USING ERRCODE='AV001';
    END IF;

    SELECT n.nspname || '.' || p.proname INTO v_bad_relation
    FROM pg_catalog.pg_proc p
    JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
    CROSS JOIN LATERAL pg_catalog.aclexplode(
        COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) grant_acl
    WHERE n.nspname IN ('pipeline','raw','extracted','site','kb')
      AND p.prorettype='pg_catalog.trigger'::regtype AND p.prosecdef
      AND grant_acl.grantee<>p.proowner AND grant_acl.privilege_type='EXECUTE'
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'non-owner EXECUTE on SECURITY DEFINER trigger blocks cutover: %',
            v_bad_relation USING ERRCODE='AV001';
    END IF;

    -- Once FORCE RLS is installed, a SECURITY DEFINER with row_security=off
    -- cannot read all rows unless its real owner is explicitly BYPASSRLS or
    -- superuser. Refuse a misleading partial census instead of returning GO.
    FOR v_audit_owner IN
        SELECT p.proname, r.rolname, r.rolsuper, r.rolbypassrls
        FROM pg_catalog.pg_proc p
        JOIN pg_catalog.pg_roles r ON r.oid=p.proowner
        WHERE p.oid IN (
            'pipeline.tenant_historical_quarantine_v1()'::regprocedure,
            'pipeline.assert_no_tenant_quarantine_v1()'::regprocedure,
            'pipeline.assert_tenant_force_rls_preflight_v1()'::regprocedure)
    LOOP
        IF NOT v_audit_owner.rolsuper AND NOT v_audit_owner.rolbypassrls THEN
            RAISE EXCEPTION 'global tenant audit function % has non-bypass owner %',
                v_audit_owner.proname, v_audit_owner.rolname USING ERRCODE='AV001';
        END IF;
    END LOOP;

    -- Check every authenticated LOGIN *before* the expensive global history
    -- census. Even a NOBYPASSRLS LOGIN can SET ROLE to an unrelated privileged
    -- group role and defeat FORCE RLS; direct role flags and membership in
    -- the protected tables' owner roles are not sufficient.
    SELECT binding.role_name INTO v_bad_role
    FROM pipeline.tenant_role_ownership binding
    LEFT JOIN pg_catalog.pg_roles r ON r.rolname=binding.role_name
    WHERE r.oid IS NULL OR NOT r.rolcanlogin OR r.rolsuper OR r.rolbypassrls
       OR r.rolcreaterole OR r.rolcreatedb OR r.rolreplication
       OR EXISTS (
           SELECT 1 FROM pg_catalog.pg_roles elevated
           WHERE (elevated.rolsuper OR elevated.rolbypassrls
                  OR elevated.rolcreaterole OR elevated.rolcreatedb
                  OR elevated.rolreplication)
             AND pg_catalog.pg_has_role(r.oid,elevated.oid,'MEMBER'))
       OR EXISTS (
           SELECT 1 FROM pg_catalog.pg_class rel
           WHERE rel.oid=ANY(ARRAY[
               'raw.pages'::regclass,'raw.sections'::regclass,
               'extracted.rule_candidates'::regclass,'pipeline.execution_runs'::regclass,
               'site.seo_hitl_tasks'::regclass,'pipeline.pg_effect_v3_admissions'::regclass,
               'pipeline.pg_effect_v3_receipts'::regclass,
               'pipeline.qdrant_projection_v1_admissions'::regclass,
               'pipeline.qdrant_projection_v1_attempts'::regclass,
               'pipeline.qdrant_projection_v1_receipts'::regclass,
               -- Unscoped authority registries are not protected by tenant
               -- RLS: owner membership can mint new role/context bindings or
               -- promote a source to canonical Truth without UPDATE rights.
               'pipeline.tenant_role_ownership'::regclass,
               'pipeline.tenant_context_ownership'::regclass,
               'kb.sources'::regclass,'kb.visa_contexts'::regclass])
             AND pg_catalog.pg_has_role(r.oid,rel.relowner,'MEMBER'))
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'tenant-bound LOGIN % still has unsafe owner/bypass privileges',v_bad_role
            USING ERRCODE='AV001';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pipeline.tenant_role_ownership) THEN
        RAISE EXCEPTION 'no authenticated tenant LOGINs have been provisioned'
            USING ERRCODE='AV001';
    END IF;

    -- No extra permissive policy may silently turn an exact tenant predicate
    -- into a broader OR. Compare the canonical expression, not merely the
    -- policy name; parentheses/whitespace are PostgreSQL pretty-print detail.
    SELECT target.relation_name INTO v_bad_relation
    FROM (VALUES
        ('raw.pages','raw_pages_tenant_v1'),
        ('raw.sections','raw_sections_tenant_v1'),
        ('extracted.rule_candidates','extracted_candidates_tenant_v1'),
        ('pipeline.execution_runs','execution_runs_tenant_v1'),
        ('site.seo_hitl_tasks','site_hitl_tasks_tenant_v1'),
        ('pipeline.pg_effect_v3_admissions','pg_effect_admissions_tenant_v1'),
        ('pipeline.pg_effect_v3_receipts','pg_effect_receipts_tenant_v1'),
        ('pipeline.qdrant_projection_v1_admissions','qdrant_admission_tenant_v1'),
        ('pipeline.qdrant_projection_v1_attempts','qdrant_attempt_tenant_v1'),
        ('pipeline.qdrant_projection_v1_receipts','qdrant_receipt_tenant_v1')
    ) AS target(relation_name, expected_policy)
    JOIN pg_catalog.pg_class rel ON rel.oid=target.relation_name::regclass
    CROSS JOIN LATERAL (
        SELECT count(*) AS policy_count,
               count(*) FILTER (WHERE p.polname=target.expected_policy
                   AND p.polcmd='*' AND p.polpermissive
                   AND p.polroles=ARRAY[0::oid]
                   AND pg_catalog.regexp_replace(
                       COALESCE(pg_catalog.pg_get_expr(p.polqual,p.polrelid),''),
                       '[[:space:]()]','','g')='tenant_id=pipeline.current_session_tenant_v1'
                   AND pg_catalog.regexp_replace(
                       COALESCE(pg_catalog.pg_get_expr(p.polwithcheck,p.polrelid),''),
                       '[[:space:]()]','','g')='tenant_id=pipeline.current_session_tenant_v1')
                   AS canonical_count
        FROM pg_catalog.pg_policy p WHERE p.polrelid=rel.oid
    ) policy
    WHERE NOT rel.relrowsecurity OR NOT rel.relforcerowsecurity
       OR policy.policy_count<>1 OR policy.canonical_count<>1
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'production FORCE RLS/policy not ready on %',v_bad_relation
            USING ERRCODE='AV001';
    END IF;

    -- Mandatory complete global history: NULL and inconsistent labels must
    -- be resolved by a separate reviewed backfill, never by this function.
    SELECT count(*) INTO v_count FROM pipeline.tenant_historical_quarantine_v1();
    IF v_count<>10 THEN
        RAISE EXCEPTION 'FORCE RLS audit did not enumerate exactly ten tenant relations'
            USING ERRCODE='AV001';
    END IF;
    PERFORM pipeline.assert_no_tenant_quarantine_v1();

    RETURN TRUE; -- narrow DBA policy/history preflight, NEVER cutover authorization
END;
$$;
REVOKE ALL ON FUNCTION pipeline.assert_tenant_force_rls_preflight_v1() FROM PUBLIC;

COMMIT;
