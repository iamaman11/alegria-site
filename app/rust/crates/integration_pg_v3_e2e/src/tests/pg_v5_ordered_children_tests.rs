// Native Temporal V5 positive acceptance: two *ordered*, independently
// attested V4 PostgreSQL child Workflows in one canonical Truth run. A single
// PostgreSQL `execution_runs` row owns one context_key, so unordered sibling
// effects on that context fail whole-DAG preflight; they are NOT fake fanout.

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn pg_v3_v5_live_ordered_two_child_operator_attestations() {
    use operation_protocol::{
        ArtifactBinding, ContractCatalog, ExecutionPlan, ExternalArtifact, Operation,
        OperationNode, PlanHash, RunScope, PLAN_SCHEMA_REVISION,
    };
    use postgres_store::truth_review_effect_v3::ReconcileTruthReviewsOperation;
    use temporalio_common::data_converters::{PayloadConverter, RawValue};
    use tokio::time::sleep;

    let postgres = PostgresHarness::start().await.expect("disposable V5 PG schema");
    let temporal = TemporalHarness::start().await.expect("real Temporal server for V5 proof");
    let fixture = pg_v3_seed_review_fixture(&postgres.pool).await;
    let (effect_pool, worker_url) = pg_v3_unprivileged_effect_identity(&postgres).await;
    let _pg_worker = spawn_pg_v3_effect_worker(&temporal, &worker_url);

    let orchestrator_bin = PathBuf::from(
        env::var_os("INTEGRATION_EFFECT_DAG_WORKER_BIN")
            .expect("CI requires exact-source native V5 parent binary"),
    );
    assert!(orchestrator_bin.is_file());
    let parent_worker = Command::new(orchestrator_bin)
        .current_dir(workspace_root())
        .env("ALLOW_EFFECT_DAG_ORCHESTRATOR_V5", "true")
        .env("ALLOW_EFFECT_DAG_ORCHESTRATOR_V4", "false")
        .env("ALLOW_PURE_DAG_ORCHESTRATOR", "false")
        .env("TEMPORAL_ORCHESTRATOR_TASK_QUEUE", "alegria-effect-dag-v5")
        .env("WORKER_BUILD_ID", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn()
        .expect("start exact V5 parent Workflow Worker");
    let _parent_worker = ChildGuard { child: parent_worker };

    let scope = RunScope {
        tenant_id: fixture.command.scope.tenant_id.clone(),
        run_id: fixture.command.run_id.to_string(),
    };
    let contract = ReconcileTruthReviewsOperation::contract();
    let input_schema = contract.inputs[&operation_protocol::PortId::from("truth_review_scope")].clone();
    let payload = serde_json::to_vec(
        &truth_domain::review_effect_contract::TruthReviewScopeArtifactV1 {
            run_id: fixture.command.run_id,
            context_key: fixture.command.context_key.clone(),
        },
    )
    .unwrap();
    let provenance = "real-v5-parent:operator-must-verify-source".to_string();
    let instances = ["review_first", "review_after_first"];
    let nodes: Vec<_> = instances.iter().enumerate().map(|(ordinal, name)| OperationNode {
        instance: (*name).into(),
        operation: contract.reference.clone(),
        inputs: std::collections::BTreeMap::from([(
            "truth_review_scope".into(),
            ArtifactBinding::External { artifact: "review_scope_v5".into() },
        )]),
        outputs: std::collections::BTreeMap::from([(
            "review_queue_receipt".into(),
            format!("receipt_v5_{ordinal}").as_str().into(),
        )]),
        after: if ordinal == 0 {
            std::collections::BTreeSet::new()
        } else {
            std::collections::BTreeSet::from([instances[0].into()])
        },
        permitted_effects: contract.effects.clone(),
    }).collect();
    let mut catalog = ContractCatalog::default();
    catalog.register::<ReconcileTruthReviewsOperation>()
        .expect("register genuine PG owner contract");
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: scope.clone(),
        allowed_effects: contract.effects.clone(),
        external_artifacts: std::collections::BTreeMap::from([(
            "review_scope_v5".into(),
            ExternalArtifact { schema: input_schema.clone(), scope: scope.clone(), provenance: provenance.clone() },
        )]),
        pinned_contracts: vec![],
        nodes,
        hash: PlanHash(String::new()),
    }.seal(&catalog).expect("seal ordered two-owner DAG");
    let mut commands = Vec::new();
    for (ordinal, instance) in instances.iter().enumerate() {
        let mut command = fixture.command.clone();
        command.admission_id = if ordinal == 0 { fixture.command.admission_id } else { Uuid::new_v4() };
        command.operation_instance = (*instance).to_owned();
        command.plan_hash = plan.hash.as_str().to_owned();
        commands.push(command);
    }
    let parent_input = serde_json::json!({
        "plan": plan,
        "admitted": {
            "review_scope_v5": {
                "schema": input_schema,
                "scope": scope,
                "provenance": provenance,
                "payload": payload
            }
        },
        "owner_commands": {
            "review_first": { "kind": "pg_truth_review_reconcile_v1", "input": commands[0] },
            "review_after_first": { "kind": "pg_truth_review_reconcile_v1", "input": commands[1] }
        }
    });
    let client = connect_client(
        &temporal.temporal_url,
        format!("native-v5-operator-{}", Uuid::new_v4()),
        &temporal.namespace,
    ).await.expect("V5 Temporal client");
    let parent = client.start_workflow(
        UntypedWorkflow::new("AlegriaAdmittedEffectDagV3Workflow"),
        RawValue::from_value(&parent_input, &PayloadConverter::serde_json()),
        WorkflowStartOptions::new("alegria-effect-dag-v5", fixture.command.run_id.to_string()).build(),
    ).await.expect("start V5 native parent");
    let parent_run_id = parent.info().run_id.clone().expect("actual Temporal parent Run ID");
    sqlx::query("UPDATE pipeline.execution_runs SET workflow_run_id=$1 WHERE run_id=$2")
        .bind(&parent_run_id)
        .bind(fixture.command.run_id)
        .execute(&postgres.pool)
        .await
        .expect("test-only canonical run bootstrap after Temporal Start");

    let role = format!("ci_pg_v5_operator_{}", Uuid::new_v4().simple());
    const PASSWORD: &str = "disposable_v5_operator_password";
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {role};
         GRANT USAGE ON SCHEMA pipeline, extracted, raw, kb TO {role};
         GRANT SELECT ON kb.sources TO {role};
         GRANT SELECT ON pipeline.execution_runs, pipeline.pg_effect_v3_admissions,
                         pipeline.pg_effect_v3_receipts, extracted.rule_candidates,
                         raw.sections, raw.pages TO {role};
         GRANT SELECT, INSERT, UPDATE ON pipeline.pg_effect_v3_admissions TO {role};"
    )))
    .execute(&postgres.pool)
    .await
    .expect("independent restricted operator login for two children");
    // The DBA-owned role binding, not an environment label or SET-able GUC,
    // establishes tenant authority for the new PostgreSQL RLS policy.
    sqlx::query(
        "INSERT INTO pipeline.tenant_role_ownership(role_name, tenant_id)
         VALUES ($1::name, 'ci-pg-v3-tenant')",
    )
    .bind(&role)
    .execute(&postgres.pool)
    .await
    .expect("DBA provisions authenticated V5 operator tenant");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT EXECUTE ON FUNCTION pipeline.tenant_truth_scope_ready_v1(TEXT,TEXT,UUID) TO {role}"
    )))
    .execute(&postgres.pool)
    .await
    .expect("DBA grants sensitive tenant audit helper only to authenticated V5 operator");
    let operator_url = postgres.database_url.replacen(
        "postgres://postgres:postgres_password@",
        &format!("postgres://{role}:{PASSWORD}@"), 1,
    );
    for (ordinal, command) in commands.iter().enumerate() {
        // A later child MUST NOT start before the first true PG receipt has
        // been received: the manifest's `after` edge is a server-visible
        // parent History barrier, not an in-memory success bit.
        let (child_id, child_run_id) = timeout(Duration::from_secs(60), async {
            loop {
                let events = parent.fetch_history(WorkflowFetchHistoryOptions::builder().build())
                    .into_events().await.expect("real V5 parent Temporal History");
                let started: Vec<_> = events.iter().filter_map(|event| {
                    let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs)) =
                        &event.attributes else { return None; };
                    attrs.workflow_execution.as_ref().map(|execution| (
                        execution.workflow_id.clone(), execution.run_id.clone(),
                    ))
                }).collect();
                if started.len() > ordinal {
                    break started[ordinal].clone();
                }
                sleep(Duration::from_millis(200)).await;
            }
        }).await.expect("V5 native PG child not started in time");
        let prior_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pipeline.pg_effect_v3_receipts WHERE run_id=$1",
        ).bind(command.run_id).fetch_one(&postgres.pool).await.unwrap();
        assert_eq!(prior_count, ordinal as i64,
            "each child must remain blocked without its separate admission");

        let locator = serde_json::json!({
            "admission_id": command.admission_id,
            "canonical_run_id": command.run_id,
            "parent_workflow_id": command.run_id.to_string(),
            "parent_workflow_run_id": parent_run_id,
            "child_workflow_id": child_id,
            "child_workflow_run_id": child_run_id,
            "expires_in_seconds": 120
        });
        let path = env::temp_dir().join(format!("ci-pg-v5-op-{}.json", Uuid::new_v4()));
        fs::write(&path, serde_json::to_vec(&locator).unwrap()).unwrap();
        let operator = Command::new(pg_v3_effect_worker_binary())
            .arg("--operator-issue-v4").arg(&path)
            .env("ALEGRIA_ENABLE_PG_EFFECT_OPERATOR_V4", "true")
            .env("PG_EFFECT_OPERATOR_DATABASE_URL", &operator_url)
            .env("PG_EFFECT_OPERATOR_TENANT_ID", &scope_for_v5_operator())
            .env("PG_EFFECT_OPERATOR_TEMPORAL_URL", &temporal.temporal_url)
            .env("PG_EFFECT_OPERATOR_TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .output().expect("independent V5 owner operator invocation");
        fs::remove_file(&path).unwrap();
        assert!(operator.status.success(),
            "V5 child #{} operator issue refused: stdout={} stderr={}",
            ordinal, String::from_utf8_lossy(&operator.stdout),
            String::from_utf8_lossy(&operator.stderr));
    }
    let completion = timeout(
        Duration::from_secs(60),
        parent.get_result(WorkflowGetResultOptions::default()),
    ).await.expect("V5 parent never completed after both separately admitted children")
     .expect("V5 parent returned a failed Workflow result");
    let result_json: String = completion.to_value(&PayloadConverter::serde_json());
    let result: serde_json::Value = serde_json::from_str(&result_json).unwrap();
    assert_eq!(result["schema_revision"], 5);
    assert_eq!(result["result"]["owner_receipts"].as_object().unwrap().len(), 2);
    let durable: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.pg_effect_v3_receipts WHERE run_id=$1",
    ).bind(fixture.command.run_id).fetch_one(&effect_pool).await.unwrap();
    assert_eq!(durable, 2, "two independently authorized real PG receipts missing");
    eprintln!("CI_PG_V5_NATIVE_ORDERED_TWO_CHILD_OPERATOR_EFFECT=PASS");
}

#[cfg(feature = "e2e")]
fn scope_for_v5_operator() -> String {
    "ci-pg-v3-tenant".into()
}
