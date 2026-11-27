// V3 owns a new Temporal Task Queue and a provider-free process. This suite is
// deliberately independent of the V1/V2 canonical worker and of the expensive
// application Postgres/Neo4j/Qdrant fixture. TemporalHarness starts a real
// Temporal server (with its own private persistence database).

#[cfg(feature = "e2e")]
fn pure_v3_orchestrator_binary() -> PathBuf {
    let binary = env::var_os("INTEGRATION_PURE_DAG_ORCHESTRATOR_BIN")
        .expect("INTEGRATION_PURE_DAG_ORCHESTRATOR_BIN must point to the exact-source, prebuilt V3 worker; no nested cargo build is permitted");
    let path = PathBuf::from(binary);
    assert!(
        path.is_file(),
        "exact-source V3 orchestrator binary does not exist: {}",
        path.display()
    );
    path
}

#[cfg(feature = "e2e")]
fn pure_v3_source_sha() -> String {
    // The V3 process intentionally rejects random build IDs. Read the commit
    // checked out for this exact-source suite rather than inventing one.
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root())
        .output()
        .expect("resolve exact-source V3 checkout SHA");
    assert!(output.status.success(), "V3 suite requires a Git checkout");
    let sha = String::from_utf8(output.stdout)
        .expect("git SHA must be UTF-8")
        .trim()
        .to_owned();
    assert!(
        sha.len() == 40 && sha.bytes().all(|c| c.is_ascii_hexdigit()),
        "V3 worker build ID must be the exact 40-hex checkout SHA"
    );
    sha
}

#[cfg(feature = "e2e")]
fn spawn_pure_orchestrator(
    temporal: &TemporalHarness,
    source_sha: &str,
    strict_v4: bool,
) -> ChildGuard {
    let queue = if strict_v4 {
        "alegria-pure-dag-v4"
    } else {
        temporal_gateway::PURE_DAG_ORCHESTRATOR_QUEUE
    };
    let mut command = Command::new(pure_v3_orchestrator_binary());
    command
        .current_dir(workspace_root())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .env("RUST_LOG", "warn")
        .env("ALLOW_PURE_DAG_ORCHESTRATOR", (!strict_v4).to_string())
        .env("ALLOW_PURE_DAG_ORCHESTRATOR_V4", strict_v4.to_string())
        .env("TEMPORAL_ORCHESTRATOR_TASK_QUEUE", queue)
        .env("TEMPORAL_WORKER_VERSIONING", "false")
        .env("WORKER_BUILD_ID", source_sha)
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace);
    // Parent CI environments may happen to expose live provider credentials.
    // The new orchestrator must run with none of those inherited by its child.
    for key in [
        "DATABASE_URL",
        "NEO4J_URI",
        "NEO4J_PASSWORD",
        "QDRANT_URL",
        "OPENAI_API_KEY",
        "SEO_LLM_API_KEY",
        "DATAFORSEO_LOGIN",
        "DATAFORSEO_PASSWORD",
        "VOYAGE_API_KEY",
    ] {
        command.env_remove(key);
    }
    let child = command.spawn().expect("start exact-source pure DAG worker");
    ChildGuard { child }
}

#[cfg(feature = "e2e")]
fn assert_pure_history_replays(history_json: &[u8], strict_v4: bool) {
    assert!(
        history_json.len() > 100,
        "real V3 Temporal server returned no meaningful history"
    );
    let history_path = env::temp_dir().join(format!(
        "alegria-pure-v3-history-{}-{}.json",
        std::process::id(),
        Uuid::new_v4()
    ));
    fs::write(&history_path, history_json).expect("save real V3 history for SDK replay");
    let output = Command::new(pure_v3_orchestrator_binary())
        .arg(if strict_v4 {
            "--replay-pure-v4-history"
        } else {
            "--replay-history"
        })
        .arg(&history_path)
        .current_dir(workspace_root())
        .output();
    fs::remove_file(&history_path).expect("remove disposable V3 Temporal history");
    let output = output.expect("run real V3 SDK replay against captured server history");
    assert!(
        output.status.success(),
        "pure V3 SDK replay failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS"),
        "V3 replayer exited without SDK replay completion marker"
    );
}

