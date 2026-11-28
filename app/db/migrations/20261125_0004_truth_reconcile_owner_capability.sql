-- The one canonical Truth-review reconciliation policy, callable with the
-- INVOKER's DML grants. A restricted PG effect Worker cannot execute the
-- underlying writes directly; the admission-gated SECURITY DEFINER below
-- invokes this policy only after independent authorization in one tx.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.pg_truth_candidate_set_hash_v1(p_context_key TEXT)
RETURNS TEXT LANGUAGE sql STABLE SECURITY INVOKER
SET search_path = pg_catalog AS $$
    SELECT encode(public.digest(COALESCE(jsonb_agg(
        jsonb_build_object(
            'candidate', to_jsonb(c),
            'raw_section', to_jsonb(s),
            'evidence_section', to_jsonb(es),
            'raw_page', CASE WHEN p.id IS NULL THEN NULL ELSE jsonb_build_object(
                'id', p.id, 'url', p.url, 'domain', p.domain, 'dtype', p.dtype,
                'status_code', p.status_code, 'content_type', p.content_type,
                'snapshot_at', p.snapshot_at, 'crawled_at', p.crawled_at,
                'final_url', p.final_url, 'title', p.title,
                'meta_desc', p.meta_desc, 'canonical', p.canonical,
                'word_count', p.word_count, 'content_hash', p.content_hash,
                'raw_html_bytes', p.raw_html_bytes,
                'content_sha256', CASE WHEN p.content IS NULL THEN NULL
                    ELSE encode(public.digest(p.content::text, 'sha256'), 'hex') END,
                'raw_html_sha256', CASE WHEN p.raw_html IS NULL THEN NULL
                    ELSE encode(public.digest(p.raw_html, 'sha256'), 'hex') END,
                'redirect_chain', p.redirect_chain, 'robots_trace', p.robots_trace,
                'source_observation', p.source_observation
            ) END,
            'evidence_page', CASE WHEN ep.id IS NULL THEN NULL ELSE jsonb_build_object(
                'id', ep.id, 'url', ep.url, 'domain', ep.domain, 'dtype', ep.dtype,
                'status_code', ep.status_code, 'content_type', ep.content_type,
                'snapshot_at', ep.snapshot_at, 'crawled_at', ep.crawled_at,
                'final_url', ep.final_url, 'title', ep.title,
                'meta_desc', ep.meta_desc, 'canonical', ep.canonical,
                'word_count', ep.word_count, 'content_hash', ep.content_hash,
                'raw_html_bytes', ep.raw_html_bytes,
                'content_sha256', CASE WHEN ep.content IS NULL THEN NULL
                    ELSE encode(public.digest(ep.content::text, 'sha256'), 'hex') END,
                'raw_html_sha256', CASE WHEN ep.raw_html IS NULL THEN NULL
                    ELSE encode(public.digest(ep.raw_html, 'sha256'), 'hex') END,
                'redirect_chain', ep.redirect_chain, 'robots_trace', ep.robots_trace,
                'source_observation', ep.source_observation
            ) END
        ) ORDER BY c.rule_candidate_id)::text, '[]'), 'sha256'), 'hex')
    FROM extracted.rule_candidates AS c
    LEFT JOIN raw.sections AS s ON s.id = c.raw_section_id
    LEFT JOIN raw.sections AS es ON es.id = c.evidence_section_id
    LEFT JOIN raw.pages AS p ON p.id = s.page_id
    LEFT JOIN raw.pages AS ep ON ep.id = es.page_id
    WHERE c.context_key = p_context_key
$$;

CREATE OR REPLACE FUNCTION site.reconcile_truth_candidate_reviews_v1(
    p_context_key TEXT, p_run_id TEXT
) RETURNS BIGINT LANGUAGE plpgsql SECURITY INVOKER
SET search_path = pg_catalog AS $$
DECLARE
    created_count BIGINT;
