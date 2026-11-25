-- Forward-only tenant provenance for the admitted PostgreSQL Truth effect.
-- Historical rows are deliberately NOT assigned an invented tenant. An
-- administrator must provision context ownership and explicitly attest a
-- historical source/run backfill before future effect admission is possible.
BEGIN;

CREATE TABLE IF NOT EXISTS pipeline.tenant_context_ownership (
    context_key TEXT PRIMARY KEY REFERENCES kb.visa_contexts(context_key) ON UPDATE CASCADE,
    tenant_id TEXT NOT NULL CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0),
    provisioned_by NAME NOT NULL DEFAULT session_user,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX IF NOT EXISTS idx_tenant_context_ownership_tenant
    ON pipeline.tenant_context_ownership(tenant_id, context_key);

CREATE TABLE IF NOT EXISTS pipeline.tenant_role_ownership (
    role_name NAME PRIMARY KEY,
    tenant_id TEXT NOT NULL CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0),
    provisioned_by NAME NOT NULL DEFAULT session_user,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX IF NOT EXISTS idx_tenant_role_ownership_tenant
    ON pipeline.tenant_role_ownership(tenant_id, role_name);

REVOKE ALL ON pipeline.tenant_context_ownership, pipeline.tenant_role_ownership FROM PUBLIC;

-- These administrative bindings cannot silently drift underneath a previously
-- issued admission. Tenant transfers need a separately reviewed drain/rebind.
CREATE OR REPLACE FUNCTION pipeline.deny_tenant_ownership_mutation_v1()
RETURNS trigger LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'immutable tenant ownership; drain and use an explicit transfer protocol'
        USING ERRCODE = 'AV001';
END;
$$;
DROP TRIGGER IF EXISTS tenant_context_owner_immutable ON pipeline.tenant_context_ownership;
CREATE TRIGGER tenant_context_owner_immutable
    BEFORE UPDATE OR DELETE OR TRUNCATE ON pipeline.tenant_context_ownership
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_ownership_mutation_v1();
DROP TRIGGER IF EXISTS tenant_role_owner_immutable ON pipeline.tenant_role_ownership;
CREATE TRIGGER tenant_role_owner_immutable
    BEFORE UPDATE OR DELETE OR TRUNCATE ON pipeline.tenant_role_ownership
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_ownership_mutation_v1();

ALTER TABLE pipeline.execution_runs ADD COLUMN IF NOT EXISTS tenant_id TEXT;
ALTER TABLE raw.pages ADD COLUMN IF NOT EXISTS tenant_id TEXT;
ALTER TABLE raw.sections ADD COLUMN IF NOT EXISTS tenant_id TEXT;
ALTER TABLE extracted.rule_candidates ADD COLUMN IF NOT EXISTS tenant_id TEXT;
ALTER TABLE site.seo_hitl_tasks ADD COLUMN IF NOT EXISTS tenant_id TEXT;
ALTER TABLE pipeline.pg_effect_v3_receipts ADD COLUMN IF NOT EXISTS tenant_id TEXT;

CREATE INDEX IF NOT EXISTS idx_execution_runs_tenant_context
    ON pipeline.execution_runs(tenant_id, context_key, run_id);
CREATE INDEX IF NOT EXISTS idx_raw_pages_tenant ON raw.pages(tenant_id, id);
CREATE INDEX IF NOT EXISTS idx_raw_sections_tenant ON raw.sections(tenant_id, page_id);
CREATE INDEX IF NOT EXISTS idx_rule_candidates_tenant_context
    ON extracted.rule_candidates(tenant_id, context_key);
CREATE INDEX IF NOT EXISTS idx_hitl_tasks_tenant_scope
    ON site.seo_hitl_tasks(tenant_id, scope_signature);

-- A custom GUC is never tenant authority: anybody can SET one. Only the
-- authenticated PostgreSQL LOGIN (session_user), bound by DBA-owned rows, is.
CREATE OR REPLACE FUNCTION pipeline.current_session_tenant_v1()
RETURNS TEXT LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog AS $$
    SELECT ownership.tenant_id
    FROM pipeline.tenant_role_ownership ownership
    WHERE ownership.role_name = session_user::name;
