// The actual native operator CLI, isolated from the V3 Activity poller,
// exercised against a disposable PostgreSQL schema and a distinct LOGIN.
// This is mandatory in the existing PG V3 `--features e2e` test shard, not
// an ignored unit test or a direct admin INSERT masquerading as issuance.

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn pg_v3_native_operator_cli_least_privilege_issue_revoke() {
    let postgres = PostgresHarness::start()
        .await
        .expect("real isolated PG V3 operator database");
    let fixture = pg_v3_seed_review_fixture(&postgres.pool).await;
    let role = format!("ci_pg_v3_operator_{}", Uuid::new_v4().simple());
    const PASSWORD: &str = "pg_v3_disposable_operator_password";
    let provision = format!(
        "CREATE ROLE {role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {role};
         GRANT USAGE ON SCHEMA pipeline, extracted, raw TO {role};
         GRANT SELECT ON pipeline.execution_runs, pipeline.pg_effect_v3_admissions,
                         pipeline.pg_effect_v3_receipts, extracted.rule_candidates,
                         raw.sections, raw.pages TO {role};
         GRANT SELECT, INSERT, UPDATE ON pipeline.pg_effect_v3_admissions TO {role};"
    );
    // Identifier = fixed prefix + UUID hex, in disposable testcontainer only.
    sqlx::raw_sql(sqlx::AssertSqlSafe(provision))
        .execute(&postgres.pool)
        .await
        .expect("provision actual restricted V3 issuer LOGIN");
    let prefix = "postgres://postgres:postgres_password@";
    assert!(postgres.database_url.starts_with(prefix));
    let restricted_url = postgres.database_url.replacen(
        prefix,
        &format!("postgres://{role}:{PASSWORD}@"),
        1,
    );
    let restricted = sqlx::PgPool::connect(&restricted_url)
        .await
        .expect("connect actual V3 operator LOGIN");
    let actual_role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&restricted)
        .await
        .expect("read session's actual PG operator identity");
    assert_eq!(actual_role, role);

    let command_json = serde_json::json!({
        "command": fixture.command,
        "workflow_id": fixture.workflow_id,
        "workflow_run_id": fixture.workflow_run_id,
        "expires_in_seconds": 120
    });
    let temp_file = env::temp_dir().join(format!("ci-pg-v3-operator-{}.json", Uuid::new_v4()));
    let write_request = |payload: &serde_json::Value| {
        fs::write(&temp_file, serde_json::to_vec(payload).expect("encode operator request"))
            .expect("write disposable operator request file");
    };
    let run_operator = |command: &str, argument: &std::ffi::OsStr, tenant: &str, dsn: &str| {
        Command::new(pg_v3_effect_worker_binary())
            .arg(command)
            .arg(argument)
            .current_dir(workspace_root())
            .env("ALEGRIA_ENABLE_PG_EFFECT_OPERATOR_V3", "true")
            .env("PG_EFFECT_OPERATOR_DATABASE_URL", dsn)
            .env("PG_EFFECT_OPERATOR_TENANT_ID", tenant)
            .env_remove("PG_EFFECT_WORKER_DATABASE_URL")
            .env_remove("DATABASE_URL")
            .output()
            .expect("run isolated native PG V3 operator command")
    };
    let assert_denied = |output: std::process::Output, case: &str| {
        assert!(
            !output.status.success(),
            "{case} wrongly permitted: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };

    write_request(&command_json);
    assert_denied(
        run_operator("--operator-issue", temp_file.as_os_str(), "other-tenant", &restricted_url),
        "cross-tenant operator policy spoof",
    );
    assert_denied(
        run_operator("--operator-issue", temp_file.as_os_str(),
            "ci-pg-v3-tenant", &postgres.database_url),
        "owner/admin issuing via privileged session",
    );
    let mut mismatched_workflow = command_json.clone();
    mismatched_workflow["workflow_run_id"] = serde_json::json!(Uuid::new_v4().to_string());
    write_request(&mismatched_workflow);
    assert_denied(
        run_operator("--operator-issue", temp_file.as_os_str(), "ci-pg-v3-tenant", &restricted_url),
        "forged canonical Temporal Workflow Run ID",
    );
    let mut mismatched_run = command_json.clone();
    let unknown_run_id = Uuid::new_v4();
    mismatched_run["command"]["run_id"] = serde_json::json!(unknown_run_id);
    mismatched_run["command"]["scope"]["run_id"] =
        serde_json::json!(unknown_run_id.to_string());
    write_request(&mismatched_run);
    assert_denied(
        run_operator("--operator-issue", temp_file.as_os_str(), "ci-pg-v3-tenant", &restricted_url),
        "forged canonical Truth run",
    );

    write_request(&command_json);
    let issued = run_operator("--operator-issue", temp_file.as_os_str(),
        "ci-pg-v3-tenant", &restricted_url);
    assert!(
        issued.status.success(),
        "dedicated operator issue denied: {}",
        String::from_utf8_lossy(&issued.stderr)
    );
    let issued_json: serde_json::Value =
        serde_json::from_slice(&issued.stdout).expect("decode operator issue receipt");
    assert_eq!(issued_json["status"], "issued");
    assert_eq!(issued_json["operator_role"], role);
    assert_eq!(issued_json["admission_id"], fixture.command.admission_id.to_string());
    let issuer: String = sqlx::query_scalar(
        "SELECT issued_by FROM pipeline.pg_effect_v3_admissions WHERE admission_id = $1",
    )
    .bind(fixture.command.admission_id)
    .fetch_one(&restricted)
    .await
    .expect("issuer identity recorded in authoritative admission");
    assert_eq!(issuer, role);

    let refused_run_mutation = sqlx::query(
        "UPDATE pipeline.execution_runs SET status = 'done' WHERE run_id = $1",
    )
    .bind(fixture.command.run_id)
    .execute(&restricted)
    .await
    .expect_err("operator must not change canonical run");
    assert!(matches!(refused_run_mutation, sqlx::Error::Database(ref db)
        if db.code().as_deref() == Some("42501")));
    let refused_receipt = sqlx::query(
        "INSERT INTO pipeline.pg_effect_v3_receipts(admission_id) VALUES ($1)",
    )
    .bind(fixture.command.admission_id)
    .execute(&restricted)
    .await
    .expect_err("operator must not forge effect receipt");
    assert!(matches!(refused_receipt, sqlx::Error::Database(ref db)
        if db.code().as_deref() == Some("42501")));

    let revocation_id = fixture.command.admission_id.to_string();
    assert_denied(
        run_operator("--operator-revoke", std::ffi::OsStr::new(&revocation_id),
            "other-tenant", &restricted_url),
        "cross-tenant revocation",
    );
    let revoke = run_operator("--operator-revoke", std::ffi::OsStr::new(&revocation_id),
        "ci-pg-v3-tenant", &restricted_url);
    assert!(revoke.status.success(), "valid revocation denied: {}",
        String::from_utf8_lossy(&revoke.stderr));
    let revoke_json: serde_json::Value =
        serde_json::from_slice(&revoke.stdout).expect("decode revocation result");
    assert_eq!(revoke_json["status"], "revoked");
    let again = run_operator("--operator-revoke", std::ffi::OsStr::new(&revocation_id),
        "ci-pg-v3-tenant", &restricted_url);
    assert!(again.status.success());
    let again_json: serde_json::Value =
        serde_json::from_slice(&again.stdout).expect("decode idempotent revocation result");
    assert_eq!(again_json["status"], "already_revoked");

    // A second admitted operation has an already durable receipt. The admin
    // fixture alone injects that synthetic receipt; the restricted operator
    // is forbidden to INSERT receipts. Revocation must report failure and
    // must NOT claim or persist a successful rollback of a committed effect.
    let mut committed_command = fixture.command.clone();
    committed_command.admission_id = Uuid::new_v4();
    committed_command.operation_instance = "receipt-committed-before-revoke".into();
    let mut committed_json = command_json.clone();
    committed_json["command"] =
        serde_json::to_value(&committed_command).expect("encode committed typed fixture");
    write_request(&committed_json);
    let admitted = run_operator("--operator-issue", temp_file.as_os_str(),
        "ci-pg-v3-tenant", &restricted_url);
    assert!(admitted.status.success(), "second operator issuance failed: {}",
        String::from_utf8_lossy(&admitted.stderr));
    let candidate_hash: String = sqlx::query_scalar(
        "SELECT candidate_set_hash FROM pipeline.pg_effect_v3_admissions
         WHERE admission_id = $1",
    )
    .bind(committed_command.admission_id)
    .fetch_one(&postgres.pool)
    .await
    .expect("read admitted candidate fingerprint");
    let key = postgres_store::truth_review_effect_v3::execution_key(
        &fixture.workflow_id, &fixture.workflow_run_id, &committed_command,
    )
    .expect("derive committed fixture's canonical execution key");
    sqlx::query(
        "INSERT INTO pipeline.pg_effect_v3_receipts
         (admission_id, execution_key, operation_id, operation_revision, run_id,
          context_key, candidate_set_hash, created_review_tasks, open_review_tasks)
         VALUES ($1,$2,'pg.truth.reconcile_candidate_reviews',1,$3,$4,$5,0,0)",
    )
    .bind(committed_command.admission_id)
    .bind(key)
    .bind(committed_command.run_id)
    .bind(&committed_command.context_key)
    .bind(candidate_hash)
    .execute(&postgres.pool)
    .await
    .expect("admin fixture records committed receipt");
    let committed_id = committed_command.admission_id.to_string();
    assert_denied(
        run_operator("--operator-revoke", std::ffi::OsStr::new(&committed_id),
            "ci-pg-v3-tenant", &restricted_url),
        "revoking an already committed PostgreSQL effect",
    );
    let not_revoked: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NULL FROM pipeline.pg_effect_v3_admissions WHERE admission_id = $1",
    )
    .bind(committed_command.admission_id)
    .fetch_one(&postgres.pool)
    .await
    .expect("check failed revocation preserved authority row");
    assert!(not_revoked, "rejected post-commit revocation must not mutate admission");
    fs::remove_file(&temp_file).expect("remove exact disposable operator request file");
    eprintln!("CI_PG_V3_NATIVE_OPERATOR_AUTH=PASS");
}
