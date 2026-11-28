-- A newer verified_truth_write step can supersede a run even if its run-status
-- row is unchanged. Serialize that step's actual row mutation with V3/V4
-- source/HITL/receipt commit under the existing Truth context key.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.guard_verified_truth_step_context()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE
    old_context TEXT;
    new_context TEXT;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF ROW(OLD.run_id, OLD.step_name, OLD.status)
           IS NOT DISTINCT FROM ROW(NEW.run_id, NEW.step_name, NEW.status) THEN
            RETURN NEW;
        END IF;
        IF OLD.step_name <> 'verified_truth_write'
           AND NEW.step_name <> 'verified_truth_write' THEN
            RETURN NEW;
        END IF;
    ELSIF TG_OP = 'INSERT' THEN
        IF NEW.step_name <> 'verified_truth_write' THEN
            RETURN NEW;
        END IF;
    ELSE
        IF OLD.step_name <> 'verified_truth_write' THEN
            RETURN OLD;
        END IF;
    END IF;

    IF TG_OP <> 'INSERT' AND OLD.step_name = 'verified_truth_write' THEN
        SELECT context_key INTO old_context
        FROM pipeline.execution_runs WHERE run_id = OLD.run_id;
    END IF;
    IF TG_OP <> 'DELETE' AND NEW.step_name = 'verified_truth_write' THEN
        SELECT context_key INTO new_context
        FROM pipeline.execution_runs WHERE run_id = NEW.run_id;
    END IF;
    -- During an execution_run DELETE CASCADE, the parent may already be
    -- gone within this transaction. A deleted canonical run cannot authorize
    -- an admitted effect (its FK admission prevents such deletion).
    IF old_context IS NULL AND new_context IS NULL THEN
        IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
        RETURN NEW;
    END IF;
    IF old_context IS NULL THEN old_context := new_context; END IF;
    IF new_context IS NULL THEN new_context := old_context; END IF;

    -- A row-level writer already owns a step row lock. Fail fast rather than
    -- holding it waiting on an Activity transaction that reads chronology.
    IF old_context <= new_context THEN
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || old_context, 0)) THEN
            RAISE EXCEPTION 'verified Truth chronology is locked by admitted effect'
                USING ERRCODE = '55P03';
        END IF;
        IF old_context <> new_context AND NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || new_context, 0)) THEN
            RAISE EXCEPTION 'destination Truth chronology is locked'
                USING ERRCODE = '55P03';
        END IF;
    ELSE
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || new_context, 0)) THEN
            RAISE EXCEPTION 'destination Truth chronology is locked'
                USING ERRCODE = '55P03';
        END IF;
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || old_context, 0)) THEN
            RAISE EXCEPTION 'verified Truth chronology is locked by admitted effect'
                USING ERRCODE = '55P03';
        END IF;
    END IF;
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS guard_verified_truth_step_context ON pipeline.step_executions;
CREATE TRIGGER guard_verified_truth_step_context
BEFORE INSERT OR UPDATE OR DELETE ON pipeline.step_executions
FOR EACH ROW EXECUTE FUNCTION pipeline.guard_verified_truth_step_context();

COMMIT;
