//! Real PostgreSQL authority tests for the opt-in DataForSEO Temporal V2
//! owner. No provider traffic, paid task, or simulated operator attestation.

#[tokio::test]
#[serial]
async fn dataforseo_owner_pg_missing_history_admission_remains_inert() {
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
}
