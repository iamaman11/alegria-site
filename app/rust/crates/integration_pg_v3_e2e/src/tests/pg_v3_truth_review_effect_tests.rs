// A focused real-PostgreSQL acceptance test for the independent, admitted V3
// Truth-review effect owner. There is no Temporal server, Graph, Vector index,
// LLM or legacy application Worker here. Admission belongs to a privileged
// operator; the Activity uses a DISTINCT role that cannot issue authority.

#[cfg(feature = "e2e")]
struct PgV3ReviewFixture {
    command: postgres_store::truth_review_effect_v3::ReconcileTruthReviewsInput,
    workflow_id: String,
    workflow_run_id: String,
    candidate_id: String,
}

#[cfg(feature = "e2e")]
async fn pg_v3_seed_review_fixture(pool: &sqlx::PgPool) -> PgV3ReviewFixture {
    use operation_protocol::RunScope;
    use postgres_store::truth_review_effect_v3::ReconcileTruthReviewsInput;

    let marker = Uuid::new_v4().simple().to_string();
    let context_key = format!("pg-v3:truth-review:{marker}");
    let source_key = format!("https://ci-{marker}.example.gov/fees");
    let candidate_id = format!("pg_v3_candidate_{marker}");
    let run_id = Uuid::new_v4();
    let workflow_id = format!("pg-v3-truth-review-{marker}");
    // This is a distinct server-issued Temporal Workflow Run ID, not the
    // application `pipeline.execution_runs.run_id` or Workflow ID.
    let workflow_run_id = Uuid::new_v4().to_string();
    let raw_text = "Consular fee is 35 EUR";

    sqlx::query(
        "INSERT INTO kb.visa_contexts
         (context_key, country_code, visa_family, citizenship_code)
         VALUES ($1, $2, 'tourist', 'BY')",
    )
    .bind(&context_key)
    .bind(format!("X{}", &marker[..8]))
    .execute(pool)
    .await
    .expect("seed exact scoped Truth context");
    // Explicit disposable DBA provisioning: no tenant is inferred from a
    // caller-supplied Temporal scope or from legacy NULL source rows.
    sqlx::query(
        "INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id)
         VALUES ($1,'ci-pg-v3-tenant')",
    )
    .bind(&context_key)
    .execute(pool)
    .await
    .expect("provision authoritative fixture tenant context");
    sqlx::query(
        "INSERT INTO kb.sources
         (source_key, source_type, source_label, trust_level)
         VALUES ($1, 'government', 'CI government source', 5)",
    )
    .bind(&source_key)
    .execute(pool)
    .await
    .expect("seed Truth evidence source");
    let page_id: i64 = sqlx::query_scalar(
        "INSERT INTO raw.pages (url, domain, dtype, tenant_id)
         VALUES ($1, $2, 'government', 'ci-pg-v3-tenant') RETURNING id",
    )
    .bind(&source_key)
    .bind(format!("ci-{marker}.example.gov"))
    .fetch_one(pool)
    .await
    .expect("seed immutable source page");
    let section_id: i64 = sqlx::query_scalar(
        "INSERT INTO raw.sections (page_id, content_md, content_hash, tenant_id)
         VALUES ($1, $2, 'pg-v3-immutable-source', 'ci-pg-v3-tenant') RETURNING id",
    )
    .bind(page_id)
    .bind(raw_text)
    .fetch_one(pool)
    .await
    .expect("seed evidence section");
    sqlx::query(
        "INSERT INTO extracted.rule_candidates
         (rule_candidate_id, context_key, raw_section_id, role, concept_canonical_key,
          raw_mention, confidence, evidence_section_id, evidence_quote,
          span_start, span_end, source_key, source_snapshot_hash,
          llm_provider, llm_model, prompt_version, epistemic_status, tenant_id)
         VALUES ($1, $2, $3, 'FEE_ITEM', 'consular_fee', '35 EUR', 0.98,
                 $3, $4, 0, $5, $6, 'pg-v3-immutable-source',
                 'chat_manual', 'gpt-6', 'ci_truth@1', 'needs_hitl', 'ci-pg-v3-tenant')",
    )
    .bind(&candidate_id)
    .bind(&context_key)
    .bind(section_id)
    .bind(raw_text)
    .bind(raw_text.len() as i32)
    .bind(&source_key)
    .execute(pool)
    .await
    .expect("seed actual unresolved Truth candidate");
    sqlx::query(
        "INSERT INTO pipeline.execution_runs
         (run_id, workflow_run_id, workflow_type, context_key, status, tenant_id)
         VALUES ($1, $2, 'seo_site_build', $3, 'verifying', 'ci-pg-v3-tenant')",
    )
    .bind(run_id)
    .bind(&workflow_run_id)
    .bind(&context_key)
    .execute(pool)
    .await
    .expect("seed canonical verifying Truth run");
    PgV3ReviewFixture {
        command: ReconcileTruthReviewsInput {
            admission_id: Uuid::new_v4(),
            run_id,
            scope: RunScope {
                tenant_id: "ci-pg-v3-tenant".into(),
                run_id: run_id.to_string(),
            },
            context_key,
            operation_instance: "review-reconcile".into(),
            plan_hash: "a".repeat(64),
        },
        workflow_id,
        workflow_run_id,
        candidate_id,
    }
}

