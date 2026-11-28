-- One-shot, user-scoped request authorization for a SINGLE native Temporal
-- V2 operation. This never grants the right to Start Temporal by itself.
--
-- Four independent principals:
--   DBA provisions a per-approval-login immutable principal/policy/context;
--   approval LOGIN authenticates the user and inserts a sealed approval event;
--   distinct grant issuer LOGIN checks that event and inserts its one-use grant;
--   distinct Start LOGIN atomically reserves a grant and irreversibly marks
--   an UNKNOWN RPC outcome *before* making the Temporal Start call.
-- The existing independent business-root, source and V2 History issuers remain
-- mandatory. No role grants, credential provisioning or production cutover.
-- A PostgreSQL LOGIN cannot by itself authenticate a remote ChatGPT account;
-- the external user-identity handoff is an additional release gate.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE FUNCTION pipeline.one_step_limits_valid_v1(p JSONB)
RETURNS BOOLEAN LANGUAGE plpgsql IMMUTABLE SET search_path = pg_catalog AS $$
DECLARE
    k TEXT;
    n TEXT;
BEGIN
    IF p IS NULL OR jsonb_typeof(p) <> 'object'
       OR (SELECT count(*) FROM jsonb_object_keys(p)) <> 5
       OR NOT (p ?& ARRAY['max_admitted_source_bytes',
                    'max_effect_dispatches','max_provider_requests',
                    'currency_code','max_charge_minor_units'])
       OR jsonb_typeof(p->'currency_code') <> 'string'
       OR (p->>'currency_code') !~ '^[A-Z]{3}$' THEN
       RETURN FALSE;
    END IF;
    FOREACH k IN ARRAY ARRAY['max_admitted_source_bytes',
                 'max_effect_dispatches','max_provider_requests',
                 'max_charge_minor_units'] LOOP
        IF jsonb_typeof(p->k) <> 'number' THEN RETURN FALSE; END IF;
        n := p->>k;
        IF n !~ '^(0|[1-9][0-9]{0,17})$' THEN RETURN FALSE; END IF;
    END LOOP;
    RETURN (p->>'max_admitted_source_bytes')::numeric BETWEEN 1 AND 524288
       AND (p->>'max_effect_dispatches')::numeric BETWEEN 0 AND 1
       AND (p->>'max_provider_requests')::numeric BETWEEN 0 AND 1000
       AND (p->>'max_charge_minor_units')::numeric BETWEEN 0 AND 900000000000000000;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.one_step_limits_valid_v1(JSONB) FROM PUBLIC;

CREATE FUNCTION pipeline.one_step_limits_within_v1(p JSONB,p_max JSONB)
RETURNS BOOLEAN LANGUAGE sql IMMUTABLE SET search_path = pg_catalog AS $$
   SELECT CASE WHEN NOT pipeline.one_step_limits_valid_v1(p)
                 OR NOT pipeline.one_step_limits_valid_v1(p_max) THEN FALSE
       ELSE (p->>'currency_code') = (p_max->>'currency_code')
        AND (p->>'max_admitted_source_bytes')::numeric
             <= (p_max->>'max_admitted_source_bytes')::numeric
        AND (p->>'max_effect_dispatches')::numeric
             <= (p_max->>'max_effect_dispatches')::numeric
        AND (p->>'max_provider_requests')::numeric
             <= (p_max->>'max_provider_requests')::numeric
        AND (p->>'max_charge_minor_units')::numeric
             <= (p_max->>'max_charge_minor_units')::numeric
       END;
$$;
REVOKE ALL ON FUNCTION pipeline.one_step_limits_within_v1(JSONB,JSONB) FROM PUBLIC;