$$;
REVOKE ALL ON FUNCTION pipeline.current_session_tenant_v1() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION pipeline.current_session_tenant_v1() TO PUBLIC;

-- RLS is active for non-owner roles. No permissive NULL/legacy policy exists.
-- Superuser/table-owner bypass remains a distinct production acceptance NO-GO
-- until each legacy database role is replaced with tenant-scoped credentials.
ALTER TABLE pipeline.execution_runs ENABLE ROW LEVEL SECURITY;
ALTER TABLE raw.pages ENABLE ROW LEVEL SECURITY;
ALTER TABLE raw.sections ENABLE ROW LEVEL SECURITY;
ALTER TABLE extracted.rule_candidates ENABLE ROW LEVEL SECURITY;
ALTER TABLE site.seo_hitl_tasks ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.pg_effect_v3_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.pg_effect_v3_receipts ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS execution_runs_tenant_v1 ON pipeline.execution_runs;
CREATE POLICY execution_runs_tenant_v1 ON pipeline.execution_runs
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS raw_pages_tenant_v1 ON raw.pages;
CREATE POLICY raw_pages_tenant_v1 ON raw.pages
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS raw_sections_tenant_v1 ON raw.sections;
CREATE POLICY raw_sections_tenant_v1 ON raw.sections
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS extracted_candidates_tenant_v1 ON extracted.rule_candidates;
CREATE POLICY extracted_candidates_tenant_v1 ON extracted.rule_candidates
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS site_hitl_tasks_tenant_v1 ON site.seo_hitl_tasks;
CREATE POLICY site_hitl_tasks_tenant_v1 ON site.seo_hitl_tasks
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS pg_effect_admissions_tenant_v1 ON pipeline.pg_effect_v3_admissions;
CREATE POLICY pg_effect_admissions_tenant_v1 ON pipeline.pg_effect_v3_admissions
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS pg_effect_receipts_tenant_v1 ON pipeline.pg_effect_v3_receipts;
CREATE POLICY pg_effect_receipts_tenant_v1 ON pipeline.pg_effect_v3_receipts
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