#[cfg(feature = "e2e")]
async fn pg_v3_issue_operator_admission(pool: &sqlx::PgPool, fixture: &PgV3ReviewFixture) {
    use postgres_store::truth_review_effect_v3::{
        contract_fingerprint, execution_key, CANDIDATE_SET_HASH_SQL,
    };

    // Only the privileged operator connection sees this INSERT. The hash is
    // recomputed from live PostgreSQL candidate rows, never accepted from a
    // forged `ExecutionPlan` or the Activity command.
    let observed_hash: String = sqlx::query_scalar(CANDIDATE_SET_HASH_SQL)
        .bind(&fixture.command.context_key)
        .fetch_one(pool)
        .await
        .expect("hash canonical candidate snapshot in PostgreSQL");
    sqlx::query(
        "INSERT INTO pipeline.pg_effect_v3_admissions
         (admission_id, run_id, tenant_id, context_key, workflow_id,
          workflow_execution_run_id, operation_instance, plan_hash,
          contract_fingerprint, candidate_set_hash, v3_expected_execution_key,
          issued_by, expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,
                 'ci_authorized_operator',now() + interval '1 hour')",
    )
    .bind(fixture.command.admission_id)
    .bind(fixture.command.run_id)
    .bind(&fixture.command.scope.tenant_id)
    .bind(&fixture.command.context_key)
    .bind(&fixture.workflow_id)
    .bind(&fixture.workflow_run_id)
    .bind(&fixture.command.operation_instance)
    .bind(&fixture.command.plan_hash)
    .bind(contract_fingerprint().expect("pin typed owner contract"))
    .bind(observed_hash)
    .bind(execution_key(&fixture.workflow_id, &fixture.workflow_run_id, &fixture.command)
        .expect("pin canonical admitted V3 receipt key"))
    .execute(pool)
    .await
    .expect("operator issues independently authorized PG effect admission");
}

#[cfg(feature = "e2e")]
async fn pg_v3_issue_historical_unpinned_admission(
    pool: &sqlx::PgPool,
    fixture: &PgV3ReviewFixture,
) {
    use postgres_store::truth_review_effect_v3::contract_fingerprint;
    sqlx::query(
        "INSERT INTO pipeline.pg_effect_v3_admissions
         (admission_id, run_id, tenant_id, context_key, workflow_id,
          workflow_execution_run_id, operation_instance, plan_hash,
          contract_fingerprint, candidate_set_hash, issued_by, expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,
                 pipeline.pg_truth_candidate_set_hash_v1($4),
                 'historical_v3_operator',now() + interval '1 hour')",
    )
    .bind(fixture.command.admission_id)
    .bind(fixture.command.run_id)
    .bind(&fixture.command.scope.tenant_id)
    .bind(&fixture.command.context_key)
    .bind(&fixture.workflow_id)
    .bind(&fixture.workflow_run_id)
    .bind(&fixture.command.operation_instance)
    .bind(&fixture.command.plan_hash)
    .bind(contract_fingerprint().expect("original V3 contract hash"))
    .execute(pool)
    .await
    .expect("seed historical V3 admission with NULL key before upgrade");
}

#[cfg(feature = "e2e")]
async fn pg_v3_unprivileged_effect_identity(postgres: &PostgresHarness) -> (sqlx::PgPool, String) {
    use postgres_store::truth_review_effect_v3::assert_unprivileged_pg_effect_role;

    // Never use the admin/owner connection to execute an Activity: doing so
    // would accidentally let the Activity forge or revoke its own admission.
    assert!(
        assert_unprivileged_pg_effect_role(&postgres.pool).await.is_err(),
        "admin/table-owner role must be rejected at V3 Activity startup"
    );
    let role = format!("ci_pg_v3_worker_{}", Uuid::new_v4().simple());
    const PASSWORD: &str = "pg_v3_disposable_role_password";
    let grant_sql = format!(
        "CREATE ROLE {role} LOGIN PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {role};
         GRANT USAGE ON SCHEMA pipeline, site, extracted, raw, verified, system TO {role};
         GRANT SELECT ON pipeline.execution_runs, pipeline.step_executions,
                         pipeline.pg_effect_v3_admissions TO {role};
         GRANT SELECT ON pipeline.pg_effect_v3_receipts TO {role};
         GRANT SELECT ON extracted.rule_candidates, raw.sections, raw.pages TO {role};
         GRANT SELECT ON site.seo_hitl_tasks TO {role};
         GRANT EXECUTE ON FUNCTION pipeline.consume_pg_truth_review_v1(
             UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,SMALLINT,
             TEXT,TEXT,TEXT,BYTEA,BYTEA,BYTEA
         ) TO {role};
         INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id)
         VALUES ('{role}', 'ci-pg-v3-tenant');"
    );
    // Identifiers are composed solely of a fixed prefix and generated UUID
    // hex, and this PostgreSQL lives inside this test's disposable container.
    // `CREATE ROLE` identifiers cannot be bind parameters. The only dynamic
    // identifier is a fixed prefix plus server-generated UUID hex; the
    // password is a local constant and the database is disposable. The SQLx
    // 0.9 safety assertion is intentionally confined to this test fixture.
    sqlx::raw_sql(sqlx::AssertSqlSafe(grant_sql))
        .execute(&postgres.pool)
        .await
        .expect("provision least-privilege disposable PG Activity role");
    let expected_prefix = "postgres://postgres:postgres_password@";
    assert!(
        postgres.database_url.starts_with(expected_prefix),
        "refuse to rewrite a non-disposable PostgreSQL DSN"
    );
    let restricted_url = postgres.database_url.replacen(
        expected_prefix,
        &format!("postgres://{role}:{PASSWORD}@"),
        1,
    );
    let pool = sqlx::PgPool::connect(&restricted_url)
        .await
        .expect("connect as least-privileged PG Activity identity");
    let actual_role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("verify Activity's effective PostgreSQL identity");
    assert_eq!(actual_role, role);
    assert_unprivileged_pg_effect_role(&pool)
        .await
        .expect("least-privilege Activity role passes fail-closed startup guard");
    (pool, restricted_url)
}

