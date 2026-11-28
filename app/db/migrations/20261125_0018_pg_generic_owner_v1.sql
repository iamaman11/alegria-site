-- Opt-in PG generic owner bridge. Existing V3/V4 SQL and Temporal histories
-- are immutable; schema 5 pins canonical OwnerInvocationV1 bytes and exact
-- PostgreSQL source snapshot under the SAME tenant/HITL/security fences.
-- This migration does NOT authorize new generic run governance or Produced input.
BEGIN;
SET LOCAL lock_timeout = '5s';

ALTER TABLE pipeline.pg_effect_v3_admissions
    ADD COLUMN generic_parent_start_sha256 TEXT;

ALTER TABLE pipeline.pg_effect_v3_admissions
    DROP CONSTRAINT pg_effect_v4_admission_revision_check,
    ADD CONSTRAINT pg_effect_v4_admission_revision_check
        CHECK (admission_schema_revision IN (3,4,5)),
    DROP CONSTRAINT pg_effect_v4_admission_shape,
    ADD CONSTRAINT pg_effect_v4_admission_shape CHECK (
        (admission_schema_revision = 3 AND
            parent_workflow_id IS NULL AND parent_workflow_run_id IS NULL AND
            dispatch_execution_key IS NULL AND dispatch_input_hash IS NULL AND
            invocation_sha256 IS NULL AND bound_input_sha256 IS NULL AND
            owner_command_sha256 IS NULL AND temporal_attestation_sha256 IS NULL AND
            temporal_child_started_event_id IS NULL AND
            generic_parent_start_sha256 IS NULL)
        OR (admission_schema_revision IN (4,5) AND
            num_nonnulls(parent_workflow_id, parent_workflow_run_id,
                dispatch_execution_key, dispatch_input_hash,
                invocation_sha256, bound_input_sha256, owner_command_sha256,
                temporal_attestation_sha256, temporal_child_started_event_id) = 9 AND
            length(trim(parent_workflow_id)) > 0 AND
            length(trim(parent_workflow_run_id)) > 0 AND
            length(dispatch_execution_key) = 64 AND
            length(dispatch_input_hash) = 64 AND
            length(invocation_sha256) = 64 AND
            length(bound_input_sha256) = 64 AND
            length(owner_command_sha256) = 64 AND
            length(temporal_attestation_sha256) = 64 AND
            temporal_child_started_event_id > 0 AND
            ((admission_schema_revision = 4 AND
              generic_parent_start_sha256 IS NULL)
             OR (admission_schema_revision = 5 AND
                 generic_parent_start_sha256 IS NOT NULL AND
                 generic_parent_start_sha256 ~ '^[0-9a-f]{64}$')))
    );

-- The existing V3/V4 immutable-row guard deliberately remains untouched.
-- This additional trigger protects the new V5-only parent input SHA field.
CREATE FUNCTION pipeline.guard_pg_generic_parent_start_sha256_v1()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF NEW.generic_parent_start_sha256 IS DISTINCT FROM
       OLD.generic_parent_start_sha256 THEN
        RAISE EXCEPTION 'native DAG parent start SHA attestation is immutable'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_pg_generic_parent_start_sha256_v1() FROM PUBLIC;
CREATE TRIGGER guard_pg_generic_parent_start_sha256_v1
    BEFORE UPDATE ON pipeline.pg_effect_v3_admissions
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_pg_generic_parent_start_sha256_v1();

ALTER TABLE pipeline.pg_effect_v3_receipts
    DROP CONSTRAINT pg_effect_v4_receipt_revision_check,
    ADD CONSTRAINT pg_effect_v4_receipt_revision_check
        CHECK (admission_schema_revision IN (3,4,5));

CREATE UNIQUE INDEX uq_pg_effect_v5_parent_instance
    ON pipeline.pg_effect_v3_admissions(run_id, plan_hash, operation_instance)
    WHERE admission_schema_revision = 5;

-- Deliberately separate SQL capability: an old V4 worker may consume only
-- revision 4, a new typed owner may consume only revision 5. Do not broaden
-- consume_pg_truth_review_v1 and do not grant this capability to PUBLIC.
CREATE FUNCTION pipeline.consume_pg_truth_review_owner_v1(
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
    IF p_admission_revision <> 5
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
    -- Serialize native DAG effects against status transitions/issuance. This
    -- lock does not modify any legacy V3/V4 permission or History.
    PERFORM pg_advisory_xact_lock(
        hashtextextended('truth-run-status|' || p_run_id::text, 0));
    -- Governance revocation/second admission serialize with the complete
    -- irreversible PG Truth transaction, not merely its initial validity
    -- check. Lock order matches governance issuer: status, then canonical.
    PERFORM pg_advisory_xact_lock(
        hashtextextended('operation-dag-canonical|' || p_run_id::text, 0));
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
      -- The site-build source and generic Temporal DAG parent are distinct
      -- real Workflow Run IDs. Exact identity is separately operator-attested.
      AND pipeline.operation_dag_governance_matches_v1(
          auth.run_id,auth.tenant_id,auth.context_key,
          auth.parent_workflow_id,auth.parent_workflow_run_id,
          auth.plan_hash,auth.generic_parent_start_sha256,FALSE)
      AND auth.v3_expected_execution_key IS NULL
      AND auth.parent_workflow_id = p_parent_workflow_id
      AND auth.parent_workflow_run_id = p_parent_workflow_run_id
      AND auth.dispatch_execution_key = p_execution_key
      AND auth.dispatch_input_hash = p_dispatch_input_hash
      AND auth.invocation_sha256 = encode(public.digest(p_invocation_bytes, 'sha256'), 'hex')
      AND auth.bound_input_sha256 = encode(public.digest(p_bound_input_bytes, 'sha256'), 'hex')
      AND auth.owner_command_sha256 = encode(public.digest(p_owner_command_bytes, 'sha256'), 'hex');
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
    IF NOT pipeline.operation_dag_governance_matches_v1(
        a.run_id,a.tenant_id,a.context_key,
        a.parent_workflow_id,a.parent_workflow_run_id,
        a.plan_hash,a.generic_parent_start_sha256,TRUE) THEN
        RAISE EXCEPTION 'native DAG governance expired, revoked or source status drifted'
            USING ERRCODE = 'AV001';
    END IF;
    -- The legacy table predated per-parent instance uniqueness. A V3/V4/V5
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

REVOKE ALL ON FUNCTION pipeline.consume_pg_truth_review_owner_v1(
    UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,SMALLINT,
    TEXT,TEXT,TEXT,BYTEA,BYTEA,BYTEA
) FROM PUBLIC;

COMMIT;