#[cfg(feature = "e2e")]
fn attach_unknown_authored_field_to_route(
    run: &mut seo_application::operation_adapter::PureDagRun,
) {
    let admitted = run
        .admitted
        .get_mut(&operation_protocol::ArtifactId::from("route-in"))
        .expect("route-only fixture has one admitted real typed owner input");
    let mut authored: serde_json::Value =
        serde_json::from_slice(&admitted.payload).expect("typed owner input JSON");
    authored
        .as_object_mut()
        .expect("route input struct is a JSON object")
        .insert("unknown_future_setting".into(), serde_json::json!(true));
    admitted.payload = serde_json::to_vec(&authored).expect("serialize authored JSON fixture");
}

/// Real Temporal+persistence, real standalone process, three executed domain
/// DAGs, three durable rejections, restart from an empty process and SDK replay
/// of EVERY actual server history. No provider or mock workflow is involved.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn pure_v3_temporal_new_queue_provider_free_replay_and_restart() {
    use temporalio_common::data_converters::PayloadConverter;

    assert_eq!(
        temporal_gateway::PURE_DAG_ORCHESTRATOR_QUEUE,
        "alegria-pure-dag-v3",
        "V3 must never poll the legacy V1/V2 Task Queue"
    );
    let temporal = TemporalHarness::start()
        .await
        .expect("start real isolated Temporal V3 server");
    let source_sha = pure_v3_source_sha();
    let mut worker = Some(spawn_pure_orchestrator(&temporal, &source_sha, false));
    assert!(
        worker.as_mut().unwrap().child.try_wait().unwrap().is_none(),
        "provider-free V3 orchestrator exited before polling"
    );
    let client = connect_client(
        &temporal.temporal_url,
        format!("pure-v3-client-{}", Uuid::new_v4()),
        &temporal.namespace,
    )
    .await
    .expect("connect to actual V3 Temporal server");
    let converter = PayloadConverter::serde_json();

    for (index, case) in [
        "route-only",
        "entity-to-canonical",
        "independent-editorial-operations",
    ]
    .into_iter()
    .enumerate()
    {
        let run_id = Uuid::new_v4().to_string();
        let run = pure_dag_e2e_case(case, &run_id);
        if index == 2 {
            // Stop the original poller BEFORE submitting the third plan. Its
            // server-persisted task has no Worker until the new independent
            // process starts, proving no in-memory registry continuity.
            drop(worker.take());
        }
        let handle = client
            .start_workflow(
                UntypedWorkflow::new("AlegriaPureOperationDagV1Workflow"),
                RawValue::from_value(&run, &converter),
                WorkflowStartOptions::new(
                    temporal_gateway::PURE_DAG_ORCHESTRATOR_QUEUE,
                    run_id.clone(),
                )
                .build(),
            )
            .await
            .expect("create V3 Workflow on its dedicated Task Queue");
        if index == 2 {
            worker = Some(spawn_pure_orchestrator(&temporal, &source_sha, false));
            assert!(
                worker.as_mut().unwrap().child.try_wait().unwrap().is_none(),
                "restarted provider-free V3 orchestrator exited before polling"
            );
        }
        let completed = timeout(
            Duration::from_secs(45),
            handle.get_result(WorkflowGetResultOptions::default()),
        )
        .await
        .unwrap_or_else(|_| panic!("V3 {case} did not finish after restart"))
        .unwrap_or_else(|error| panic!("V3 {case} workflow failed: {error}"));
        let actual_json: String = completed.to_value(&converter);
        let actual: seo_application::operation_adapter::PureDagReceipt =
            serde_json::from_str(&actual_json).expect("decode actual V3 DAG receipt");
        let expected = seo_application::operation_adapter::execute_pure_dag(&run_id, &run)
            .expect("execute same-source pure DAG reference");
        assert_eq!(actual.plan_hash, run.plan.hash);
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(&expected).unwrap(),
            "V3 {case} returned unexpected domain operation artifacts"
        );
        let history = handle
            .fetch_history(WorkflowFetchHistoryOptions::builder().build())
            .to_json()
            .await
            .expect("read actual completed V3 Temporal server history");
        assert!(
            scheduled_temporal_activities(&history).is_empty(),
            "provider-free V3 {case} scheduled an Activity"
        );
        assert_pure_history_replays(&history, false);
        eprintln!("CI_PURE_V3_DAG=PASS case={case} queue=alegria-pure-dag-v3");
    }

    // Invalid schema, changed contract and effect authority are rejected by
    // the deployed Workflow itself, not just a plan.validate() unit test.
    for rejection in ["schema", "pinned-contract", "effect-permission"] {
        let run_id = Uuid::new_v4().to_string();
        let mut run = pure_dag_e2e_case("route-only", &run_id);
        match rejection {
            "schema" => run.plan.schema_revision += 1,
            "pinned-contract" => {
                run.plan.pinned_contracts[0]
                    .fingerprint
                    .0
                    .push_str("-changed");
                run.plan.hash = run.plan.canonical_hash().unwrap();
            }
            "effect-permission" => {
                run.plan.allowed_effects.insert("publication.write".into());
                run.plan.hash = run.plan.canonical_hash().unwrap();
            }
            _ => unreachable!(),
        }
        let handle = client
            .start_workflow(
                UntypedWorkflow::new("AlegriaPureOperationDagV1Workflow"),
                RawValue::from_value(&run, &converter),
                WorkflowStartOptions::new(temporal_gateway::PURE_DAG_ORCHESTRATOR_QUEUE, run_id)
                    .build(),
            )
            .await
            .expect("submit invalid V3 run to real server");
        let rejection_result = timeout(
            Duration::from_secs(25),
            handle.get_result(WorkflowGetResultOptions::default()),
        )
        .await
        .unwrap_or_else(|_| panic!("V3 {rejection} rejection did not terminate"));
        assert!(
            rejection_result.is_err(),
            "V3 {rejection} unexpectedly succeeded on real server"
        );
        let history = handle
            .fetch_history(WorkflowFetchHistoryOptions::builder().build())
            .to_json()
            .await
            .expect("read actual rejected V3 Temporal server history");
        assert!(
            scheduled_temporal_activities(&history).is_empty(),
            "rejected V3 {rejection} scheduled an Activity"
        );
        assert_pure_history_replays(&history, false);
        eprintln!("CI_PURE_V3_REJECTION=PASS reason={rejection}");
    }

    // Historical V3 ingests the pre-strict Serde-only owner DTO: its old
    // histories may contain extra authored keys. A new release must not
    // reinterpret such runs with V4 fail-closed ingress during replay.
    let historical_run_id = Uuid::new_v4().to_string();
    let mut historical = pure_dag_e2e_case("route-only", &historical_run_id);
    attach_unknown_authored_field_to_route(&mut historical);
    let historical_handle = client
        .start_workflow(
            UntypedWorkflow::new("AlegriaPureOperationDagV1Workflow"),
            RawValue::from_value(&historical, &converter),
            WorkflowStartOptions::new(
                temporal_gateway::PURE_DAG_ORCHESTRATOR_QUEUE,
                historical_run_id.clone(),
            )
            .build(),
        )
        .await
        .expect("submit V3 legacy-compatible authored input");
    let historical_completed = timeout(
        Duration::from_secs(45),
        historical_handle.get_result(WorkflowGetResultOptions::default()),
    )
    .await
    .expect("V3 legacy-compatible run must terminate")
    .expect("V3 must retain pre-strict input semantics");
    let historical_json: String = historical_completed.to_value(&converter);
    let historical_receipt: seo_application::operation_adapter::PureDagReceipt =
        serde_json::from_str(&historical_json).expect("decode historical V3 receipt");
    let expected_legacy = seo_application::operation_adapter::execute_pure_dag_legacy_v3(
        &historical_run_id,
        &historical,
    )
    .expect("pre-strict V3 reference still accepts this authored JSON");
    assert_eq!(
        serde_json::to_value(historical_receipt).unwrap(),
        serde_json::to_value(expected_legacy).unwrap()
    );
    let historical_history = historical_handle
        .fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .to_json()
        .await
        .expect("read V3 legacy-compatible actual server History");
    assert!(scheduled_temporal_activities(&historical_history).is_empty());
    assert_pure_history_replays(&historical_history, false);
    eprintln!("CI_PURE_V3_LEGACY_INGRESS_REPLAY=PASS");
    drop(worker);

    // V4 is a NEW queue and NEW History-visible Workflow Type. Reuse the
    // same exact-source prebuilt binary, without an extra Cargo build/job.
    // The V4 result is a concrete Rust receipt, never a historical JSON String.
    let mut strict_worker = spawn_pure_orchestrator(&temporal, &source_sha, true);
    assert!(strict_worker.child.try_wait().unwrap().is_none());
    let strict_run_id = Uuid::new_v4().to_string();
    let strict = pure_dag_e2e_case("entity-to-canonical", &strict_run_id);
    let strict_handle = client
        .start_workflow(
            UntypedWorkflow::new("AlegriaPureOperationDagV2Workflow"),
            RawValue::from_value(&strict, &converter),
            WorkflowStartOptions::new("alegria-pure-dag-v4", strict_run_id.clone()).build(),
        )
        .await
        .expect("submit strict V4 typed pure DAG on separate Task Queue");
    let strict_completed = timeout(
        Duration::from_secs(45),
        strict_handle.get_result(WorkflowGetResultOptions::default()),
    )
    .await
    .expect("V4 typed pure DAG must terminate")
    .expect("valid V4 typed pure DAG must succeed");
    let strict_receipt: seo_application::operation_adapter::PureDagReceipt =
        strict_completed.to_value(&converter);
    let strict_expected =
        seo_application::operation_adapter::execute_pure_dag(&strict_run_id, &strict)
            .expect("strict source-aligned pure DAG reference");
    assert_eq!(
        serde_json::to_value(strict_receipt).unwrap(),
        serde_json::to_value(strict_expected).unwrap()
    );
    let strict_history = strict_handle
        .fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .to_json()
        .await
        .expect("read completed V4 Temporal server History");
    assert!(scheduled_temporal_activities(&strict_history).is_empty());
    assert_pure_history_replays(&strict_history, true);

    let rejected_run_id = Uuid::new_v4().to_string();
    let mut rejected = pure_dag_e2e_case("route-only", &rejected_run_id);
    attach_unknown_authored_field_to_route(&mut rejected);
    let rejected_handle = client
        .start_workflow(
            UntypedWorkflow::new("AlegriaPureOperationDagV2Workflow"),
            RawValue::from_value(&rejected, &converter),
            WorkflowStartOptions::new("alegria-pure-dag-v4", rejected_run_id).build(),
        )
        .await
        .expect("submit V4 strict rejection fixture");
    let rejection = timeout(
        Duration::from_secs(25),
        rejected_handle.get_result(WorkflowGetResultOptions::default()),
    )
    .await
    .expect("strict V4 must terminate rather than hang");
    assert!(
        rejection.is_err(),
        "V4 must reject unknown authored JSON field"
    );
    let rejected_history = rejected_handle
        .fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .to_json()
        .await
        .expect("read rejected V4 Temporal server History");
    assert!(scheduled_temporal_activities(&rejected_history).is_empty());
    assert_pure_history_replays(&rejected_history, true);
    drop(strict_worker);
    eprintln!("CI_PURE_V4_STRICT_TYPED_E2E=PASS queue=alegria-pure-dag-v4");
    eprintln!("CI_PURE_V3_TEMPORAL_E2E=PASS");
}
