// True disposable PostgreSQL proof of independently authenticated, one-use
// siteArchitect Start grants. The grant issuer, policy approver and Start
// operator are distinct tenant-bound login roles; no Temporal server, fake
// provider billing or user-supplied principal is treated as authority.

#[tokio::test]
#[serial]
async fn one_step_grant_pg_roles_and_atomic_one_use_lifecycle() {
    let pg = PostgresHarness::start().await.expect("disposable PostgreSQL");
    for (label, migration) in [
        ("Qdrant mixed owner", include_str!("../../../../../db/migrations/20261125_0017_qdrant_mixed_owner_v2.sql")),
        ("PG generic owner", include_str!("../../../../../db/migrations/20261125_0018_pg_generic_owner_v1.sql")),
        ("Qdrant generic owner", include_str!("../../../../../db/migrations/20261125_0019_qdrant_generic_owner_v3.sql")),
        ("DAG V1 governance", include_str!("../../../../../db/migrations/20261125_0020_operation_dag_run_governance_v1.sql")),
        ("PG vector projection", include_str!("../../../../../db/migrations/20261125_0021_pg_vector_projection_v1.sql")),
        ("independent V2 business root", include_str!("../../../../../db/migrations/20261125_0022_pg_owner_v2_admission.sql")),
        ("trusted raw source", include_str!("../../../../../db/migrations/20261127_0002_operation_dag_external_source_attestation_v1.sql")),
        ("one-use user Start grant", include_str!("../../../../../db/migrations/20261127_0003_operation_dag_one_step_request_grants_v1.sql")),
    ] {
        sqlx::raw_sql(migration).execute(&pg.pool).await
            .unwrap_or_else(|error| panic!("{label} migration failed: {error}"));
    }

    let nonce = Uuid::new_v4().simple().to_string();
    let tenant = format!("one-step-{nonce}");
    let issuer = format!("grant_issuer_{nonce}");
    let approver = format!("grant_approver_{nonce}");
    let operator = format!("grant_operator_{nonce}");
    let business = format!("grant_business_{nonce}");
    let foreign = format!("grant_foreign_{nonce}");
    const PASSWORD: &str = "disposable_one_step_grant_pg_test_password";
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {issuer} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {approver} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {operator} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {business} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {foreign} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {issuer},{approver},{operator},{business},{foreign};
         GRANT USAGE ON SCHEMA pipeline TO {issuer},{approver},{operator},{business},{foreign};
         GRANT INSERT ON pipeline.operation_dag_business_runs_v1 TO {business};"
    )))
    .execute(&pg.pool)
    .await
    .expect("create four physically independent PostgreSQL identities");
    for role in [&issuer, &approver, &operator, &business] {
        sqlx::query(
            "INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id) VALUES ($1::name,$2)",
        )
        .bind(role)
        .bind(&tenant)
        .execute(&pg.pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id) VALUES ($1::name,$2)",
    )
    .bind(&foreign)
    .bind(format!("foreign-{nonce}"))
    .execute(&pg.pool)
    .await
    .unwrap();
    let prefix = "postgres://postgres:postgres_password@";
    assert!(pg.database_url.starts_with(prefix));
    let as_role = |role: &str| {
        pg.database_url
            .replacen(prefix, &format!("postgres://{role}:{PASSWORD}@"), 1)
    };
    let issuer_pool = sqlx::PgPool::connect(&as_role(&issuer)).await.unwrap();
    let approver_pool = sqlx::PgPool::connect(&as_role(&approver)).await.unwrap();
    let operator_pool = sqlx::PgPool::connect(&as_role(&operator)).await.unwrap();
    let business_pool = sqlx::PgPool::connect(&as_role(&business)).await.unwrap();
    let foreign_pool = sqlx::PgPool::connect(&as_role(&foreign)).await.unwrap();

    let table = "pipeline.operation_dag_one_step_request_grants_v1";
    for (label, pool) in [
        ("issuer", &issuer_pool),
        ("approver", &approver_pool),
        ("operator", &operator_pool),
        ("foreign", &foreign_pool),
    ] {
        for privilege in ["INSERT", "UPDATE", "DELETE", "TRUNCATE"] {
            let allowed: bool = sqlx::query_scalar(
                "SELECT has_table_privilege(session_user,$1,$2)",
            )
            .bind(table)
            .bind(privilege)
            .fetch_one(pool)
            .await
            .unwrap();
            assert!(!allowed, "{label} must not get implicit {privilege} grant authority");
        }
    }

    // All later grants must be provisioned deliberately by an independent
    // DBA; the migration itself must neither authorize nor start anything.
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.operation_dag_one_step_request_grants_v1",
    )
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(count, 0);

    let context = format!("one-step-{nonce}");
    let run_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    let authorization_event_id = Uuid::new_v4();
    let parent_id = format!("alegria-operation-dag-v2-{run_id}");
    let plan_hash = "a".repeat(64);
    let expected = serde_json::json!({
        "schema_revision": 1,
        "plan_hash": plan_hash,
        "command_hashes": {"one_step": "b".repeat(64)},
        "source_payload_hashes": {},
    });
    let limits = serde_json::json!({
        "max_admitted_source_bytes": 65536,
        "max_effect_dispatches": 0,
        "max_provider_requests": 0,
        "currency_code": "USD",
        "max_charge_minor_units": 0,
    });
    let maximum_limits = serde_json::json!({
        "max_admitted_source_bytes": 524288,
        "max_effect_dispatches": 1,
        "max_provider_requests": 1000,
        "currency_code": "USD",
        "max_charge_minor_units": 10000,
    });
    sqlx::query(
        "INSERT INTO kb.visa_contexts(context_key,country_code,visa_family,citizenship_code)
         VALUES ($1,'ZZ','tourist','BY')",
    )
    .bind(&context)
    .execute(&pg.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id) VALUES ($1,$2)",
    )
    .bind(&context)
    .bind(&tenant)
    .execute(&pg.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_business_runs_v1
           (run_id,tenant_id,context_key,plan_hash,parent_workflow_id,expires_at)
         VALUES ($1,$2,$3,$4,$5,clock_timestamp()+interval '15 minutes')",
    )
    .bind(run_id)
    .bind(&tenant)
    .bind(&context)
    .bind(&plan_hash)
    .bind(&parent_id)
    .execute(&business_pool)
    .await
    .expect("independent authenticated business root");

    // Only an administrator can bind the actual approving SQL LOGIN to one
    // principal/tenant/context and a finite budget ceiling. This is **not**
    // a public user-controlled ACL table.
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_one_step_principal_policies_v1
           (role_name,context_key,tenant_id,principal_id,policy_id,max_limits)
         VALUES ($1::name,$2,$3,$4,$5,$6::jsonb)",
    )
    .bind(&approver)
    .bind(&context)
    .bind(&tenant)
    .bind("authenticated-ci-principal")
    .bind("ci-bounded-approval-v1")
    .bind(maximum_limits.to_string())
    .execute(&pg.pool)
    .await
    .expect("DBA installs independent approval policy");
    let bind_approved_identity =
        "(authorization_event_id,request_id,run_id,tenant_id,principal_id,policy_id,
           context_key,operation_id,operation_revision,parent_workflow_id,plan_hash,
           expected_start,limits,expires_at)";
    let grant_fields = format!(
        "INSERT INTO pipeline.operation_dag_one_step_request_approvals_v1 {bind_approved_identity}
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12::jsonb,$13::jsonb,
                 clock_timestamp()+interval '5 minutes')"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT INSERT ON pipeline.operation_dag_one_step_request_approvals_v1 TO {approver};
         GRANT SELECT ON pipeline.operation_dag_one_step_request_approvals_v1,
             pipeline.operation_dag_one_step_principal_policies_v1 TO {issuer};
         GRANT INSERT, SELECT ON pipeline.operation_dag_one_step_request_grants_v1 TO {issuer};
         GRANT SELECT ON pipeline.operation_dag_one_step_request_grants_v1 TO {operator},{foreign};
         GRANT EXECUTE ON FUNCTION pipeline.one_step_limits_valid_v1(jsonb)
             TO {approver},{issuer};
         GRANT EXECUTE ON FUNCTION pipeline.one_step_expected_valid_v1(jsonb,text)
             TO {approver},{issuer};
         GRANT EXECUTE ON FUNCTION pipeline.reserve_operation_dag_one_step_request_v1(uuid,uuid,text,text,text,text,text,uuid)
             TO {operator},{foreign};
         GRANT EXECUTE ON FUNCTION pipeline.mark_operation_dag_one_step_start_unknown_v1(uuid,uuid)
             TO {operator},{foreign};
         GRANT EXECUTE ON FUNCTION pipeline.confirm_operation_dag_one_step_start_v1(uuid,uuid,text,text)
             TO {operator},{foreign};"
    )))
    .execute(&pg.pool)
    .await
    .expect("DBA provisions independent approver, issuer and read/claim operator");

    let excessive_limits = serde_json::json!({
        "max_admitted_source_bytes": 65536,
        "max_effect_dispatches": 0,
        "max_provider_requests": 0,
        "currency_code": "USD",
        "max_charge_minor_units": 10001,
    });
    assert!(sqlx::query(&grant_fields)
        .bind(authorization_event_id).bind(request_id).bind(run_id).bind(&tenant)
        .bind("authenticated-ci-principal").bind("ci-bounded-approval-v1")
        .bind(&context).bind("acquisition.classify_source_page").bind(1_i32)
        .bind(&parent_id).bind(&plan_hash)
        .bind(expected.to_string()).bind(excessive_limits.to_string())
        .execute(&approver_pool).await.is_err(),
        "approver cannot overrule DBA-authorized budget ceilings");
    assert!(sqlx::query(&grant_fields)
        .bind(authorization_event_id).bind(request_id).bind(run_id).bind(&tenant)
        .bind("forged-other-principal").bind("ci-bounded-approval-v1")
        .bind(&context).bind("acquisition.classify_source_page").bind(1_i32)
        .bind(&parent_id).bind(&plan_hash)
        .bind(expected.to_string()).bind(limits.to_string())
        .execute(&approver_pool).await.is_err(),
        "approver may not sign a different user than independently DBA-mapped LOGIN");
    assert!(sqlx::query(&grant_fields)
        .bind(authorization_event_id).bind(request_id).bind(run_id).bind(&tenant)
        .bind("authenticated-ci-principal").bind("ci-bounded-approval-v1")
        .bind(&context).bind("acquisition.classify_source_page").bind(1_i32)
        .bind(&parent_id).bind(&plan_hash)
        .bind(expected.to_string()).bind(limits.to_string())
        .execute(&issuer_pool).await.is_err(),
        "grant issuer cannot also sign an authorization event");

    let approval = sqlx::query(&grant_fields)
        .bind(authorization_event_id)
        .bind(request_id)
        .bind(run_id)
        .bind(&tenant)
        .bind("authenticated-ci-principal")
        .bind("ci-bounded-approval-v1")
        .bind(&context)
        .bind("acquisition.classify_source_page")
        .bind(1_i32)
        .bind(&parent_id)
        .bind(&plan_hash)
        .bind(expected.to_string())
        .bind(limits.to_string())
        .execute(&approver_pool)
        .await;
    approval.expect("a separate authenticated approver signs a bounded exact request");

    let grant_insert = format!(
        "INSERT INTO pipeline.operation_dag_one_step_request_grants_v1 {bind_approved_identity}
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12::jsonb,$13::jsonb,
                 clock_timestamp()+interval '4 minutes')"
    );
    assert!(sqlx::query(&grant_insert)
        .bind(authorization_event_id).bind(request_id).bind(run_id)
        .bind(&tenant).bind("authenticated-ci-principal").bind("ci-bounded-approval-v1")
        .bind(&context).bind("acquisition.classify_source_page").bind(1_i32)
        .bind(&parent_id).bind(&plan_hash)
        .bind(expected.to_string()).bind(limits.to_string())
        .execute(&approver_pool).await.is_err(),
        "authorization approver cannot mint executable reservation grants");
    // The issuer cannot invent a tenant/plan/ACL/charge. The original
    // independently signed approval must match every pinned field.
    assert!(sqlx::query(&grant_insert)
        .bind(authorization_event_id).bind(request_id).bind(run_id)
        .bind(&tenant).bind("authenticated-ci-principal").bind("ci-bounded-approval-v1")
        .bind(&context).bind("acquisition.classify_source_page").bind(1_i32)
        .bind(&parent_id).bind("d".repeat(64))
        .bind(expected.to_string()).bind(limits.to_string())
        .execute(&issuer_pool).await.is_err(),
        "an issuer cannot substitute a new plan for the independently approved one");
    sqlx::query(&grant_insert)
        .bind(authorization_event_id).bind(request_id).bind(run_id)
        .bind(&tenant).bind("authenticated-ci-principal").bind("ci-bounded-approval-v1")
        .bind(&context).bind("acquisition.classify_source_page").bind(1_i32)
        .bind(&parent_id).bind(&plan_hash)
        .bind(expected.to_string()).bind(limits.to_string())
        .execute(&issuer_pool).await
        .expect("only an exact approved request can be minted once");

    let reserved_nonce_a = Uuid::new_v4();
    let reserved_nonce_b = Uuid::new_v4();
    let reserve =
        "SELECT pipeline.reserve_operation_dag_one_step_request_v1($1,$2,$3,$4,$5,$6,$7,$8)";
    let unbound_start: bool = sqlx::query_scalar(reserve)
        .bind(request_id).bind(run_id).bind(&tenant)
        .bind("authenticated-ci-principal").bind("ci-bounded-approval-v1")
        .bind(&context).bind(&plan_hash).bind(Uuid::new_v4())
        .fetch_one(&operator_pool).await.unwrap();
    assert!(!unbound_start,
        "even a tenant-bound operator must be refused until DBA maps its LOGIN to this principal and policy");
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_one_step_start_principals_v1
           (role_name,context_key,tenant_id,principal_id,policy_id,max_limits)
         VALUES ($1::name,$2,$3,$4,$5,$6::jsonb)",
    )
    .bind(&operator).bind(&context).bind(&tenant)
    .bind("authenticated-ci-principal").bind("ci-bounded-approval-v1")
    .bind(maximum_limits.to_string())
    .execute(&pg.pool).await
    .expect("DBA independently binds real Start LOGIN to principal, ACL and charge ceiling");
    let forged_principal: bool = sqlx::query_scalar(reserve)
        .bind(request_id).bind(run_id).bind(&tenant)
        .bind("forged-other-user").bind("ci-bounded-approval-v1")
        .bind(&context).bind(&plan_hash).bind(Uuid::new_v4())
        .fetch_one(&operator_pool).await.unwrap();
    assert!(!forged_principal,
        "the right SQL LOGIN may not substitute a forged principal after DBA policy installation");
    let forged_policy_before_reserve: bool = sqlx::query_scalar(reserve)
        .bind(request_id).bind(run_id).bind(&tenant)
        .bind("authenticated-ci-principal").bind("forged-policy")
        .bind(&context).bind(&plan_hash).bind(Uuid::new_v4())
        .fetch_one(&operator_pool).await.unwrap();
    assert!(!forged_policy_before_reserve,
        "the independently pinned SQL LOGIN policy cannot be changed before reservation");
    let (claim_a, claim_b) = tokio::join!(
        sqlx::query_scalar::<_, bool>(reserve)
            .bind(request_id).bind(run_id).bind(&tenant)
            .bind("authenticated-ci-principal").bind("ci-bounded-approval-v1")
            .bind(&context).bind(&plan_hash).bind(reserved_nonce_a)
            .fetch_one(&operator_pool),
        sqlx::query_scalar::<_, bool>(reserve)
            .bind(request_id).bind(run_id).bind(&tenant)
            .bind("authenticated-ci-principal").bind("ci-bounded-approval-v1")
            .bind(&context).bind(&plan_hash).bind(reserved_nonce_b)
            .fetch_one(&operator_pool),
    );
    let claim_a = claim_a.expect("first reserved transaction must return a definitive bool");
    let claim_b = claim_b.expect("racing reserved transaction must return a definitive bool");
    assert_ne!(claim_a, claim_b, "exactly ONE concurrent Start claimant may win");
    let winner = if claim_a { reserved_nonce_a } else { reserved_nonce_b };
    let loser = if claim_a { reserved_nonce_b } else { reserved_nonce_a };

    let foreign_claim: bool = sqlx::query_scalar(reserve)
        .bind(request_id).bind(run_id).bind(&tenant)
        .bind("authenticated-ci-principal").bind("ci-bounded-approval-v1")
        .bind(&context).bind(&plan_hash).bind(Uuid::new_v4())
        .fetch_one(&foreign_pool).await.unwrap();
    assert!(!foreign_claim, "a different tenant cannot claim another tenant's request");
    let forged_policy: bool = sqlx::query_scalar(reserve)
        .bind(request_id).bind(run_id).bind(&tenant)
        .bind("authenticated-ci-principal").bind("forged-policy")
        .bind(&context).bind(&plan_hash).bind(Uuid::new_v4())
        .fetch_one(&operator_pool).await.unwrap();
    assert!(!forged_policy, "Start policy is DBA-bound to SQL LOGIN; caller cannot choose another");
    let duplicate_claim: bool = sqlx::query_scalar(reserve)
        .bind(request_id).bind(run_id).bind(&tenant)
        .bind("authenticated-ci-principal").bind("ci-bounded-approval-v1")
        .bind(&context).bind(&plan_hash).bind(winner)
        .fetch_one(&operator_pool).await.unwrap();
    assert!(!duplicate_claim, "same reservation nonce must not re-open an exclusive claim");

    let mark_unknown: bool = sqlx::query_scalar(
        "SELECT pipeline.mark_operation_dag_one_step_start_unknown_v1($1,$2)",
    )
    .bind(request_id)
    .bind(winner)
    .fetch_one(&operator_pool)
    .await
    .unwrap();
    assert!(mark_unknown, "durably record uncertainty BEFORE external Start RPC");
    let loser_unknown: bool = sqlx::query_scalar(
        "SELECT pipeline.mark_operation_dag_one_step_start_unknown_v1($1,$2)",
    )
    .bind(request_id)
    .bind(loser)
    .fetch_one(&operator_pool)
    .await
    .unwrap();
    assert!(!loser_unknown, "loser cannot mutate winner's durable request state");
    let forbidden_retry: bool = sqlx::query_scalar(reserve)
        .bind(request_id).bind(run_id).bind(&tenant)
        .bind("authenticated-ci-principal").bind("ci-bounded-approval-v1")
        .bind(&context).bind(&plan_hash).bind(Uuid::new_v4())
        .fetch_one(&operator_pool).await.unwrap();
    assert!(!forbidden_retry, "after an unknown outcome, a Start retry is forbidden");

    let unverified_confirmation: bool = sqlx::query_scalar(
        "SELECT pipeline.confirm_operation_dag_one_step_start_v1($1,$2,$3,$4)",
    )
    .bind(request_id)
    .bind(winner)
    .bind("forged-temporal-run-id")
    .bind("d".repeat(64))
    .fetch_one(&operator_pool)
    .await
    .unwrap();
    assert!(!unverified_confirmation,
        "an operator's claimed Temporal run cannot confirm without independent server History governance");

    let persisted_state: String = sqlx::query_scalar(
        "SELECT state FROM pipeline.operation_dag_one_step_request_grants_v1 WHERE request_id=$1",
    )
    .bind(request_id)
    .fetch_one(&pg.pool)
    .await
    .unwrap();
    assert_eq!(persisted_state, "start_outcome_unknown");
    assert!(sqlx::query(
        "UPDATE pipeline.operation_dag_one_step_request_grants_v1
         SET state='unclaimed' WHERE request_id=$1",
    )
    .bind(request_id)
    .execute(&issuer_pool)
    .await
    .is_err(), "issuer cannot rewrite an unknown outcome into a new chance");

    issuer_pool.close().await;
    approver_pool.close().await;
    operator_pool.close().await;
    business_pool.close().await;
    foreign_pool.close().await;
}
