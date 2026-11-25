-- Raw evidence is independently writable by crawl/recovery and SQL adapters.
-- V3's context advisory lock alone cannot protect those writes. Keep row
-- snapshots and admitted Truth review reconciliation serialized without
-- granting the Activity role UPDATE on raw tables.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION raw.guard_evidence_writer_lock()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    -- TRUNCATE bypasses row locks and is never a valid source revision.
    IF TG_OP = 'TRUNCATE' THEN
        RAISE EXCEPTION 'raw evidence snapshots cannot be truncated'
            USING ERRCODE = '23514';
    END IF;

    -- The crawler may mark a page as processed while an admitted Truth effect
    -- runs. This operational bit is intentionally absent from the snapshot
    -- fingerprint; all other source fields (also future added columns) remain
    -- serialized through the per-record advisory lock.
    IF TG_TABLE_NAME = 'pages' AND TG_OP = 'UPDATE'
       AND (to_jsonb(NEW) - 'processed') IS NOT DISTINCT FROM
           (to_jsonb(OLD) - 'processed') THEN
        RETURN NEW;
    END IF;

    -- V3 holds these advisory keys through its canonical receipt COMMIT.
    -- Reject a concurrent raw rewrite immediately (55P03), rather than
    -- allowing a writer to interleave between its hash and HITL reconciliation.
    -- A raw snapshot INSERT is independent until a candidate references it.
    IF NOT pg_try_advisory_xact_lock(
        hashtextextended('raw-evidence|' || TG_TABLE_NAME || '|' || OLD.id::text, 0)
    ) THEN
        RAISE EXCEPTION 'raw evidence snapshot is locked by admitted Truth effect'
            USING ERRCODE = '55P03';
    END IF;

    IF TG_OP = 'UPDATE' THEN
        RETURN NEW;
    END IF;
    RETURN OLD;
END;
$$;

DROP TRIGGER IF EXISTS guard_raw_page_writer_lock ON raw.pages;
CREATE TRIGGER guard_raw_page_writer_lock
BEFORE UPDATE OR DELETE ON raw.pages
FOR EACH ROW EXECUTE FUNCTION raw.guard_evidence_writer_lock();
DROP TRIGGER IF EXISTS guard_raw_section_writer_lock ON raw.sections;
CREATE TRIGGER guard_raw_section_writer_lock
BEFORE UPDATE OR DELETE ON raw.sections
FOR EACH ROW EXECUTE FUNCTION raw.guard_evidence_writer_lock();
DROP TRIGGER IF EXISTS guard_raw_page_truncate ON raw.pages;
CREATE TRIGGER guard_raw_page_truncate
BEFORE TRUNCATE ON raw.pages
FOR EACH STATEMENT EXECUTE FUNCTION raw.guard_evidence_writer_lock();
DROP TRIGGER IF EXISTS guard_raw_section_truncate ON raw.sections;
CREATE TRIGGER guard_raw_section_truncate
BEFORE TRUNCATE ON raw.sections
FOR EACH STATEMENT EXECUTE FUNCTION raw.guard_evidence_writer_lock();

-- All direct candidate writes, not just the canonical Rust writer, join the
-- same truth-context lock. This protects the selected section-ID set against
-- phantom inserts and cross-context reparenting while V3 holds its admission.
CREATE OR REPLACE FUNCTION extracted.lock_candidate_truth_context()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    old_context text;
    new_context text;
BEGIN
    IF TG_OP = 'INSERT' THEN
        PERFORM pg_advisory_xact_lock(
            hashtextextended('truth-context|' || NEW.context_key, 0)
        );
        RETURN NEW;
    ELSIF TG_OP = 'DELETE' THEN
        PERFORM pg_advisory_xact_lock(
            hashtextextended('truth-context|' || OLD.context_key, 0)
        );
        RETURN OLD;
    END IF;

    old_context := OLD.context_key;
    new_context := NEW.context_key;
    -- Order locks across two scopes so concurrent reparenting does not
    -- create a simple A->B / B->A advisory deadlock.
    IF old_context <= new_context THEN
        PERFORM pg_advisory_xact_lock(
            hashtextextended('truth-context|' || old_context, 0)
        );
        IF old_context <> new_context THEN
            PERFORM pg_advisory_xact_lock(
                hashtextextended('truth-context|' || new_context, 0)
            );
        END IF;
    ELSE
        PERFORM pg_advisory_xact_lock(
            hashtextextended('truth-context|' || new_context, 0)
        );
        PERFORM pg_advisory_xact_lock(
            hashtextextended('truth-context|' || old_context, 0)
        );
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS lock_candidate_truth_context ON extracted.rule_candidates;
CREATE TRIGGER lock_candidate_truth_context
BEFORE INSERT OR UPDATE OR DELETE ON extracted.rule_candidates
FOR EACH ROW EXECUTE FUNCTION extracted.lock_candidate_truth_context();

COMMIT;
