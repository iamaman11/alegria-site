// End-to-end *live* Temporal Signal -> scheduled PG Activity -> real PostgreSQL
// review queue + receipt, with two disjoint V3 Task Queues. Unlike the PG-only
// admission test, this starts the actual native Worker binary and uses the
// Temporal server's own Workflow Run ID for operator authorization.

#[cfg(feature = "e2e")]
fn pg_v3_effect_worker_binary() -> PathBuf {
    let binary = env::var_os("INTEGRATION_PG_EFFECT_WORKER_BIN").expect(
        "INTEGRATION_PG_EFFECT_WORKER_BIN requires an exact-source prebuilt PG V3 binary",
    );
    let path = PathBuf::from(binary);
    assert!(
        path.is_file(),
        "exact-source PG V3 Worker binary missing: {}",
        path.display()
    );
    path
}

#[cfg(feature = "e2e")]
fn spawn_pg_v3_effect_worker(temporal: &TemporalHarness, restricted_url: &str) -> ChildGuard {
    let mut command = Command::new(pg_v3_effect_worker_binary());
    command
        .current_dir(workspace_root())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .env("RUST_LOG", "warn")
        .env("ALEGRIA_ENABLE_PG_EFFECT_V3", "true")
        .env("PG_EFFECT_WORKER_DATABASE_URL", restricted_url)
        .env("TEMPORAL_URL", &temporal.temporal_url);
    // The Activity must connect with the isolated minimal role, never a
    // privileged inherited DATABASE_URL or a provider credential.
    for name in [
        "DATABASE_URL",
        "NEO4J_URI",
        "NEO4J_PASSWORD",
        "QDRANT_URL",
        "OPENAI_API_KEY",
        "VOYAGE_API_KEY",
    ] {
        command.env_remove(name);
    }
    ChildGuard {
        child: command.spawn().expect("spawn native PostgreSQL V3 Workflow + Activity pollers"),
    }
}

#[cfg(feature = "e2e")]
fn pg_v3_assert_offline_sdk_replay(history_json: &[u8]) {
    assert!(
        history_json.len() > 100,
        "actual Temporal server did not return a meaningful PG V3 History"
    );
    let path = env::temp_dir().join(format!(
        "alegria-pg-v3-live-history-{}-{}.json",
        std::process::id(),
        Uuid::new_v4()
    ));
    fs::write(&path, history_json).expect("save captured PG V3 Temporal History");
    let output = Command::new(pg_v3_effect_worker_binary())
        .arg("--replay-history")
        .arg(&path)
        .current_dir(workspace_root())
        .env("ALEGRIA_ENABLE_PG_EFFECT_V3", "false")
        .env("PG_EFFECT_WORKER_DATABASE_URL", "postgres://not-a-real-server:1/offline")
        .env("TEMPORAL_URL", "http://127.0.0.1:1")
        .output();
    fs::remove_file(&path).expect("remove disposable PG V3 server History");
    let output = output.expect("run native PG V3 offline SDK History replay");
    assert!(
        output.status.success(),
        "PG V3 SDK History replay failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("CI_TEMPORAL_PG_EFFECT_REPLAY=PASS"),
        "PG V3 replay CLI did not report official SDK completion"
    );
}