#[cfg(feature = "e2e")]
async fn pg_v3_unprivileged_effect_pool(postgres: &PostgresHarness) -> sqlx::PgPool {
    pg_v3_unprivileged_effect_identity(postgres).await.0
}

/// A schema migration or an inherited grant may add a writer permission to a
/// table that was not on a hand-maintained protected-table list. The activity
/// role must fail startup before any Temporal command can consume admission.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn pg_v3_rejects_new_app_writer_and_schema_ddl_privileges() {
    use postgres_store::truth_review_effect_v3::{
        assert_unprivileged_pg_effect_role, ReconcileTruthReviewsError,
    };

    let postgres = PostgresHarness::start()
        .await
        .expect("disposable PostgreSQL for effective-permission boundary");
    let effect_pool = pg_v3_unprivileged_effect_pool(&postgres).await;
    let worker_role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&effect_pool)
        .await
        .expect("worker role name");
    assert!(worker_role.starts_with("ci_pg_v3_worker_"));

    // A privileged LOGIN could impersonate this exact restricted role via
    // SET ROLE, pass checks against current_user, then RESET ROLE to regain
    // superuser access on the same physical connection. This must be denied
    // even though its effective table grants look identical to the worker.
    let impersonating_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&postgres.database_url)
        .await
        .expect("privileged LOGIN identity for role-impersonation regression");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("SET ROLE {worker_role}")))
        .execute(&impersonating_pool)
        .await
        .expect("privileged LOGIN impersonates restricted effective role");
    let (impersonated_current, privileged_session): (String, String) =
        sqlx::query_as("SELECT current_user::text, session_user::text")
            .fetch_one(&impersonating_pool)
            .await
            .expect("observe both real PG backend role identities");
    assert_eq!(impersonated_current, worker_role);
    assert_ne!(privileged_session, impersonated_current);
    assert!(matches!(
        assert_unprivileged_pg_effect_role(&impersonating_pool).await,
        Err(ReconcileTruthReviewsError::PrivilegedWorkerRole { .. })
    ));
    impersonating_pool.close().await;
    eprintln!("CI_PG_V3_REJECT_SET_ROLE_IMPERSONATION=PASS");

    // The owner operation is also callable directly by Rust consumers. Its
    // own runtime check must reject admin credentials, even when a valid
    // operator admission exists (bypassing startup is not a privilege grant).
    let privileged_call = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &privileged_call).await;
    assert!(matches!(
        postgres_store::truth_review_effect_v3::execute_admitted_reconcile(
            &postgres.pool,
            &privileged_call.workflow_id,
            &privileged_call.workflow_run_id,
            &privileged_call.command,
        )
        .await,
        Err(ReconcileTruthReviewsError::PrivilegedWorkerRole { .. })
    ));
    pg_v3_assert_no_effect(&postgres.pool, &privileged_call).await;

    // A column-level UPDATE is intentionally distinct from a table-level
    // grant. Also cover raw evidence, the operator admission, sequences and
    // schema DDL, all outside the exact two allowed effect write relations.
    let cases = [
        (
            format!("GRANT UPDATE (status) ON pipeline.execution_runs TO {worker_role}"),
            format!("REVOKE UPDATE (status) ON pipeline.execution_runs FROM {worker_role}"),
            "pipeline.execution_runs",
        ),
        (
            format!("GRANT UPDATE ON raw.sections TO {worker_role}"),
            format!("REVOKE UPDATE ON raw.sections FROM {worker_role}"),
            "raw.sections",
        ),
        (
            format!("GRANT INSERT ON pipeline.pg_effect_v3_admissions TO {worker_role}"),
            format!("REVOKE INSERT ON pipeline.pg_effect_v3_admissions FROM {worker_role}"),
            "pipeline.pg_effect_v3_admissions",
        ),
        (
            format!("GRANT UPDATE ON pipeline.pg_effect_v3_receipts TO {worker_role}"),
            format!("REVOKE UPDATE ON pipeline.pg_effect_v3_receipts FROM {worker_role}"),
            "pipeline.pg_effect_v3_receipts",
        ),
        (
            format!("GRANT DELETE ON site.seo_hitl_tasks TO {worker_role}"),
            format!("REVOKE DELETE ON site.seo_hitl_tasks FROM {worker_role}"),
            "site.seo_hitl_tasks",
        ),
        (
            format!("GRANT USAGE ON SEQUENCE raw.pages_id_seq TO {worker_role}"),
            format!("REVOKE USAGE ON SEQUENCE raw.pages_id_seq FROM {worker_role}"),
            "raw.pages_id_seq",
        ),
        (
            format!("GRANT CREATE ON SCHEMA raw TO {worker_role}"),
            format!("REVOKE CREATE ON SCHEMA raw FROM {worker_role}"),
            "CREATE ON SCHEMA raw",
        ),
    ];

    for (grant, revoke, expected_object) in cases {
        // Only identifiers derived from a UUID-backed disposable fixture and
        // static SQL clauses; SQLx 0.9's raw SQL assertion remains test-only.
        sqlx::raw_sql(sqlx::AssertSqlSafe(grant))
            .execute(&postgres.pool)
            .await
            .expect("grant extra permission as disposable admin");
        match assert_unprivileged_pg_effect_role(&effect_pool).await {
            Err(ReconcileTruthReviewsError::PrivilegedWorkerRole { target }) => {
                assert_eq!(target, expected_object);
            }
            other => panic!("unexpected PG writer privilege accepted: {other:?}"),
        }
        sqlx::raw_sql(sqlx::AssertSqlSafe(revoke))
            .execute(&postgres.pool)
            .await
            .expect("revoke excess permission in disposable test");
        assert_unprivileged_pg_effect_role(&effect_pool)
            .await
            .expect("minimal permissions accepted after removing excess authority");
    }
    // A PostgreSQL 18 NOINHERIT/SET membership is not visible as the
    // current user's table privilege, but SET ROLE would elevate this
    // connection after startup. The effect identity must reject it.
    let hidden_role = format!("ci_pg_v3_hidden_writer_{}", Uuid::new_v4().simple());
    let create_hidden = format!(
        "CREATE ROLE {hidden_role} NOLOGIN;
         GRANT UPDATE ON raw.sections TO {hidden_role};
         GRANT {hidden_role} TO {worker_role} WITH INHERIT FALSE, SET TRUE;"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(create_hidden))
        .execute(&postgres.pool)
        .await
        .expect("create disposable non-inherited SET ROLE escalation");
    assert!(matches!(
        assert_unprivileged_pg_effect_role(&effect_pool).await,
        Err(ReconcileTruthReviewsError::PrivilegedWorkerRole { target })
            if target == format!("SET ROLE {hidden_role}")
    ));
    let remove_hidden = format!(
        "REVOKE {hidden_role} FROM {worker_role};
         REVOKE UPDATE ON raw.sections FROM {hidden_role};
         DROP ROLE {hidden_role};"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(remove_hidden))
        .execute(&postgres.pool)
        .await
        .expect("drop disposable SET ROLE escalation");
    assert_unprivileged_pg_effect_role(&effect_pool)
        .await
        .expect("effect identity is safe after revoking hidden role");
    eprintln!("CI_PG_V3_GLOBAL_WRITE_PRIVILEGE_FENCE=PASS");
}

#[cfg(feature = "e2e")]
async fn pg_v3_assert_no_effect(pool: &sqlx::PgPool, fixture: &PgV3ReviewFixture) {
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM pipeline.pg_effect_v3_receipts WHERE run_id = $1",
    )
    .bind(fixture.command.run_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let tasks: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM site.seo_hitl_tasks
         WHERE task_type = 'truth_candidate_review' AND scope_signature = $1",
    )
    .bind(&fixture.command.context_key)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(receipts, 0, "denied command must not write receipt");
    assert_eq!(tasks, 0, "denied command must not mutate review queue");
}

