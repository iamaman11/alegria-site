-- Independently issued, tenant-bound external-source identity for a one-node
-- native Temporal V2 operation. This is a NEW authority, not an interpretation
-- of the caller's URL, provenance text, generic DAG run, or owner settings.
--
-- Only a separately provisioned, tenant-authenticated source issuer may INSERT
-- and revoke rows. Its access must be limited to SELECT of authenticated raw
-- snapshots plus INSERT/UPDATE here; the Temporal Start process gets SELECT
-- only, never issuance rights. There are deliberately NO automatic grants.
--
-- The issuer is responsible for deriving typed operation bytes from the pinned
-- captured raw snapshot and computing their BLAKE3 payload hash. SQL checks
-- immutable snapshot identity and independently issued business-run identity;
-- it cannot itself prove the fidelity of an arbitrary JSON transformation.
-- Until a trusted source transformer is installed, the bytes must remain
-- external-source-unattested at the business boundary. A text URL is NEVER
-- source evidence. Historical runs and earlier grants remain unchanged.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE TABLE pipeline.operation_dag_external_source_attestations_v1 (
    record_id UUID PRIMARY KEY
        CHECK (record_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    run_id UUID NOT NULL REFERENCES pipeline.operation_dag_business_runs_v1(run_id),
    tenant_id TEXT NOT NULL
        CHECK (length(tenant_id) > 0 AND tenant_id = btrim(tenant_id)),
    context_key TEXT NOT NULL
        CHECK (length(context_key) > 0 AND context_key = btrim(context_key)),
    plan_hash TEXT NOT NULL CHECK (plan_hash ~ '^[0-9a-f]{64}$'),
    artifact_id TEXT NOT NULL
        CHECK (length(artifact_id) BETWEEN 1 AND 128 AND artifact_id = btrim(artifact_id)),
    artifact_schema JSONB NOT NULL CHECK (jsonb_typeof(artifact_schema) = 'object'),
    payload_hash TEXT NOT NULL CHECK (payload_hash ~ '^[0-9a-f]{64}$'),
    source_page_id BIGINT NOT NULL REFERENCES raw.pages(id),
    source_page_content_hash TEXT NOT NULL
        CHECK (source_page_content_hash ~ '^[0-9a-f]{64}$'),
    -- Independently derived by PostgreSQL from the actual captured HTML,
    -- never trusted from a caller-provided content_hash or URL. A separate
    -- deterministic source transformer must also verify BLAKE3(raw_html).
    source_page_raw_sha256 TEXT NOT NULL
        CHECK (source_page_raw_sha256 ~ '^[0-9a-f]{64}$'),
    issued_by NAME NOT NULL DEFAULT session_user,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    UNIQUE (run_id, artifact_id),
    CHECK (expires_at > issued_at AND expires_at <= issued_at + interval '10 minutes'),
    CHECK (revoked_at IS NULL OR revoked_at >= issued_at)
);
CREATE INDEX idx_operation_dag_external_source_attestations_v1_lookup
    ON pipeline.operation_dag_external_source_attestations_v1
    (run_id, tenant_id, context_key, plan_hash, artifact_id);
CREATE INDEX idx_operation_dag_external_source_attestations_v1_raw_page
    ON pipeline.operation_dag_external_source_attestations_v1(source_page_id);
REVOKE ALL ON pipeline.operation_dag_external_source_attestations_v1 FROM PUBLIC;
ALTER TABLE pipeline.operation_dag_external_source_attestations_v1 ENABLE ROW LEVEL SECURITY;
CREATE POLICY operation_dag_external_source_tenant_v1
    ON pipeline.operation_dag_external_source_attestations_v1
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

CREATE FUNCTION pipeline.guard_operation_dag_external_source_attestation_v1()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE
    v_business RECORD;
    v_source RECORD;
    v_actual_raw_sha256 TEXT;
    v_now TIMESTAMPTZ;
BEGIN
    IF pg_catalog.current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'external source attestation requires READ COMMITTED'
            USING ERRCODE='AV001';
    END IF;
    IF TG_OP IN ('DELETE','TRUNCATE') THEN
        RAISE EXCEPTION 'trusted external source attestation cannot be deleted'
            USING ERRCODE='AV001';
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF pipeline.current_session_tenant_v1() IS DISTINCT FROM OLD.tenant_id
           OR OLD.issued_by <> session_user
           OR OLD.revoked_at IS NOT NULL OR NEW.revoked_at IS NULL
           OR (to_jsonb(NEW) - 'revoked_at') IS DISTINCT FROM
              (to_jsonb(OLD) - 'revoked_at') THEN
            RAISE EXCEPTION 'external source attestation permits only issuer-owned revocation'
                USING ERRCODE='AV001';
        END IF;
        PERFORM pg_advisory_xact_lock(hashtextextended(
            'operation-dag-business|' || OLD.run_id::text,0));
        PERFORM pg_advisory_xact_lock(hashtextextended(
            'operation-dag-external-source|' || OLD.record_id::text,0));
        NEW.revoked_at := clock_timestamp();
        RETURN NEW;
    END IF;

    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM NEW.tenant_id
       OR NEW.issued_by <> session_user OR NEW.revoked_at IS NOT NULL
       OR has_table_privilege(session_user,
            'pipeline.operation_dag_business_runs_v1','INSERT')
       OR has_table_privilege(session_user,
            'pipeline.operation_dag_run_governance_v2','INSERT')
       -- An issuer must not be able to create OR rewrite its own purported
       -- source of truth. Fail closed on misprovisioned source-writer powers,
       -- even when the current immutable snapshot happens to be intact.
       OR has_table_privilege(session_user,'raw.pages','INSERT')
       OR has_table_privilege(session_user,'raw.pages','UPDATE')
       OR has_table_privilege(session_user,'raw.pages','DELETE')
       OR has_table_privilege(session_user,'raw.pages','TRUNCATE')
       OR has_table_privilege(session_user,'raw.sections','INSERT')
       OR has_table_privilege(session_user,'raw.sections','UPDATE')
       OR has_table_privilege(session_user,'raw.sections','DELETE')
       OR has_table_privilege(session_user,'raw.sections','TRUNCATE')
       -- NOINHERIT does not protect this boundary: a member could SET ROLE
       -- to a raw writer, rewrite evidence, then RESET ROLE and issue.
       -- Independent source issuers must be membership-free LOGIN roles.
       OR EXISTS (SELECT 1 FROM pg_catalog.pg_auth_members m
            JOIN pg_catalog.pg_roles r ON r.oid = m.member
           WHERE r.rolname = session_user)
       OR NOT has_column_privilege(session_user,'raw.pages','id','SELECT')
       OR NOT has_column_privilege(session_user,'raw.pages','tenant_id','SELECT')
       OR NOT has_column_privilege(session_user,'raw.pages','url','SELECT')
       OR NOT has_column_privilege(session_user,'raw.pages','final_url','SELECT')
       OR NOT has_column_privilege(session_user,'raw.pages','content_type','SELECT')
       OR NOT has_column_privilege(session_user,'raw.pages','status_code','SELECT')
       OR NOT has_column_privilege(session_user,'raw.pages','raw_html','SELECT')
       OR NOT has_column_privilege(session_user,'raw.pages','raw_html_bytes','SELECT')
       OR NOT has_column_privilege(session_user,'raw.pages','content_hash','SELECT')
       OR EXISTS (SELECT 1 FROM pg_catalog.pg_roles r
          WHERE r.rolname=session_user
            AND (r.rolsuper OR r.rolbypassrls OR r.rolcreaterole
                 OR r.rolcreatedb OR r.rolreplication)) THEN
        RAISE EXCEPTION 'external source issuer must be distinct and tenant authenticated'
            USING ERRCODE='AV001';
    END IF;
    -- Canonical business-run lock ordering prevents admission/revocation
    -- racing within the SQL authority boundary. The Start must re-read both
    -- roots after its Temporal connection and immediately before Start RPC.
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'operation-dag-business|' || NEW.run_id::text,0));
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'operation-dag-external-source|' || NEW.record_id::text,0));
    v_now := clock_timestamp();
    -- Do not acquire a row lock after the advisory lock: the business-root
    -- UPDATE trigger holds its row lock *before* requesting this advisory
    -- lock, so SELECT FOR SHARE here could deadlock with revocation. The
    -- subsequent Start performs a fresh root/source read before dispatch.
    SELECT b.tenant_id,b.context_key,b.plan_hash,b.expires_at,b.revoked_at
      INTO v_business FROM pipeline.operation_dag_business_runs_v1 b
     WHERE b.run_id = NEW.run_id;
    IF NOT FOUND OR v_business.tenant_id IS DISTINCT FROM NEW.tenant_id
       OR v_business.context_key IS DISTINCT FROM NEW.context_key
       OR v_business.plan_hash IS DISTINCT FROM NEW.plan_hash
       OR v_business.revoked_at IS NOT NULL
       OR v_business.expires_at <= v_now + interval '90 seconds'
       OR NOT EXISTS (SELECT 1 FROM pipeline.tenant_context_ownership own
            WHERE own.tenant_id=NEW.tenant_id AND own.context_key=NEW.context_key)
    THEN
        RAISE EXCEPTION 'external source not bound to live independent business authority'
            USING ERRCODE='AV001';
    END IF;
    -- The source must be an existing immutable, tenant-attributed ingress
    -- snapshot with a real captured BLAKE3. Caller-supplied URL/text does not
    -- satisfy this rule, and historical NULL-tenant pages remain quarantined.
    -- Unlike the business root, raw page UPDATE does not take this business
    -- advisory lock. FOR SHARE here coordinates with the snapshot mutation
    -- guard below and prevents an old-hash admission from racing a page edit.
    SELECT p.tenant_id,p.content_hash,p.raw_html,p.raw_html_bytes INTO v_source
      FROM raw.pages p WHERE p.id=NEW.source_page_id FOR SHARE;
    IF NOT FOUND OR v_source.tenant_id IS DISTINCT FROM NEW.tenant_id
       OR v_source.content_hash IS DISTINCT FROM NEW.source_page_content_hash
       OR v_source.content_hash !~ '^[0-9a-f]{64}$'
       OR v_source.raw_html IS NULL
       OR v_source.raw_html_bytes IS DISTINCT FROM
          octet_length(v_source.raw_html) THEN
        RAISE EXCEPTION 'external source lacks matching tenant-owned raw snapshot'
            USING ERRCODE='AV001';
    END IF;
    v_actual_raw_sha256 := encode(public.digest(v_source.raw_html,'sha256'),'hex');
    IF NEW.source_page_raw_sha256 IS NOT NULL
       AND NEW.source_page_raw_sha256 <> v_actual_raw_sha256 THEN
        RAISE EXCEPTION 'external source raw HTML SHA256 mismatch'
            USING ERRCODE='AV001';
    END IF;
    NEW.source_page_raw_sha256 := v_actual_raw_sha256;
    -- A signed caller's supplied "issued_at" is not authority. The actual
    -- database issuance instant always wins; ten-minute expiry is finite.
    NEW.issued_at := v_now;
    IF NEW.expires_at <= v_now
       OR NEW.expires_at > v_now + interval '10 minutes'
       OR NEW.expires_at > v_business.expires_at THEN
        RAISE EXCEPTION 'external source attestation expiry invalid'
            USING ERRCODE='AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_operation_dag_external_source_attestation_v1()
    FROM PUBLIC;