/// A Workflow is created first; the operator copies the *server-issued*
/// Temporal Run ID into canonical PostgreSQL and only then grants an
/// immutable exact-source admission. Signalling the Workflow does not itself
/// grant authorization.
#[cfg(feature = "e2e")]
async fn pg_v3_start_live_command(
    client: &temporalio_client::Client,
    postgres: &PostgresHarness,
    mut fixture: PgV3ReviewFixture,
) -> (
    temporalio_client::WorkflowHandle<temporalio_client::Client, UntypedWorkflow>,
    PgV3ReviewFixture,
) {
    use temporalio_common::data_converters::{PayloadConverter, RawValue};

    let handle = client
        .start_workflow(
            UntypedWorkflow::new("PgTruthReviewReconcileV3Workflow"),
            RawValue::from_value(&fixture.command, &PayloadConverter::serde_json()),
            WorkflowStartOptions::new("alegria-pg-command-v3", fixture.workflow_id.clone()).build(),
        )
        .await
        .expect("create actual PG V3 Workflow on isolated command Task Queue");
    let actual_run_id = handle
        .info()
        .run_id
        .clone()
        .expect("server must assign a real Temporal Workflow Run ID");
    assert!(
        Uuid::parse_str(&actual_run_id).is_ok(),
        "Temporal server returned an invalid Workflow Run ID"
    );
    assert_ne!(
        actual_run_id,
        fixture.command.run_id.to_string(),
        "Temporal Run ID is not the application's canonical run UUID"
    );
    // The fixture initially uses an arbitrary placeholder Run ID; replace it
    // only AFTER the Temporal service has created this real execution.
    fixture.workflow_run_id = actual_run_id;
    sqlx::query(
        "UPDATE pipeline.execution_runs
         SET workflow_run_id = $1
         WHERE run_id = $2",
    )
    .bind(&fixture.workflow_run_id)
    .bind(fixture.command.run_id)
    .execute(&postgres.pool)
    .await
    .expect("record server-issued Temporal Run ID in canonical run");
    (handle, fixture)
}

#[cfg(feature = "e2e")]
async fn pg_v3_signal_admission_ready(
    handle: &temporalio_client::WorkflowHandle<temporalio_client::Client, UntypedWorkflow>,
) {
    use temporalio_client::{UntypedSignal, WorkflowSignalOptions};
    use temporalio_common::data_converters::RawValue;

    handle
        .signal(
            UntypedSignal::<UntypedWorkflow>::new("operator_admission_ready"),
            RawValue::default(),
            WorkflowSignalOptions::default(),
        )
        .await
        .expect("deliver real Temporal operator_admission_ready Signal");
}

#[cfg(feature = "e2e")]
async fn pg_v3_verify_live_receipt(
    handle: &temporalio_client::WorkflowHandle<temporalio_client::Client, UntypedWorkflow>,
    postgres: &PostgresHarness,
    fixture: &PgV3ReviewFixture,
) {
    use temporalio_common::data_converters::PayloadConverter;

    let result = timeout(
        Duration::from_secs(45),
        handle.get_result(WorkflowGetResultOptions::default()),
    )
    .await
    .expect("admitted PG Workflow did not finish")
    .expect("admitted PG Workflow must complete its real Activity");
    let json: String = result.to_value(&PayloadConverter::serde_json());
    let receipt: postgres_store::truth_review_effect_v3::ReconcileTruthReviewsReceipt =
        serde_json::from_str(&json).expect("decode PG receipt from actual Workflow completion");
    assert_eq!(receipt.admission_id, fixture.command.admission_id);
    assert_eq!(receipt.run_id, fixture.command.run_id);
    assert_eq!(receipt.created_review_tasks, 1);
    assert_eq!(receipt.open_review_tasks, 1);
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM pipeline.pg_effect_v3_receipts
         WHERE admission_id = $1",
    )
    .bind(fixture.command.admission_id)
    .fetch_one(&postgres.pool)
    .await
    .expect("read real atomically committed PG Activity receipt");
    assert_eq!(receipts, 1, "one admission must create one durable receipt");
    let tasks: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM site.seo_hitl_tasks
         WHERE scope_signature = $1 AND task_type = 'truth_candidate_review'",
    )
    .bind(&fixture.command.context_key)
    .fetch_one(&postgres.pool)
    .await
    .expect("read actual canonical HITL queue");
    assert_eq!(tasks, 1, "PG Activity must reconcile exactly one real review task");
    let history = handle
        .fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .to_json()
        .await
        .expect("fetch completed cross-queue Workflow History from real Temporal server");
    assert_eq!(
        scheduled_temporal_activities(&history),
        vec!["reconcile_truth_candidate_reviews_v3"],
        "Workflow must schedule precisely the narrow PG V3 Activity"
    );
    pg_v3_assert_offline_sdk_replay(&history);
}

