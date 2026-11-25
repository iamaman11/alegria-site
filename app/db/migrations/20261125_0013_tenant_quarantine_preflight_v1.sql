-- V7 tenant cutover preflight only. No historical row is assigned a tenant,
-- no production role is granted DML, and RLS is not FORCE-enabled here.
-- This forward-only migration makes new malformed labels fail closed and
-- provides a DBA-only, source-verifiable orphan/inconsistency census.
BEGIN;
SET LOCAL lock_timeout = '5s';

-- Existing legacy NULLs and malformed labels are deliberately NOT VALIDATED:
-- they remain visible as historical quarantine in the census below. New
-- inserts/updates cannot create new whitespace or empty owner labels.
ALTER TABLE raw.pages ADD CONSTRAINT raw_pages_tenant_label_v1
    CHECK (tenant_id IS NULL OR (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0)) NOT VALID;
ALTER TABLE raw.sections ADD CONSTRAINT raw_sections_tenant_label_v1
    CHECK (tenant_id IS NULL OR (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0)) NOT VALID;
ALTER TABLE extracted.rule_candidates ADD CONSTRAINT extracted_candidates_tenant_label_v1
    CHECK (tenant_id IS NULL OR (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0)) NOT VALID;
ALTER TABLE pipeline.execution_runs ADD CONSTRAINT execution_runs_tenant_label_v1
    CHECK (tenant_id IS NULL OR (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0)) NOT VALID;
ALTER TABLE site.seo_hitl_tasks ADD CONSTRAINT seo_hitl_tasks_tenant_label_v1
    CHECK (tenant_id IS NULL OR (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0)) NOT VALID;
ALTER TABLE pipeline.pg_effect_v3_admissions ADD CONSTRAINT pg_effect_admissions_tenant_label_v1
    CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0) NOT VALID;
ALTER TABLE pipeline.pg_effect_v3_receipts ADD CONSTRAINT pg_effect_receipts_tenant_label_v1
    CHECK (tenant_id IS NULL OR (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0)) NOT VALID;
ALTER TABLE pipeline.qdrant_projection_v1_admissions ADD CONSTRAINT qdrant_admissions_tenant_label_v1
    CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0) NOT VALID;
ALTER TABLE pipeline.qdrant_projection_v1_attempts ADD CONSTRAINT qdrant_attempts_tenant_label_v1
    CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0) NOT VALID;
ALTER TABLE pipeline.qdrant_projection_v1_receipts ADD CONSTRAINT qdrant_receipts_tenant_label_v1
    CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0) NOT VALID;