BEGIN
    IF length(btrim(p_context_key)) = 0 OR length(btrim(p_run_id)) = 0 THEN
        RAISE EXCEPTION 'truth HITL context and run are required'
            USING ERRCODE = '22023';
    END IF;
    UPDATE site.seo_hitl_tasks AS t
    SET queue_state = 'superseded',
        audit_log_payload = t.audit_log_payload || jsonb_build_array(
            jsonb_build_object('event', 'truth_candidate_rechecked',
                'candidate_status', c.epistemic_status,
                'run_id', p_run_id, 'at', now())),
        updated_at = now()
    FROM extracted.rule_candidates AS c
    WHERE t.task_type = 'truth_candidate_review'
      AND t.scope_signature = p_context_key
      AND t.blocking_execution_key = c.rule_candidate_id
      AND t.queue_state IN ('open', 'in_review', 'reopened')
      AND (
          (t.decision_payload->>'source_snapshot_hash')
              IS DISTINCT FROM c.source_snapshot_hash
          OR (c.epistemic_status <> 'needs_hitl'
              AND NOT (
                  c.epistemic_status = 'rejected'
                  AND t.decision_payload->'review'->>'decision' = 'reject'
              ))
      );

    UPDATE site.seo_hitl_tasks AS t
    SET queue_state = 'superseded',
        audit_log_payload = t.audit_log_payload || jsonb_build_array(
            jsonb_build_object('event', 'truth_candidate_source_missing',
                'run_id', p_run_id, 'at', now())),
        updated_at = now()
    WHERE t.task_type = 'truth_candidate_review'
      AND t.scope_signature = p_context_key
      AND t.queue_state IN ('open', 'in_review', 'reopened')
      AND NOT EXISTS (
          SELECT 1 FROM extracted.rule_candidates AS c
          WHERE c.rule_candidate_id = t.blocking_execution_key
            AND c.context_key = p_context_key
      );

    UPDATE site.seo_hitl_tasks AS t
    SET queue_state = 'open',
        current_owner_role = 'truth_reviewer',
        decision_payload = (
            t.decision_payload || jsonb_build_object(
                'raw_section_id', c.raw_section_id,
                'role', c.role, 'concept', c.concept_canonical_key,
                'params', c.params, 'scope', c.scope,
                'source_url', c.source_key,
                'source_snapshot_hash', c.source_snapshot_hash,
                'evidence_quote', c.evidence_quote,
                'span_start', c.span_start, 'span_end', c.span_end,
                'llm_provider', c.llm_provider, 'llm_model', c.llm_model,
                'prompt_version', c.prompt_version,
                'epistemic_status', 'needs_hitl', 'recheck_required', true)
        ) - 'review',
        audit_log_payload = t.audit_log_payload || jsonb_build_array(
            jsonb_build_object('event', 'truth_candidate_reopened_for_review',
                'run_id', p_run_id, 'at', now())),
        updated_at = now()
    FROM extracted.rule_candidates AS c
    WHERE t.task_key = 'truth_candidate_review@1:' ||
            c.rule_candidate_id || ':' || c.source_snapshot_hash
      AND t.task_type = 'truth_candidate_review'
      AND t.scope_signature = p_context_key
      AND c.context_key = p_context_key
      AND c.epistemic_status = 'needs_hitl'
      AND (
          t.queue_state = 'superseded'
          OR (t.decision_payload->>'raw_section_id')::bigint
              IS DISTINCT FROM c.raw_section_id
          OR (t.decision_payload->>'role') IS DISTINCT FROM c.role
          OR (t.decision_payload->>'concept') IS DISTINCT FROM c.concept_canonical_key
          OR t.decision_payload->'params' IS DISTINCT FROM c.params
          OR t.decision_payload->'scope' IS DISTINCT FROM c.scope
          OR (t.decision_payload->>'source_url') IS DISTINCT FROM c.source_key
          OR (t.decision_payload->>'evidence_quote') IS DISTINCT FROM c.evidence_quote
          OR (t.decision_payload->>'span_start')::integer IS DISTINCT FROM c.span_start
          OR (t.decision_payload->>'span_end')::integer IS DISTINCT FROM c.span_end
          OR (t.decision_payload->>'llm_provider') IS DISTINCT FROM c.llm_provider
          OR (t.decision_payload->>'llm_model') IS DISTINCT FROM c.llm_model
          OR (t.decision_payload->>'prompt_version') IS DISTINCT FROM c.prompt_version
      );

    INSERT INTO site.seo_hitl_tasks
        (task_key, task_type, queue_state, first_owner_role, current_owner_role,
         scope_signature, blocking_step_name, blocking_execution_key,
         severity, decision_payload, audit_log_payload)
    SELECT
        'truth_candidate_review@1:' || c.rule_candidate_id || ':' || c.source_snapshot_hash,
        'truth_candidate_review', 'open', 'truth_reviewer', 'truth_reviewer',
        c.context_key, 'truth_adjudication', c.rule_candidate_id,
        'blocking',
        jsonb_build_object(
            'schema_version', 1, 'candidate_id', c.rule_candidate_id,
            'context_key', c.context_key, 'raw_section_id', c.raw_section_id,
            'role', c.role, 'concept', c.concept_canonical_key,
            'params', c.params, 'scope', c.scope,
            'source_url', c.source_key,
            'source_snapshot_hash', c.source_snapshot_hash,
            'evidence_quote', c.evidence_quote,
            'span_start', c.span_start, 'span_end', c.span_end,
            'llm_provider', c.llm_provider, 'llm_model', c.llm_model,
            'prompt_version', c.prompt_version, 'epistemic_status', 'needs_hitl',
            'recheck_required', true),
        jsonb_build_array(jsonb_build_object(
            'event', 'truth_candidate_needs_review',
            'run_id', p_run_id,
            'source_snapshot_hash', c.source_snapshot_hash))
    FROM extracted.rule_candidates c
    WHERE c.context_key = p_context_key
      AND c.epistemic_status = 'needs_hitl'
    ON CONFLICT (task_key) DO NOTHING;
    GET DIAGNOSTICS created_count = ROW_COUNT;
    RETURN created_count;
END;
$$;

-- The ONLY privileged effect capability. It independently validates the
-- immutable operator-issued V3/V4 admission and current server-owned Truth
-- state, then applies the exact invoker reconciliation policy and receipt in
-- one PostgreSQL transaction. No SET LOCAL/GUC supplied by a caller grants
-- authority. Worker DML on site.seo_hitl_tasks and receipts is unnecessary.
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

REVOKE ALL ON FUNCTION pipeline.consume_pg_truth_review_v1(
    UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,SMALLINT,
    TEXT,TEXT,TEXT,BYTEA,BYTEA,BYTEA
) FROM PUBLIC;

COMMIT;