/// Real PG acceptance: an independent operator admits EXACT source Truth;
/// the Activity's unprivileged role may execute once and replay indefinitely,
/// but cannot self-issue, alter history, cross a newer Truth write or accept a
/// changed source snapshot. Reuse one disposable PostgreSQL for all branches.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn pg_v3_truth_review_admission_effect_idempotence_staleness_and_role_boundary() {
    use postgres_store::truth_review_effect_v3::{
        assert_unprivileged_pg_effect_role, execute_admitted_reconcile,
        ReconcileTruthReviewsError,
    };

    let postgres = PostgresHarness::start()
        .await
        .expect("start disposable real application PostgreSQL");
    let effect_pool = pg_v3_unprivileged_effect_pool(&postgres).await;

    // A stolen Worker DB credential is not an HITL adjudicator and cannot
    // manufacture a receipt. Only the independently validating owner SQL
    // function has definer write rights. Its underlying invoker-only policy
    // cannot be called by this SELECT/EXECUTE-only Activity login either.
    for forbidden_sql in [
        "UPDATE site.seo_hitl_tasks SET queue_state = 'resolved'",
        "UPDATE site.seo_hitl_tasks SET decision_payload = '{\"review\":{\"decision\":\"approve\"}}'::jsonb",
        "INSERT INTO pipeline.pg_effect_v3_receipts
         (admission_id, execution_key, operation_id, operation_revision,
          run_id, context_key, candidate_set_hash, created_review_tasks, open_review_tasks)
         VALUES (gen_random_uuid(), repeat('a',64),
                 'pg.truth.reconcile_candidate_reviews',1,gen_random_uuid(),
                 'forged',repeat('b',64),0,0)",
        "SELECT site.reconcile_truth_candidate_reviews_v1('forged','forged')",
    ] {
        let refused = sqlx::query(forbidden_sql)
            .execute(&effect_pool)
            .await
            .expect_err("restricted Worker exercised arbitrary HITL/receipt SQL");
        assert_eq!(
            refused
                .as_database_error()
                .and_then(|error| error.code())
                .as_deref(),
            Some("42501"),
            "expected privilege denial for direct effect: {forbidden_sql}"
        );
    }
    eprintln!("CI_PG_V3_NO_DIRECT_HITL_OR_RECEIPT_DML=PASS");

    // Forged DAG-compatible command cannot grant a PG effect. Neither review
    // tasks nor receipts exist when there was no operator admission.
    let forged = pg_v3_seed_review_fixture(&postgres.pool).await;
    assert!(matches!(
        execute_admitted_reconcile(
            &effect_pool,
            &forged.workflow_id,
            &forged.workflow_run_id,
            &forged.command,
        ).await,
        Err(ReconcileTruthReviewsError::AdmissionDenied)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &forged).await;
    eprintln!("CI_PG_V3_FORGED=PASS");

    // V4 admission is NOT a V3 capability. A database constraint must reject
    // missing V4 parent-child attestation fields (SQL CHECK NULL is otherwise
    // UNKNOWN/accepted), and a fully shaped V4 row cannot be downgraded into
    // the legacy V3 Activity even if its child/parent run IDs were identical.
    let downgrade = pg_v3_seed_review_fixture(&postgres.pool).await;
    let malformed_v4 = sqlx::query(
        "INSERT INTO pipeline.pg_effect_v3_admissions
         (admission_id, admission_schema_revision, run_id, tenant_id, context_key,
          workflow_id, workflow_execution_run_id, operation_instance, plan_hash,
          contract_fingerprint, candidate_set_hash, issued_by, expires_at)
         VALUES ($1,4,$2,$3,$4,$5,$6,$7,$8,$9,
                 pipeline.pg_truth_candidate_set_hash_v1($4),'ci_v4_shape',now()+interval '1 hour')",
    )
    .bind(downgrade.command.admission_id)
    .bind(downgrade.command.run_id)
    .bind(&downgrade.command.scope.tenant_id)
    .bind(&downgrade.command.context_key)
    .bind(&downgrade.workflow_id)
    .bind(&downgrade.workflow_run_id)
    .bind(&downgrade.command.operation_instance)
    .bind(&downgrade.command.plan_hash)
    .bind(postgres_store::truth_review_effect_v3::contract_fingerprint().unwrap())
    .execute(&postgres.pool)
    .await
    .expect_err("PostgreSQL accepted missing mandatory V4 attestations");
    assert_eq!(
        malformed_v4
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514")
    );
    sqlx::query(
        "INSERT INTO pipeline.pg_effect_v3_admissions
         (admission_id, admission_schema_revision, run_id, tenant_id, context_key,
          workflow_id, workflow_execution_run_id, operation_instance, plan_hash,
          contract_fingerprint, candidate_set_hash, issued_by, expires_at,
          parent_workflow_id, parent_workflow_run_id, dispatch_execution_key,
          dispatch_input_hash, invocation_sha256, bound_input_sha256,
          owner_command_sha256, temporal_attestation_sha256,
          temporal_child_started_event_id)
         VALUES ($1,4,$2,$3,$4,$5,$6,$7,$8,$9,
                 pipeline.pg_truth_candidate_set_hash_v1($4),'ci_v4_shape',now()+interval '1 hour',
                 'parent-workflow',$6,repeat('a',64),repeat('b',64),repeat('c',64),
                 repeat('d',64),repeat('e',64),repeat('f',64),1)",
    )
    .bind(downgrade.command.admission_id)
    .bind(downgrade.command.run_id)
    .bind(&downgrade.command.scope.tenant_id)
    .bind(&downgrade.command.context_key)
    .bind(&downgrade.workflow_id)
    .bind(&downgrade.workflow_run_id)
    .bind(&downgrade.command.operation_instance)
    .bind(&downgrade.command.plan_hash)
    .bind(postgres_store::truth_review_effect_v3::contract_fingerprint().unwrap())
    .execute(&postgres.pool)
    .await
    .expect("operator-owned V4-specific attestation row inserted for negative downgrade test");
    assert!(matches!(
        execute_admitted_reconcile(
            &effect_pool,
            &downgrade.workflow_id,
            &downgrade.workflow_run_id,
            &downgrade.command,
        )
        .await,
        Err(ReconcileTruthReviewsError::AdmissionDenied)
    ));
    // Call the SECURITY DEFINER entrypoint DIRECTLY over the stolen Worker
    // DSN, bypassing all Rust typed validation. Matching claimed parent/child
    // IDs and checked key still cannot substitute for the operator-pinned
    // exact Temporal invocation/bound-command byte SHA256 fingerprints.
    let forged_v4_bytes = sqlx::query(
        "SELECT * FROM pipeline.consume_pg_truth_review_v1(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
    )
    .bind(downgrade.command.admission_id)
    .bind(downgrade.command.run_id)
    .bind(&downgrade.command.scope.tenant_id)
    .bind(&downgrade.command.context_key)
    .bind(&downgrade.workflow_id)
    .bind(&downgrade.workflow_run_id)
    .bind(&downgrade.command.operation_instance)
    .bind(&downgrade.command.plan_hash)
    .bind(postgres_store::truth_review_effect_v3::contract_fingerprint().unwrap())
    .bind("a".repeat(64))
    .bind(4_i16)
    .bind("parent-workflow")
    .bind(&downgrade.workflow_run_id)
    .bind("b".repeat(64))
    .bind(b"forged-child-history-bytes".to_vec())
    .bind(b"forged-artifact".to_vec())
    .bind(b"forged-command".to_vec())
    .fetch_one(&effect_pool)
    .await
    .expect_err("direct SQL V4 invocation-byte forgery consumed admission");
    assert_eq!(
        forged_v4_bytes
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("AV001")
    );
    pg_v3_assert_no_effect(&postgres.pool, &downgrade).await;
    eprintln!("CI_PG_V4_SHAPE_AND_V3_DOWNGRADE_DENIED=PASS");

    // An independently admitted V4 child may share the canonical parent run,
    // plan and operation instance with an older V3 child, while the old
    // uniqueness constraint sees two different child Temporal Run IDs.
    // Neither revision may then create a SECOND business receipt. Require
    // an explicit operator drain/cutover rather than consume either blindly.
    let cross_revision = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &cross_revision).await;
    let competing_v4_admission_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO pipeline.pg_effect_v3_admissions
         (admission_id, admission_schema_revision, run_id, tenant_id, context_key,
          workflow_id, workflow_execution_run_id, operation_instance, plan_hash,
          contract_fingerprint, candidate_set_hash, issued_by, expires_at,
          parent_workflow_id, parent_workflow_run_id, dispatch_execution_key,
          dispatch_input_hash, invocation_sha256, bound_input_sha256,
          owner_command_sha256, temporal_attestation_sha256,
          temporal_child_started_event_id)
         VALUES ($1,4,$2,$3,$4,'different-v4-child',$5,$6,$7,$8,
                 pipeline.pg_truth_candidate_set_hash_v1($4),
                 'ci_cross_revision',now()+interval '1 hour',
                 'parent-workflow',$9,repeat('a',64),repeat('b',64),repeat('c',64),
                 repeat('d',64),repeat('e',64),repeat('f',64),1)",
    )
    .bind(competing_v4_admission_id)
    .bind(cross_revision.command.run_id)
    .bind(&cross_revision.command.scope.tenant_id)
    .bind(&cross_revision.command.context_key)
    .bind(Uuid::new_v4().to_string())
    .bind(&cross_revision.command.operation_instance)
    .bind(&cross_revision.command.plan_hash)
    .bind(postgres_store::truth_review_effect_v3::contract_fingerprint().unwrap())
    .bind(&cross_revision.workflow_run_id)
    .execute(&postgres.pool)
    .await
    .expect("both independently shaped revision admissions can historically coexist");
    assert!(matches!(
        execute_admitted_reconcile(
            &effect_pool,
            &cross_revision.workflow_id,
            &cross_revision.workflow_run_id,
            &cross_revision.command,
        )
        .await,
        Err(ReconcileTruthReviewsError::AdmissionDenied)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &cross_revision).await;
    eprintln!("CI_PG_V4_CROSS_REVISION_DOUBLE_EFFECT_DENIED=PASS");

    // Unconsumed opposite-revision authority can be explicitly revoked and
    // drained: no receipt exists, so the original admitted V3 business effect
    // is legal again. A completed opposite-revision receipt would permanently
    // block this other instance from producing a duplicate.
    sqlx::query(
        "UPDATE pipeline.pg_effect_v3_admissions
         SET revoked_at = clock_timestamp() WHERE admission_id = $1",
    )
    .bind(competing_v4_admission_id)
    .execute(&postgres.pool)
    .await
    .expect("operator revokes unconsumed opposite-revision admission");
    let resumed = execute_admitted_reconcile(
        &effect_pool,
        &cross_revision.workflow_id,
        &cross_revision.workflow_run_id,
        &cross_revision.command,
    )
    .await
    .expect("revoked, unconsumed opposite revision does not poison cutover");
    assert_eq!(resumed.created_review_tasks, 1);
    eprintln!("CI_PG_V4_REVOKED_UNCONSUMED_CUTOVER_ALLOWED=PASS");

    // Historical upgrade boundary: a V3 admission issued before the
    // expected-key column existed cannot originate an effect (otherwise a
    // stolen Worker credential can choose a forged receipt key), while a
    // committed historical receipt still replays read-only and unchanged.
    let legacy_unconsumed = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_historical_unpinned_admission(&postgres.pool, &legacy_unconsumed).await;
    assert!(matches!(
        execute_admitted_reconcile(
            &effect_pool,
            &legacy_unconsumed.workflow_id,
            &legacy_unconsumed.workflow_run_id,
            &legacy_unconsumed.command,
        )
        .await,
        Err(ReconcileTruthReviewsError::AdmissionDenied)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &legacy_unconsumed).await;

    let historical_replay = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_historical_unpinned_admission(&postgres.pool, &historical_replay).await;
    let historical_key = postgres_store::truth_review_effect_v3::execution_key(
        &historical_replay.workflow_id,
        &historical_replay.workflow_run_id,
        &historical_replay.command,
    )
    .expect("original receipt execution key");
    let old_receipt: Uuid = sqlx::query_scalar(
        "INSERT INTO pipeline.pg_effect_v3_receipts
         (admission_id, execution_key, operation_id, operation_revision,
          run_id, context_key, candidate_set_hash,
          created_review_tasks, open_review_tasks)
         VALUES ($1,$2,'pg.truth.reconcile_candidate_reviews',1,$3,$4,
                 pipeline.pg_truth_candidate_set_hash_v1($4),0,0)
         RETURNING receipt_id",
    )
    .bind(historical_replay.command.admission_id)
    .bind(&historical_key)
    .bind(historical_replay.command.run_id)
    .bind(&historical_replay.command.context_key)
    .fetch_one(&postgres.pool)
    .await
    .expect("seed originally committed V3 receipt before key migration");
    let replayed = execute_admitted_reconcile(
        &effect_pool,
        &historical_replay.workflow_id,
        &historical_replay.workflow_run_id,
        &historical_replay.command,
    )
    .await
    .expect("historical durable receipt replay remains valid");
    assert_eq!(replayed.receipt_id, old_receipt);
    assert_eq!(replayed.execution_key, historical_key);
    assert_eq!(replayed.created_review_tasks, 0);
    let historical_tasks: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM site.seo_hitl_tasks WHERE scope_signature = $1",
    )
    .bind(&historical_replay.command.context_key)
    .fetch_one(&postgres.pool)
    .await
    .expect("historic replay did not create new human review");
    assert_eq!(historical_tasks, 0);
    eprintln!("CI_PG_V3_HISTORICAL_REPLAY_ONLY_NO_NEW_EFFECT=PASS");

    let authorized = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &authorized).await;
    let mut forged_session = effect_pool.begin().await.expect("stolen Worker SQL session");
    sqlx::query("SELECT set_config('app.pg_effect_admitted','true',true)")
        .execute(&mut *forged_session)
        .await
        .expect("attacker may set arbitrary local GUC, never authorization");
    let forged_execution_key = sqlx::query(
        "SELECT * FROM pipeline.consume_pg_truth_review_v1(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
    )
    .bind(authorized.command.admission_id)
    .bind(authorized.command.run_id)
    .bind(&authorized.command.scope.tenant_id)
    .bind(&authorized.command.context_key)
    .bind(&authorized.workflow_id)
    .bind(&authorized.workflow_run_id)
    .bind(&authorized.command.operation_instance)
    .bind(&authorized.command.plan_hash)
    .bind(postgres_store::truth_review_effect_v3::contract_fingerprint().unwrap())
    .bind("f".repeat(64))
    .bind(3_i16)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<Vec<u8>>)
    .bind(None::<Vec<u8>>)
    .bind(None::<Vec<u8>>)
    .fetch_one(&mut *forged_session)
    .await
    .expect_err("attacker-chosen V3 receipt key bypassed owner SQL capability");
    assert_eq!(
        forged_execution_key
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("AV001")
    );
    forged_session.rollback().await.expect("clear rejected attack transaction");
    pg_v3_assert_no_effect(&postgres.pool, &authorized).await;
    eprintln!("CI_PG_V3_V4_DIRECT_SQL_CAPABILITY_FORGERY_DENIED=PASS");
    // An unprivileged Activity cannot update or revoke the admission, even
    // if it knows the exact admission UUID and every command field.
    assert!(
        sqlx::query(
            "UPDATE pipeline.pg_effect_v3_admissions SET revoked_at = now()
             WHERE admission_id = $1",
        )
        .bind(authorized.command.admission_id)
        .execute(&effect_pool)
        .await
        .is_err(),
        "PG Activity role was able to self-revoke/modify operator admission"
    );
    assert_unprivileged_pg_effect_role(&effect_pool)
        .await
        .expect("role permissions must remain unprivileged after command");
    let wrong_execution_run_id = Uuid::new_v4().to_string();
    assert!(matches!(
        execute_admitted_reconcile(
            &effect_pool,
            &authorized.workflow_id,
            &wrong_execution_run_id,
            &authorized.command,
        ).await,
        Err(ReconcileTruthReviewsError::AdmissionDenied)
    ), "a replay under a distinct server-issued Workflow Run ID cannot consume admission");
    pg_v3_assert_no_effect(&postgres.pool, &authorized).await;
    let receipt = execute_admitted_reconcile(
        &effect_pool,
        &authorized.workflow_id,
        &authorized.workflow_run_id,
        &authorized.command,
    )
    .await
    .expect("authorized V3 PG owner executes existing Truth review reconciliation");
    assert_eq!(receipt.admission_id, authorized.command.admission_id);
    assert_eq!(receipt.created_review_tasks, 1);
    assert_eq!(receipt.open_review_tasks, 1);
    let task_id = format!(
        "truth_candidate_review@1:{}:pg-v3-immutable-source",
        authorized.candidate_id
    );
    let queue_state: String = sqlx::query_scalar(
        "SELECT queue_state FROM site.seo_hitl_tasks WHERE task_key = $1",
    )
    .bind(&task_id)
    .fetch_one(&postgres.pool)
    .await
    .expect("actual canonical HITL review task was created");
    assert_eq!(queue_state, "open");
    // A committed receipt is the durable idempotence authority even after
    // the operator revokes the admission. Natural expiry, without mutating
    // immutable issued authority, is tested separately under the context lock.
    sqlx::query(
        "UPDATE pipeline.pg_effect_v3_admissions
         SET revoked_at = now()
         WHERE admission_id = $1",
    )
    .bind(authorized.command.admission_id)
    .execute(&postgres.pool)
    .await
    .expect("privileged operator revokes spent admission");
    let replay = execute_admitted_reconcile(
        &effect_pool,
        &authorized.workflow_id,
        &authorized.workflow_run_id,
        &authorized.command,
    )
    .await
    .expect("same Activity replay must return committed receipt after revocation");
    assert_eq!(receipt, replay, "replay must preserve the exact receipt ID");
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM pipeline.pg_effect_v3_receipts
         WHERE admission_id = $1",
    )
    .bind(authorized.command.admission_id)
    .fetch_one(&postgres.pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "exactly one durable effect receipt per admission");
    let mut wrong_instance = authorized.command.clone();
    wrong_instance.operation_instance.push_str("-forged");
    assert!(matches!(
        execute_admitted_reconcile(
            &effect_pool,
            &authorized.workflow_id,
            &authorized.workflow_run_id,
            &wrong_instance,
        ).await,
        Err(ReconcileTruthReviewsError::AdmissionDenied)
    ));
    eprintln!("CI_PG_V3_AUTHORIZED_REPLAY=PASS");

    // Same admitted plan, but changed source rows after operator issuance:
    // no old snapshot may open review tasks; operator must re-admit exact bytes.
    let changed = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &changed).await;
    sqlx::query(
        "UPDATE extracted.rule_candidates
         SET params = '{\"amount\":999,\"currency\":\"EUR\"}'::jsonb
         WHERE rule_candidate_id = $1",
    )
    .bind(&changed.candidate_id)
    .execute(&postgres.pool)
    .await
    .expect("independent Truth source snapshot changes after admission");
    assert!(matches!(
        execute_admitted_reconcile(
            &effect_pool,
            &changed.workflow_id,
            &changed.workflow_run_id,
            &changed.command,
        ).await,
        Err(ReconcileTruthReviewsError::CandidateSetChanged)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &changed).await;
    eprintln!("CI_PG_V3_SNAPSHOT_FENCE=PASS");

    // The canonical Truth fence compares RUN CREATION order. A later run in
    // the same context with verified_truth_write=done supersedes the older
    // admitted run; this test intentionally does not redefine that policy as
    // a completion-timestamp or arbitrary Workflow-history comparison.
    let stale = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &stale).await;
    sqlx::query(
        "UPDATE pipeline.execution_runs SET created_at = now() - interval '5 minutes'
         WHERE run_id = $1",
    )
    .bind(stale.command.run_id)
    .execute(&postgres.pool)
    .await
    .expect("make admitted run older by canonical creation order");
    let newer_run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO pipeline.execution_runs
         (run_id, workflow_run_id, workflow_type, context_key, status, created_at)
         VALUES ($1, $2, 'seo_site_build', $3, 'done', now() - interval '1 minute')",
    )
    .bind(newer_run_id)
    .bind(format!("newer-truth-{}", newer_run_id.simple()))
    .bind(&stale.command.context_key)
    .execute(&postgres.pool)
    .await
    .expect("seed independently committed newer canonical Truth run");
    sqlx::query(
        "INSERT INTO pipeline.step_executions
         (run_id, step_name, input_hash, output_hash, idempotency_key, status)
         VALUES ($1, 'verified_truth_write', $2, $2, $3, 'done')",
    )
    .bind(newer_run_id)
    .bind("b".repeat(64))
    .bind(format!("newer-truth-write-{}", newer_run_id.simple()))
    .execute(&postgres.pool)
    .await
    .expect("mark newer canonical verified Truth write done");
    let newer_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM pipeline.execution_runs newer
         JOIN pipeline.execution_runs old ON old.run_id = $1
         JOIN pipeline.step_executions done ON done.run_id = newer.run_id
           AND done.step_name = 'verified_truth_write' AND done.status = 'done'
         WHERE newer.context_key = old.context_key
           AND newer.run_id <> old.run_id
           AND newer.created_at > old.created_at",
    )
    .bind(stale.command.run_id)
    .fetch_one(&postgres.pool)
    .await
    .unwrap();
    assert_eq!(newer_count, 1, "test must exercise exact newer-run chronology");
    assert!(matches!(
        execute_admitted_reconcile(
            &effect_pool,
            &stale.workflow_id,
            &stale.workflow_run_id,
            &stale.command,
        ).await,
        Err(ReconcileTruthReviewsError::StaleTruthRun)
    ));
    pg_v3_assert_no_effect(&postgres.pool, &stale).await;
    eprintln!("CI_PG_V3_NEWER_TRUTH_FENCE=PASS");
    eprintln!("CI_PG_V3_POSTGRES_E2E=PASS");
}