-- A read-only global inventory. Each artifact is counted once; unattributed
-- is NULL tenant_id, inconsistent is a non-NULL label that violates its
-- immutable context or source edge. This intentionally does not scan only
-- one caller-supplied tenant: missing attribution cannot safely be scoped.
CREATE OR REPLACE FUNCTION pipeline.tenant_historical_quarantine_v1()
RETURNS TABLE (relation_name TEXT, unattributed BIGINT, inconsistent BIGINT)
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
    SELECT 'raw.pages'::text,
           count(*) FILTER (WHERE p.tenant_id IS NULL),
           count(*) FILTER (WHERE p.tenant_id IS NOT NULL
                            AND (p.tenant_id <> btrim(p.tenant_id) OR length(p.tenant_id) = 0))
    FROM raw.pages p
    UNION ALL
    SELECT 'raw.sections'::text,
           count(*) FILTER (WHERE s.tenant_id IS NULL),
           count(*) FILTER (WHERE s.tenant_id IS NOT NULL
                            AND (s.tenant_id <> p.tenant_id OR p.tenant_id IS NULL
                                 OR s.tenant_id <> btrim(s.tenant_id) OR length(s.tenant_id) = 0))
    FROM raw.sections s LEFT JOIN raw.pages p ON p.id = s.page_id
    UNION ALL
    SELECT 'extracted.rule_candidates'::text,
           count(*) FILTER (WHERE c.tenant_id IS NULL),
           count(*) FILTER (WHERE c.tenant_id IS NOT NULL AND
                (c.tenant_id <> owner.tenant_id OR owner.tenant_id IS NULL
                 OR c.tenant_id IS DISTINCT FROM src.tenant_id
                 OR c.tenant_id IS DISTINCT FROM src_page.tenant_id
                 OR c.tenant_id IS DISTINCT FROM ev.tenant_id
                 OR c.tenant_id IS DISTINCT FROM ev_page.tenant_id))
    FROM extracted.rule_candidates c
    LEFT JOIN pipeline.tenant_context_ownership owner ON owner.context_key=c.context_key
    LEFT JOIN raw.sections src ON src.id=c.raw_section_id
    LEFT JOIN raw.pages src_page ON src_page.id=src.page_id
    LEFT JOIN raw.sections ev ON ev.id=c.evidence_section_id
    LEFT JOIN raw.pages ev_page ON ev_page.id=ev.page_id
    UNION ALL
    SELECT 'pipeline.execution_runs'::text,
           count(*) FILTER (WHERE r.tenant_id IS NULL),
           count(*) FILTER (WHERE r.tenant_id IS NOT NULL AND
                (r.tenant_id <> owner.tenant_id OR owner.tenant_id IS NULL))
    FROM pipeline.execution_runs r
    LEFT JOIN pipeline.tenant_context_ownership owner ON owner.context_key=r.context_key
    UNION ALL
    SELECT 'site.seo_hitl_tasks'::text,
           count(*) FILTER (WHERE t.tenant_id IS NULL),
           count(*) FILTER (WHERE t.tenant_id IS NOT NULL AND
                (t.tenant_id <> btrim(t.tenant_id) OR length(t.tenant_id)=0
                 OR (t.task_type='truth_candidate_review' AND
                     (t.tenant_id <> owner.tenant_id OR owner.tenant_id IS NULL))))
    FROM site.seo_hitl_tasks t
    LEFT JOIN pipeline.tenant_context_ownership owner
           ON owner.context_key=t.scope_signature
    UNION ALL
    SELECT 'pipeline.pg_effect_v3_admissions'::text,
           0::bigint,
           count(*) FILTER (WHERE a.tenant_id IS DISTINCT FROM r.tenant_id
                OR a.context_key IS DISTINCT FROM r.context_key
                OR a.tenant_id IS DISTINCT FROM owner.tenant_id)
    FROM pipeline.pg_effect_v3_admissions a
    LEFT JOIN pipeline.execution_runs r ON r.run_id=a.run_id
    LEFT JOIN pipeline.tenant_context_ownership owner ON owner.context_key=a.context_key
    UNION ALL
    SELECT 'pipeline.pg_effect_v3_receipts'::text,
           count(*) FILTER (WHERE receipt.tenant_id IS NULL),
           count(*) FILTER (WHERE receipt.tenant_id IS NOT NULL AND
                (receipt.tenant_id IS DISTINCT FROM a.tenant_id
                 OR receipt.run_id IS DISTINCT FROM a.run_id
                 OR receipt.context_key IS DISTINCT FROM a.context_key))
    FROM pipeline.pg_effect_v3_receipts receipt
    LEFT JOIN pipeline.pg_effect_v3_admissions a ON a.admission_id=receipt.admission_id
    UNION ALL
    SELECT 'pipeline.qdrant_projection_v1_admissions'::text,
           0::bigint,
           count(*) FILTER (WHERE a.tenant_id IS DISTINCT FROM r.tenant_id
                OR a.context_key IS DISTINCT FROM r.context_key
                OR a.tenant_id IS DISTINCT FROM owner.tenant_id)
    FROM pipeline.qdrant_projection_v1_admissions a
    LEFT JOIN pipeline.execution_runs r ON r.run_id=a.run_id
    LEFT JOIN pipeline.tenant_context_ownership owner ON owner.context_key=a.context_key
    UNION ALL
    SELECT 'pipeline.qdrant_projection_v1_attempts'::text,
           0::bigint,
           count(*) FILTER (WHERE attempt.tenant_id IS DISTINCT FROM a.tenant_id)
    FROM pipeline.qdrant_projection_v1_attempts attempt
    LEFT JOIN pipeline.qdrant_projection_v1_admissions a
           ON a.admission_id=attempt.admission_id
    UNION ALL
    SELECT 'pipeline.qdrant_projection_v1_receipts'::text,
           0::bigint,
           count(*) FILTER (WHERE receipt.tenant_id IS DISTINCT FROM a.tenant_id)
    FROM pipeline.qdrant_projection_v1_receipts receipt
    LEFT JOIN pipeline.qdrant_projection_v1_admissions a
           ON a.admission_id=receipt.admission_id;