CREATE TRIGGER guard_operation_dag_external_source_attestation_v1
    BEFORE INSERT OR UPDATE OR DELETE
    ON pipeline.operation_dag_external_source_attestations_v1
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_operation_dag_external_source_attestation_v1();
CREATE TRIGGER guard_operation_dag_external_source_attestation_truncate_v1
    BEFORE TRUNCATE ON pipeline.operation_dag_external_source_attestations_v1
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.guard_operation_dag_external_source_attestation_v1();

-- An attested raw snapshot must not silently change after admission. The
-- ordinary `processed` progress flag may still advance; every field that can
-- influence source identity or a derived typed operation payload stays frozen.
-- This guard is scoped to pages referenced by NEW V1 source attestations.
-- The FK already blocks DELETE of referenced pages.
CREATE FUNCTION raw.guard_attested_external_source_snapshot_v1()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM pipeline.operation_dag_external_source_attestations_v1 a
        WHERE a.source_page_id = OLD.id
    ) AND (to_jsonb(NEW) - 'processed') IS DISTINCT FROM
          (to_jsonb(OLD) - 'processed') THEN
        RAISE EXCEPTION 'tenant raw snapshot is immutable after source attestation'
            USING ERRCODE='AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION raw.guard_attested_external_source_snapshot_v1() FROM PUBLIC;
CREATE TRIGGER guard_attested_external_source_snapshot_v1
    BEFORE UPDATE ON raw.pages FOR EACH ROW
    EXECUTE FUNCTION raw.guard_attested_external_source_snapshot_v1();

COMMENT ON TABLE pipeline.operation_dag_external_source_attestations_v1 IS
    'Privileged issuer-owned raw snapshot + exact payload identity for native Temporal V2 External inputs; SELECT-only Start role, no public grants';
COMMENT ON COLUMN pipeline.operation_dag_external_source_attestations_v1.payload_hash IS
    'BLAKE3 hex64 of the exact typed admitted JSON bytes, independently checked by operation_protocol; distinct from raw_page_content_hash';
COMMENT ON COLUMN pipeline.operation_dag_external_source_attestations_v1.source_page_raw_sha256 IS
    'PostgreSQL SHA256 of actual raw.pages.raw_html bytes at issuance; not an issuer-supplied value. Does not replace BLAKE3 verification by trusted Rust source transformer';

COMMIT;
