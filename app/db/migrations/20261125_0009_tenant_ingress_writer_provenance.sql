-- Forward-only ingress owner provenance. An authenticated tenant's database
-- LOGIN is the only source of new raw snapshots; a request GUC or URL is not
-- sufficient identity. Existing unattributed historical rows stay NULL and
-- cannot become admitted Truth evidence without a separately audited backfill.
-- This migration deliberately does not FORCE RLS for legacy superuser/table-
-- owner paths: full role cutover is still an explicit production NO-GO.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.tenant_ingress_dba_v1()
RETURNS BOOLEAN LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog AS $$
    SELECT COALESCE((SELECT r.rolsuper FROM pg_catalog.pg_roles r
                     WHERE r.rolname = session_user), false);
$$;
REVOKE ALL ON FUNCTION pipeline.tenant_ingress_dba_v1() FROM PUBLIC;

-- No API-supplied tenant is trusted. A bound LOGIN gets its tenant from the
-- immutable owner ledger; a legacy unbound role may write only NULL rows.
CREATE OR REPLACE FUNCTION raw.stamp_ingress_page_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE
    actor_tenant TEXT := pipeline.current_session_tenant_v1();
    is_dba BOOLEAN := pipeline.tenant_ingress_dba_v1();
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF actor_tenant IS NOT NULL THEN
            IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id <> actor_tenant THEN
                RAISE EXCEPTION 'raw page tenant differs from authenticated LOGIN'
                    USING ERRCODE = 'AV001';
            END IF;
            NEW.tenant_id := actor_tenant;
        ELSIF NEW.tenant_id IS NOT NULL AND NOT is_dba THEN
            RAISE EXCEPTION 'unprovisioned LOGIN cannot claim raw page ownership'
                USING ERRCODE = 'AV001';
        END IF;
    ELSE
        IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id AND NOT is_dba THEN
            RAISE EXCEPTION 'raw page tenant reassignment requires audited DBA migration'
                USING ERRCODE = 'AV001';
        END IF;
        IF NOT is_dba AND (actor_tenant IS DISTINCT FROM OLD.tenant_id) THEN
            RAISE EXCEPTION 'raw page writer LOGIN does not own source evidence'
                USING ERRCODE = 'AV001';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION raw.stamp_ingress_page_tenant_v1() FROM PUBLIC;
DROP TRIGGER IF EXISTS stamp_ingress_page_tenant_v1 ON raw.pages;
CREATE TRIGGER stamp_ingress_page_tenant_v1
    BEFORE INSERT OR UPDATE ON raw.pages FOR EACH ROW
    EXECUTE FUNCTION raw.stamp_ingress_page_tenant_v1();

-- Section tenancy follows its actual parent page, never an independent
-- caller-supplied label. Reparenting across tenants is rejected atomically.
CREATE OR REPLACE FUNCTION raw.stamp_ingress_section_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE
    parent_tenant TEXT;
    actor_tenant TEXT := pipeline.current_session_tenant_v1();
    is_dba BOOLEAN := pipeline.tenant_ingress_dba_v1();
BEGIN
    SELECT p.tenant_id INTO parent_tenant FROM raw.pages p WHERE p.id = NEW.page_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'raw section has no canonical parent page'
            USING ERRCODE = 'AV001';
    END IF;
    IF parent_tenant IS NULL THEN
        IF actor_tenant IS NOT NULL OR NEW.tenant_id IS NOT NULL THEN
            RAISE EXCEPTION 'tenant writer cannot attach to unattributed raw page'
                USING ERRCODE = 'AV001';
        END IF;
    ELSE
        IF NOT is_dba AND actor_tenant IS DISTINCT FROM parent_tenant THEN
            RAISE EXCEPTION 'section writer LOGIN does not own parent raw page'
                USING ERRCODE = 'AV001';
        END IF;
        IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id <> parent_tenant THEN
            RAISE EXCEPTION 'raw section and parent page tenant mismatch'
                USING ERRCODE = 'AV001';
        END IF;
        IF TG_OP = 'INSERT' THEN
            NEW.tenant_id := parent_tenant;
        ELSIF OLD.tenant_id IS DISTINCT FROM parent_tenant OR
              NEW.tenant_id IS DISTINCT FROM OLD.tenant_id THEN
            RAISE EXCEPTION 'raw section tenant transfer requires audited backfill'
                USING ERRCODE = 'AV001';
        END IF;
    END IF;
    IF TG_OP = 'UPDATE' AND NOT is_dba AND
       OLD.tenant_id IS DISTINCT FROM actor_tenant THEN
        RAISE EXCEPTION 'unowned historical section is quarantined'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION raw.stamp_ingress_section_tenant_v1() FROM PUBLIC;
DROP TRIGGER IF EXISTS stamp_ingress_section_tenant_v1 ON raw.sections;
CREATE TRIGGER stamp_ingress_section_tenant_v1
    BEFORE INSERT OR UPDATE ON raw.sections FOR EACH ROW
    EXECUTE FUNCTION raw.stamp_ingress_section_tenant_v1();

-- A registered canonical run is attributed to its DBA-owned Truth context.
-- A tenant-bound LOGIN cannot create an unprovisioned run or impersonate
-- another tenant, even through a SECURITY DEFINER caller or forged GUC.
CREATE OR REPLACE FUNCTION pipeline.stamp_ingress_run_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE
    canonical_tenant TEXT;
    actor_tenant TEXT := pipeline.current_session_tenant_v1();
    is_dba BOOLEAN := pipeline.tenant_ingress_dba_v1();
