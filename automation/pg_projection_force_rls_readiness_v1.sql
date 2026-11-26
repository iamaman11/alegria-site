-- #94 forward-only, READ-ONLY production readiness gate for SQL 0017-0023.
-- Execute as an independent DBA against an exact release candidate AFTER the
-- reviewed FORCE RLS/credential cutover. It changes no persistent data.
-- The older 0015 preflight covers only its original ten relations; this gate
-- covers ALL sixteen additional tenant tables introduced in 0017-0023. Passing
-- does NOT authorize production deployment or historical Temporal cutover.
-- Expected outcome before FORCE RLS is NO-GO (SQLSTATE AV001).

DO $pg_projection_preflight$
DECLARE
    bad_relation TEXT;
    bad_role TEXT;
    bad_function TEXT;
    checked_tables INTEGER;
    required_guard RECORD;
BEGIN
    IF NOT COALESCE((
        SELECT rolsuper OR rolbypassrls
        FROM pg_catalog.pg_roles WHERE rolname = session_user
    ), FALSE) THEN
        RAISE EXCEPTION 'PG projection FORCE RLS audit requires an independent DBA session'
            USING ERRCODE = 'AV001';
    END IF;

    -- Exact RLS status and policy shape for all post-0015 tenant tables.
    -- The PG source and admission require separate SELECT/INSERT policies;
    -- its neutral receipt has SELECT only. Every Qdrant ledger, reservation,
    -- business authority and DAG governance table uses one ALL tenant policy.
    -- An extra permissive OR-policy (or changed policy command) is NO-GO.
    SELECT relation_name INTO bad_relation
    FROM (
        VALUES
            ('pipeline.qdrant_projection_v2_admissions', 1, 'qdrant_v2_admission_tenant', NULL),
            ('pipeline.qdrant_projection_v2_attempts', 1, 'qdrant_v2_attempt_tenant', NULL),
            ('pipeline.qdrant_projection_v2_receipts', 1, 'qdrant_v2_receipt_tenant', NULL),
            ('pipeline.qdrant_projection_business_key_reservations_v3', 1, 'qdrant_business_reservation_v3_tenant', NULL),
            ('pipeline.qdrant_projection_v3_admissions', 1, 'qdrant_v3_admission_tenant', NULL),
            ('pipeline.qdrant_projection_v3_attempts', 1, 'qdrant_v3_attempt_tenant', NULL),
            ('pipeline.qdrant_projection_v3_receipts', 1, 'qdrant_v3_receipt_tenant', NULL),
            ('pipeline.operation_dag_run_governance_v1', 1, 'operation_dag_run_governance_v1_tenant', NULL),
            ('pipeline.vector_projection_sources_v1', 2, 'vector_projection_source_authenticated_tenant_v1', 'vector_projection_source_authorized_tenant_insert_v1'),
            ('pipeline.pg_vector_projection_admissions_v1', 2, 'pg_vector_projection_admission_tenant_v1', 'pg_vector_projection_admission_issuer_v1'),
            ('pipeline.operation_artifact_receipts_v1', 1, 'operation_artifact_receipt_tenant_v1', NULL),
            ('pipeline.operation_dag_business_runs_v1', 1, 'operation_dag_business_run_tenant_v1', NULL),
            ('pipeline.operation_dag_run_governance_v2', 1, 'operation_dag_run_governance_v2_tenant', NULL),
            ('pipeline.qdrant_projection_v4_admissions', 1, 'qdrant_v4_admission_tenant', NULL),
            ('pipeline.qdrant_projection_v4_attempts', 1, 'qdrant_v4_attempt_tenant', NULL),
            ('pipeline.qdrant_projection_v4_receipts', 1, 'qdrant_v4_receipt_tenant', NULL)
    ) AS expected(relation_name, expected_policy_count, owner_policy, insert_policy)
    JOIN pg_catalog.pg_class rel ON rel.oid = expected.relation_name::regclass
    CROSS JOIN LATERAL (
        SELECT COUNT(*) AS policy_count,
               COUNT(*) FILTER (
                   WHERE p.polpermissive AND p.polroles = ARRAY[0::oid]
                     AND (
                         (p.polname = expected.owner_policy
                          AND p.polcmd = (CASE
                              WHEN expected.expected_policy_count=2
                                OR expected.relation_name='pipeline.operation_artifact_receipts_v1'
                              THEN 'r' ELSE '*' END)::"char"
                          AND pg_catalog.regexp_replace(
                              COALESCE(pg_catalog.pg_get_expr(p.polqual,p.polrelid),''),
                              '[[:space:]()]','','g') = 'tenant_id=pipeline.current_session_tenant_v1'
                          -- PostgreSQL uses USING as the implicit WITH CHECK
                          -- for ALL policies whose WITH CHECK is omitted
                          -- (reservation and governance migration policies).
                          AND (p.polcmd <> '*' OR p.polwithcheck IS NULL OR
                              pg_catalog.regexp_replace(
                                  COALESCE(pg_catalog.pg_get_expr(p.polwithcheck,p.polrelid),''),
                                  '[[:space:]()]','','g') = 'tenant_id=pipeline.current_session_tenant_v1')
                          AND (p.polcmd <> 'r' OR p.polwithcheck IS NULL))
                         OR (expected.expected_policy_count=2
                          AND p.polname=expected.insert_policy AND p.polcmd='a'
                          AND p.polqual IS NULL
                          AND pg_catalog.regexp_replace(
                              COALESCE(pg_catalog.pg_get_expr(p.polwithcheck,p.polrelid),''),
                              '[[:space:]()]','','g') = 'tenant_id=pipeline.current_session_tenant_v1')
                     )
               )) AS accepted_count
        FROM pg_catalog.pg_policy p WHERE p.polrelid = rel.oid
    ) AS policies
    WHERE NOT rel.relrowsecurity OR NOT rel.relforcerowsecurity
       OR policies.policy_count <> expected.expected_policy_count
       OR policies.accepted_count <> expected.expected_policy_count
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'PG projection production FORCE RLS/policy NO-GO: %', bad_relation
            USING ERRCODE = 'AV001';
    END IF;
    SELECT COUNT(*) INTO checked_tables FROM pg_catalog.pg_class
    WHERE oid = ANY(ARRAY[
        'pipeline.qdrant_projection_v2_admissions'::regclass,
        'pipeline.qdrant_projection_v2_attempts'::regclass,
        'pipeline.qdrant_projection_v2_receipts'::regclass,
        'pipeline.qdrant_projection_business_key_reservations_v3'::regclass,
        'pipeline.qdrant_projection_v3_admissions'::regclass,
        'pipeline.qdrant_projection_v3_attempts'::regclass,
        'pipeline.qdrant_projection_v3_receipts'::regclass,
        'pipeline.operation_dag_run_governance_v1'::regclass,
        'pipeline.vector_projection_sources_v1'::regclass,
        'pipeline.pg_vector_projection_admissions_v1'::regclass,
        'pipeline.operation_artifact_receipts_v1'::regclass,
        'pipeline.operation_dag_business_runs_v1'::regclass,
        'pipeline.operation_dag_run_governance_v2'::regclass,
        'pipeline.qdrant_projection_v4_admissions'::regclass,
        'pipeline.qdrant_projection_v4_attempts'::regclass,
        'pipeline.qdrant_projection_v4_receipts'::regclass
    ]);
    IF checked_tables <> 16 THEN
        RAISE EXCEPTION 'supplemental FORCE RLS audit did not cover exactly sixteen tenant relations'
            USING ERRCODE = 'AV001';
    END IF;

    -- Even an ostensibly unprivileged LOGIN may inherit a protected table
    -- owner's identity or use an unsafe capability. All 0017-0023 relations
    -- must be included, independently of the older 0015 owner census.
    SELECT binding.role_name INTO bad_role
    FROM pipeline.tenant_role_ownership binding
    LEFT JOIN pg_catalog.pg_roles login ON login.rolname = binding.role_name
    WHERE login.oid IS NULL OR NOT login.rolcanlogin
       OR login.rolsuper OR login.rolbypassrls OR login.rolcreaterole
       OR login.rolcreatedb OR login.rolreplication
       OR EXISTS (
           SELECT 1 FROM pg_catalog.pg_roles elevated
           WHERE (elevated.rolsuper OR elevated.rolbypassrls
                  OR elevated.rolcreaterole OR elevated.rolcreatedb
                  OR elevated.rolreplication)
             AND pg_catalog.pg_has_role(login.oid,elevated.oid,'MEMBER')
       )
       OR EXISTS (
           SELECT 1 FROM pg_catalog.pg_class protected
           WHERE protected.oid = ANY(ARRAY[
               'pipeline.qdrant_projection_v2_admissions'::regclass,
               'pipeline.qdrant_projection_v2_attempts'::regclass,
               'pipeline.qdrant_projection_v2_receipts'::regclass,
               'pipeline.qdrant_projection_business_key_reservations_v3'::regclass,
               'pipeline.qdrant_projection_v3_admissions'::regclass,
               'pipeline.qdrant_projection_v3_attempts'::regclass,
               'pipeline.qdrant_projection_v3_receipts'::regclass,
               'pipeline.operation_dag_run_governance_v1'::regclass,
               'pipeline.vector_projection_sources_v1'::regclass,
               'pipeline.pg_vector_projection_admissions_v1'::regclass,
               'pipeline.operation_artifact_receipts_v1'::regclass,
               'pipeline.operation_dag_business_runs_v1'::regclass,
               'pipeline.operation_dag_run_governance_v2'::regclass,
               'pipeline.qdrant_projection_v4_admissions'::regclass,
               'pipeline.qdrant_projection_v4_attempts'::regclass,
               'pipeline.qdrant_projection_v4_receipts'::regclass
           ])
             AND (pg_catalog.pg_has_role(login.oid, protected.relowner, 'MEMBER')
                  OR has_table_privilege(login.rolname, protected.oid, 'TRUNCATE')
                  OR has_table_privilege(login.rolname, protected.oid, 'DELETE'))
       )
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'PG projection tenant LOGIN has unsafe authority: %', bad_role
            USING ERRCODE = 'AV001';
    END IF;

    -- Explicitly protected effect boundaries: no PUBLIC EXECUTE on either
    -- SECURITY DEFINER entry point. Function owners must remain able to read
    -- the exact tenant proof after FORCE RLS (not merely by table ownership).
    SELECT p.proname INTO bad_function
    FROM pg_catalog.pg_proc p
    JOIN pg_catalog.pg_roles owner ON owner.oid = p.proowner
    CROSS JOIN LATERAL pg_catalog.aclexplode(
        COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) grant_acl
    WHERE p.oid = ANY(ARRAY[
        'pipeline.consume_pg_vector_projection_owner_v1(uuid,uuid,text,text,text,text,text,text,text,text,text,text,text,bytea)'::regprocedure,
        'pipeline.operation_artifact_receipt_matches_v1(uuid,text,uuid,text,text,text,text,text,text,text,text,text,text,jsonb)'::regprocedure
    ])
      AND (NOT p.prosecdef
           OR NOT (owner.rolsuper OR owner.rolbypassrls)
           OR (grant_acl.grantee = 0 AND grant_acl.privilege_type = 'EXECUTE'))
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'PG projection SQL SECURITY DEFINER authority/PUBLIC EXECUTE NO-GO: %', bad_function
            USING ERRCODE = 'AV001';
    END IF;

    -- Source/receipt removal must remain unrepresentable even after an
    -- accidental privileged TRUNCATE grant. Admission monotonic revocation
    -- has a separate update trigger and is never a reason to delete a receipt.
    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_trigger
        WHERE tgrelid = 'pipeline.vector_projection_sources_v1'::regclass
          AND tgname = 'guard_vector_projection_source_immutable_v1'
          AND NOT tgisinternal AND tgenabled IN ('O','A')
    ) OR NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_trigger
        WHERE tgrelid = 'pipeline.vector_projection_sources_v1'::regclass
          AND tgname = 'guard_vector_projection_source_truncate_v1'
          AND NOT tgisinternal AND tgenabled IN ('O','A')
    ) OR NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_trigger
        WHERE tgrelid = 'pipeline.pg_vector_projection_admissions_v1'::regclass
          AND tgname = 'guard_vector_projection_admission_immutable_v1'
          AND NOT tgisinternal AND tgenabled IN ('O','A')
    ) OR NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_trigger
        WHERE tgrelid = 'pipeline.operation_artifact_receipts_v1'::regclass
          AND tgname = 'guard_operation_artifact_receipt_immutable_v1'
          AND NOT tgisinternal AND tgenabled IN ('O','A')
    ) OR NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_trigger
        WHERE tgrelid = 'pipeline.operation_artifact_receipts_v1'::regclass
          AND tgname = 'guard_operation_artifact_receipt_truncate_v1'
          AND NOT tgisinternal AND tgenabled IN ('O','A')
    ) THEN
        RAISE EXCEPTION 'PG projection immutable source/admission/receipt DML or TRUNCATE guard missing'
            USING ERRCODE = 'AV001';
    END IF;

    -- Every existing cross-version reservation, admission, governance and
    -- append-only receipt guard must still be attached and enabled. These
    -- triggers protect owner-authorized writes; no new FORCE RLS grants or
    -- TRUNCATE privileges are conferred by this read-only inspection.
    FOR required_guard IN
        SELECT * FROM (VALUES
            ('pipeline.qdrant_projection_v2_admissions', 'guard_qdrant_projection_v2_admission'),
            ('pipeline.qdrant_projection_v2_attempts', 'deny_qdrant_projection_v2_attempt_rewrite'),
            ('pipeline.qdrant_projection_v2_receipts', 'deny_qdrant_projection_v2_receipt_rewrite'),
            ('pipeline.qdrant_projection_business_key_reservations_v3', 'deny_qdrant_business_key_reservation_update_v3'),
            ('pipeline.qdrant_projection_business_key_reservations_v3', 'deny_qdrant_business_key_reservation_truncate_v3'),
            ('pipeline.qdrant_projection_v1_admissions', 'reserve_qdrant_v1_business_key_v3'),
            ('pipeline.qdrant_projection_v2_admissions', 'reserve_qdrant_v2_business_key_v3'),
            ('pipeline.qdrant_projection_v3_admissions', 'guard_qdrant_projection_v3_admission'),
            ('pipeline.qdrant_projection_v3_admissions', 'reserve_qdrant_v3_business_key_v3'),
            ('pipeline.qdrant_projection_v3_attempts', 'deny_qdrant_projection_v3_attempt_rewrite'),
            ('pipeline.qdrant_projection_v3_receipts', 'deny_qdrant_projection_v3_receipt_rewrite'),
            ('pipeline.operation_dag_run_governance_v1', 'guard_operation_dag_run_governance_v1'),
            ('pipeline.operation_dag_run_governance_v1', 'guard_operation_dag_run_governance_truncate_v1'),
            ('pipeline.operation_dag_run_governance_v1', 'guard_v1_run_against_active_v2'),
            ('pipeline.operation_dag_business_runs_v1', 'guard_operation_dag_business_run_v1'),
            ('pipeline.operation_dag_business_runs_v1', 'guard_operation_dag_business_run_truncate_v1'),
            ('pipeline.operation_dag_run_governance_v2', 'guard_operation_dag_run_governance_v2'),
            ('pipeline.operation_dag_run_governance_v2', 'guard_operation_dag_run_governance_truncate_v2'),
            ('pipeline.qdrant_projection_v4_admissions', 'guard_qdrant_projection_v4_admission'),
            ('pipeline.qdrant_projection_v4_admissions', 'reserve_qdrant_v4_business_key'),
            ('pipeline.qdrant_projection_v4_admissions', 'deny_qdrant_v4_admission_truncate'),
            ('pipeline.qdrant_projection_v4_attempts', 'deny_qdrant_v4_attempt_rewrite'),
            ('pipeline.qdrant_projection_v4_attempts', 'deny_qdrant_v4_attempt_truncate'),
            ('pipeline.qdrant_projection_v4_receipts', 'deny_qdrant_v4_receipt_rewrite'),
            ('pipeline.qdrant_projection_v4_receipts', 'deny_qdrant_v4_receipt_truncate')
        ) AS required(relation_name, trigger_name)
    LOOP
        IF NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_trigger t
            WHERE t.tgrelid = required_guard.relation_name::regclass
              AND t.tgname = required_guard.trigger_name
              AND NOT t.tgisinternal AND t.tgenabled IN ('O','A')
        ) THEN
            RAISE EXCEPTION 'supplemental FORCE RLS immutable owner trigger missing: %.%',
                required_guard.relation_name, required_guard.trigger_name
                USING ERRCODE = 'AV001';
        END IF;
    END LOOP;

    RAISE NOTICE 'PG_PROJECTION_FORCE_RLS_ADDITIONAL_SIXTEEN_RELATIONS=READY; cutover still requires #94 full history/rollback gates';
END;
$pg_projection_preflight$;
