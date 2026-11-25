-- RLS never applies to TRUNCATE. An accidentally overgranted native LOGIN,
-- table owner, or privileged maintenance session must not erase canonical
-- Truth evidence, executions, or Qdrant operator admission/receipts through
-- that bypass. This remains a forward-only fail-closed guard, not a historical
-- tenant backfill or authorization to use a production table-owner LOGIN.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.deny_tenant_scoped_truncate_v1()
RETURNS trigger LANGUAGE plpgsql
SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'tenant-bound canonical operation ledger cannot be truncated'
        USING ERRCODE = '23514';
END;
$$;
REVOKE ALL ON FUNCTION pipeline.deny_tenant_scoped_truncate_v1() FROM PUBLIC;

-- Revocation alone is insufficient when a specific operator LOGIN has an
-- inherited or mistakenly granted TRUNCATE privilege. The BEFORE triggers
-- enforce the prohibition even then, and also for table-owner DBA paths.
REVOKE TRUNCATE ON extracted.rule_candidates,
    pipeline.execution_runs,
    pipeline.pg_effect_v3_receipts,
    pipeline.qdrant_projection_v1_admissions,
    pipeline.qdrant_projection_v1_attempts,
    pipeline.qdrant_projection_v1_receipts FROM PUBLIC;

DROP TRIGGER IF EXISTS deny_tenant_scoped_truncate_v1 ON extracted.rule_candidates;
CREATE TRIGGER deny_tenant_scoped_truncate_v1
    BEFORE TRUNCATE ON extracted.rule_candidates
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_scoped_truncate_v1();

DROP TRIGGER IF EXISTS deny_tenant_scoped_truncate_v1 ON pipeline.execution_runs;
CREATE TRIGGER deny_tenant_scoped_truncate_v1
    BEFORE TRUNCATE ON pipeline.execution_runs
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_scoped_truncate_v1();

DROP TRIGGER IF EXISTS deny_tenant_scoped_truncate_v1 ON pipeline.pg_effect_v3_receipts;
CREATE TRIGGER deny_tenant_scoped_truncate_v1
    BEFORE TRUNCATE ON pipeline.pg_effect_v3_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_scoped_truncate_v1();

DROP TRIGGER IF EXISTS deny_tenant_scoped_truncate_v1 ON pipeline.qdrant_projection_v1_admissions;
CREATE TRIGGER deny_tenant_scoped_truncate_v1
    BEFORE TRUNCATE ON pipeline.qdrant_projection_v1_admissions
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_scoped_truncate_v1();

DROP TRIGGER IF EXISTS deny_tenant_scoped_truncate_v1 ON pipeline.qdrant_projection_v1_attempts;
CREATE TRIGGER deny_tenant_scoped_truncate_v1
    BEFORE TRUNCATE ON pipeline.qdrant_projection_v1_attempts
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_scoped_truncate_v1();

DROP TRIGGER IF EXISTS deny_tenant_scoped_truncate_v1 ON pipeline.qdrant_projection_v1_receipts;
CREATE TRIGGER deny_tenant_scoped_truncate_v1
    BEFORE TRUNCATE ON pipeline.qdrant_projection_v1_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_scoped_truncate_v1();

COMMIT;