CREATE FUNCTION pipeline.one_step_expected_valid_v1(p JSONB,p_plan_hash TEXT)
RETURNS BOOLEAN LANGUAGE plpgsql IMMUTABLE SET search_path = pg_catalog AS $$
BEGIN
   IF p IS NULL OR jsonb_typeof(p) <> 'object'
      OR (SELECT count(*) FROM jsonb_object_keys(p)) <> 4
      OR NOT (p ?& ARRAY['schema_revision','plan_hash',
                         'command_hashes','source_payload_hashes'])
      OR jsonb_typeof(p->'schema_revision') <> 'number'
      OR p->>'schema_revision' <> '1'
      OR jsonb_typeof(p->'plan_hash') <> 'string'
      OR p->>'plan_hash' IS DISTINCT FROM p_plan_hash
      OR p_plan_hash IS NULL OR p_plan_hash !~ '^[0-9a-f]{64}$'
      OR jsonb_typeof(p->'command_hashes') <> 'object'
      OR jsonb_typeof(p->'source_payload_hashes') <> 'object'
      OR (SELECT count(*) FROM jsonb_object_keys(p->'command_hashes')) <> 1
      OR (SELECT count(*) FROM jsonb_object_keys(p->'source_payload_hashes')) > 32
   THEN RETURN FALSE; END IF;
   IF EXISTS (SELECT 1 FROM jsonb_each(p->'command_hashes') e
       WHERE length(e.key) NOT BETWEEN 1 AND 128
          OR e.key <> btrim(e.key) OR jsonb_typeof(e.value) <> 'string'
          OR e.value #>> '{}' !~ '^[0-9a-f]{64}$')
      OR EXISTS (SELECT 1 FROM jsonb_each(p->'source_payload_hashes') e
       WHERE length(e.key) NOT BETWEEN 1 AND 128
          OR e.key <> btrim(e.key) OR jsonb_typeof(e.value) <> 'string'
          OR e.value #>> '{}' !~ '^[0-9a-f]{64}$') THEN
       RETURN FALSE;
   END IF;
   RETURN TRUE;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.one_step_expected_valid_v1(JSONB,TEXT) FROM PUBLIC;

-- Provisioned by a distinct DBA, never by the approver/issuer/Start roles.
-- An independent credential per user/policy is required; an arbitrary
-- application-supplied principal_id or tenant GUC is never identity proof.
CREATE TABLE pipeline.operation_dag_one_step_principal_policies_v1 (
    role_name NAME NOT NULL,
    context_key TEXT NOT NULL CHECK (length(context_key) BETWEEN 1 AND 512
       AND context_key=btrim(context_key)),
    tenant_id TEXT NOT NULL CHECK (length(tenant_id) BETWEEN 1 AND 128
       AND tenant_id=btrim(tenant_id)),
    principal_id TEXT NOT NULL CHECK (length(principal_id) BETWEEN 1 AND 128
       AND principal_id=btrim(principal_id)),
    policy_id TEXT NOT NULL CHECK (length(policy_id) BETWEEN 1 AND 128
       AND policy_id=btrim(policy_id)),
    max_limits JSONB NOT NULL CHECK (pipeline.one_step_limits_valid_v1(max_limits)),
    provisioned_by NAME NOT NULL DEFAULT session_user,
    provisioned_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (role_name,context_key)
);
REVOKE ALL ON pipeline.operation_dag_one_step_principal_policies_v1 FROM PUBLIC;
ALTER TABLE pipeline.operation_dag_one_step_principal_policies_v1 ENABLE ROW LEVEL SECURITY;
CREATE POLICY one_step_policy_tenant_v1
    ON pipeline.operation_dag_one_step_principal_policies_v1
    USING (tenant_id = pipeline.current_session_tenant_v1());
CREATE TRIGGER deny_one_step_policy_mutation_v1
    BEFORE UPDATE OR DELETE ON pipeline.operation_dag_one_step_principal_policies_v1
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_tenant_ownership_mutation_v1();
CREATE TRIGGER deny_one_step_policy_truncate_v1
    BEFORE TRUNCATE ON pipeline.operation_dag_one_step_principal_policies_v1
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_ownership_mutation_v1();

-- The Start service must also be independently bound by the DBA to exactly
-- one authenticated user/policy within its own tenant and context. A generic
-- tenant-scoped Start service may NOT pick any principal_id in the tenant by
-- copying an approved grant. This never reads an arbitrary request GUC.
CREATE TABLE pipeline.operation_dag_one_step_start_principals_v1 (
    role_name NAME NOT NULL,
    context_key TEXT NOT NULL CHECK (length(context_key) BETWEEN 1 AND 512
       AND context_key=btrim(context_key)),
    tenant_id TEXT NOT NULL CHECK (length(tenant_id) BETWEEN 1 AND 128
       AND tenant_id=btrim(tenant_id)),
    principal_id TEXT NOT NULL CHECK (length(principal_id) BETWEEN 1 AND 128
       AND principal_id=btrim(principal_id)),
    policy_id TEXT NOT NULL CHECK (length(policy_id) BETWEEN 1 AND 128
       AND policy_id=btrim(policy_id)),
    max_limits JSONB NOT NULL CHECK (pipeline.one_step_limits_valid_v1(max_limits)),
    provisioned_by NAME NOT NULL DEFAULT session_user,
    provisioned_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (role_name,context_key)
);
REVOKE ALL ON pipeline.operation_dag_one_step_start_principals_v1 FROM PUBLIC;
ALTER TABLE pipeline.operation_dag_one_step_start_principals_v1 ENABLE ROW LEVEL SECURITY;
CREATE POLICY one_step_start_principal_tenant_v1
    ON pipeline.operation_dag_one_step_start_principals_v1
    USING (tenant_id = pipeline.current_session_tenant_v1());
CREATE TRIGGER deny_one_step_start_principal_mutation_v1
    BEFORE UPDATE OR DELETE ON pipeline.operation_dag_one_step_start_principals_v1
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_tenant_ownership_mutation_v1();
CREATE TRIGGER deny_one_step_start_principal_truncate_v1
    BEFORE TRUNCATE ON pipeline.operation_dag_one_step_start_principals_v1
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_ownership_mutation_v1();

-- A mistaken INSERT grant must not let an ordinary service role provision its
-- own ACL identity. Only a separate elevated DBA may insert either policy;
-- even that insert must target an already bound immutable tenant+context and
-- login. These are administrative configuration rows, never request data.
CREATE FUNCTION pipeline.guard_one_step_dba_provision_v1()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
  IF NEW.provisioned_by<>session_user
     OR NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles admin
          WHERE admin.rolname=session_user AND (admin.rolsuper OR admin.rolcreaterole))
     OR NOT EXISTS (SELECT 1 FROM pipeline.tenant_context_ownership own
          WHERE own.context_key=NEW.context_key AND own.tenant_id=NEW.tenant_id)
     OR NOT EXISTS (SELECT 1 FROM pipeline.tenant_role_ownership own
          WHERE own.role_name=NEW.role_name AND own.tenant_id=NEW.tenant_id) THEN
     RAISE EXCEPTION 'one-step principal policy requires independent DBA provisioning'
       USING ERRCODE='AV001';
  END IF;
  NEW.provisioned_at := clock_timestamp();
  RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_one_step_dba_provision_v1() FROM PUBLIC;
CREATE TRIGGER guard_one_step_approval_policy_provision_v1
    BEFORE INSERT ON pipeline.operation_dag_one_step_principal_policies_v1
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_one_step_dba_provision_v1();
CREATE TRIGGER guard_one_step_start_policy_provision_v1
    BEFORE INSERT ON pipeline.operation_dag_one_step_start_principals_v1
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_one_step_dba_provision_v1();

CREATE TABLE pipeline.operation_dag_one_step_request_approvals_v1 (
    authorization_event_id UUID PRIMARY KEY
       CHECK (authorization_event_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    request_id UUID NOT NULL UNIQUE
       CHECK (request_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    run_id UUID NOT NULL UNIQUE REFERENCES pipeline.operation_dag_business_runs_v1(run_id),
    tenant_id TEXT NOT NULL CHECK (length(tenant_id) BETWEEN 1 AND 128 AND tenant_id=btrim(tenant_id)),
    principal_id TEXT NOT NULL CHECK (length(principal_id) BETWEEN 1 AND 128 AND principal_id=btrim(principal_id)),
    policy_id TEXT NOT NULL CHECK (length(policy_id) BETWEEN 1 AND 128 AND policy_id=btrim(policy_id)),
    context_key TEXT NOT NULL CHECK (length(context_key) BETWEEN 1 AND 512 AND context_key=btrim(context_key)),
    operation_id TEXT NOT NULL CHECK (length(operation_id) BETWEEN 1 AND 128 AND operation_id=btrim(operation_id)),
    operation_revision INTEGER NOT NULL CHECK (operation_revision > 0),
    parent_workflow_id TEXT NOT NULL
       CHECK (parent_workflow_id = 'alegria-operation-dag-v2-' || run_id::text),
    plan_hash TEXT NOT NULL CHECK (plan_hash ~ '^[0-9a-f]{64}$'),
    expected_start JSONB NOT NULL
       CHECK (pipeline.one_step_expected_valid_v1(expected_start,plan_hash)),
    limits JSONB NOT NULL CHECK (pipeline.one_step_limits_valid_v1(limits)),
    approved_by NAME NOT NULL DEFAULT session_user,
    approved_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    CHECK (expires_at > approved_at AND expires_at <= approved_at + interval '10 minutes'),
    CHECK (revoked_at IS NULL OR revoked_at >= approved_at)
);
REVOKE ALL ON pipeline.operation_dag_one_step_request_approvals_v1 FROM PUBLIC;
ALTER TABLE pipeline.operation_dag_one_step_request_approvals_v1 ENABLE ROW LEVEL SECURITY;
CREATE POLICY one_step_approval_tenant_v1 ON pipeline.operation_dag_one_step_request_approvals_v1
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

CREATE TABLE pipeline.operation_dag_one_step_request_grants_v1 (
    request_id UUID PRIMARY KEY
       CHECK (request_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    authorization_event_id UUID NOT NULL UNIQUE
       REFERENCES pipeline.operation_dag_one_step_request_approvals_v1(authorization_event_id),
    run_id UUID NOT NULL UNIQUE REFERENCES pipeline.operation_dag_business_runs_v1(run_id),
    tenant_id TEXT NOT NULL CHECK (length(tenant_id) BETWEEN 1 AND 128 AND tenant_id=btrim(tenant_id)),
    principal_id TEXT NOT NULL CHECK (length(principal_id) BETWEEN 1 AND 128 AND principal_id=btrim(principal_id)),
    policy_id TEXT NOT NULL CHECK (length(policy_id) BETWEEN 1 AND 128 AND policy_id=btrim(policy_id)),
    context_key TEXT NOT NULL CHECK (length(context_key) BETWEEN 1 AND 512 AND context_key=btrim(context_key)),
    operation_id TEXT NOT NULL CHECK (length(operation_id) BETWEEN 1 AND 128 AND operation_id=btrim(operation_id)),
    operation_revision INTEGER NOT NULL CHECK (operation_revision > 0),
    parent_workflow_id TEXT NOT NULL
       CHECK (parent_workflow_id = 'alegria-operation-dag-v2-' || run_id::text),
    plan_hash TEXT NOT NULL CHECK (plan_hash ~ '^[0-9a-f]{64}$'),
    expected_start JSONB NOT NULL
       CHECK (pipeline.one_step_expected_valid_v1(expected_start,plan_hash)),
    limits JSONB NOT NULL CHECK (pipeline.one_step_limits_valid_v1(limits)),
    issued_by NAME NOT NULL DEFAULT session_user,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    state TEXT NOT NULL DEFAULT 'unclaimed'
       CHECK (state IN ('unclaimed','reserved','start_outcome_unknown','start_confirmed')),
    reservation_nonce UUID,
    reserved_by NAME,
    reserved_at TIMESTAMPTZ,
    unknown_at TIMESTAMPTZ,
    confirmed_at TIMESTAMPTZ,
    CHECK (expires_at > issued_at AND expires_at <= issued_at + interval '10 minutes'),
    CHECK (revoked_at IS NULL OR revoked_at >= issued_at),
    CHECK ((state='unclaimed' AND reservation_nonce IS NULL AND reserved_by IS NULL
             AND reserved_at IS NULL AND unknown_at IS NULL AND confirmed_at IS NULL)
        OR (state='reserved' AND reservation_nonce IS NOT NULL AND reserved_by IS NOT NULL
            AND reserved_at IS NOT NULL AND unknown_at IS NULL AND confirmed_at IS NULL)
        OR (state='start_outcome_unknown' AND reservation_nonce IS NOT NULL
            AND reserved_by IS NOT NULL AND reserved_at IS NOT NULL
            AND unknown_at IS NOT NULL AND confirmed_at IS NULL)
        OR (state='start_confirmed' AND reservation_nonce IS NOT NULL
            AND reserved_by IS NOT NULL AND reserved_at IS NOT NULL
            AND unknown_at IS NOT NULL AND confirmed_at IS NOT NULL))
);
REVOKE ALL ON pipeline.operation_dag_one_step_request_grants_v1 FROM PUBLIC;
ALTER TABLE pipeline.operation_dag_one_step_request_grants_v1 ENABLE ROW LEVEL SECURITY;
CREATE POLICY one_step_grant_tenant_v1 ON pipeline.operation_dag_one_step_request_grants_v1
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
CREATE INDEX idx_one_step_grants_v1_scope
    ON pipeline.operation_dag_one_step_request_grants_v1
    (tenant_id,principal_id,context_key,run_id,state);

-- An SQL LOGIN is an authority only when the DBA bound that very login to
-- this tenant. Membership/SET ROLE/elevated logins are denied regardless of
-- requested identity strings or role-inheritance settings.
CREATE FUNCTION pipeline.one_step_is_separate_login_v1()
RETURNS BOOLEAN LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog AS $$
  SELECT COALESCE((SELECT r.rolcanlogin AND NOT r.rolinherit
      AND NOT r.rolsuper AND NOT r.rolbypassrls AND NOT r.rolcreaterole
      AND NOT r.rolcreatedb AND NOT r.rolreplication
      AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_auth_members m
                      WHERE m.member=r.oid)
      FROM pg_catalog.pg_roles r WHERE r.rolname=session_user),FALSE);
$$;
REVOKE ALL ON FUNCTION pipeline.one_step_is_separate_login_v1() FROM PUBLIC;

CREATE FUNCTION pipeline.guard_operation_dag_one_step_approval_v1()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE v_policy RECORD; v_root RECORD; v_now TIMESTAMPTZ;
BEGIN
  IF current_setting('transaction_isolation') <> 'read committed' THEN
     RAISE EXCEPTION 'one-step approval requires READ COMMITTED' USING ERRCODE='AV001';
  END IF;
  IF TG_OP='DELETE' OR TG_OP='TRUNCATE' THEN
     RAISE EXCEPTION 'one-step approval is immutable' USING ERRCODE='AV001';
  END IF;
  IF TG_OP='UPDATE' THEN
     IF OLD.approved_by<>session_user OR OLD.revoked_at IS NOT NULL
        OR NEW.revoked_at IS NULL
        OR (to_jsonb(NEW)-'revoked_at') IS DISTINCT FROM (to_jsonb(OLD)-'revoked_at')
        OR pipeline.current_session_tenant_v1() IS DISTINCT FROM OLD.tenant_id THEN
        RAISE EXCEPTION 'only approval issuer may revoke approval' USING ERRCODE='AV001';
     END IF;
     PERFORM pg_advisory_xact_lock(hashtextextended(
       'operation-dag-business|' || OLD.run_id::text,0));
     NEW.revoked_at := clock_timestamp();
     RETURN NEW;
  END IF;
  IF NOT pipeline.one_step_is_separate_login_v1()
     OR NEW.approved_by<>session_user
     OR pipeline.current_session_tenant_v1() IS DISTINCT FROM NEW.tenant_id
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_request_grants_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_principal_policies_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_principal_policies_v1','UPDATE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_principal_policies_v1','DELETE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_start_principals_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_start_principals_v1','UPDATE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_start_principals_v1','DELETE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_business_runs_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_external_source_attestations_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_run_governance_v2','INSERT') THEN
     RAISE EXCEPTION 'approval principal is not isolated or tenant authenticated'
       USING ERRCODE='AV001';
  END IF;
  SELECT * INTO v_policy FROM pipeline.operation_dag_one_step_principal_policies_v1 p
   WHERE p.role_name=session_user AND p.context_key=NEW.context_key;
  IF NOT FOUND OR v_policy.tenant_id IS DISTINCT FROM NEW.tenant_id
     OR v_policy.principal_id IS DISTINCT FROM NEW.principal_id
     OR v_policy.policy_id IS DISTINCT FROM NEW.policy_id
     OR NOT pipeline.one_step_limits_within_v1(NEW.limits,v_policy.max_limits) THEN
     RAISE EXCEPTION 'approval not backed by independently provisioned ACL/budget'
       USING ERRCODE='AV001';
  END IF;
  PERFORM pg_advisory_xact_lock(hashtextextended(
      'operation-dag-business|' || NEW.run_id::text,0));
  v_now := clock_timestamp();
  SELECT b.* INTO v_root FROM pipeline.operation_dag_business_runs_v1 b
   WHERE b.run_id=NEW.run_id;
  IF NOT FOUND OR v_root.tenant_id IS DISTINCT FROM NEW.tenant_id
     OR v_root.context_key IS DISTINCT FROM NEW.context_key
     OR v_root.plan_hash IS DISTINCT FROM NEW.plan_hash
     OR v_root.parent_workflow_id IS DISTINCT FROM NEW.parent_workflow_id
     OR v_root.revoked_at IS NOT NULL
     OR v_root.expires_at <= v_now + interval '90 seconds'
     OR NOT EXISTS (SELECT 1 FROM pipeline.tenant_context_ownership own
           WHERE own.tenant_id=NEW.tenant_id AND own.context_key=NEW.context_key) THEN
     RAISE EXCEPTION 'approval requires live independently authorized business root'
       USING ERRCODE='AV001';
  END IF;
  NEW.approved_at := v_now;
  IF NEW.revoked_at IS NOT NULL OR NEW.expires_at<=v_now
     OR NEW.expires_at>v_now+interval '10 minutes'
     OR NEW.expires_at>v_root.expires_at THEN
     RAISE EXCEPTION 'approval expiry or state invalid' USING ERRCODE='AV001';
  END IF;
  RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_operation_dag_one_step_approval_v1() FROM PUBLIC;
CREATE TRIGGER guard_one_step_approval_v1
   BEFORE INSERT OR UPDATE OR DELETE
   ON pipeline.operation_dag_one_step_request_approvals_v1
   FOR EACH ROW EXECUTE FUNCTION pipeline.guard_operation_dag_one_step_approval_v1();
CREATE TRIGGER guard_one_step_approval_truncate_v1
   BEFORE TRUNCATE ON pipeline.operation_dag_one_step_request_approvals_v1
   FOR EACH STATEMENT EXECUTE FUNCTION pipeline.guard_operation_dag_one_step_approval_v1();

CREATE FUNCTION pipeline.guard_operation_dag_one_step_grant_v1()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE v_approval RECORD; v_root RECORD; v_now TIMESTAMPTZ;
BEGIN
  IF current_setting('transaction_isolation') <> 'read committed' THEN
     RAISE EXCEPTION 'one-step grant requires READ COMMITTED' USING ERRCODE='AV001';
  END IF;
  IF TG_OP='DELETE' OR TG_OP='TRUNCATE' THEN
     RAISE EXCEPTION 'one-step grant is immutable and cannot be deleted'
       USING ERRCODE='AV001';
  END IF;
  IF TG_OP='UPDATE' THEN
     IF pipeline.current_session_tenant_v1() IS DISTINCT FROM OLD.tenant_id
        OR (to_jsonb(NEW)-'revoked_at'-'state'-'reservation_nonce'
                        -'reserved_by'-'reserved_at'-'unknown_at'-'confirmed_at')
              IS DISTINCT FROM
           (to_jsonb(OLD)-'revoked_at'-'state'-'reservation_nonce'
                        -'reserved_by'-'reserved_at'-'unknown_at'-'confirmed_at') THEN
        RAISE EXCEPTION 'one-step grant cannot change sealed request identity'
          USING ERRCODE='AV001';
     END IF;
     IF OLD.state='unclaimed' AND NEW.state='unclaimed'
        AND OLD.issued_by=session_user AND OLD.revoked_at IS NULL
        AND NEW.revoked_at IS NOT NULL AND NEW.reservation_nonce IS NULL THEN
        -- UPDATE owns this grant row before invoking its trigger. Do not
        -- acquire the business advisory key here: the Start reservation
        -- acquires business first, then this row, so doing both here in the
        -- opposite order could deadlock with a concurrent grant revocation.
        -- Row-level serialization alone suffices for revocation vs claim.
        NEW.revoked_at := clock_timestamp();
        RETURN NEW;
     END IF;
     IF NEW.revoked_at IS DISTINCT FROM OLD.revoked_at
        OR OLD.reserved_by IS DISTINCT FROM session_user
           AND OLD.state <> 'unclaimed' THEN
        RAISE EXCEPTION 'one-step state may change only by reservation owner'
          USING ERRCODE='AV001';
     END IF;
     IF OLD.state='unclaimed' AND NEW.state='reserved'
        AND NOT has_table_privilege(session_user,
               'pipeline.operation_dag_one_step_request_grants_v1','UPDATE')
        AND NEW.reserved_by=session_user AND NEW.reservation_nonce IS NOT NULL
        AND NEW.reserved_at IS NOT NULL
        AND NEW.unknown_at IS NULL AND NEW.confirmed_at IS NULL THEN
        RETURN NEW;
     END IF;
     IF OLD.state='reserved' AND NEW.state='start_outcome_unknown'
        AND NOT has_table_privilege(session_user,
               'pipeline.operation_dag_one_step_request_grants_v1','UPDATE')
        AND OLD.reservation_nonce=NEW.reservation_nonce
        AND OLD.reserved_at=NEW.reserved_at
        AND NEW.unknown_at IS NOT NULL AND NEW.confirmed_at IS NULL THEN
        RETURN NEW;
     END IF;
     IF OLD.state='start_outcome_unknown' AND NEW.state='start_confirmed'
        AND NOT has_table_privilege(session_user,
               'pipeline.operation_dag_one_step_request_grants_v1','UPDATE')
        AND OLD.reservation_nonce=NEW.reservation_nonce
        AND OLD.reserved_at=NEW.reserved_at
        AND OLD.unknown_at=NEW.unknown_at
        AND NEW.confirmed_at IS NOT NULL THEN
        RETURN NEW;
     END IF;
     RAISE EXCEPTION 'one-step grant has forbidden or replayed state transition'
       USING ERRCODE='AV001';
  END IF;
  IF NOT pipeline.one_step_is_separate_login_v1()
     OR NEW.issued_by<>session_user
     OR pipeline.current_session_tenant_v1() IS DISTINCT FROM NEW.tenant_id
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_request_approvals_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_principal_policies_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_principal_policies_v1','UPDATE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_principal_policies_v1','DELETE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_start_principals_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_start_principals_v1','UPDATE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_start_principals_v1','DELETE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_business_runs_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_run_governance_v2','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_external_source_attestations_v1','INSERT')
     OR NEW.state<>'unclaimed' OR NEW.reservation_nonce IS NOT NULL
     OR NEW.reserved_by IS NOT NULL OR NEW.reserved_at IS NOT NULL
     OR NEW.unknown_at IS NOT NULL OR NEW.confirmed_at IS NOT NULL
     OR NEW.revoked_at IS NOT NULL THEN
     RAISE EXCEPTION 'grant issuer lacks independent narrow SQL principal'
       USING ERRCODE='AV001';
  END IF;
  PERFORM pg_advisory_xact_lock(hashtextextended(
      'operation-dag-business|' || NEW.run_id::text,0));
  v_now := clock_timestamp();
  SELECT a.* INTO v_approval FROM pipeline.operation_dag_one_step_request_approvals_v1 a
   WHERE a.authorization_event_id=NEW.authorization_event_id;
  IF NOT FOUND OR v_approval.request_id IS DISTINCT FROM NEW.request_id
     OR v_approval.run_id IS DISTINCT FROM NEW.run_id
     OR v_approval.tenant_id IS DISTINCT FROM NEW.tenant_id
     OR v_approval.principal_id IS DISTINCT FROM NEW.principal_id
     OR v_approval.policy_id IS DISTINCT FROM NEW.policy_id
     OR v_approval.context_key IS DISTINCT FROM NEW.context_key
     OR v_approval.operation_id IS DISTINCT FROM NEW.operation_id
     OR v_approval.operation_revision IS DISTINCT FROM NEW.operation_revision
     OR v_approval.parent_workflow_id IS DISTINCT FROM NEW.parent_workflow_id
     OR v_approval.plan_hash IS DISTINCT FROM NEW.plan_hash
     OR v_approval.expected_start IS DISTINCT FROM NEW.expected_start
     OR v_approval.limits IS DISTINCT FROM NEW.limits
     OR v_approval.approved_by=NEW.issued_by
     OR v_approval.revoked_at IS NOT NULL OR v_approval.expires_at<=v_now THEN
     RAISE EXCEPTION 'grant not identically bound to independent live approval'
       USING ERRCODE='AV001';
  END IF;
  SELECT b.* INTO v_root FROM pipeline.operation_dag_business_runs_v1 b
   WHERE b.run_id=NEW.run_id;
  IF NOT FOUND OR v_root.tenant_id IS DISTINCT FROM NEW.tenant_id
     OR v_root.context_key IS DISTINCT FROM NEW.context_key
     OR v_root.plan_hash IS DISTINCT FROM NEW.plan_hash
     OR v_root.parent_workflow_id IS DISTINCT FROM NEW.parent_workflow_id
     OR v_root.revoked_at IS NOT NULL
     OR v_root.expires_at<=v_now + interval '90 seconds' THEN
     RAISE EXCEPTION 'grant has no active independent business authority'
       USING ERRCODE='AV001';
  END IF;
  NEW.issued_at := v_now;
  IF NEW.expires_at<=v_now OR NEW.expires_at>v_now+interval '10 minutes'
     OR NEW.expires_at>v_approval.expires_at
     OR NEW.expires_at>v_root.expires_at THEN
     RAISE EXCEPTION 'grant expiry invalid' USING ERRCODE='AV001';
  END IF;
  RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.guard_operation_dag_one_step_grant_v1() FROM PUBLIC;
CREATE TRIGGER guard_one_step_grant_v1
   BEFORE INSERT OR UPDATE OR DELETE
   ON pipeline.operation_dag_one_step_request_grants_v1
   FOR EACH ROW EXECUTE FUNCTION pipeline.guard_operation_dag_one_step_grant_v1();
CREATE TRIGGER guard_one_step_grant_truncate_v1
   BEFORE TRUNCATE ON pipeline.operation_dag_one_step_request_grants_v1
   FOR EACH STATEMENT EXECUTE FUNCTION pipeline.guard_operation_dag_one_step_grant_v1();

-- SELECT-only Start role may call this narrowly granted SECURITY DEFINER
-- capability. Never grant direct table UPDATE. All checks and the one-use
-- transition are atomic in READ COMMITTED; a replay or concurrent caller
-- receives FALSE and must not attempt any Temporal effect.
CREATE FUNCTION pipeline.reserve_operation_dag_one_step_request_v1(
    p_request_id UUID,p_run_id UUID,p_tenant_id TEXT,p_principal_id TEXT,
    p_policy_id TEXT,p_context_key TEXT,p_plan_hash TEXT,p_reservation_nonce UUID
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE g RECORD; a RECORD; b RECORD; v_now TIMESTAMPTZ;
BEGIN
  IF current_setting('transaction_isolation') <> 'read committed'
     OR NOT pipeline.one_step_is_separate_login_v1()
     OR pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
     OR p_request_id IS NULL OR p_run_id IS NULL
     OR p_reservation_nonce IS NULL
     OR p_reservation_nonce='00000000-0000-0000-0000-000000000000'::uuid
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_request_grants_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_request_grants_v1','UPDATE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_request_approvals_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_principal_policies_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_principal_policies_v1','UPDATE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_principal_policies_v1','DELETE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_start_principals_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_start_principals_v1','UPDATE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_start_principals_v1','DELETE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_run_governance_v2','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_external_source_attestations_v1','INSERT')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_business_runs_v1','INSERT') THEN
     RETURN FALSE;
  END IF;
  PERFORM pg_advisory_xact_lock(hashtextextended(
      'operation-dag-business|' || p_run_id::text,0));
  PERFORM pg_advisory_xact_lock(hashtextextended(
      'one-step-request|' || p_request_id::text,0));
  v_now := clock_timestamp();
  SELECT * INTO g FROM pipeline.operation_dag_one_step_request_grants_v1
   WHERE request_id=p_request_id FOR UPDATE;
  IF NOT FOUND OR g.run_id IS DISTINCT FROM p_run_id
     OR g.tenant_id IS DISTINCT FROM p_tenant_id
     OR g.principal_id IS DISTINCT FROM p_principal_id
     OR g.policy_id IS DISTINCT FROM p_policy_id
     OR g.context_key IS DISTINCT FROM p_context_key
     OR g.plan_hash IS DISTINCT FROM p_plan_hash
     OR g.parent_workflow_id IS DISTINCT FROM
          'alegria-operation-dag-v2-' || p_run_id::text
     OR g.state<>'unclaimed' OR g.revoked_at IS NOT NULL
     OR g.issued_at>v_now OR g.expires_at<=v_now + interval '90 seconds'
     OR g.issued_by=session_user THEN RETURN FALSE; END IF;
  SELECT * INTO a FROM pipeline.operation_dag_one_step_request_approvals_v1
   WHERE authorization_event_id=g.authorization_event_id;
  IF NOT FOUND OR a.request_id IS DISTINCT FROM p_request_id
     OR a.run_id IS DISTINCT FROM p_run_id OR a.tenant_id IS DISTINCT FROM p_tenant_id
     OR a.principal_id IS DISTINCT FROM p_principal_id
     OR a.policy_id IS DISTINCT FROM p_policy_id
     OR a.context_key IS DISTINCT FROM p_context_key
     OR a.plan_hash IS DISTINCT FROM p_plan_hash
     OR a.expected_start IS DISTINCT FROM g.expected_start
     OR a.limits IS DISTINCT FROM g.limits
     OR a.revoked_at IS NOT NULL OR a.expires_at<=v_now + interval '90 seconds'
     OR a.approved_by=session_user THEN RETURN FALSE; END IF;
  SELECT * INTO b FROM pipeline.operation_dag_business_runs_v1
   WHERE run_id=p_run_id;
  IF NOT FOUND OR b.tenant_id IS DISTINCT FROM p_tenant_id
     OR b.context_key IS DISTINCT FROM p_context_key
     OR b.plan_hash IS DISTINCT FROM p_plan_hash
     OR b.parent_workflow_id IS DISTINCT FROM g.parent_workflow_id
     OR b.revoked_at IS NOT NULL OR b.expires_at<=v_now + interval '90 seconds'
     OR b.issued_by=session_user
     OR NOT EXISTS (SELECT 1 FROM pipeline.tenant_context_ownership o
              WHERE o.tenant_id=p_tenant_id AND o.context_key=p_context_key) THEN
     RETURN FALSE;
  END IF;
  -- The Start LOGIN's own independently provisioned identity + ceiling,
  -- not a principal copied from the client's locator or the grant itself.
  IF NOT EXISTS (
      SELECT 1 FROM pipeline.operation_dag_one_step_start_principals_v1 sp
       WHERE sp.role_name=session_user AND sp.context_key=p_context_key
         AND sp.tenant_id=p_tenant_id AND sp.principal_id=p_principal_id
         AND sp.policy_id=p_policy_id
         AND pipeline.one_step_limits_within_v1(g.limits,sp.max_limits)
  ) THEN RETURN FALSE; END IF;
  -- An approved zero-effect *pure calculation* may consume one exact-hash
  -- user-supplied DTO without claiming authenticated source provenance. Both
  -- independent SQL roles already bound the exact command, payload hash,
  -- operation, tenant, user and one-use budget. This exception is strictly
  -- enumerated: never admit a provider/effect, a future revision, a multi-
  -- source request or a nonzero budget through the pure-input path.
  -- All other externally supplied inputs still REQUIRE a separately issued,
  -- immutable raw-snapshot attestation (a URL or provenance label is not one).
  IF NOT (
      g.operation_revision=1
      AND g.operation_id IN (
          'seo_steps.extract_procedural_rules',
          'seo_steps.normalize_serp_queries',
          'seo_steps.build_seo_opportunities',
          'truth.judge_completeness',
          'truth.detect_contradictions'
      )
      AND (g.limits->>'max_effect_dispatches')::numeric=0
      AND (g.limits->>'max_provider_requests')::numeric=0
      AND (g.limits->>'max_charge_minor_units')::numeric=0
      AND (SELECT count(*) FROM jsonb_object_keys(
          g.expected_start->'source_payload_hashes'))=1
  ) AND EXISTS (SELECT 1 FROM jsonb_each_text(g.expected_start->'source_payload_hashes') src
       WHERE NOT EXISTS (
         SELECT 1 FROM pipeline.operation_dag_external_source_attestations_v1 att
          WHERE att.run_id=p_run_id AND att.tenant_id=p_tenant_id
            AND att.context_key=p_context_key AND att.plan_hash=p_plan_hash
            AND att.artifact_id=src.key AND att.payload_hash=src.value
            AND att.revoked_at IS NULL AND att.expires_at>v_now + interval '90 seconds'
       )) THEN RETURN FALSE; END IF;
  UPDATE pipeline.operation_dag_one_step_request_grants_v1
    SET state='reserved', reservation_nonce=p_reservation_nonce,
        reserved_by=session_user,reserved_at=v_now
    WHERE request_id=p_request_id AND state='unclaimed';
  RETURN FOUND;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.reserve_operation_dag_one_step_request_v1(
    UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,UUID) FROM PUBLIC;

-- MUST COMMIT this transition BEFORE any Temporal RPC. A crash/lost ACK
-- yields an explicit durable unknown outcome, never a second Start attempt.
CREATE FUNCTION pipeline.mark_operation_dag_one_step_start_unknown_v1(
    p_request_id UUID,p_reservation_nonce UUID
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE g RECORD;
BEGIN
  IF current_setting('transaction_isolation') <> 'read committed'
     OR NOT pipeline.one_step_is_separate_login_v1()
     OR p_request_id IS NULL OR p_reservation_nonce IS NULL
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_request_grants_v1','UPDATE') THEN
     RETURN FALSE;
  END IF;
  SELECT * INTO g FROM pipeline.operation_dag_one_step_request_grants_v1
   WHERE request_id=p_request_id;
  IF NOT FOUND OR g.reservation_nonce IS DISTINCT FROM p_reservation_nonce
     OR g.reserved_by IS DISTINCT FROM session_user
     OR g.state<>'reserved'
     OR pipeline.current_session_tenant_v1() IS DISTINCT FROM g.tenant_id THEN
     RETURN FALSE;
  END IF;
  PERFORM pg_advisory_xact_lock(hashtextextended(
      'one-step-request|' || p_request_id::text,0));
  UPDATE pipeline.operation_dag_one_step_request_grants_v1
      SET state='start_outcome_unknown',unknown_at=clock_timestamp()
   WHERE request_id=p_request_id AND reservation_nonce=p_reservation_nonce
     AND reserved_by=session_user AND state='reserved';
  RETURN FOUND;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.mark_operation_dag_one_step_start_unknown_v1(UUID,UUID)
    FROM PUBLIC;

-- Confirmation is not based on a successful network response. A separate
-- governance issuer must first validate REAL Temporal parent History and
-- issue the exact immutable V2 run seal. SQL Start role cannot issue it.
-- This records evidence of an already-started workflow; it never retries.
CREATE FUNCTION pipeline.confirm_operation_dag_one_step_start_v1(
    p_request_id UUID,p_reservation_nonce UUID,
    p_parent_workflow_run_id TEXT,p_parent_start_sha256 TEXT
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE g RECORD;
BEGIN
  IF current_setting('transaction_isolation') <> 'read committed'
     OR NOT pipeline.one_step_is_separate_login_v1()
     OR p_request_id IS NULL OR p_reservation_nonce IS NULL
     OR p_parent_workflow_run_id IS NULL OR length(btrim(p_parent_workflow_run_id))=0
     OR p_parent_start_sha256 !~ '^[0-9a-f]{64}$'
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_one_step_request_grants_v1','UPDATE')
     OR has_table_privilege(session_user,
          'pipeline.operation_dag_run_governance_v2','INSERT') THEN
     RETURN FALSE;
  END IF;
  SELECT * INTO g FROM pipeline.operation_dag_one_step_request_grants_v1
   WHERE request_id=p_request_id;
  IF NOT FOUND OR g.reservation_nonce IS DISTINCT FROM p_reservation_nonce
     OR g.reserved_by IS DISTINCT FROM session_user
     OR g.state<>'start_outcome_unknown'
     OR pipeline.current_session_tenant_v1() IS DISTINCT FROM g.tenant_id THEN
     RETURN FALSE;
  END IF;
  PERFORM pg_advisory_xact_lock(hashtextextended(
      'operation-dag-business|' || g.run_id::text,0));
  PERFORM pg_advisory_xact_lock(hashtextextended(
      'one-step-request|' || p_request_id::text,0));
  IF NOT pipeline.operation_dag_governance_matches_v2(
      g.run_id,g.tenant_id,g.context_key,g.parent_workflow_id,
      p_parent_workflow_run_id,g.plan_hash,p_parent_start_sha256,FALSE) THEN
     RETURN FALSE;
  END IF;
  UPDATE pipeline.operation_dag_one_step_request_grants_v1
     SET state='start_confirmed',confirmed_at=clock_timestamp()
   WHERE request_id=p_request_id AND reservation_nonce=p_reservation_nonce
     AND reserved_by=session_user AND state='start_outcome_unknown';
  RETURN FOUND;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.confirm_operation_dag_one_step_start_v1(
    UUID,UUID,TEXT,TEXT) FROM PUBLIC;

COMMENT ON TABLE pipeline.operation_dag_one_step_principal_policies_v1 IS
  'DBA-authored, immutable LOGIN/context→user-policy-budget mapping; no remote ChatGPT identity accepted without separate authenticated integration';
COMMENT ON TABLE pipeline.operation_dag_one_step_start_principals_v1 IS
  'DBA-authored immutable Start LOGIN/context→exact user/policy/budget ceiling; a tenant-shared Start role cannot impersonate other users';
COMMENT ON TABLE pipeline.operation_dag_one_step_request_approvals_v1 IS
  'User-approved event sealed by independent tenant-bound SQL approval principal; not grant issuer and not Temporal Start';
COMMENT ON TABLE pipeline.operation_dag_one_step_request_grants_v1 IS
  'Independently issued single-use Start request; unclaimed→reserved→durably unknown→history-confirmed, never arbitrary replay';
COMMIT;
