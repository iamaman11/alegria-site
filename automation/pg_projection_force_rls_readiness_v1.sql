-- #94 forward-only, READ-ONLY production readiness gate for SQL 0021.
-- Execute as an independent DBA against an exact release candidate AFTER the
-- reviewed FORCE RLS/credential cutover. It changes no persistent data.
-- The older 0015 preflight covers only its original ten relations; this gate
-- covers three additional generic DAG projection/evidence relations. Passing
-- does NOT authorize production deployment or historical Temporal cutover.
-- Expected outcome before FORCE RLS is NO-GO (SQLSTATE AV001).

DO $pg_projection_preflight$
DECLARE
    bad_relation TEXT;
    bad_role TEXT;
    bad_function TEXT;
    checked_tables INTEGER;
BEGIN
    IF NOT COALESCE((
        SELECT rolsuper OR rolbypassrls
        FROM pg_catalog.pg_roles WHERE rolname = session_user
    ), FALSE) THEN
        RAISE EXCEPTION 'PG projection FORCE RLS audit requires an independent DBA session'
            USING ERRCODE = 'AV001';
    END IF;

    -- Exact RLS status, two explicit tenant policies for source/admissions,
    -- one read-only policy for committed neutral receipts. Any additional
    -- permissive OR-policy is a fail-closed NO-GO.
    SELECT relation_name INTO bad_relation
    FROM (
        VALUES
            ('pipeline.vector_projection_sources_v1', 2),
            ('pipeline.pg_vector_projection_admissions_v1', 2),
            ('pipeline.operation_artifact_receipts_v1', 1)
    ) AS expected(relation_name, expected_policy_count)
    JOIN pg_catalog.pg_class rel ON rel.oid = expected.relation_name::regclass
    CROSS JOIN LATERAL (
        SELECT COUNT(*) AS policy_count,
               COUNT(*) FILTER (
                   WHERE p.polpermissive AND p.polroles = ARRAY[0::oid]
                     AND ((
                         (expected.relation_name = 'pipeline.vector_projection_sources_v1'
                          AND p.polname = 'vector_projection_source_authenticated_tenant_v1')
                         OR (expected.relation_name = 'pipeline.pg_vector_projection_admissions_v1'
                          AND p.polname = 'pg_vector_projection_admission_tenant_v1')
                         OR (expected.relation_name = 'pipeline.operation_artifact_receipts_v1'
                          AND p.polname = 'operation_artifact_receipt_tenant_v1')
                     ) AND p.polcmd = 'r'
                     AND pg_catalog.regexp_replace(
                         COALESCE(pg_catalog.pg_get_expr(p.polqual,p.polrelid),''),
                         '[[:space:]()]','','g') = 'tenant_id=pipeline.current_session_tenant_v1'
                     AND p.polwithcheck IS NULL
                     OR (
                         (expected.relation_name = 'pipeline.vector_projection_sources_v1'
                          AND p.polname = 'vector_projection_source_authorized_tenant_insert_v1')
                         OR (expected.relation_name = 'pipeline.pg_vector_projection_admissions_v1'
                          AND p.polname = 'pg_vector_projection_admission_issuer_v1')
                     ) AND p.polcmd = 'a'
                     AND p.polqual IS NULL
                     AND pg_catalog.regexp_replace(
                         COALESCE(pg_catalog.pg_get_expr(p.polwithcheck,p.polrelid),''),
                         '[[:space:]()]','','g') = 'tenant_id=pipeline.current_session_tenant_v1'
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
        'pipeline.vector_projection_sources_v1'::regclass,
        'pipeline.pg_vector_projection_admissions_v1'::regclass,
        'pipeline.operation_artifact_receipts_v1'::regclass
    ]);
    IF checked_tables <> 3 THEN
        RAISE EXCEPTION 'PG projection audit did not cover exactly three additional relations'
            USING ERRCODE = 'AV001';
    END IF;

    -- Even an ostensibly unprivileged LOGIN may inherit a protected table
    -- owner's identity or use an unsafe capability. New generic DAG relations
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
               'pipeline.vector_projection_sources_v1'::regclass,
               'pipeline.pg_vector_projection_admissions_v1'::regclass,
               'pipeline.operation_artifact_receipts_v1'::regclass
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

    RAISE NOTICE 'PG_PROJECTION_FORCE_RLS_ADDITIONAL_THREE_RELATIONS=READY; cutover still requires #94 full history/rollback gates';
END;
$pg_projection_preflight$;
