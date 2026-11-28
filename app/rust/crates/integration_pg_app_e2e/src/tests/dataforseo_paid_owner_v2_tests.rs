// SQL-only physical authority regression for the deliberately opt-in V2 paid
// DataForSEO owner. This supplies fixture identity/History attestations after
// their respective independent issuers; it is NOT a live Temporal or paid API
// test. No provider request, secret, or billing occurs in this suite.

#[tokio::test]
#[serial]
async fn dataforseo_paid_owner_v2_requires_one_use_budget_and_never_reposts_unknown() {
    let pg = PostgresHarness::start().await.expect("disposable PostgreSQL");
    for (label, migration) in [
        ("mixed owner", include_str!("../../../../../db/migrations/20261125_0017_qdrant_mixed_owner_v2.sql")),
        ("PG generic", include_str!("../../../../../db/migrations/20261125_0018_pg_generic_owner_v1.sql")),
        ("Qdrant generic", include_str!("../../../../../db/migrations/20261125_0019_qdrant_generic_owner_v3.sql")),
        ("V2 governance", include_str!("../../../../../db/migrations/20261125_0020_operation_dag_run_governance_v1.sql")),
        ("PG vector", include_str!("../../../../../db/migrations/20261125_0021_pg_vector_projection_v1.sql")),
        ("PG V2 admission", include_str!("../../../../../db/migrations/20261125_0022_pg_owner_v2_admission.sql")),
        ("Qdrant V2 admission", include_str!("../../../../../db/migrations/20261125_0023_qdrant_owner_v2_admission.sql")),
        ("DataForSEO stub V1", include_str!("../../../../../db/migrations/20261127_0001_dataforseo_owner_v1_admission.sql")),
        ("source attestations", include_str!("../../../../../db/migrations/20261127_0002_operation_dag_external_source_attestation_v1.sql")),
        ("user one-use grant", include_str!("../../../../../db/migrations/20261127_0003_operation_dag_one_step_request_grants_v1.sql")),
        ("DataForSEO paid V2", include_str!("../../../../../db/migrations/20261128_0001_dataforseo_paid_owner_v2.sql")),
    ] {
        sqlx::raw_sql(migration).execute(&pg.pool).await
            .unwrap_or_else(|error| panic!("{label} migration failed: {error}"));
    }

    let suffix = Uuid::new_v4().simple().to_string();
    let tenant = format!("paid-tenant-{suffix}");
    let context = format!("paid-context-{suffix}");
    let approver = format!("paid_approver_{suffix}");
    let grant_issuer = format!("paid_grant_issuer_{suffix}");
    let starter = format!("paid_start_{suffix}");
    let business = format!("paid_business_{suffix}");
    let governance = format!("paid_governance_{suffix}");
    let paid_issuer = format!("paid_effect_issuer_{suffix}");
    let worker = format!("paid_effect_worker_{suffix}");
    let foreign = format!("paid_foreign_{suffix}");
    const PASSWORD: &str = "disposable_paid_owner_test_password";

    let roles = [
        &approver, &grant_issuer, &starter, &business,
        &governance, &paid_issuer, &worker, &foreign,
    ];
    let create_roles = roles.iter().map(|r| format!(
        "CREATE ROLE {r} LOGIN NOINHERIT PASSWORD '{PASSWORD}';"
    )).collect::<Vec<_>>().join("\n");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "{create_roles}\nGRANT CONNECT ON DATABASE alegria TO {};\nGRANT USAGE ON SCHEMA pipeline TO {};",
        roles.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(","),
        roles.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(","),
    )))
    .execute(&pg.pool).await.expect("independent paid-effect actors");
    for role in &roles[..7] {
        sqlx::query("INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id) VALUES ($1::name,$2)")
            .bind(role.as_str()).bind(&tenant).execute(&pg.pool).await.unwrap();
    }
    sqlx::query("INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id) VALUES ($1::name,$2)")
        .bind(&foreign).bind(format!("foreign-{suffix}"))
        .execute(&pg.pool).await.unwrap();
    const PREFIX: &str = "postgres://postgres:postgres_password@";
    assert!(pg.database_url.starts_with(PREFIX));
    let pool_for = |role: &str| sqlx::PgPool::connect_lazy(
        &pg.database_url.replacen(PREFIX, &format!("postgres://{role}:{PASSWORD}@"), 1)
    ).expect("role PostgreSQL URL");
    let approver_pg=pool_for(&approver);
    let grant_pg=pool_for(&grant_issuer);
    let start_pg=pool_for(&starter);
    let business_pg=pool_for(&business);
    let governance_pg=pool_for(&governance);
    let issuer_pg=pool_for(&paid_issuer);
    let worker_pg=pool_for(&worker);
    let foreign_pg=pool_for(&foreign);

    let run_id=Uuid::new_v4();
    let request_id=Uuid::new_v4();
    let auth_id=Uuid::new_v4();
    let admission_id=Uuid::new_v4();
    let parent_workflow_id=format!("alegria-operation-dag-v2-{run_id}");
    let parent_run_id=format!("server-parent-paid-{run_id}");
    let parent_sha="a".repeat(64);
    let child_sha="b".repeat(64);
    let plan_hash="c".repeat(64);
    let command_hash="d".repeat(64);
    let input_hash="e".repeat(64);
    let dispatch_key="f".repeat(64);
    let instance="google_search";
    let child_workflow_id=format!("alegria-owner-operation-v2-{dispatch_key}");
    let child_run_id=format!("server-child-paid-{run_id}");
    let command=b"immutable-dto-command-paid-budget".to_vec();
    let expected=serde_json::json!({
        "schema_revision":1,
        "plan_hash":plan_hash,
        "command_hashes":{"google_search":command_hash},
        "source_payload_hashes":{},
    });
    let limits=serde_json::json!({
        "max_admitted_source_bytes":65536,
        "max_effect_dispatches":1,
        "max_provider_requests":1,
        "currency_code":"USD",
        "max_charge_minor_units":2,
    });
    let maxima=serde_json::json!({
        "max_admitted_source_bytes":524288,
        "max_effect_dispatches":1,
        "max_provider_requests":2,
        "currency_code":"USD",
        "max_charge_minor_units":10,
    });
    sqlx::query("INSERT INTO kb.visa_contexts(context_key,country_code,visa_family,citizenship_code) VALUES ($1,'ZZ','tourist','BY')")
        .bind(&context).execute(&pg.pool).await.unwrap();
    sqlx::query("INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id) VALUES ($1,$2)")
        .bind(&context).bind(&tenant).execute(&pg.pool).await.unwrap();

    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT INSERT ON pipeline.operation_dag_business_runs_v1 TO {business};
         GRANT INSERT ON pipeline.operation_dag_run_governance_v2 TO {governance};
         GRANT INSERT ON pipeline.operation_dag_one_step_request_approvals_v1 TO {approver};
         GRANT SELECT ON pipeline.operation_dag_one_step_request_approvals_v1,
             pipeline.operation_dag_one_step_principal_policies_v1 TO {grant_issuer};
         GRANT INSERT,SELECT ON pipeline.operation_dag_one_step_request_grants_v1 TO {grant_issuer};
         GRANT EXECUTE ON FUNCTION pipeline.one_step_limits_valid_v1(jsonb),
             pipeline.one_step_expected_valid_v1(jsonb,text) TO {approver},{grant_issuer};
         GRANT EXECUTE ON FUNCTION pipeline.reserve_operation_dag_one_step_request_v1(uuid,uuid,text,text,text,text,text,uuid),
             pipeline.mark_operation_dag_one_step_start_unknown_v1(uuid,uuid),
             pipeline.confirm_operation_dag_one_step_start_v1(uuid,uuid,text,text) TO {starter};
         GRANT INSERT ON pipeline.dataforseo_paid_owner_admissions_v2 TO {paid_issuer};
         GRANT EXECUTE ON FUNCTION pipeline.lookup_dataforseo_paid_owner_admission_v2(text,uuid,text,text,text,text,text,text,text,text,text,bigint),
             pipeline.begin_dataforseo_paid_owner_v2(uuid,bytea,text,bigint),
             pipeline.record_dataforseo_paid_task_v2(uuid,bytea,text,text,bytea,bigint),
             pipeline.read_dataforseo_paid_task_v2(uuid,bytea,text),
             pipeline.confirm_dataforseo_paid_owner_v2(uuid,bytea,text,text,bytea,bytea,bytea,bigint),
             pipeline.read_confirmed_dataforseo_paid_owner_receipt_v2(uuid,bytea,text) TO {worker};
         GRANT EXECUTE ON FUNCTION pipeline.lookup_dataforseo_paid_owner_admission_v2(text,uuid,text,text,text,text,text,text,text,text,text,bigint) TO {foreign};"
    )))
    .execute(&pg.pool).await.expect("explicit DBA capability grants");

    // The migration itself never grants a paid effect; even a tenant-bound
    // worker has no admission and no table write capability.
    let begin=|| sqlx::query_scalar::<_,String>(
        "SELECT pipeline.begin_dataforseo_paid_owner_v2($1,$2,$3,$4)"
    ).bind(admission_id).bind(&command).bind(&dispatch_key).bind(20000_i64);
    assert!(begin().fetch_one(&worker_pg).await.is_err());
    for table in [
        "pipeline.dataforseo_paid_owner_admissions_v2",
        "pipeline.dataforseo_paid_owner_attempts_v2",
        "pipeline.dataforseo_paid_owner_posted_tasks_v2",
        "pipeline.dataforseo_paid_owner_receipts_v2",
    ] {
        let can_write:bool=sqlx::query_scalar("SELECT has_table_privilege(session_user,$1,'INSERT')")
            .bind(table).fetch_one(&worker_pg).await.unwrap();
        assert!(!can_write,"worker cannot directly write {table}");
        let forced:bool=sqlx::query_scalar(
            "SELECT relforcerowsecurity FROM pg_class WHERE oid=$1::regclass"
        ).bind(table).fetch_one(&pg.pool).await.unwrap();
        assert!(forced,"paid owner {table} enforces FORCE RLS");
    }
    assert!(sqlx::query(
        "INSERT INTO pipeline.dataforseo_paid_owner_admissions_v2
         (admission_id,grant_request_id,run_id,tenant_id,context_key,
          operation_instance,plan_hash,dispatch_execution_key,
          parent_workflow_id,parent_workflow_run_id,child_workflow_id,
          child_workflow_run_id,child_started_event_id,parent_sha256,child_sha256,
          command_sha256,command_hash,input_hash,command_bytes,
          max_cost_usd_micros,expires_at)
         VALUES ($1,$2,$3,$4,$5,'google_search',$6,$7,$8,$9,$10,$11,
             17,$12,$13,encode(public.digest($14,'sha256'),'hex'),$15,$16,$14,
             20000,clock_timestamp()+interval '4 minutes')"
    ).bind(admission_id).bind(request_id).bind(run_id).bind(&tenant)
     .bind(&context).bind(&plan_hash).bind(&dispatch_key).bind(&parent_workflow_id)
     .bind(&parent_run_id).bind(&child_workflow_id).bind(&child_run_id)
     .bind(&parent_sha).bind(&child_sha).bind(&command).bind(&command_hash)
     .bind(&input_hash).execute(&issuer_pg).await.is_err(),
     "an issuer cannot fabricate paid authority before the exact user grant");

    sqlx::query(
        "INSERT INTO pipeline.operation_dag_business_runs_v1
         (run_id,tenant_id,context_key,plan_hash,parent_workflow_id,expires_at)
         VALUES ($1,$2,$3,$4,$5,clock_timestamp()+interval '15 minutes')"
    ).bind(run_id).bind(&tenant).bind(&context).bind(&plan_hash)
     .bind(&parent_workflow_id).execute(&business_pg).await.unwrap();
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_one_step_principal_policies_v1
         (role_name,context_key,tenant_id,principal_id,policy_id,max_limits)
         VALUES ($1::name,$2,$3,'user-1','policy-paid',$4::jsonb)"
    ).bind(&approver).bind(&context).bind(&tenant).bind(maxima.to_string())
     .execute(&pg.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_one_step_start_principals_v1
         (role_name,context_key,tenant_id,principal_id,policy_id,max_limits)
         VALUES ($1::name,$2,$3,'user-1','policy-paid',$4::jsonb)"
    ).bind(&starter).bind(&context).bind(&tenant).bind(maxima.to_string())
     .execute(&pg.pool).await.unwrap();
    let auth_sql=
        "INSERT INTO pipeline.operation_dag_one_step_request_approvals_v1
           (authorization_event_id,request_id,run_id,tenant_id,principal_id,policy_id,
            context_key,operation_id,operation_revision,parent_workflow_id,
            plan_hash,expected_start,limits,expires_at)
         VALUES ($1,$2,$3,$4,'user-1','policy-paid',$5,
             'dataforseo.google_organic.search',1,$6,$7,$8::jsonb,$9::jsonb,
             clock_timestamp()+interval '5 minutes')";
    sqlx::query(auth_sql)
        .bind(auth_id).bind(request_id).bind(run_id).bind(&tenant)
        .bind(&context).bind(&parent_workflow_id).bind(&plan_hash)
        .bind(expected.to_string()).bind(limits.to_string())
        .execute(&approver_pg).await.unwrap();
    let grant_sql=
        "INSERT INTO pipeline.operation_dag_one_step_request_grants_v1
           (authorization_event_id,request_id,run_id,tenant_id,principal_id,policy_id,
            context_key,operation_id,operation_revision,parent_workflow_id,
            plan_hash,expected_start,limits,expires_at)
         VALUES ($1,$2,$3,$4,'user-1','policy-paid',$5,
             'dataforseo.google_organic.search',1,$6,$7,$8::jsonb,$9::jsonb,
             clock_timestamp()+interval '4 minutes')";
    sqlx::query(grant_sql)
        .bind(auth_id).bind(request_id).bind(run_id).bind(&tenant)
        .bind(&context).bind(&parent_workflow_id).bind(&plan_hash)
        .bind(expected.to_string()).bind(limits.to_string())
        .execute(&grant_pg).await.unwrap();
    let nonce=Uuid::new_v4();
    let reserved:bool=sqlx::query_scalar(
        "SELECT pipeline.reserve_operation_dag_one_step_request_v1($1,$2,$3,$4,$5,$6,$7,$8)"
    ).bind(request_id).bind(run_id).bind(&tenant).bind("user-1")
     .bind("policy-paid").bind(&context).bind(&plan_hash).bind(nonce)
     .fetch_one(&start_pg).await.unwrap();
    assert!(reserved);
    let unknown:bool=sqlx::query_scalar(
        "SELECT pipeline.mark_operation_dag_one_step_start_unknown_v1($1,$2)"
    ).bind(request_id).bind(nonce).fetch_one(&start_pg).await.unwrap();
    assert!(unknown);

    // SQL fixture for the independently verified SERVER parent History. The
    // test only exercises SQL grant reconciliation, not actual Temporal IO.
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_run_governance_v2
           (parent_workflow_run_id,canonical_run_id,tenant_id,context_key,
            parent_workflow_id,parent_workflow_type,parent_task_queue,
            plan_hash,parent_start_sha256,expires_at)
         VALUES ($1,$2,$3,$4,$5,'AlegriaOperationDagV2Workflow',
             'alegria-operation-dag-v2',$6,$7,
             clock_timestamp()+interval '10 minutes')"
    ).bind(&parent_run_id).bind(run_id).bind(&tenant).bind(&context)
     .bind(&parent_workflow_id).bind(&plan_hash).bind(&parent_sha)
     .execute(&governance_pg).await.unwrap();
    let confirmed:bool=sqlx::query_scalar(
        "SELECT pipeline.confirm_operation_dag_one_step_start_v1($1,$2,$3,$4)"
    ).bind(request_id).bind(nonce).bind(&parent_run_id).bind(&parent_sha)
     .fetch_one(&start_pg).await.unwrap();
    assert!(confirmed,"only History-bound one-use Start may authorize paid issuer");

    let insert_paid=
        "INSERT INTO pipeline.dataforseo_paid_owner_admissions_v2
           (admission_id,grant_request_id,run_id,tenant_id,context_key,
            operation_instance,plan_hash,dispatch_execution_key,
            parent_workflow_id,parent_workflow_run_id,child_workflow_id,
            child_workflow_run_id,child_started_event_id,parent_sha256,child_sha256,
            command_sha256,command_hash,input_hash,command_bytes,
            max_cost_usd_micros,expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,
             17,$13,$14,encode(public.digest($15,'sha256'),'hex'),$16,$17,$15,
             $18,clock_timestamp()+interval '3 minutes')";
    assert!(sqlx::query(insert_paid)
        .bind(admission_id).bind(request_id).bind(run_id).bind(&tenant)
        .bind(&context).bind(instance).bind(&plan_hash).bind(&dispatch_key)
        .bind(&parent_workflow_id).bind(&parent_run_id).bind(&child_workflow_id)
        .bind(&child_run_id).bind(&parent_sha).bind(&child_sha).bind(&command)
        .bind(&command_hash).bind(&input_hash).bind(30000_i64)
        .execute(&issuer_pg).await.is_err(),
        "one-step two-cent budget rejects a three-cent paid effect");
    sqlx::query(insert_paid)
        .bind(admission_id).bind(request_id).bind(run_id).bind(&tenant)
        .bind(&context).bind(instance).bind(&plan_hash).bind(&dispatch_key)
        .bind(&parent_workflow_id).bind(&parent_run_id).bind(&child_workflow_id)
        .bind(&child_run_id).bind(&parent_sha).bind(&child_sha).bind(&command)
        .bind(&command_hash).bind(&input_hash).bind(20000_i64)
        .execute(&issuer_pg).await.expect("paid issuer requires exact user budget and History");
    let locate=||sqlx::query_scalar::<_,Option<Uuid>>(
        "SELECT pipeline.lookup_dataforseo_paid_owner_admission_v2(
          $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"
    ).bind(&tenant).bind(run_id).bind(&plan_hash).bind(instance)
     .bind(&parent_workflow_id).bind(&parent_run_id).bind(&child_workflow_id)
     .bind(&child_run_id).bind(&dispatch_key).bind(&command_hash)
     .bind(&input_hash).bind(20000_i64);
    assert_eq!(locate().fetch_one(&worker_pg).await.unwrap(),Some(admission_id));
    assert!(locate().fetch_one(&foreign_pg).await.is_err(),
        "even if function is granted foreign login, cross-tenant access is denied");
    // SECURITY INVOKER insert triggers distinguish the table-owning definer
    // function from a direct worker SQL statement. Prove this still blocks
    // forged evidence after an operator ACCIDENTALLY grants table INSERT.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT INSERT ON pipeline.dataforseo_paid_owner_attempts_v2,
             pipeline.dataforseo_paid_owner_posted_tasks_v2 TO {worker};"
    )))
    .execute(&pg.pool).await.unwrap();
    assert!(sqlx::query(
        "INSERT INTO pipeline.dataforseo_paid_owner_attempts_v2(admission_id,tenant_id)
         VALUES ($1,$2)"
    ).bind(admission_id).bind(&tenant).execute(&worker_pg).await.is_err(),
        "accidental direct table INSERT must not bypass owner-only first-claim function");
    let direct_claim_count:i64=sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.dataforseo_paid_owner_attempts_v2"
    ).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(direct_claim_count,0,
        "failed direct worker INSERT did not create an effect attempt");
    assert_eq!(begin().fetch_one(&worker_pg).await.unwrap(),"started");
    assert_eq!(begin().fetch_one(&worker_pg).await.unwrap(),"uncertain",
        "an interrupted paid POST never permits a second POST");
    assert!(sqlx::query(
        "INSERT INTO pipeline.dataforseo_paid_owner_posted_tasks_v2
         (admission_id,tenant_id,provider_task_id,provider_post_response_bytes,
          provider_post_sha256,post_reported_cost_usd_micros)
         VALUES($1,$2,'forged_paid_post',$3,encode(public.digest($3,'sha256'),'hex'),0)"
    ).bind(admission_id).bind(&tenant).bind(b"forged-provider-response".as_slice())
     .execute(&worker_pg).await.is_err(),
     "accidental direct table INSERT cannot fabricate a paid provider task");
    let post_raw=b"immutable-provider-http-response".to_vec();
    let posted:bool=sqlx::query_scalar(
        "SELECT pipeline.record_dataforseo_paid_task_v2($1,$2,$3,$4,$5,$6)"
    ).bind(admission_id).bind(&command).bind(&dispatch_key)
     .bind("paid_task_2").bind(&post_raw).bind(10000_i64)
     .fetch_one(&worker_pg).await.unwrap();
    assert!(posted);
    assert_eq!(begin().fetch_one(&worker_pg).await.unwrap(),"posted",
        "recorded provider task permits GET only");
    let task:String=sqlx::query_scalar(
        "SELECT pipeline.read_dataforseo_paid_task_v2($1,$2,$3)"
    ).bind(admission_id).bind(&command).bind(&dispatch_key)
     .fetch_one(&worker_pg).await.unwrap();
    assert_eq!(task,"paid_task_2");
    assert!(sqlx::query_scalar::<_,bool>(
        "SELECT pipeline.record_dataforseo_paid_task_v2($1,$2,$3,$4,$5,$6)"
    ).bind(admission_id).bind(&command).bind(&dispatch_key)
     .bind("forged_other_task").bind(&post_raw).bind(10000_i64)
     .fetch_one(&worker_pg).await.is_err(),
     "posted provider task ID is immutable");
    let result_raw=b"real-provider-raw-bytes-fixture".to_vec();
    let typed=b"typed-provider-output-fixture".to_vec();
    let receipt=b"owner-receipt-fixture".to_vec();
    let confirm=|cost:i64|sqlx::query_scalar::<_,bool>(
        "SELECT pipeline.confirm_dataforseo_paid_owner_v2($1,$2,$3,$4,$5,$6,$7,$8)"
    ).bind(admission_id).bind(&command).bind(&dispatch_key).bind("paid_task_2")
     .bind(&result_raw).bind(&typed).bind(&receipt).bind(cost);
    assert!(confirm(30000).fetch_one(&worker_pg).await.is_err(),
        "provider billing over admitted budget cannot be called confirmed");
    assert!(confirm(15000).fetch_one(&worker_pg).await.unwrap());
    assert!(confirm(15000).fetch_one(&worker_pg).await.unwrap(),
        "identical immutable owner receipt is idempotent");
    assert!(confirm(16000).fetch_one(&worker_pg).await.is_err(),
        "confirmed charge cannot be silently changed");
    assert_eq!(begin().fetch_one(&worker_pg).await.unwrap(),"confirmed");
    let saved:Vec<u8>=sqlx::query_scalar(
        "SELECT pipeline.read_confirmed_dataforseo_paid_owner_receipt_v2($1,$2,$3)"
    ).bind(admission_id).bind(&command).bind(&dispatch_key)
     .fetch_one(&worker_pg).await.unwrap();
    assert_eq!(saved,receipt);
    assert!(sqlx::query("TRUNCATE pipeline.dataforseo_paid_owner_attempts_v2")
        .execute(&pg.pool).await.is_err(),
        "append-only claim ledger cannot be truncated even by DBA");
    assert!(sqlx::query("UPDATE pipeline.dataforseo_paid_owner_receipts_v2 SET actual_cost_usd_micros=0")
        .execute(&pg.pool).await.is_err(),
        "immutable raw provider receipt cannot be rewritten");
}
