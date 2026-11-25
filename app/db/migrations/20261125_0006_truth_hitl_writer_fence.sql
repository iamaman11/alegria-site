-- Protect the last mutable input to a Truth-review receipt: the operator HITL
-- queue. The source/candidate/run fences alone do not serialize an independent
-- review UI's UPDATE/DELETE/INSERT with the admitted Activity's queue-state
-- count. Every writer joins the same context key before its mutation commits.
--
-- An UPDATE trigger runs after PostgreSQL has taken the row lock. Using TRY
-- avoids an Activity/queue-writer lock inversion; a colliding edit fails with
-- a retriable 55P03, leaving the operator free to retry after the receipt.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION site.guard_truth_review_hitl_context()
RETURNS trigger LANGUAGE plpgsql
SET search_path = pg_catalog AS $$
DECLARE
    previous_scope TEXT;
    next_scope TEXT;
BEGIN
    IF TG_OP = 'TRUNCATE' THEN
        RAISE EXCEPTION 'Truth HITL history cannot be truncated around admitted effects'
            USING ERRCODE = '23514';
    END IF;
    IF TG_OP <> 'INSERT' AND OLD.task_type = 'truth_candidate_review' THEN
        previous_scope := OLD.scope_signature;
    END IF;
    IF TG_OP <> 'DELETE' AND NEW.task_type = 'truth_candidate_review' THEN
        next_scope := NEW.scope_signature;
    END IF;
    IF previous_scope IS NULL AND next_scope IS NULL THEN
        IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
        RETURN NEW;
    END IF;
    IF (previous_scope IS NOT NULL AND length(btrim(previous_scope)) = 0)
        OR (next_scope IS NOT NULL AND length(btrim(next_scope)) = 0) THEN
        RAISE EXCEPTION 'Truth HITL context must be nonempty'
            USING ERRCODE = '23514';
    END IF;
    IF previous_scope IS NULL THEN previous_scope := next_scope; END IF;
    IF next_scope IS NULL THEN next_scope := previous_scope; END IF;
    IF previous_scope <= next_scope THEN
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || previous_scope, 0)) THEN
            RAISE EXCEPTION 'Truth HITL context is locked by admitted effect'
                USING ERRCODE = '55P03';
        END IF;
        IF previous_scope <> next_scope AND NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || next_scope, 0)) THEN
            RAISE EXCEPTION 'destination Truth HITL context is locked'
                USING ERRCODE = '55P03';
        END IF;
    ELSE
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || next_scope, 0)) THEN
            RAISE EXCEPTION 'destination Truth HITL context is locked'
                USING ERRCODE = '55P03';
        END IF;
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || previous_scope, 0)) THEN
            RAISE EXCEPTION 'Truth HITL context is locked by admitted effect'
                USING ERRCODE = '55P03';
        END IF;
    END IF;
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS guard_truth_review_hitl_context ON site.seo_hitl_tasks;
CREATE TRIGGER guard_truth_review_hitl_context
BEFORE INSERT OR UPDATE OR DELETE ON site.seo_hitl_tasks
FOR EACH ROW EXECUTE FUNCTION site.guard_truth_review_hitl_context();

DROP TRIGGER IF EXISTS guard_truth_review_hitl_truncate ON site.seo_hitl_tasks;
CREATE TRIGGER guard_truth_review_hitl_truncate
BEFORE TRUNCATE ON site.seo_hitl_tasks
FOR EACH STATEMENT EXECUTE FUNCTION site.guard_truth_review_hitl_context();

COMMIT;
