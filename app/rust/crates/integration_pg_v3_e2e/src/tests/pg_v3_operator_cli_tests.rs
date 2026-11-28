// The actual native operator CLI, isolated from the V3 Activity poller,
// exercised against a disposable PostgreSQL schema and a distinct LOGIN.
// This is mandatory in the existing PG V3 `--features e2e` test shard, not
// an ignored unit test or a direct admin INSERT masquerading as issuance.

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn pg_v3_tenant_context_rls_blocks_unprovisioned_cross_tenant_effect() {
    use postgres_store::truth_review_effect_v3::{contract_fingerprint, execution_key};

    let postgres = PostgresHarness::start()
        .await
        .expect("real isolated PostgreSQL tenant RLS database");
    let fixture = pg_v3_seed_review_fixture(&postgres.pool).await;
    pg_v3_issue_operator_admission(&postgres.pool, &fixture).await;
    // A real task owned by tenant A must remain globally visible to the
    // TRUNCATE guard even when tenant B's RLS SELECT hides it entirely.
    let protected_task = format!("ci-cross-tenant-hitl-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO site.seo_hitl_tasks
         (task_key, task_type, queue_state, scope_signature, tenant_id)
         VALUES ($1, 'truth_candidate_review', 'resolved', $2, $3)",
    )
    .bind(&protected_task)
    .bind(&fixture.command.context_key)
    .bind(&fixture.command.scope.tenant_id)
    .execute(&postgres.pool)
    .await
    .expect("seed other tenant's protected HITL history");

    // A second tenant may legitimately have its own DB login, but can never
    // impersonate a shared natural visa context claimed by the first tenant.
    let role = format!("ci_pg_v3_other_tenant_{}", Uuid::new_v4().simple());
    const PASSWORD: &str = "disposable_cross_tenant_password";
    let provision = format!(
        "CREATE ROLE {role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {role};
         GRANT USAGE ON SCHEMA pipeline, site, raw, extracted, kb TO {role};
         GRANT SELECT ON kb.sources TO {role};
         GRANT SELECT ON pipeline.execution_runs, pipeline.pg_effect_v3_admissions,
                         pipeline.pg_effect_v3_receipts, extracted.rule_candidates,
                         raw.sections, raw.pages, site.seo_hitl_tasks TO {role};
         GRANT TRUNCATE ON site.seo_hitl_tasks TO {role};
         GRANT SELECT, INSERT, UPDATE ON pipeline.pg_effect_v3_admissions TO {role};
         GRANT EXECUTE ON FUNCTION pipeline.consume_pg_truth_review_v1(
             UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,SMALLINT,
             TEXT,TEXT,TEXT,BYTEA,BYTEA,BYTEA
         ) TO {role};
         INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id)
         VALUES ('{role}', 'different-tenant');"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(provision))
        .execute(&postgres.pool)
        .await
        .expect("DBA provisions second tenant independent of malicious command scope");
    let restricted_url = postgres.database_url.replacen(
        "postgres://postgres:postgres_password@",
        &format!("postgres://{role}:{PASSWORD}@"),
        1,
    );
    let restricted = sqlx::PgPool::connect(&restricted_url)
        .await
        .expect("second tenant PostgreSQL login");

    let visible: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.execution_runs WHERE run_id=$1",
    )
    .bind(fixture.command.run_id)
    .fetch_one(&restricted)
    .await
    .unwrap();
    assert_eq!(visible, 0, "RLS must hide another tenant's canonical run");
    let visible: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.pg_effect_v3_admissions WHERE admission_id=$1",
    )
    .bind(fixture.command.admission_id)
    .fetch_one(&restricted)
    .await
    .unwrap();
    assert_eq!(visible, 0, "RLS must hide another tenant's admitted effect");
    let visible_hitl: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.seo_hitl_tasks WHERE task_key = $1",
    )
    .bind(&protected_task)
    .fetch_one(&restricted)
    .await
    .expect("cross-tenant SELECT is RLS-filtered");
    assert_eq!(visible_hitl, 0, "RLS must hide another tenant's HITL history");
    // TRUNCATE ignores RLS row filters. An accidentally over-granted role
    // must still fail closed in the owner-privileged BEFORE TRUNCATE trigger.
    let denied_truncate = sqlx::query("TRUNCATE site.seo_hitl_tasks")
        .execute(&restricted)
        .await
        .expect_err("RLS-hidden tenant A task was destructively truncated by tenant B");
    assert_eq!(
        denied_truncate
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514"),
        "the TRUNCATE guard must inspect all tenants, not the caller's RLS view"
    );
    let retained: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM site.seo_hitl_tasks WHERE task_key = $1",
    )
    .bind(&protected_task)
    .fetch_one(&postgres.pool)
    .await
    .expect("DBA sees protected task after rejected cross-tenant TRUNCATE");
    assert_eq!(retained, 1, "other tenant's HITL history must survive");
    eprintln!("CI_PG_V5_CROSS_TENANT_TRUNCATE_GUARD=PASS");

    let duplicate_context = sqlx::query(
        "INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id)
         VALUES ($1,'different-tenant')",
    )
    .bind(&fixture.command.context_key)
    .execute(&postgres.pool)
    .await;
    assert!(
        duplicate_context.is_err(),
        "same natural visa context must not silently acquire a second owner"
    );

    // Crucially, SECURITY DEFINER bypasses the invoker's RLS. The capability
    // itself must independently check session_user and deny even a complete
    // stolen admission/dispatch payload from the other tenant.
    let wrong_effect = sqlx::query(
        "SELECT receipt_id FROM pipeline.consume_pg_truth_review_v1(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
    )
    .bind(fixture.command.admission_id)
    .bind(fixture.command.run_id)
    .bind(&fixture.command.scope.tenant_id)
    .bind(&fixture.command.context_key)
    .bind(&fixture.workflow_id)
    .bind(&fixture.workflow_run_id)
    .bind(&fixture.command.operation_instance)
    .bind(&fixture.command.plan_hash)
    .bind(contract_fingerprint().unwrap())
    .bind(execution_key(&fixture.workflow_id, &fixture.workflow_run_id, &fixture.command).unwrap())
    .bind(3_i16)
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind(Option::<Vec<u8>>::None)
    .bind(Option::<Vec<u8>>::None)
    .bind(Option::<Vec<u8>>::None)
    .fetch_one(&restricted)
    .await
    .expect_err("cross-tenant SECURITY DEFINER invocation must be denied");
    assert_eq!(
        wrong_effect.as_database_error().and_then(|database| database.code()).as_deref(),
        Some("AV001"),
        "tenant denial must be typed and independently testable"
    );
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.pg_effect_v3_receipts WHERE admission_id=$1",
    )
    .bind(fixture.command.admission_id)
    .fetch_one(&postgres.pool)
    .await
    .unwrap();
    assert_eq!(receipts, 0, "denied tenant must not commit any receipt");

    // Historical NULL ownership is quarantined, not opportunistically
    // reclassified to whichever tenant an operator claims.
    sqlx::query("UPDATE raw.pages SET tenant_id=NULL WHERE url LIKE $1")
        .bind(format!("%{}%", fixture.command.context_key.split(':').last().unwrap()))
        .execute(&postgres.pool)
        .await
        .expect("simulate unattributed historical page with DBA test role");
    let ready: bool = sqlx::query_scalar(
        "SELECT pipeline.tenant_truth_scope_ready_v1($1,$2,$3)",
    )
    .bind(&fixture.command.scope.tenant_id)
    .bind(&fixture.command.context_key)
    .bind(fixture.command.run_id)
    .fetch_one(&postgres.pool)
    .await
    .unwrap();
    assert!(!ready, "NULL historical page must quarantine subsequent effects");
}

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
         GRANT USAGE ON SCHEMA pipeline, extracted, raw, kb TO {role};
         GRANT SELECT ON kb.sources TO {role};
         GRANT SELECT ON pipeline.execution_runs, pipeline.pg_effect_v3_admissions,
                         pipeline.pg_effect_v3_receipts, extracted.rule_candidates,
                         raw.sections, raw.pages TO {role};
         GRANT SELECT, INSERT, UPDATE ON pipeline.pg_effect_v3_admissions TO {role};
         INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id)
         VALUES ('{role}', 'ci-pg-v3-tenant');
         -- Disposable, explicitly tenant-bound operator LOGIN only.
         GRANT EXECUTE ON FUNCTION pipeline.tenant_truth_scope_ready_v1(TEXT,TEXT,UUID) TO {role};"
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

