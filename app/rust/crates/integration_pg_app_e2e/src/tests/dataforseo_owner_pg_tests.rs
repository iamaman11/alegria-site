// Real PostgreSQL authority tests for the opt-in DataForSEO Temporal V2
// owner. No provider traffic, paid task, or simulated operator attestation.

#[tokio::test]
#[serial]
async fn dataforseo_owner_pg_independent_admission_and_durable_stub_ledger() {
    let pg = PostgresHarness::start().await.expect("disposable PostgreSQL");
    // The checked-in baseline intentionally excludes pilot owner migrations.
    // Install their immutable dependency chain on this disposable database.
    for (label, migration) in [
        (
            "qdrant mixed owner compatibility",
            include_str!("../../../../../db/migrations/20261125_0017_qdrant_mixed_owner_v2.sql"),
        ),
        (
            "PostgreSQL generic owner",
            include_str!("../../../../../db/migrations/20261125_0018_pg_generic_owner_v1.sql"),
        ),
        (
            "Qdrant generic owner",
            include_str!("../../../../../db/migrations/20261125_0019_qdrant_generic_owner_v3.sql"),
        ),
        (
            "DAG governance",
            include_str!("../../../../../db/migrations/20261125_0020_operation_dag_run_governance_v1.sql"),
        ),
        (
            "PG vector projection",
            include_str!("../../../../../db/migrations/20261125_0021_pg_vector_projection_v1.sql"),
        ),
        (
            "V2 PostgreSQL owner governance",
            include_str!("../../../../../db/migrations/20261125_0022_pg_owner_v2_admission.sql"),
        ),
        (
            "V2 Qdrant owner governance",
            include_str!("../../../../../db/migrations/20261125_0023_qdrant_owner_v2_admission.sql"),
        ),
        (
            "DataForSEO separately admitted stub owner",
            include_str!("../../../../../db/migrations/20261127_0001_dataforseo_owner_v1_admission.sql"),
        ),
    ] {
        sqlx::raw_sql(migration)
            .execute(&pg.pool)
            .await
            .unwrap_or_else(|error| panic!("{label} PostgreSQL migration failed: {error}"));
    }

    let nonce = Uuid::new_v4().simple().to_string();
    let worker = format!("dataforseo_worker_{nonce}");
    let issuer = format!("dataforseo_issuer_{nonce}");
    let tenant = format!("dataforseo-tenant-{nonce}");
    const PASSWORD: &str = "disposable_dataforseo_pg_test_password";
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {worker} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {issuer} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {worker},{issuer};
         GRANT USAGE ON SCHEMA pipeline TO {worker},{issuer};"
    )))
    .execute(&pg.pool)
    .await
    .expect("create separately credentialed test owner and issuer");
    for role in [&worker, &issuer] {
        sqlx::query(
            "INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id)
             VALUES ($1::name,$2)",
        )
        .bind(role)
        .bind(&tenant)
        .execute(&pg.pool)
        .await
        .expect("bind the isolated role to one tenant");
    }
    let prefix = "postgres://postgres:postgres_password@";
    assert!(
        pg.database_url.starts_with(prefix),
        "only disposable PostgresHarness credentials are allowed"
    );
    let as_role = |role: &str| {
        pg.database_url.replacen(
            prefix,
            &format!("postgres://{role}:{PASSWORD}@"),
            1,
        )
    };
    let worker_pool = sqlx::PgPool::connect(&as_role(&worker))
        .await
        .expect("connect isolated DataForSEO worker");
    let issuer_pool = sqlx::PgPool::connect(&as_role(&issuer))
        .await
        .expect("connect isolated DataForSEO operator");

    // Migration alone grants neither provider effects nor issuance authority.
    let worker_can_issue: bool = sqlx::query_scalar(
        "SELECT has_table_privilege(session_user,
            'pipeline.dataforseo_owner_admissions_v1','INSERT')",
    )
    .fetch_one(&worker_pool)
    .await
    .unwrap();
    assert!(!worker_can_issue, "a DataForSEO worker cannot issue admissions");
    let operator_can_call_begin: bool = sqlx::query_scalar(
        "SELECT has_function_privilege(session_user,
            'pipeline.begin_dataforseo_owner_v1(uuid,bytea,text,bigint)','EXECUTE')",
    )
    .fetch_one(&issuer_pool)
    .await
    .unwrap();
    assert!(
        !operator_can_call_begin,
        "the independent issuer cannot invoke owner effects"
    );

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT EXECUTE ON FUNCTION pipeline.lookup_dataforseo_owner_admission_v1(
            text,uuid,text,text,text,text,text,text,text,text,text,bigint)
         TO {worker};
         GRANT EXECUTE ON FUNCTION pipeline.begin_dataforseo_owner_v1(
            uuid,bytea,text,bigint) TO {worker};
         GRANT INSERT ON pipeline.dataforseo_owner_admissions_v1 TO {issuer};"
    )))
    .execute(&pg.pool)
    .await
    .expect("explicit DBA least-privilege grants for one test");

    let unknown = Uuid::new_v4();
    let none: Option<Uuid> = sqlx::query_scalar(
        "SELECT pipeline.lookup_dataforseo_owner_admission_v1(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    )
    .bind(&tenant)
    .bind(unknown)
    .bind("a".repeat(64))
    .bind("discovery")
    .bind(format!("alegria-operation-dag-v2-{unknown}"))
    .bind("nonexistent-parent-run")
    .bind("alegria-owner-operation-v2-".to_owned() + &"b".repeat(64))
    .bind("nonexistent-child-run")
    .bind("b".repeat(64))
    .bind("c".repeat(64))
    .bind("d".repeat(64))
    .bind(1_i64)
    .fetch_one(&worker_pool)
    .await
    .expect("missing independent History-issued admission returns NULL");
    assert!(none.is_none(), "a fabricated command cannot mint an admission");

    assert!(
        sqlx::query_scalar::<_, String>(
            "SELECT pipeline.begin_dataforseo_owner_v1($1,$2,$3,$4)",
        )
        .bind(unknown)
        .bind(b"unadmitted-command".as_slice())
        .bind("b".repeat(64))
        .bind(1_i64)
        .fetch_one(&worker_pool)
        .await
        .is_err(),
        "an unadmitted owner cannot claim even a zero-cost stub task"
    );
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.dataforseo_owner_attempts_v1",
    )
        .fetch_one(&pg.pool)
        .await
        .unwrap();
    assert_eq!(
        attempt_count, 0,
        "a forged owner cannot create a paid/stub attempt"
    );
    assert!(
        sqlx::query("TRUNCATE pipeline.dataforseo_owner_attempts_v1")
            .execute(&pg.pool)
            .await
            .is_err(),
        "immutable effect-attempt journal cannot be truncated by anyone"
    );
    assert!(
        sqlx::query("TRUNCATE pipeline.dataforseo_owner_receipts_v1")
            .execute(&pg.pool)
            .await
            .is_err(),
        "immutable provider receipts cannot be truncated"
    );

    // Positive physical SQL progression uses four *separate* role sessions.
    // This is NOT a fabricated Temporal History attestation: the separate
    // native issuer must still prove actual server Histories before writing
    // these exact rows in an executable scenario.
    let business_role = format!("dataforseo_business_{nonce}");
    let governance_role = format!("dataforseo_governance_{nonce}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {business_role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {governance_role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {business_role},{governance_role};
         GRANT USAGE ON SCHEMA pipeline TO {business_role},{governance_role};
         GRANT INSERT ON pipeline.operation_dag_business_runs_v1 TO {business_role};
         GRANT SELECT ON pipeline.operation_dag_business_runs_v1 TO {governance_role};
         GRANT INSERT ON pipeline.operation_dag_run_governance_v2 TO {governance_role};
         GRANT EXECUTE ON FUNCTION pipeline.record_stub_dataforseo_task_v1(
             uuid,bytea,text,text),
             pipeline.lookup_stub_dataforseo_task_v1(uuid,bytea,text),
             pipeline.confirm_dataforseo_owner_v1(
                 uuid,bytea,text,text,bytea,bytea,bigint),
             pipeline.read_confirmed_dataforseo_owner_receipt_v1(uuid,bytea,text)
         TO {worker};"
    )))
    .execute(&pg.pool)
    .await
    .expect("grant exact SQL capabilities to separated native DataForSEO actors");
    for role in [&business_role, &governance_role] {
        sqlx::query(
            "INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id)
             VALUES ($1::name,$2)",
        )
        .bind(role)
        .bind(&tenant)
        .execute(&pg.pool)
        .await
        .expect("bind independent root issuers to the same test tenant");
    }

    let business = sqlx::PgPool::connect(&as_role(&business_role))
        .await
        .expect("independent business issuer session");
    let governance = sqlx::PgPool::connect(&as_role(&governance_role))
        .await
        .expect("independent Temporal governance issuer session");
    let run_id = Uuid::new_v4();
    let admission_id = Uuid::new_v4();
    let context_key = format!("dataforseo-durable-test-{run_id}");
    let plan_hash = "a".repeat(64);
    let dispatch_key = "b".repeat(64);
    let parent_hash = "c".repeat(64);
    let child_hash = "d".repeat(64);
    let command_hash = "e".repeat(64);
    let input_hash = "f".repeat(64);
    let parent_workflow_id = format!("alegria-operation-dag-v2-{run_id}");
    let child_workflow_id = format!("alegria-owner-operation-v2-{dispatch_key}");
    let parent_run_id = format!("server-parent-{run_id}");
    let child_run_id = format!("server-child-{run_id}");
    let command_bytes = b"bounded-local-dataforseo-stub-command".to_vec();
    sqlx::query(
        "INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id)
         VALUES ($1,$2)",
    )
    .bind(&context_key)
    .bind(&tenant)
    .execute(&pg.pool)
    .await
    .expect("establish tenant-owned canonical research context");
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_business_runs_v1
           (run_id,tenant_id,context_key,plan_hash,parent_workflow_id,expires_at)
         VALUES ($1,$2,$3,$4,$5,clock_timestamp()+interval '10 minutes')",
    )
    .bind(run_id)
    .bind(&tenant)
    .bind(&context_key)
    .bind(&plan_hash)
    .bind(&parent_workflow_id)
    .execute(&business)
    .await
    .expect("independent root issuer binds tenant, run and exact plan");
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_run_governance_v2
           (parent_workflow_run_id,canonical_run_id,tenant_id,context_key,
            parent_workflow_id,parent_workflow_type,parent_task_queue,
            plan_hash,parent_start_sha256,expires_at)
         VALUES ($1,$2,$3,$4,$5,'AlegriaOperationDagV2Workflow',
                 'alegria-operation-dag-v2',$6,$7,
                 clock_timestamp()+interval '10 minutes')",
    )
    .bind(&parent_run_id)
    .bind(run_id)
    .bind(&tenant)
    .bind(&context_key)
    .bind(&parent_workflow_id)
    .bind(&plan_hash)
    .bind(&parent_hash)
    .execute(&governance)
    .await
    .expect("independent governance role binds exact parent identity");

    // The tenant-scoped issuer is neither the business issuer nor the owner.
    // SQL cannot itself authenticate Temporal Histories: this row represents
    // only the *post-attestation* transaction tested in isolation.
    sqlx::query(
        "INSERT INTO pipeline.dataforseo_owner_admissions_v1
           (admission_id,run_id,tenant_id,context_key,operation_instance,
            plan_hash,dispatch_execution_key,parent_workflow_id,
            parent_workflow_run_id,child_workflow_id,child_workflow_run_id,
            child_started_event_id,parent_sha256,child_sha256,
            command_sha256,command_hash,input_hash,command_bytes,
            max_cost_usd_micros,expires_at)
         VALUES ($1,$2,$3,$4,'google_search',$5,$6,$7,$8,$9,$10,
                 17,$11,$12,encode(public.digest($13,'sha256'),'hex'),
                 $14,$15,$13,0,clock_timestamp()+interval '5 minutes')",
    )
    .bind(admission_id)
    .bind(run_id)
    .bind(&tenant)
    .bind(&context_key)
    .bind(&plan_hash)
    .bind(&dispatch_key)
    .bind(&parent_workflow_id)
    .bind(&parent_run_id)
    .bind(&child_workflow_id)
    .bind(&child_run_id)
    .bind(&parent_hash)
    .bind(&child_hash)
    .bind(&command_bytes)
    .bind(&command_hash)
    .bind(&input_hash)
    .execute(&issuer_pool)
    .await
    .expect("separate owner issuer can commit exactly one bound stub admission");

    let locate = || {
        sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT pipeline.lookup_dataforseo_owner_admission_v1(
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        )
        .bind(&tenant)
        .bind(run_id)
        .bind(&plan_hash)
        .bind("google_search")
        .bind(&parent_workflow_id)
        .bind(&parent_run_id)
        .bind(&child_workflow_id)
        .bind(&child_run_id)
        .bind(&dispatch_key)
        .bind(&command_hash)
        .bind(&input_hash)
        .bind(0_i64)
    };
    assert_eq!(
        locate().fetch_one(&worker_pool).await.unwrap(),
        Some(admission_id),
        "only exact role/tenant/History identity locates an admitted stub"
    );
    assert!(
        sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT pipeline.lookup_dataforseo_owner_admission_v1(
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        )
        .bind(&tenant)
        .bind(run_id)
        .bind(&plan_hash)
        .bind("google_search")
        .bind(&parent_workflow_id)
        .bind(&parent_run_id)
        .bind(&child_workflow_id)
        .bind("forged-child-Temporal-Run-ID")
        .bind(&dispatch_key)
        .bind(&command_hash)
        .bind(&input_hash)
        .bind(0_i64)
        .fetch_one(&worker_pool)
        .await
        .unwrap()
        .is_none(),
        "a claimed owner-child Run ID cannot substitute for the server Run ID"
    );

    let begin = || {
        sqlx::query_scalar::<_, String>(
            "SELECT pipeline.begin_dataforseo_owner_v1($1,$2,$3,$4)",
        )
        .bind(admission_id)
        .bind(&command_bytes)
        .bind(&dispatch_key)
        .bind(0_i64)
    };
    assert_eq!(begin().fetch_one(&worker_pool).await.unwrap(), "started");
    assert_eq!(
        begin().fetch_one(&worker_pool).await.unwrap(),
        "uncertain",
        "after an ambiguous POST there must not be a second automatic POST"
    );
    assert!(
        sqlx::query_scalar::<_, String>(
            "SELECT pipeline.begin_dataforseo_owner_v1($1,$2,$3,$4)",
        )
        .bind(admission_id)
        .bind(b"tampered-command".as_slice())
        .bind(&dispatch_key)
        .bind(0_i64)
        .fetch_one(&worker_pool)
        .await
        .is_err(),
        "changed owner-command bytes cannot replay a legitimate admission"
    );
    let recorded: bool = sqlx::query_scalar(
        "SELECT pipeline.record_stub_dataforseo_task_v1($1,$2,$3,$4)",
    )
    .bind(admission_id)
    .bind(&command_bytes)
    .bind(&dispatch_key)
    .bind("stub-serp-7")
    .fetch_one(&worker_pool)
    .await
    .unwrap();
    assert!(recorded);
    assert_eq!(
        begin().fetch_one(&worker_pool).await.unwrap(),
        "posted",
        "a durably saved task ID permits GET-only resume"
    );
    let persisted_task: String = sqlx::query_scalar(
        "SELECT pipeline.lookup_stub_dataforseo_task_v1($1,$2,$3)",
    )
    .bind(admission_id)
    .bind(&command_bytes)
    .bind(&dispatch_key)
    .fetch_one(&worker_pool)
    .await
    .unwrap();
    assert_eq!(persisted_task, "stub-serp-7");
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT pipeline.record_stub_dataforseo_task_v1($1,$2,$3,$4)",
        )
        .bind(admission_id)
        .bind(&command_bytes)
        .bind(&dispatch_key)
        .bind("stub-serp-8")
        .fetch_one(&worker_pool)
        .await
        .is_err(),
        "one immutable task ID cannot be swapped after an accepted POST"
    );
    let result_bytes = b"typed-stub-output-only".to_vec();
    let receipt_bytes = b"durable-stub-owner-receipt".to_vec();
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT pipeline.confirm_dataforseo_owner_v1(
                $1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(admission_id)
        .bind(&command_bytes)
        .bind(&dispatch_key)
        .bind("stub-serp-7")
        .bind(&result_bytes)
        .bind(&receipt_bytes)
        .bind(1_i64)
        .fetch_one(&worker_pool)
        .await
        .is_err(),
        "a stub can never claim a nonzero paid-provider cost"
    );
    let confirmed: bool = sqlx::query_scalar(
        "SELECT pipeline.confirm_dataforseo_owner_v1(
            $1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(admission_id)
    .bind(&command_bytes)
    .bind(&dispatch_key)
    .bind("stub-serp-7")
    .bind(&result_bytes)
    .bind(&receipt_bytes)
    .bind(0_i64)
    .fetch_one(&worker_pool)
    .await
    .unwrap();
    assert!(confirmed);
    let repeated_confirmation: bool = sqlx::query_scalar(
        "SELECT pipeline.confirm_dataforseo_owner_v1(
            $1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(admission_id)
    .bind(&command_bytes)
    .bind(&dispatch_key)
    .bind("stub-serp-7")
    .bind(&result_bytes)
    .bind(&receipt_bytes)
    .bind(0_i64)
    .fetch_one(&worker_pool)
    .await
    .unwrap();
    assert!(repeated_confirmation, "exact same receipt is safely idempotent");
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT pipeline.confirm_dataforseo_owner_v1(
                $1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(admission_id)
        .bind(&command_bytes)
        .bind(&dispatch_key)
        .bind("stub-serp-7")
        .bind(b"changed-result".as_slice())
        .bind(&receipt_bytes)
        .bind(0_i64)
        .fetch_one(&worker_pool)
        .await
        .is_err(),
        "a confirmed receipt is immutable even to the original worker"
    );
    assert_eq!(begin().fetch_one(&worker_pool).await.unwrap(), "confirmed");
    let stored_receipt: Vec<u8> = sqlx::query_scalar(
        "SELECT pipeline.read_confirmed_dataforseo_owner_receipt_v1($1,$2,$3)",
    )
    .bind(admission_id)
    .bind(&command_bytes)
    .bind(&dispatch_key)
    .fetch_one(&worker_pool)
    .await
    .unwrap();
    assert_eq!(stored_receipt, receipt_bytes);
    let attempts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.dataforseo_owner_attempts_v1
         WHERE admission_id=$1",
    )
    .bind(admission_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(attempts, 1, "the admission can create only one durable attempt");
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.dataforseo_owner_receipts_v1
         WHERE admission_id=$1",
    )
    .bind(admission_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(receipts, 1, "idempotent confirmation cannot create multiple receipts");
}