$$;
REVOKE ALL ON FUNCTION pipeline.tenant_historical_quarantine_v1() FROM PUBLIC;

CREATE OR REPLACE FUNCTION pipeline.assert_no_tenant_quarantine_v1()
RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE v_quarantine RECORD;
BEGIN
    SELECT * INTO v_quarantine FROM pipeline.tenant_historical_quarantine_v1()
    WHERE unattributed > 0 OR inconsistent > 0 LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'historical tenant quarantine blocks cutover: %, missing %, inconsistent %',
            v_quarantine.relation_name, v_quarantine.unattributed, v_quarantine.inconsistent
            USING ERRCODE = 'AV001';
    END IF;
    RETURN TRUE; -- inventory only; does not attest production role cutover
END;
$$;
REVOKE ALL ON FUNCTION pipeline.assert_no_tenant_quarantine_v1() FROM PUBLIC;

-- Prospective, DBA-only proof for an ingress LOGIN. Session/GUC identity is
-- not a substitute for an immutable tenant_role_ownership row. No role grants
-- or table ownership transfers occur here. Operator/worker roles must have
-- their own narrower, separately attested capabilities.
CREATE OR REPLACE FUNCTION pipeline.assert_tenant_ingress_login_v1(
    p_role NAME, p_tenant TEXT
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE
    v_role_oid OID;
    v_role RECORD;
    v_bound TEXT;
BEGIN
    SELECT r.oid, r.rolcanlogin, r.rolsuper, r.rolbypassrls, r.rolcreaterole,
           r.rolcreatedb, r.rolreplication
    INTO v_role FROM pg_catalog.pg_roles r WHERE r.rolname=p_role;
    IF NOT FOUND OR NOT v_role.rolcanlogin OR v_role.rolsuper OR v_role.rolbypassrls
       OR v_role.rolcreaterole OR v_role.rolcreatedb OR v_role.rolreplication THEN
        RAISE EXCEPTION 'tenant ingress LOGIN has unsafe role attributes'
            USING ERRCODE = 'AV001';
    END IF;
    v_role_oid := v_role.oid;
    SELECT tenant_id INTO v_bound FROM pipeline.tenant_role_ownership
    WHERE role_name=p_role;
    IF p_tenant IS NULL OR p_tenant <> btrim(p_tenant) OR length(p_tenant)=0
       OR v_bound IS DISTINCT FROM p_tenant THEN
        RAISE EXCEPTION 'tenant ingress LOGIN has no immutable exact tenant binding'
            USING ERRCODE = 'AV001';
    END IF;
    IF EXISTS (
        SELECT 1 FROM pg_catalog.pg_roles r
        WHERE (r.rolsuper OR r.rolbypassrls OR r.rolcreaterole)
          AND pg_catalog.pg_has_role(v_role_oid,r.oid,'MEMBER')
    ) OR EXISTS (
        SELECT 1 FROM pg_catalog.pg_class c
        WHERE c.oid = ANY(ARRAY[
            'raw.pages'::regclass, 'raw.sections'::regclass,
            'extracted.rule_candidates'::regclass, 'pipeline.execution_runs'::regclass,
            'site.seo_hitl_tasks'::regclass, 'kb.sources'::regclass,
            'pipeline.pg_effect_v3_admissions'::regclass,
            'pipeline.pg_effect_v3_receipts'::regclass,
            'pipeline.qdrant_projection_v1_admissions'::regclass,
            'pipeline.qdrant_projection_v1_attempts'::regclass,
            'pipeline.qdrant_projection_v1_receipts'::regclass])
          AND pg_catalog.pg_has_role(v_role_oid,c.relowner,'MEMBER')
    ) THEN
        RAISE EXCEPTION 'tenant ingress LOGIN can assume an RLS-bypassing owner role'
            USING ERRCODE = 'AV001';
    END IF;
    IF EXISTS (
        SELECT 1 FROM pg_catalog.pg_class c
        WHERE c.oid = ANY(ARRAY[
            'raw.pages'::regclass, 'raw.sections'::regclass,
            'extracted.rule_candidates'::regclass, 'pipeline.execution_runs'::regclass,
            'site.seo_hitl_tasks'::regclass,
            'pipeline.pg_effect_v3_admissions'::regclass,
            'pipeline.pg_effect_v3_receipts'::regclass,
            'pipeline.qdrant_projection_v1_admissions'::regclass,
            'pipeline.qdrant_projection_v1_attempts'::regclass,
            'pipeline.qdrant_projection_v1_receipts'::regclass])
          AND (pg_catalog.has_table_privilege(v_role_oid,c.oid,'TRUNCATE')
               OR pg_catalog.has_table_privilege(v_role_oid,c.oid,'DELETE'))
    ) OR EXISTS (
        SELECT 1 FROM pg_catalog.pg_namespace n
        WHERE n.nspname IN ('raw','extracted','pipeline','kb','site')
          AND pg_catalog.has_schema_privilege(v_role_oid,n.oid,'CREATE')
    ) THEN
        RAISE EXCEPTION 'tenant ingress LOGIN has unsafe destructive or DDL privileges'
            USING ERRCODE = 'AV001';
    END IF;
    IF EXISTS (
        SELECT 1 FROM pg_catalog.pg_class c
        WHERE c.oid = ANY(ARRAY[
            'pipeline.pg_effect_v3_admissions'::regclass,
            'pipeline.pg_effect_v3_receipts'::regclass,
            'pipeline.qdrant_projection_v1_admissions'::regclass,
            'pipeline.qdrant_projection_v1_attempts'::regclass,
            'pipeline.qdrant_projection_v1_receipts'::regclass])
          AND (pg_catalog.has_table_privilege(v_role_oid,c.oid,'INSERT')
               OR pg_catalog.has_table_privilege(v_role_oid,c.oid,'UPDATE'))
    ) THEN
        RAISE EXCEPTION 'tenant ingress LOGIN has operator admission or receipt write privileges'
            USING ERRCODE = 'AV001';
    END IF;
    -- A native crawler/extractor LOGIN must never be able to mint canonical
    -- context/role ownership or promote a source to authoritative evidence.
    -- These are administrative/curator-owned inputs to the tenant and Truth
    -- decisions, even if RLS would correctly restrict the raw evidence rows.
    IF EXISTS (
        SELECT 1 FROM pg_catalog.pg_class c
        WHERE c.oid = ANY(ARRAY[
            'kb.sources'::regclass, 'kb.visa_contexts'::regclass,
            'pipeline.tenant_context_ownership'::regclass,
            'pipeline.tenant_role_ownership'::regclass,
            'site.seo_hitl_tasks'::regclass])
          AND (pg_catalog.has_table_privilege(v_role_oid,c.oid,'INSERT')
               OR pg_catalog.has_table_privilege(v_role_oid,c.oid,'UPDATE')
               OR pg_catalog.has_table_privilege(v_role_oid,c.oid,'DELETE')
               OR pg_catalog.has_table_privilege(v_role_oid,c.oid,'TRUNCATE'))
    ) THEN
        RAISE EXCEPTION 'tenant ingress LOGIN can mutate curator or DBA-owned authority'
            USING ERRCODE = 'AV001';
    END IF;
    IF EXISTS (
        SELECT 1 FROM pg_catalog.pg_class c
        WHERE c.oid = ANY(ARRAY[
            'raw.pages'::regclass, 'raw.sections'::regclass,
            'extracted.rule_candidates'::regclass, 'pipeline.execution_runs'::regclass])
          AND (NOT pg_catalog.has_table_privilege(v_role_oid,c.oid,'SELECT')
               OR NOT pg_catalog.has_table_privilege(v_role_oid,c.oid,'INSERT'))
    ) THEN
        RAISE EXCEPTION 'tenant ingress LOGIN lacks required scoped source writer privileges'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN TRUE;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.assert_tenant_ingress_login_v1(NAME,TEXT) FROM PUBLIC;

COMMIT;