BEGIN
    SELECT t.tenant_id INTO canonical_tenant
    FROM pipeline.tenant_context_ownership t WHERE t.context_key = NEW.context_key;
    IF canonical_tenant IS NULL THEN
        IF NOT is_dba AND (actor_tenant IS NOT NULL OR NEW.tenant_id IS NOT NULL) THEN
            RAISE EXCEPTION 'tenant run requires DBA-provisioned canonical context'
                USING ERRCODE = 'AV001';
        END IF;
    ELSE
        IF NOT is_dba AND actor_tenant IS DISTINCT FROM canonical_tenant THEN
            RAISE EXCEPTION 'run writer LOGIN does not own canonical context'
                USING ERRCODE = 'AV001';
        END IF;
        IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id <> canonical_tenant THEN
            RAISE EXCEPTION 'run tenant differs from canonical context owner'
                USING ERRCODE = 'AV001';
        END IF;
        IF TG_OP = 'UPDATE' AND OLD.tenant_id IS DISTINCT FROM canonical_tenant THEN
            RAISE EXCEPTION 'legacy run needs audited tenant backfill before use'
                USING ERRCODE = 'AV001';
        END IF;
        NEW.tenant_id := canonical_tenant;
    END IF;
    IF TG_OP = 'UPDATE' AND NOT is_dba AND
       (NEW.tenant_id IS DISTINCT FROM OLD.tenant_id OR
        NEW.context_key IS DISTINCT FROM OLD.context_key) THEN
        RAISE EXCEPTION 'run tenant/context identity cannot be reassigned'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.stamp_ingress_run_tenant_v1() FROM PUBLIC;
DROP TRIGGER IF EXISTS stamp_ingress_run_tenant_v1 ON pipeline.execution_runs;
CREATE TRIGGER stamp_ingress_run_tenant_v1
    BEFORE INSERT OR UPDATE ON pipeline.execution_runs FOR EACH ROW
    EXECUTE FUNCTION pipeline.stamp_ingress_run_tenant_v1();

-- A candidate can never claim a tenant other than both source/evidence
-- sections and the immutably provisioned context. This closes an ingress
-- path where RLS would otherwise filter the tenant mismatch from a reviewer.
CREATE OR REPLACE FUNCTION extracted.stamp_ingress_candidate_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE
    canonical_tenant TEXT;
    source_tenant TEXT;
    evidence_tenant TEXT;
    actor_tenant TEXT := pipeline.current_session_tenant_v1();
    is_dba BOOLEAN := pipeline.tenant_ingress_dba_v1();
BEGIN
    SELECT t.tenant_id INTO canonical_tenant FROM pipeline.tenant_context_ownership t
    WHERE t.context_key = NEW.context_key;
    SELECT s.tenant_id INTO source_tenant FROM raw.sections s WHERE s.id = NEW.raw_section_id;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate source section missing' USING ERRCODE = 'AV001'; END IF;
    SELECT s.tenant_id INTO evidence_tenant FROM raw.sections s WHERE s.id = NEW.evidence_section_id;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate evidence section missing' USING ERRCODE = 'AV001'; END IF;
    IF canonical_tenant IS NULL THEN
        IF NOT is_dba AND (actor_tenant IS NOT NULL OR NEW.tenant_id IS NOT NULL) THEN
            RAISE EXCEPTION 'tenant candidate requires provisioned canonical context'
                USING ERRCODE = 'AV001';
        END IF;
    ELSE
        IF source_tenant IS DISTINCT FROM canonical_tenant OR
           evidence_tenant IS DISTINCT FROM canonical_tenant THEN
            RAISE EXCEPTION 'candidate sources differ from canonical tenant owner'
                USING ERRCODE = 'AV001';
        END IF;
        IF NOT is_dba AND actor_tenant IS DISTINCT FROM canonical_tenant THEN
            RAISE EXCEPTION 'candidate writer LOGIN does not own canonical context'
                USING ERRCODE = 'AV001';
        END IF;
        IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id <> canonical_tenant THEN
            RAISE EXCEPTION 'candidate tenant spoofing refused'
                USING ERRCODE = 'AV001';
        END IF;
        IF TG_OP = 'UPDATE' AND OLD.tenant_id IS DISTINCT FROM canonical_tenant THEN
            RAISE EXCEPTION 'legacy candidate needs audited tenant backfill'
                USING ERRCODE = 'AV001';
        END IF;
        NEW.tenant_id := canonical_tenant;
    END IF;
    IF TG_OP = 'UPDATE' AND NOT is_dba AND
       (NEW.tenant_id IS DISTINCT FROM OLD.tenant_id OR
        NEW.context_key IS DISTINCT FROM OLD.context_key) THEN
        RAISE EXCEPTION 'candidate tenant/context identity cannot be reassigned'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION extracted.stamp_ingress_candidate_tenant_v1() FROM PUBLIC;
DROP TRIGGER IF EXISTS stamp_ingress_candidate_tenant_v1 ON extracted.rule_candidates;
CREATE TRIGGER stamp_ingress_candidate_tenant_v1
    BEFORE INSERT OR UPDATE ON extracted.rule_candidates FOR EACH ROW
    EXECUTE FUNCTION extracted.stamp_ingress_candidate_tenant_v1();

COMMIT;