/// Full disposable V4 chain: native parent -> native child -> independent
/// server-History operator attestation -> distinct limited PG role -> real
/// atomic HITL reconciliation/receipt. The canonical run binding here is an
/// ADMIN TEST FIXTURE after Temporal server Start, NOT a production starter.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn pg_v3_v4_live_parent_child_operator_attestation_effect_receipt() {
    use operation_protocol::{
        ArtifactBinding, ContractCatalog, EffectId, ExecutionPlan, ExternalArtifact,
        Operation, OperationNode, PlanHash, RunScope, PLAN_SCHEMA_REVISION,
    };
    use postgres_store::truth_review_effect_v3::{
        ReconcileTruthReviewsOperation, ReconcileTruthReviewsReceipt,
    };
    use temporalio_client::{UntypedWorkflow, WorkflowGetResultOptions, WorkflowStartOptions};
    use temporalio_common::data_converters::{PayloadConverter, RawValue};
    use temporalio_common::protos::temporal::api::history::v1::history_event::Attributes;
    use tokio::time::{sleep, timeout};

    let postgres = PostgresHarness::start().await.expect("disposable V4 PG schema");
    let temporal = TemporalHarness::start().await.expect("real Temporal server for V4 child proof");
    let fixture = pg_v3_seed_review_fixture(&postgres.pool).await;
    let (effect_pool, worker_url) = pg_v3_unprivileged_effect_identity(&postgres).await;
    let _pg_worker = spawn_pg_v3_effect_worker(&temporal, &worker_url);
    let orchestrator_bin = PathBuf::from(
        env::var_os("INTEGRATION_EFFECT_DAG_WORKER_BIN")
            .expect("CI must supply exact-source temporal_orchestrator binary for V4 E2E"),
    );
    assert!(orchestrator_bin.is_file(), "native V4 parent orchestrator binary missing");
    let parent_worker = Command::new(orchestrator_bin)
        .current_dir(workspace_root())
        .env("ALLOW_EFFECT_DAG_ORCHESTRATOR_V4", "true")
        .env("ALLOW_PURE_DAG_ORCHESTRATOR", "false")
        .env("TEMPORAL_ORCHESTRATOR_TASK_QUEUE", "alegria-effect-dag-v4")
        .env("WORKER_BUILD_ID", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("start exact native admitted V4 parent Worker");
    let _parent_worker = ChildGuard { child: parent_worker };

    let scope = RunScope {
        tenant_id: fixture.command.scope.tenant_id.clone(),
        run_id: fixture.command.run_id.to_string(),
    };
    let instance: operation_protocol::OperationInstanceId = "review_native_v4".into();
    let artifact_id: operation_protocol::ArtifactId = "review_scope_v4".into();
    let contract = ReconcileTruthReviewsOperation::contract();
    let input_schema = contract.inputs[&operation_protocol::PortId::from("truth_review_scope")].clone();
    let artifact_bytes = serde_json::to_vec(
        &truth_domain::review_effect_contract::TruthReviewScopeArtifactV1 {
            run_id: fixture.command.run_id,
            context_key: fixture.command.context_key.clone(),
        },
    ).expect("serialize exact canonical typed Truth scope artifact");
    let provenance = "operator-e2e:independently-verified-source".to_owned();
    let node = OperationNode {
        instance: instance.clone(),
        operation: contract.reference.clone(),
        inputs: std::collections::BTreeMap::from([(
            "truth_review_scope".into(),
            ArtifactBinding::External { artifact: artifact_id.clone() },
        )]),
        outputs: std::collections::BTreeMap::from([(
            "review_queue_receipt".into(), "receipt_v4".into(),
        )]),
        after: std::collections::BTreeSet::new(),
        permitted_effects: contract.effects.clone(),
    };
    let mut catalog = ContractCatalog::default();
    catalog.register::<ReconcileTruthReviewsOperation>()
        .expect("register real PG Truth owner contract");
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: scope.clone(),
        allowed_effects: contract.effects.clone(),
        external_artifacts: std::collections::BTreeMap::from([(
            artifact_id.clone(),
            ExternalArtifact { schema: input_schema.clone(), scope: scope.clone(), provenance: provenance.clone() },
        )]),
        pinned_contracts: vec![],
        nodes: vec![node],
        hash: PlanHash(String::new()),
    }.seal(&catalog).expect("seal real V4 owner DAG plan");
    let mut command = fixture.command.clone();
    command.operation_instance = instance.as_str().to_owned();
    command.plan_hash = plan.hash.as_str().to_owned();
    let parent_input = serde_json::json!({
        "plan": plan,
        "admitted": {
            "review_scope_v4": {
                "schema": input_schema,
                "scope": scope,
                "provenance": provenance,
                "payload": artifact_bytes,
            }
        },
        "owner_commands": {
            "review_native_v4": {
                "kind": "pg_truth_review_reconcile_v1",
                "input": command,
            }
        }
    });
    let client = connect_client(
        &temporal.temporal_url, format!("native-v4-operator-{}", Uuid::new_v4()),
        &temporal.namespace,
    ).await.expect("Temporal V4 E2E client");
    let parent = client.start_workflow(
        UntypedWorkflow::new("AlegriaAdmittedEffectDagV2Workflow"),
        RawValue::from_value(&parent_input, &PayloadConverter::serde_json()),
        WorkflowStartOptions::new("alegria-effect-dag-v4", fixture.command.run_id.to_string()).build(),
    ).await.expect("native V4 parent start");
    let parent_run_id = parent.info().run_id.clone().expect("server-issued V4 parent Run ID");
    // Fixture-only trusted registration: never let the restricted operator
    // mutate an existing SEO site's canonical workflow_run_id in production.
    sqlx::query("UPDATE pipeline.execution_runs SET workflow_run_id=$1 WHERE run_id=$2")
        .bind(&parent_run_id).bind(fixture.command.run_id)
        .execute(&postgres.pool).await.expect("fixture binds newly started V4 parent Run ID");
    let (child_id, child_run_id) = timeout(Duration::from_secs(45), async {
        loop {
            let events = parent.fetch_history(WorkflowFetchHistoryOptions::builder().build())
                .into_events().await.expect("native parent Temporal events");
            if let Some((id, run_id)) = events.iter().find_map(|event| {
                let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs)) =
                    &event.attributes else { return None; };
                attrs.workflow_execution.as_ref().map(|execution|
                    (execution.workflow_id.clone(), execution.run_id.clone()))
            }) {
                break (id, run_id);
            }
            sleep(Duration::from_millis(200)).await;
        }
    }).await.expect("V4 parent did not start native child");
    let before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.pg_effect_v3_receipts WHERE admission_id=$1",
    ).bind(fixture.command.admission_id).fetch_one(&postgres.pool).await.unwrap();
    assert_eq!(before, 0, "child Signal gate must block PG effect before operator admission");

    let role = format!("ci_pg_v4_operator_{}", Uuid::new_v4().simple());
    const PASSWORD: &str = "pg_v4_disposable_operator_password";
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {role};
         GRANT USAGE ON SCHEMA pipeline, extracted, raw, kb TO {role};
         GRANT SELECT ON kb.sources TO {role};
         GRANT SELECT ON pipeline.execution_runs, pipeline.pg_effect_v3_admissions,
                         pipeline.pg_effect_v3_receipts, extracted.rule_candidates,
                         raw.sections, raw.pages TO {role};
         GRANT SELECT, INSERT, UPDATE ON pipeline.pg_effect_v3_admissions TO {role};
         INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id)
         VALUES ('{role}', 'ci-pg-v3-tenant');
         -- Disposable, explicitly tenant-bound operator LOGIN only.
         GRANT EXECUTE ON FUNCTION pipeline.tenant_truth_scope_ready_v1(TEXT,TEXT,UUID) TO {role};"
    ))).execute(&postgres.pool).await.expect("distinct restricted V4 operator role");
    let operator_url = postgres.database_url.replacen(
        "postgres://postgres:postgres_password@",
        &format!("postgres://{role}:{PASSWORD}@"), 1,
    );
    let locator = serde_json::json!({
        "admission_id": fixture.command.admission_id,
        "canonical_run_id": fixture.command.run_id,
        "parent_workflow_id": fixture.command.run_id.to_string(),
        "parent_workflow_run_id": parent_run_id,
        "child_workflow_id": child_id,
        "child_workflow_run_id": child_run_id,
        "expires_in_seconds": 120
    });
    let path = env::temp_dir().join(format!("ci-pg-v4-operator-{}.json", Uuid::new_v4()));
    let execute_locator = |locator: &serde_json::Value| {
        fs::write(&path, serde_json::to_vec(locator).unwrap()).unwrap();
        Command::new(pg_v3_effect_worker_binary())
            .arg("--operator-issue-v4").arg(&path)
            .env("ALEGRIA_ENABLE_PG_EFFECT_OPERATOR_V4", "true")
            .env("PG_EFFECT_OPERATOR_DATABASE_URL", &operator_url)
            .env("PG_EFFECT_OPERATOR_TENANT_ID", "ci-pg-v3-tenant")
            .env("PG_EFFECT_OPERATOR_TEMPORAL_URL", &temporal.temporal_url)
            .env("PG_EFFECT_OPERATOR_TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .output().expect("execute exact V4 native operator CLI")
    };
    let mut forged = locator.clone();
    forged["child_workflow_run_id"] = serde_json::json!(Uuid::new_v4());
    let denied = execute_locator(&forged);
    assert!(!denied.status.success(), "forged child server Run ID admitted");
    let still_none: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.pg_effect_v3_receipts WHERE admission_id=$1",
    ).bind(fixture.command.admission_id).fetch_one(&postgres.pool).await.unwrap();
    assert_eq!(still_none, 0, "forged parent-child locator may not execute an effect");
    let authorized = execute_locator(&locator);
    assert!(authorized.status.success(),
        "real server-attested V4 operator issuance failed: stdout={} stderr={}",
        String::from_utf8_lossy(&authorized.stdout),
        String::from_utf8_lossy(&authorized.stderr));
    fs::remove_file(&path).expect("remove V4 disposable operator locator");
    let parent_result = timeout(Duration::from_secs(45),
        parent.get_result(WorkflowGetResultOptions::default()))
        .await.expect("V4 parent failed to complete after admitted PG child")
        .expect("V4 parent reported native child error");
    let parent_result_json: String = parent_result.to_value(&PayloadConverter::serde_json());
    assert!(!parent_result_json.is_empty());
    let (revision, checked_key): (i16, String) = sqlx::query_as(
        "SELECT admission_schema_revision, dispatch_execution_key
         FROM pipeline.pg_effect_v3_admissions WHERE admission_id=$1",
    ).bind(fixture.command.admission_id).fetch_one(&postgres.pool).await.unwrap();
    assert_eq!(revision, 4);
    let receipt_key: String = sqlx::query_scalar(
        "SELECT execution_key FROM pipeline.pg_effect_v3_receipts WHERE admission_id=$1",
    ).bind(fixture.command.admission_id).fetch_one(&effect_pool).await
        .expect("durable PG V4 effect receipt");
    assert_eq!(receipt_key, checked_key,
        "durable PG receipt must pin the exact parent checked dispatch key");
    eprintln!("CI_PG_V4_NATIVE_PARENT_CHILD_OPERATOR_EFFECT=PASS");
}