/// Real Temporal persistence and independent native Worker process with two
/// dedicated pollers, plus a separate actual PostgreSQL application DB. Two
/// operator-authorized commands survive Worker restart and one forged Signal
/// fails closed without a database effect. Every server History is SDK-replayed
/// offline, with deliberately bogus network credentials.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn temporal_effect_v3_cross_queue_signal_admission_replay_and_restart() {
    let postgres = PostgresHarness::start()
        .await
        .expect("start real isolated application PostgreSQL");
    let temporal = TemporalHarness::start()
        .await
        .expect("start independent real Temporal server and persistence");
    let (restricted_pool, restricted_url) = pg_v3_unprivileged_effect_identity(&postgres).await;
    let mut worker = Some(spawn_pg_v3_effect_worker(&temporal, &restricted_url));
    assert!(
        worker.as_mut().unwrap().child.try_wait().unwrap().is_none(),
        "independent PG effect Worker quit before polling"
    );
    let client = connect_client(
        &temporal.temporal_url,
        format!("pg-v3-cross-queue-client-{}", Uuid::new_v4()),
        &temporal.namespace,
    )
    .await
    .expect("connect to real Temporal V3 test namespace");

    let initial = pg_v3_seed_review_fixture(&postgres.pool).await;
    let (first, initial) = pg_v3_start_live_command(&client, &postgres, initial).await;
    // The Signal gate is not permission: before the operator's SQL admission,
    // a pending Workflow must have no scheduled PG Activity.
    let pre_admission_history = first
        .fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .to_json()
        .await
        .expect("observe live signal-gated Workflow before issuance");
    assert!(
        scheduled_temporal_activities(&pre_admission_history).is_empty(),
        "PG Activity was scheduled before operator admission Signal"
    );
    pg_v3_issue_operator_admission(&postgres.pool, &initial).await;
    pg_v3_signal_admission_ready(&first).await;
    pg_v3_verify_live_receipt(&first, &postgres, &initial).await;
    eprintln!("CI_PG_V3_TEMPORAL_SIGNAL_EFFECT=PASS");

    // The old worker is gone BEFORE the next Workflow is queued. Temporal
    // persists the task and the new process can resume it without inheriting
    // any in-memory state or admission authority from the first Worker.
    drop(worker.take());
    let after_restart = pg_v3_seed_review_fixture(&postgres.pool).await;
    let (second, after_restart) =
        pg_v3_start_live_command(&client, &postgres, after_restart).await;
    pg_v3_issue_operator_admission(&postgres.pool, &after_restart).await;
    pg_v3_signal_admission_ready(&second).await;
    worker = Some(spawn_pg_v3_effect_worker(&temporal, &restricted_url));
    assert!(
        worker.as_mut().unwrap().child.try_wait().unwrap().is_none(),
        "restarted independent PG effect Worker exited"
    );
    pg_v3_verify_live_receipt(&second, &postgres, &after_restart).await;
    eprintln!("CI_PG_V3_TEMPORAL_RESTART_REPLAY=PASS");

    // A forged caller can start a correctly typed Workflow and send its
    // Signal, but cannot create the privileged admission row using the
    // Activity role. The real Activity must fail once and mutate no Truth.
    let denied = pg_v3_seed_review_fixture(&postgres.pool).await;
    let (forged, denied) = pg_v3_start_live_command(&client, &postgres, denied).await;
    pg_v3_signal_admission_ready(&forged).await;
    let rejected = timeout(
        Duration::from_secs(45),
        forged.get_result(WorkflowGetResultOptions::default()),
    )
    .await
    .expect("unadmitted PG Workflow did not terminate");
    assert!(rejected.is_err(), "Signal alone granted a PostgreSQL effect");
    pg_v3_assert_no_effect(&postgres.pool, &denied).await;
    let rejected_history = forged
        .fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .to_json()
        .await
        .expect("fetch denied PG Workflow History from real Temporal server");
    assert_eq!(
        scheduled_temporal_activities(&rejected_history),
        vec!["reconcile_truth_candidate_reviews_v3"],
        "failed Workflow must reach the real effect Activity's final veto"
    );
    pg_v3_assert_offline_sdk_replay(&rejected_history);
    eprintln!("CI_PG_V3_TEMPORAL_FORGED_SIGNAL=PASS");

    // The same identity must remain least-privileged after three live runs.
    postgres_store::truth_review_effect_v3::assert_unprivileged_pg_effect_role(&restricted_pool)
        .await
        .expect("live PG effect process never widened its Worker role");
    drop(worker);
    eprintln!("CI_PG_V3_TEMPORAL_CROSS_QUEUE_E2E=PASS");
}
