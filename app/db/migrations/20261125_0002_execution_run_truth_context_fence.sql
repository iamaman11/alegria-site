-- Serialize canonical execution run status/identity changes with admitted V3
-- Truth reconciliation. A concurrent terminal transition must not invalidate
-- the run between admission readback and the atomic HITL + receipt commit.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.guard_execution_run_truth_context()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE
    old_context text;
    new_context text;
    canonical_run boolean;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.run_id, NEW.workflow_run_id, NEW.workflow_type,
            NEW.context_key, NEW.status)
           IS NOT DISTINCT FROM
           (OLD.run_id, OLD.workflow_run_id, OLD.workflow_type,
            OLD.context_key, OLD.status) THEN
            RETURN NEW;
        END IF;
    END IF;
    -- The operator holds this run key *before* status validation and initial
    -- admission INSERT. Without this barrier, a status writer could observe
    -- no admission, hold a row lock, and later commit terminal during V3.
    IF TG_OP = 'DELETE' THEN
        canonical_run := OLD.workflow_type = 'seo_site_build';
    ELSE
        canonical_run := OLD.workflow_type = 'seo_site_build'
                         OR NEW.workflow_type = 'seo_site_build';
    END IF;
    IF canonical_run AND NOT pg_try_advisory_xact_lock(
        hashtextextended('truth-run-status|' || OLD.run_id::text, 0)
    ) THEN
        RAISE EXCEPTION 'canonical Truth run status is locked by operator issuance'
            USING ERRCODE = '55P03';
    END IF;
    -- Scope the fail-fast protocol to runs with an issued V3 admission;
    -- never penalize legacy workers merely because they share a Truth key.
    -- Keep expired/revoked admissions in scope because a previously admitted
    -- Activity can still be finishing its receipt transaction. SECURITY
    -- DEFINER grants read-only authority lookup to ordinary run writers.
    IF NOT EXISTS (
        SELECT 1 FROM pipeline.pg_effect_v3_admissions a
        WHERE a.run_id = OLD.run_id
    ) THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    old_context := OLD.context_key;
    IF TG_OP = 'DELETE' THEN
        new_context := old_context;
    ELSE
        new_context := NEW.context_key;
    END IF;
    -- Lock both contexts in stable order on reparent, fail fast if the
    -- admitted Activity owns either. PostgreSQL already owns the changed
    -- execution_runs row lock by the time a BEFORE UPDATE trigger runs;
    -- waiting here could deadlock against the Activity receipt FK.
    IF old_context <= new_context THEN
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || old_context, 0)) THEN
            RAISE EXCEPTION 'Truth execution-run context is locked by admitted effect'
                USING ERRCODE = '55P03';
        END IF;
        IF old_context <> new_context AND NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || new_context, 0)) THEN
            RAISE EXCEPTION 'Truth execution-run destination context is locked'
                USING ERRCODE = '55P03';
        END IF;
    ELSE
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || new_context, 0)) THEN
            RAISE EXCEPTION 'Truth execution-run destination context is locked'
                USING ERRCODE = '55P03';
        END IF;
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || old_context, 0)) THEN
            RAISE EXCEPTION 'Truth execution-run context is locked by admitted effect'
                USING ERRCODE = '55P03';
        END IF;
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS guard_execution_run_truth_context ON pipeline.execution_runs;
CREATE TRIGGER guard_execution_run_truth_context
BEFORE UPDATE OR DELETE ON pipeline.execution_runs
FOR EACH ROW EXECUTE FUNCTION pipeline.guard_execution_run_truth_context();

COMMIT;