-- Recheck every evidence edge under the same Truth context lock as the
-- admitted capability. A single NULL or cross-tenant source quarantines the
-- entire future effect, including older unassigned HITL tasks.
CREATE OR REPLACE FUNCTION pipeline.tenant_truth_scope_ready_v1(
    p_tenant TEXT, p_context TEXT, p_run UUID
) RETURNS BOOLEAN LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog AS $$
    SELECT EXISTS (
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
REVOKE ALL ON FUNCTION pipeline.tenant_truth_scope_ready_v1(TEXT,TEXT,UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION pipeline.tenant_truth_scope_ready_v1(TEXT,TEXT,UUID) TO PUBLIC;

CREATE OR REPLACE FUNCTION pipeline.enforce_pg_effect_admission_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE is_admin BOOLEAN;
BEGIN
    SELECT rol.rolsuper INTO is_admin FROM pg_catalog.pg_roles rol
        WHERE rol.rolname = session_user;
    IF NOT coalesce(is_admin, false) AND
       pipeline.current_session_tenant_v1() IS DISTINCT FROM NEW.tenant_id THEN
        RAISE EXCEPTION 'authenticated operator PostgreSQL LOGIN is not authorized for tenant'
            USING ERRCODE = 'AV001';
    END IF;
    IF NOT pipeline.tenant_truth_scope_ready_v1(NEW.tenant_id, NEW.context_key, NEW.run_id) THEN
        RAISE EXCEPTION 'canonical tenant run/source ownership not provisioned or stale'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS pg_effect_admission_tenant_v1 ON pipeline.pg_effect_v3_admissions;
CREATE TRIGGER pg_effect_admission_tenant_v1 BEFORE INSERT ON pipeline.pg_effect_v3_admissions
FOR EACH ROW EXECUTE FUNCTION pipeline.enforce_pg_effect_admission_tenant_v1();

-- Internal Truth-review task creation derives tenant only from the DBA-owned
-- canonical context binding, never the command's request/GUC. Existing
-- historical rows remain NULL until an explicit audited backfill.
CREATE OR REPLACE FUNCTION site.bind_new_truth_review_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE owner_tenant TEXT;
BEGIN
    IF NEW.task_type = 'truth_candidate_review' THEN
        SELECT tenant_id INTO owner_tenant FROM pipeline.tenant_context_ownership
            WHERE context_key = NEW.scope_signature;
        IF owner_tenant IS NOT NULL THEN
            IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id <> owner_tenant THEN
                RAISE EXCEPTION 'truth review task tenant differs from canonical context'
                    USING ERRCODE = 'AV001';
            END IF;
            NEW.tenant_id := owner_tenant;
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS bind_new_truth_review_tenant_v1 ON site.seo_hitl_tasks;
CREATE TRIGGER bind_new_truth_review_tenant_v1 BEFORE INSERT ON site.seo_hitl_tasks
FOR EACH ROW EXECUTE FUNCTION site.bind_new_truth_review_tenant_v1();

-- The pre-receipt SQL capability may have created HITL tasks. A denied tenant
-- receipt INSERT raises in that same transaction, rolling back ALL effects.
CREATE OR REPLACE FUNCTION pipeline.enforce_pg_effect_receipt_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE scope RECORD;
BEGIN
    SELECT tenant_id, context_key, run_id INTO scope
    FROM pipeline.pg_effect_v3_admissions a
    WHERE a.admission_id = NEW.admission_id AND a.context_key = NEW.context_key
      AND a.run_id = NEW.run_id;
    IF NOT FOUND OR NOT pipeline.tenant_truth_scope_ready_v1(
        scope.tenant_id, scope.context_key, scope.run_id) THEN
        RAISE EXCEPTION 'receipt refuses unowned or drifted canonical tenant evidence'
            USING ERRCODE = 'AV001';
    END IF;
    IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id <> scope.tenant_id THEN
        RAISE EXCEPTION 'receipt tenant mismatch' USING ERRCODE = 'AV001';
    END IF;
    NEW.tenant_id := scope.tenant_id;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS pg_effect_receipt_tenant_v1 ON pipeline.pg_effect_v3_receipts;
CREATE TRIGGER pg_effect_receipt_tenant_v1 BEFORE INSERT ON pipeline.pg_effect_v3_receipts
FOR EACH ROW EXECUTE FUNCTION pipeline.enforce_pg_effect_receipt_tenant_v1();

CREATE OR REPLACE FUNCTION pipeline.consume_pg_truth_review_v1(
    p_admission_id UUID,
    p_run_id UUID,
    p_tenant_id TEXT,
    p_context_key TEXT,
    p_workflow_id TEXT,
    p_workflow_run_id TEXT,
    p_operation_instance TEXT,
    p_plan_hash TEXT,
    p_contract_fingerprint TEXT,
    p_execution_key TEXT,
    p_admission_revision SMALLINT,
    p_parent_workflow_id TEXT,
    p_parent_workflow_run_id TEXT,
    p_dispatch_input_hash TEXT,
    p_invocation_bytes BYTEA,
    p_bound_input_bytes BYTEA,
    p_owner_command_bytes BYTEA
) RETURNS TABLE (
    receipt_id UUID, admission_id UUID, execution_key TEXT, run_id UUID,
    context_key TEXT, candidate_set_hash TEXT,
    created_review_tasks BIGINT, open_review_tasks BIGINT
) LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE
    a RECORD;
    v_newer_truth_done BOOLEAN;
    v_initial_refs JSONB;
    v_locked_refs JSONB;
    v_page BIGINT;
    v_section BIGINT;
    v_source_hash TEXT;
    v_created BIGINT;
    v_open BIGINT;
    v_receipt_key TEXT;
BEGIN
    -- SQL SECURITY DEFINER bypasses the caller's RLS. Never let a worker
    -- consume another tenant's receipt, including an old replay that returns
    -- before the receipt INSERT trigger. A custom GUC is not an identity.
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
       AND NOT EXISTS (
           SELECT 1 FROM pg_catalog.pg_roles r
           WHERE r.rolname = session_user AND r.rolsuper
       ) THEN
        RAISE EXCEPTION 'authenticated PostgreSQL LOGIN cannot consume this tenant effect'
            USING ERRCODE = 'AV001';
    END IF;
    IF p_admission_revision NOT IN (3,4)
       OR length(p_execution_key) <> 64
       OR length(p_plan_hash) <> 64
       OR length(p_contract_fingerprint) <> 64 THEN
        RAISE EXCEPTION 'unsupported or malformed admitted Truth effect'
            USING ERRCODE = 'AV001';
    END IF;

    PERFORM pg_advisory_xact_lock(
        hashtextextended('truth-context|' || p_context_key, 0));
    PERFORM pg_advisory_xact_lock(
        hashtextextended('pg-v3-admission|' || p_admission_id::text, 0));
    SELECT auth.*, er.status AS run_status INTO a
    FROM pipeline.pg_effect_v3_admissions AS auth
    JOIN pipeline.execution_runs AS er ON er.run_id = auth.run_id
    WHERE auth.admission_id = p_admission_id
      AND auth.admission_schema_revision = p_admission_revision
      AND auth.run_id = p_run_id
      AND auth.tenant_id = p_tenant_id
      AND auth.context_key = p_context_key
      AND auth.workflow_id = p_workflow_id
      AND auth.workflow_execution_run_id = p_workflow_run_id
      AND auth.operation_instance = p_operation_instance
      AND auth.plan_hash = p_plan_hash
      AND auth.contract_fingerprint = p_contract_fingerprint
      AND auth.operation_id = 'pg.truth.reconcile_candidate_reviews'
      AND auth.operation_revision = 1
      AND auth.authorized_effect = 'pg.truth.review_queue.reconcile'
      AND er.context_key = auth.context_key
      AND er.workflow_type = 'seo_site_build'
      AND (
          (p_admission_revision = 3 AND
              er.workflow_run_id = auth.workflow_execution_run_id AND
              (auth.v3_expected_execution_key = p_execution_key OR
                 (auth.v3_expected_execution_key IS NULL AND EXISTS (
                     SELECT 1 FROM pipeline.pg_effect_v3_receipts historic
                     WHERE historic.admission_id = auth.admission_id
                       AND historic.execution_key = p_execution_key
                       AND historic.admission_schema_revision = 3
                 ))) AND
              p_parent_workflow_id IS NULL AND p_parent_workflow_run_id IS NULL AND
              p_dispatch_input_hash IS NULL AND p_invocation_bytes IS NULL AND
              p_bound_input_bytes IS NULL AND p_owner_command_bytes IS NULL)
          OR (p_admission_revision = 4 AND
              er.workflow_run_id = auth.parent_workflow_run_id AND
              auth.v3_expected_execution_key IS NULL AND
              auth.parent_workflow_id = p_parent_workflow_id AND
              auth.parent_workflow_run_id = p_parent_workflow_run_id AND
              auth.dispatch_execution_key = p_execution_key AND
              auth.dispatch_input_hash = p_dispatch_input_hash AND
              auth.invocation_sha256 = encode(public.digest(p_invocation_bytes, 'sha256'), 'hex') AND
              auth.bound_input_sha256 = encode(public.digest(p_bound_input_bytes, 'sha256'), 'hex') AND
              auth.owner_command_sha256 = encode(public.digest(p_owner_command_bytes, 'sha256'), 'hex'))
      );
    IF NOT FOUND THEN
        RAISE EXCEPTION 'operator admission or canonical Temporal run denied'
            USING ERRCODE = 'AV001';
    END IF;
    -- Receipt replay is allowed after expiry/revocation, never with a
    -- different exact execution key or authorization revision.
    SELECT r.execution_key INTO v_receipt_key
    FROM pipeline.pg_effect_v3_receipts r
    WHERE r.admission_id = p_admission_id;
    IF FOUND THEN
        IF v_receipt_key <> p_execution_key THEN
            RAISE EXCEPTION 'effect receipt execution key mismatch'
                USING ERRCODE = 'AV001';
        END IF;
        RETURN QUERY SELECT r.receipt_id, r.admission_id, r.execution_key,
            r.run_id, r.context_key, r.candidate_set_hash,
            r.created_review_tasks, r.open_review_tasks
        FROM pipeline.pg_effect_v3_receipts r
        WHERE r.admission_id = p_admission_id
          AND r.admission_schema_revision = p_admission_revision;
        RETURN;
    END IF;
    -- Historic V3 admissions predating the immutable expected-key column may
    -- REPLAY an already-committed receipt above; they must never originate a
    -- new effect with a caller-chosen receipt execution key.
    IF p_admission_revision = 3 AND a.v3_expected_execution_key IS NULL THEN
        RAISE EXCEPTION 'historical admission lacks pinned V3 receipt key'
            USING ERRCODE = 'AV001';
    END IF;
    -- The legacy table predated per-parent instance uniqueness. A V3 and V4
    -- admission (or two completed same-revision admissions) must not each
    -- emit the same business effect under distinct child Workflow Run IDs.
    -- The context lock above serializes this check with every consuming
    -- Activity. Historical receipt replay returned before this guard.
    IF EXISTS (
        SELECT 1 FROM pipeline.pg_effect_v3_admissions other
        LEFT JOIN pipeline.pg_effect_v3_receipts other_receipt
            ON other_receipt.admission_id = other.admission_id
        WHERE other.run_id = a.run_id
          AND other.plan_hash = a.plan_hash
          AND other.operation_instance = a.operation_instance
          AND other.admission_id <> a.admission_id
          AND (other_receipt.receipt_id IS NOT NULL
               OR (other.admission_schema_revision <> a.admission_schema_revision
                   AND other.revoked_at IS NULL
                   AND other.expires_at > clock_timestamp()))
    ) THEN
        RAISE EXCEPTION 'competing versioned admission or completed instance effect'
            USING ERRCODE = 'AV001';
    END IF;
    IF a.revoked_at IS NOT NULL OR a.expires_at <= clock_timestamp() THEN
        RAISE EXCEPTION 'operator admission expired or revoked'
            USING ERRCODE = 'AV001';
    END IF;
    IF a.run_status NOT IN ('verifying','pending_hitl') THEN
        RAISE EXCEPTION 'canonical Truth run is no longer active'
            USING ERRCODE = 'TR001';
    END IF;
    SELECT EXISTS (
        SELECT 1 FROM pipeline.execution_runs newer
        JOIN pipeline.execution_runs this_run ON this_run.run_id = p_run_id
        JOIN pipeline.step_executions done ON done.run_id = newer.run_id
            AND done.step_name = 'verified_truth_write' AND done.status = 'done'
        WHERE newer.context_key = this_run.context_key
          AND newer.run_id <> this_run.run_id
          AND newer.created_at >= this_run.created_at
    ) INTO v_newer_truth_done;
    IF v_newer_truth_done THEN
        RAISE EXCEPTION 'newer verified Truth supersedes this run'
            USING ERRCODE = 'TR001';
    END IF;

    SELECT COALESCE(jsonb_agg(jsonb_build_array(ref.id, ref.page_id)
        ORDER BY ref.id, ref.page_id), '[]'::jsonb) INTO v_initial_refs
    FROM (
        SELECT DISTINCT s.id, s.page_id
        FROM extracted.rule_candidates c
        JOIN raw.sections s ON s.id IN (c.raw_section_id, c.evidence_section_id)
        WHERE c.context_key = p_context_key
    ) ref;
    FOR v_page IN
        SELECT DISTINCT s.page_id
        FROM extracted.rule_candidates c
        JOIN raw.sections s ON s.id IN (c.raw_section_id, c.evidence_section_id)
        WHERE c.context_key = p_context_key
        ORDER BY s.page_id
    LOOP
        PERFORM pg_advisory_xact_lock(
            hashtextextended('raw-evidence|pages|' || v_page::text, 0));
    END LOOP;
    FOR v_section IN
        SELECT DISTINCT s.id
        FROM extracted.rule_candidates c
        JOIN raw.sections s ON s.id IN (c.raw_section_id, c.evidence_section_id)
        WHERE c.context_key = p_context_key
        ORDER BY s.id
    LOOP
        PERFORM pg_advisory_xact_lock(
            hashtextextended('raw-evidence|sections|' || v_section::text, 0));
    END LOOP;
    SELECT COALESCE(jsonb_agg(jsonb_build_array(ref.id, ref.page_id)
        ORDER BY ref.id, ref.page_id), '[]'::jsonb) INTO v_locked_refs
    FROM (
        SELECT DISTINCT s.id, s.page_id
        FROM extracted.rule_candidates c
        JOIN raw.sections s ON s.id IN (c.raw_section_id, c.evidence_section_id)
        WHERE c.context_key = p_context_key
    ) ref;
    IF v_initial_refs IS DISTINCT FROM v_locked_refs THEN
        RAISE EXCEPTION 'raw source references changed while locking'
            USING ERRCODE = 'CS001';
    END IF;
    SELECT pipeline.pg_truth_candidate_set_hash_v1(p_context_key)
    INTO v_source_hash;
    IF v_source_hash <> a.candidate_set_hash THEN
        RAISE EXCEPTION 'candidate source snapshot changed since admission'
            USING ERRCODE = 'CS001';
    END IF;

    -- No DML grant to caller is needed; all three writes are within this
    -- single SECURITY DEFINER transaction and roll back together on error.
    SELECT site.reconcile_truth_candidate_reviews_v1(
        p_context_key, p_run_id::text) INTO v_created;
    SELECT count(*)::bigint INTO v_open
    FROM site.seo_hitl_tasks t
    WHERE t.scope_signature = p_context_key
      AND t.task_type = 'truth_candidate_review'
      AND t.queue_state IN ('open','reopened','in_review');
    INSERT INTO pipeline.pg_effect_v3_receipts
        (admission_id, admission_schema_revision, execution_key,
         operation_id, operation_revision, run_id, context_key,
         candidate_set_hash, created_review_tasks, open_review_tasks)
    VALUES (p_admission_id, p_admission_revision, p_execution_key,
            'pg.truth.reconcile_candidate_reviews', 1, p_run_id, p_context_key,
            v_source_hash, v_created, v_open);
    RETURN QUERY SELECT r.receipt_id, r.admission_id, r.execution_key,
        r.run_id, r.context_key, r.candidate_set_hash,
        r.created_review_tasks, r.open_review_tasks
    FROM pipeline.pg_effect_v3_receipts r
    WHERE r.admission_id = p_admission_id;
END;
$$;

-- A tenant transfer cannot race an admission issued from the formerly
-- unowned canonical run. Writer-first and operator-first are serialized by
-- the same run key. No automatic tenant transfer is permitted post-admission.
CREATE OR REPLACE FUNCTION pipeline.guard_execution_run_tenant_transfer_v1()
RETURNS trigger LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF NEW.tenant_id IS NOT DISTINCT FROM OLD.tenant_id THEN
        RETURN NEW;
    END IF;
    IF NOT pg_try_advisory_xact_lock(
        hashtextextended('truth-run-status|' || OLD.run_id::text, 0)) THEN
        RAISE EXCEPTION 'tenant transfer blocked by active admitted Truth operation'
            USING ERRCODE = '55P03';
    END IF;
    IF EXISTS (SELECT 1 FROM pipeline.pg_effect_v3_admissions a
               WHERE a.run_id = OLD.run_id) THEN
        RAISE EXCEPTION 'admitted canonical run cannot change tenant owner'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS guard_execution_run_tenant_transfer_v1
    ON pipeline.execution_runs;
CREATE TRIGGER guard_execution_run_tenant_transfer_v1
BEFORE UPDATE OF tenant_id ON pipeline.execution_runs
FOR EACH ROW EXECUTE FUNCTION pipeline.guard_execution_run_tenant_transfer_v1();

COMMIT;
