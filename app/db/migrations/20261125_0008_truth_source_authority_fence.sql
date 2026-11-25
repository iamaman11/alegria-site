-- V6: an operator admission pins the authoritative registry status and trust
-- of every candidate's source, not just its raw HTML and extraction rows.
-- A registry editor shares the canonical Truth context advisory lock with
-- the admitted PostgreSQL Activity. This migration does not backfill trust,
-- mint a new admission, or switch the production Temporal Worker.
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
            -- A missing registry row (including a broken historical FK) is
            -- explicit NULL in the digest. Operational updated_at alone is
            -- excluded; trust_level/status/authority_class/independence,
            -- override eligibility, freshness and every other field are
            -- part of the operator's admitted source identity.
            'source_authority', CASE WHEN registry.source_key IS NULL THEN NULL
                ELSE to_jsonb(registry) - 'updated_at' END,
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
    LEFT JOIN kb.sources AS registry ON registry.source_key = c.source_key
    LEFT JOIN raw.sections AS s ON s.id = c.raw_section_id
    LEFT JOIN raw.sections AS es ON es.id = c.evidence_section_id
    LEFT JOIN raw.pages AS p ON p.id = s.page_id
    LEFT JOIN raw.pages AS ep ON ep.id = es.page_id
    WHERE c.context_key = p_context_key
$$;

CREATE OR REPLACE FUNCTION kb.guard_truth_source_authority_writer_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = off AS $$
DECLARE
    affected_context TEXT;
BEGIN
    IF TG_OP = 'TRUNCATE' THEN
        -- TRUNCATE bypasses row triggers and can CASCADE to candidates.
        RAISE EXCEPTION 'source authority registry cannot be truncated'
            USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'UPDATE' AND
       (to_jsonb(NEW) - 'updated_at') IS NOT DISTINCT FROM
       (to_jsonb(OLD) - 'updated_at') THEN
        -- Registry bookkeeping, not an admission-authority change.
        RETURN NEW;
    END IF;

    -- Canonical contexts are fixed by authoritative extracted candidates.
    -- Sorted advisory keys prevent an opposite-order writer deadlock when
    -- one authority source supports multiple contexts. A direct DML writer
    -- cannot outrun an admitted effect's receipt/HITL transaction.
    FOR affected_context IN
        SELECT DISTINCT c.context_key
        FROM extracted.rule_candidates AS c
        WHERE c.source_key = OLD.source_key
        ORDER BY c.context_key
    LOOP
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || affected_context, 0)) THEN
            RAISE EXCEPTION 'source authority is locked by admitted Truth effect'
                USING ERRCODE = '55P03';
        END IF;
    END LOOP;
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS guard_truth_source_authority_write_v1 ON kb.sources;
CREATE TRIGGER guard_truth_source_authority_write_v1
    BEFORE UPDATE OR DELETE ON kb.sources
    FOR EACH ROW EXECUTE FUNCTION kb.guard_truth_source_authority_writer_v1();
DROP TRIGGER IF EXISTS guard_truth_source_authority_truncate_v1 ON kb.sources;
CREATE TRIGGER guard_truth_source_authority_truncate_v1
    BEFORE TRUNCATE ON kb.sources
    FOR EACH STATEMENT EXECUTE FUNCTION kb.guard_truth_source_authority_writer_v1();

COMMIT;
