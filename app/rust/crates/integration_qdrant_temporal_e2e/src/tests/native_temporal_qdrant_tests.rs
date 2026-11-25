// Real four-container proof: app PostgreSQL, Qdrant, Temporal PostgreSQL,
// Temporal server. Independent native parent+child Worker processes and
// operator CLI must attest server Histories before a genuine provider RPC.
#[tokio::test]
#[serial]
async fn qdrant_v8_native_temporal_history_operator_real_effect() {
    use temporalio_client::WorkflowGetResultOptions;

    let pg = PostgresHarness::start().await.expect("real application PostgreSQL");
    let qdrant = QdrantHarness::start().await.expect("real Qdrant REST+gRPC");
    let _provider_env = ScopedEnvironment::apply(&[
        ("QDRANT_REST_URL", qdrant.qdrant_rest_url.as_str()),
        ("QDRANT_URL", qdrant.qdrant_url.as_str()),
    ]);
    let temporal = TemporalHarness::start().await.expect("real Temporal server + persistence");
    let parent_bin = binary("INTEGRATION_EFFECT_DAG_WORKER_BIN");
    let qdrant_bin = binary("INTEGRATION_QDRANT_EFFECT_WORKER_BIN");
    let tenant = "ci-native-qdrant-tenant";
    let run_id = Uuid::new_v4();
    let admission = Uuid::new_v4();
    let context = format!("qdrant-v7-native-{run_id}");
    let keyword = format!("native-temporal-{admission}");

    sqlx::query(
        "INSERT INTO kb.visa_contexts (context_key,country_code,visa_family,citizenship_code)
         VALUES ($1,'ES','tourist','BY')",
    ).bind(&context).execute(&pg.pool).await.unwrap();
    sqlx::query("INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id) VALUES ($1,$2)")
        .bind(&context).bind(tenant).execute(&pg.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO pipeline.execution_runs
           (run_id,workflow_run_id,workflow_type,context_key,status,tenant_id)
         VALUES ($1,$2,'seo_site_build',$3,'verifying',$4)",
    ).bind(run_id).bind(Uuid::new_v4().to_string()).bind(&context).bind(tenant)
        .execute(&pg.pool).await.unwrap();

    sqlx::query("CREATE ROLE native_qdrant_operator LOGIN PASSWORD 'native_operator_password'")
        .execute(&pg.pool).await.unwrap();
    sqlx::query("CREATE ROLE native_qdrant_executor LOGIN PASSWORD 'native_executor_password'")
        .execute(&pg.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id)
         VALUES ('native_qdrant_operator','ci-native-qdrant-tenant'),
                ('native_qdrant_executor','ci-native-qdrant-tenant')",
    ).execute(&pg.pool).await.unwrap();
    sqlx::query("GRANT USAGE ON SCHEMA pipeline TO native_qdrant_operator,native_qdrant_executor")
        .execute(&pg.pool).await.unwrap();
    sqlx::query(
        "GRANT SELECT ON pipeline.execution_runs TO native_qdrant_operator",
    ).execute(&pg.pool).await.unwrap();
    sqlx::query(
        "GRANT INSERT ON pipeline.qdrant_projection_v1_admissions TO native_qdrant_operator",
    ).execute(&pg.pool).await.unwrap();
    // Recovery is an *explicit* DBA grant, never SELECT on the ledger.
    sqlx::query(
        "GRANT EXECUTE ON FUNCTION pipeline.verify_qdrant_operator_resignal_v1(
            UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BIGINT,BYTEA,BYTEA)
         TO native_qdrant_operator",
    ).execute(&pg.pool).await.unwrap();
    sqlx::query(
        "GRANT SELECT ON pipeline.qdrant_projection_v1_admissions TO native_qdrant_executor",
    ).execute(&pg.pool).await.unwrap();
    sqlx::query(
        "GRANT EXECUTE ON FUNCTION pipeline.begin_qdrant_projection_v1(UUID,BYTEA),
             pipeline.confirm_qdrant_projection_v1(UUID,BYTEA) TO native_qdrant_executor",
    ).execute(&pg.pool).await.unwrap();

    let operator_url = pg.database_url.replace("postgres:postgres_password",
        "native_qdrant_operator:native_operator_password");
    let worker_url = pg.database_url.replace("postgres:postgres_password",
        "native_qdrant_executor:native_executor_password");
    let operator_secret = secret("qdrant-v7-operator",&operator_url);
    let worker_secret = secret("qdrant-v7-worker",&worker_url);

    let parent = Command::new(&parent_bin)
        .env("ALLOW_PURE_DAG_ORCHESTRATOR", "false")
        .env("ALLOW_EFFECT_DAG_ORCHESTRATOR_V4", "false")
        .env("ALLOW_EFFECT_DAG_ORCHESTRATOR_V5", "false")
        .env("ALLOW_QDRANT_DAG_ORCHESTRATOR_V1", "true")
        .env("TEMPORAL_ORCHESTRATOR_TASK_QUEUE",QDRANT_DAG_PARENT_QUEUE_V1)
        .env("WORKER_BUILD_ID", "cccccccccccccccccccccccccccccccccccccccc")
        .env("TEMPORAL_URL",&temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE",&temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("native provider-free Qdrant parent Worker");
    let _parent_guard = ChildGuard(Some(parent));
    let child = Command::new(&qdrant_bin)
        .env("ALEGRIA_ENABLE_QDRANT_EFFECT_V1","true")
        .env("ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V1","1")
        .env("QDRANT_EFFECT_WORKER_DATABASE_URL_FILE",&worker_secret)
        .env("QDRANT_URL",&qdrant.qdrant_url)
        .env("QDRANT_REST_URL",&qdrant.qdrant_rest_url)
        .env("QDRANT_SKIP_COMPATIBILITY_CHECK","true")
        .env("TEMPORAL_URL",&temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE",&temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("separate native Qdrant child+Activity Worker");
    let mut child_guard = ChildGuard(Some(child));

    let command = QdrantKeywordProjectionCommandV1 {
        schema_revision: 1, admission_id: admission,
        tenant_id: tenant.into(), context_key: context.clone(),
        keyword_key: keyword.clone(), vector: vec![0.123_f32;1024],
    };
    let input = QdrantProjectionRunV1 {
        schema_revision:1, canonical_run_id:run_id,
        tenant_id:tenant.into(), context_key:context.clone(),
        commands:vec![command.clone()],
    };
    input.preflight().unwrap();
    let client = temporal_gateway::connect_client(&temporal.temporal_url,
        format!("ci-v7-qdrant-{}", Uuid::new_v4()),&temporal.namespace).await.unwrap();
    let handle = client.start_workflow(
        UntypedWorkflow::new(QDRANT_DAG_PARENT_TYPE_V1),
        RawValue::from_value(&input,&PayloadConverter::serde_json()),
        WorkflowStartOptions::new(QDRANT_DAG_PARENT_QUEUE_V1,run_id.to_string()).build(),
    ).await.expect("start actual native Temporal parent");
    let parent_run = handle.info().run_id.clone().expect("server-issued parent Run ID");
    sqlx::query("UPDATE pipeline.execution_runs SET workflow_run_id=$1 WHERE run_id=$2")
        .bind(&parent_run).bind(run_id).execute(&pg.pool).await.unwrap();

    let (child_id,child_run) = timeout(Duration::from_secs(60),async {
        loop {
            let events = handle.fetch_history(WorkflowFetchHistoryOptions::builder().build())
                .into_events().await.expect("real parent Temporal History");
            let child = events.iter().find_map(|event| {
                let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs)) =
                    &event.attributes else {return None;};
                attrs.workflow_execution.as_ref().map(|execution|
                    (execution.workflow_id.clone(),execution.run_id.clone()))
            });
            if let Some(child) = child { break child; }
            sleep(Duration::from_millis(200)).await;
        }
    }).await.expect("native Qdrant child never started");
    assert_eq!(child_id,format!("alegria-qdrant-effect-v1-{admission}"));
    let child_handle = WorkflowExecutionInfo::builder()
        .namespace(temporal.namespace.clone())
        .workflow_id(child_id.clone())
        .run_id(child_run.clone())
        .build()
        .bind_untyped(client.clone());
    let before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v1_attempts WHERE admission_id=$1",
    ).bind(admission).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(before,0,"native child must remain blocked without operator History proof");

    // An untrusted caller may know the child locator and send an early
    // operator-shaped Signal. The child must inspect SQL read-only, decline
    // this wake-up, and remain alive for a subsequent legitimate admission.
    for _ in 0..3 {
        child_handle.signal(
            UntypedSignal::<UntypedWorkflow>::new("operator_admission_ready_qdrant_v1"),
            RawValue::default(), WorkflowSignalOptions::default(),
        ).await.expect("send deliberately forged pre-admission wake-up");
    }
    timeout(Duration::from_secs(30),async {
        loop {
            let events = child_handle
                .fetch_history(WorkflowFetchHistoryOptions::builder().build())
                .into_events().await.expect("read child History after forged wake-up");
            assert!(!events.iter().any(|event| matches!(
                event.attributes.as_ref(),
                Some(Attributes::WorkflowExecutionFailedEventAttributes(_))
            )), "forged pre-admission Signal permanently poisoned the child Workflow");
            if events.iter().any(|event| matches!(
                event.attributes.as_ref(),
                Some(Attributes::ActivityTaskCompletedEventAttributes(_))
            )) {
                break;
            }
            sleep(Duration::from_millis(150)).await;
        }
    }).await.expect("forged Signal's read-only SQL admission check never completed");
    assert_eq!(sqlx::query_scalar::<_,i64>(
        "SELECT count(*) FROM pipeline.qdrant_projection_v1_attempts WHERE admission_id=$1",
    ).bind(admission).fetch_one(&pg.pool).await.unwrap(),0,
        "untrusted Signal must not initiate any Qdrant provider RPC");

    // The operator file contains only locators. First tamper an independently
    // server-issued child Run ID; it must fail with ZERO SQL admissions.
    let locator = serde_json::json!({
        "admission_id":admission,"canonical_run_id":run_id,
        "parent_workflow_id":run_id.to_string(),"parent_workflow_run_id":parent_run,
        "child_workflow_id":child_id,"child_workflow_run_id":child_run,
        "expires_in_seconds":180,
    });
    let operator_request = secret("qdrant-v7-request",
        &serde_json::to_string(&locator).unwrap());
    let wrong_run = secret("qdrant-v7-forged-request",
        &serde_json::to_string(&serde_json::json!({
            "admission_id":admission,"canonical_run_id":run_id,
            "parent_workflow_id":run_id.to_string(),"parent_workflow_run_id":parent_run,
            "child_workflow_id":child_id,"child_workflow_run_id":Uuid::new_v4().to_string(),
            "expires_in_seconds":180,
        })).unwrap());
    let mut operator_process = || Command::new(&qdrant_bin);
    let bad = operator_process().arg("--operator-issue").arg(&wrong_run)
        .env("ALEGRIA_ENABLE_QDRANT_EFFECT_OPERATOR_V1","true")
        .env("QDRANT_EFFECT_OPERATOR_DATABASE_URL_FILE",&operator_secret)
        .env("QDRANT_EFFECT_OPERATOR_TENANT_ID",tenant)
        .env("QDRANT_EFFECT_OPERATOR_TEMPORAL_URL",&temporal.temporal_url)
        .env("QDRANT_EFFECT_OPERATOR_TEMPORAL_NAMESPACE",&temporal.namespace)
        .output().expect("run independent negative History proof");
    assert!(!bad.status.success(),"forged child Run ID bypassed independent History");
    assert_eq!(sqlx::query_scalar::<_,i64>(
        "SELECT count(*) FROM pipeline.qdrant_projection_v1_admissions WHERE admission_id=$1",
    ).bind(admission).fetch_one(&pg.pool).await.unwrap(),0);

    // A wakeup without an existing committed row must never become authority,
    // even with real reciprocal server Histories and a privileged operator.
    let before_commit_recovery = operator_process().arg("--operator-resignal")
        .arg(&operator_request)
        .env("ALEGRIA_ENABLE_QDRANT_EFFECT_OPERATOR_V1","true")
        .env("QDRANT_EFFECT_OPERATOR_DATABASE_URL_FILE",&operator_secret)
        .env("QDRANT_EFFECT_OPERATOR_TENANT_ID",tenant)
        .env("QDRANT_EFFECT_OPERATOR_TEMPORAL_URL",&temporal.temporal_url)
        .env("QDRANT_EFFECT_OPERATOR_TEMPORAL_NAMESPACE",&temporal.namespace)
        .output().expect("reject recovery before an immutable SQL admission exists");
    assert!(!before_commit_recovery.status.success(),
        "recovery accepted an absent operator-issued SQL row");

    // Concurrency: when the operator holds the canonical run-status advisory
    // key, a terminal run transition must lose with SQLSTATE 55P03; it cannot
    // slip between authority validation and INSERT.
    let mut status_lock = pg.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(format!("truth-run-status|{run_id}"))
        .execute(&mut *status_lock).await.unwrap();
    let transition = sqlx::query("UPDATE pipeline.execution_runs SET status='done' WHERE run_id=$1")
        .bind(run_id).execute(&pg.pool).await.expect_err("status transition bypassed operator's run lock");
    assert_eq!(transition.as_database_error().and_then(|err|err.code()).as_deref(),
        Some("55P03"));
    status_lock.rollback().await.unwrap();

    // Force a real Worker crash with a read-only admission Activity STARTED.
    // A schedule-to-close-only 300s timeout used to strand the child after
    // restart, despite committed admission and repeated correct Signals.
    // The short start-to-close bound on this *read-only* Activity must ensure
    // Temporal retries it; the provider Activity keeps its distinct fence.
    child_handle.signal(
        UntypedSignal::<UntypedWorkflow>::new("operator_admission_ready_qdrant_v1"),
        RawValue::default(), WorkflowSignalOptions::default(),
    ).await.expect("trigger an in-flight pre-admission read-only probe");
    timeout(Duration::from_secs(12),async {
        loop {
            let events = child_handle
                .fetch_history(WorkflowFetchHistoryOptions::builder().build())
                .into_events().await.expect("inspect real pre-crash child History");
            let last_started = events.iter().filter_map(|event| matches!(
                event.attributes.as_ref(),
                Some(Attributes::ActivityTaskStartedEventAttributes(_))
            ).then_some(event.event_id)).last().unwrap_or(0);
            let last_terminal = events.iter().filter_map(|event| matches!(
                event.attributes.as_ref(),
                Some(Attributes::ActivityTaskCompletedEventAttributes(_))
                    | Some(Attributes::ActivityTaskFailedEventAttributes(_))
                    | Some(Attributes::ActivityTaskTimedOutEventAttributes(_))
                    | Some(Attributes::ActivityTaskCanceledEventAttributes(_))
            ).then_some(event.event_id)).last().unwrap_or(0);
            if last_started > last_terminal { break; }
            sleep(Duration::from_millis(50)).await;
        }
    }).await.expect("no in-flight read-only Activity before forced Worker crash");

    // Both real admission Signals enter server History while this worker is
    // down. Neither may initiate any provider RPC before recovery.
    let mut stopped = child_guard.0.take().expect("running Qdrant worker process");
    stopped.kill().expect("stop isolated Qdrant child worker");
    stopped.wait().expect("drain stopped Qdrant child worker");

    let issued = operator_process().arg("--operator-issue").arg(&operator_request)
        .env("ALEGRIA_ENABLE_QDRANT_EFFECT_OPERATOR_V1","true")
        .env("QDRANT_EFFECT_OPERATOR_DATABASE_URL_FILE",&operator_secret)
        .env("QDRANT_EFFECT_OPERATOR_TENANT_ID",tenant)
        .env("QDRANT_EFFECT_OPERATOR_TEMPORAL_URL",&temporal.temporal_url)
        .env("QDRANT_EFFECT_OPERATOR_TEMPORAL_NAMESPACE",&temporal.namespace)
        .output().expect("run separately credentialed operator History proof");
    assert!(issued.status.success(),"real Temporal operator denied: {}",
        String::from_utf8_lossy(&issued.stderr));

    // A DBA must *separately* authorize the narrow read-only recovery helper.
    // INSERT privilege on admissions must not silently grant its EXECUTE.
    sqlx::query(
        "REVOKE EXECUTE ON FUNCTION pipeline.verify_qdrant_operator_resignal_v1(
            UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BIGINT,BYTEA,BYTEA)
         FROM native_qdrant_operator",
    ).execute(&pg.pool).await.unwrap();
    let missing_recovery_grant = operator_process().arg("--operator-resignal")
        .arg(&operator_request)
        .env("ALEGRIA_ENABLE_QDRANT_EFFECT_OPERATOR_V1","true")
        .env("QDRANT_EFFECT_OPERATOR_DATABASE_URL_FILE",&operator_secret)
        .env("QDRANT_EFFECT_OPERATOR_TENANT_ID",tenant)
        .env("QDRANT_EFFECT_OPERATOR_TEMPORAL_URL",&temporal.temporal_url)
        .env("QDRANT_EFFECT_OPERATOR_TEMPORAL_NAMESPACE",&temporal.namespace)
        .output().expect("recovery without opt-in EXECUTE grant");
    assert!(!missing_recovery_grant.status.success(),
        "operator INSERT login unexpectedly had recovery EXECUTE permission");
    sqlx::query(
        "GRANT EXECUTE ON FUNCTION pipeline.verify_qdrant_operator_resignal_v1(
            UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BIGINT,BYTEA,BYTEA)
         TO native_qdrant_operator",
    ).execute(&pg.pool).await.unwrap();

    let resignal = operator_process().arg("--operator-resignal").arg(&operator_request)
        .env("ALEGRIA_ENABLE_QDRANT_EFFECT_OPERATOR_V1","true")
        .env("QDRANT_EFFECT_OPERATOR_DATABASE_URL_FILE",&operator_secret)
        .env("QDRANT_EFFECT_OPERATOR_TENANT_ID",tenant)
        .env("QDRANT_EFFECT_OPERATOR_TEMPORAL_URL",&temporal.temporal_url)
        .env("QDRANT_EFFECT_OPERATOR_TEMPORAL_NAMESPACE",&temporal.namespace)
        .output().expect("re-attest real Histories and exactly committed admission");
    assert!(resignal.status.success(),"safe recovery of committed row failed: {}",
        String::from_utf8_lossy(&resignal.stderr));
    let restricted_operator = sqlx::PgPool::connect(&operator_url).await.unwrap();
    let can_read_admission: bool = sqlx::query_scalar(
        "SELECT has_table_privilege(session_user,
            'pipeline.qdrant_projection_v1_admissions','SELECT')",
    ).fetch_one(&restricted_operator).await.unwrap();
    assert!(!can_read_admission,"recoverable operator must not gain admission SELECT");
    let admitted_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v1_admissions WHERE admission_id=$1",
    ).bind(admission).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(admitted_rows,1,"recover path must not insert a second admission");
    let pre_restart_attempts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v1_attempts WHERE admission_id=$1",
    ).bind(admission).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(pre_restart_attempts,0,"re-signaling must never issue a provider RPC itself");

    // A tampered expected command is rejected by the SQL proof even when
    // callers know the admission UUID and have EXECUTE on the helper.
    let parent_history_events = handle
        .fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .into_events().await.unwrap();
    let parent_exact_input_bytes = parent_history_events.first().and_then(|event| {
        let Some(Attributes::WorkflowExecutionStartedEventAttributes(started)) =
            &event.attributes else { return None; };
        started.input.as_ref()?.payloads.first().map(|value| value.data.clone())
    }).expect("exact server-source parent start bytes");
    let child_started_event_id = parent_history_events.iter().find_map(|event| {
            let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(started)) =
                &event.attributes else { return None; };
            (started.workflow_execution.as_ref().map(|v|v.run_id.as_str())
                == Some(child_run.as_str())).then_some(event.event_id)
    }).expect("real parent ChildStarted event ID");
    let tampered = sqlx::query_scalar::<_,bool>(
        "SELECT pipeline.verify_qdrant_operator_resignal_v1(
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    ).bind(admission).bind(run_id).bind(tenant).bind(&context).bind(&keyword)
     .bind(run_id.to_string()).bind(&parent_run).bind(&child_id).bind(&child_run)
     .bind(child_started_event_id).bind(parent_exact_input_bytes)
     .bind(b"forged-command-bytes".as_slice())
     .fetch_one(&restricted_operator).await
     .expect_err("SQL re-signal helper accepted non-History command bytes");
    assert_eq!(tampered.as_database_error().and_then(|err|err.code()).as_deref(),
        Some("AV001"));

    let restarted_child = Command::new(&qdrant_bin)
        .env("ALEGRIA_ENABLE_QDRANT_EFFECT_V1","true")
        .env("ALEGRIA_EXPERIMENTAL_QDRANT_PROJECTION_V1","1")
        .env("QDRANT_EFFECT_WORKER_DATABASE_URL_FILE",&worker_secret)
        .env("QDRANT_URL",&qdrant.qdrant_url)
        .env("QDRANT_REST_URL",&qdrant.qdrant_rest_url)
        .env("QDRANT_SKIP_COMPATIBILITY_CHECK","true")
        .env("TEMPORAL_URL",&temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE",&temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("restart Qdrant child after committed admission and duplicate wakeups");
    let _restarted_child_guard = ChildGuard(Some(restarted_child));

    let completion = match timeout(Duration::from_secs(80),
        handle.get_result(WorkflowGetResultOptions::default())).await {
        Ok(result) => result.expect("native Qdrant parent workflow failed"),
        Err(_) => {
            let history = timeout(Duration::from_secs(5),
                child_handle.fetch_history(WorkflowFetchHistoryOptions::builder().build())
                    .into_events()).await.ok().and_then(Result::ok);
            let history_counts = history.map(|events| {
                let scheduled = events.iter().filter(|event| matches!(
                    event.attributes.as_ref(),
                    Some(Attributes::ActivityTaskScheduledEventAttributes(_))
                )).count();
                let started = events.iter().filter(|event| matches!(
                    event.attributes.as_ref(),
                    Some(Attributes::ActivityTaskStartedEventAttributes(_))
                )).count();
                let timed_out = events.iter().filter(|event| matches!(
                    event.attributes.as_ref(),
                    Some(Attributes::ActivityTaskTimedOutEventAttributes(_))
                )).count();
                format!("scheduled={scheduled},started={started},timed_out={timed_out}")
            }).unwrap_or_else(||"History unavailable".to_owned());
            let attempts = sqlx::query_scalar::<_,i64>(
                "SELECT count(*) FROM pipeline.qdrant_projection_v1_attempts WHERE admission_id=$1"
            ).bind(admission).fetch_one(&pg.pool).await;
            let receipts = sqlx::query_scalar::<_,i64>(
                "SELECT count(*) FROM pipeline.qdrant_projection_v1_receipts WHERE admission_id=$1"
            ).bind(admission).fetch_one(&pg.pool).await;
            panic!("native Qdrant parent did not complete after real operator admission and Worker restart: child_history={history_counts}; SQL attempts={attempts:?}, receipts={receipts:?}");
        }
    };
    let json: String = completion.to_value(&PayloadConverter::serde_json());
    let results: Vec<operation_protocol::QdrantKeywordChildResultV1> =
        serde_json::from_str(&json).unwrap();
    assert_eq!(results.len(),1);
    assert_eq!(results[0].admission_id,admission);

    let completed_child_history = child_handle
        .fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .into_events().await.expect("inspect restarted native child History");
    assert!(completed_child_history.iter().any(|event| matches!(
        event.attributes.as_ref(),
        Some(Attributes::ActivityTaskTimedOutEventAttributes(_))
    )), "crashed in-flight admission Activity did not cross the start-to-close retry fence");

    // Replay the ORIGINAL server-issued parent and child Histories using
    // independent native SDK replay binaries. Synthetic parser fixtures
    // cannot prove deterministic Workflow recovery at this new owner seam.
    for (source, replay_binary, replay_flag, marker) in [
        (&handle, &parent_bin, "--replay-qdrant-effect-history",
            "CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS"),
        (&child_handle, &qdrant_bin, "--replay-history",
            "CI_TEMPORAL_QDRANT_EFFECT_REPLAY=PASS"),
    ] {
        let server_history = source
            .fetch_history(WorkflowFetchHistoryOptions::builder().build())
            .to_json().await.expect("fetch full authentic server Temporal History");
        assert!(server_history.len() > 100, "authentic Temporal History missing");
        let history_file = env::temp_dir().join(format!(
            "alegria-v7-native-history-{}-{}.json", std::process::id(), Uuid::new_v4()
        ));
        fs::write(&history_file,&server_history).unwrap();
        let replay = Command::new(replay_binary)
            .arg(replay_flag).arg(&history_file)
            .env_remove("DATABASE_URL")
            .env_remove("QDRANT_EFFECT_WORKER_DATABASE_URL")
            .env_remove("QDRANT_EFFECT_WORKER_DATABASE_URL_FILE")
            .env_remove("QDRANT_EFFECT_OPERATOR_DATABASE_URL_FILE")
            .output().expect("execute native offline Temporal SDK replay");
        fs::remove_file(history_file).unwrap();
        assert!(replay.status.success(),
            "real V7 Temporal History replay failed: stdout={} stderr={}",
            String::from_utf8_lossy(&replay.stdout),
            String::from_utf8_lossy(&replay.stderr));
        assert!(String::from_utf8_lossy(&replay.stdout).contains(marker),
            "native replay exited without exact Workflow History completion marker");
    }
    let durable: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v1_receipts
         WHERE admission_id=$1 AND exact_provider_readback=TRUE",
    ).bind(admission).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(durable,1,"native Temporal child returned without real SQL receipt");

    let sha: String = sqlx::query_scalar(
        "SELECT command_sha256 FROM pipeline.qdrant_projection_v1_admissions WHERE admission_id=$1",
    ).bind(admission).fetch_one(&pg.pool).await.unwrap();
    let metadata = serde_json::json!({
        "tenant_id":tenant,"context_key":context,"keyword_key":keyword,
        "operator_admission_id":admission.to_string(),"approved_command_sha256":sha,
    });
    assert!(vector_qdrant::projection_commands::projection_readback_exact(
        "seo_keyword_clusters_4",&admission.to_string(),"keyword_cluster",
        &format!("tenant/{tenant}/context/{context}/keyword/{keyword}"),
        &command.vector,&metadata,
    ).await.expect("real Qdrant REST readback after native Temporal child"));
    let via_grpc = vector_qdrant::search_dense(&qdrant.client,
        "seo_keyword_clusters_4",command.vector.clone(),10,None).await.unwrap();
    assert!(via_grpc.iter().any(|point| {
        vector_qdrant::scored_point_payload_map(point).get("operator_admission_id")
            == Some(&admission.to_string())
    }),"real Temporal child Qdrant effect absent from independent gRPC retrieval");

    // Opposite side of the run-status race: a separate, exact tenant-bound
    // admission exists but NO provider RPC has started when the canonical
    // run becomes terminal. SQL must veto begin and leave zero attempts.
    // This manually seeded *negative-only* row is not treated as independently
    // Temporal-attested; the positive effect above used the real operator.
    let not_started = Uuid::new_v4();
    let negative_command = QdrantKeywordProjectionCommandV1 {
        schema_revision:1, admission_id:not_started,
        tenant_id:tenant.into(), context_key:context.clone(),
        keyword_key:format!("terminal-denied-{not_started}"),
        vector:vec![0.21_f32;1024],
    };
    let negative_bytes = serde_json::to_vec(&negative_command).unwrap();
    sqlx::query(
        "INSERT INTO pipeline.qdrant_projection_v1_admissions
           (admission_id,run_id,tenant_id,context_key,keyword_key,
            parent_workflow_id,parent_workflow_run_id,child_workflow_id,child_workflow_run_id,
            child_started_event_id,operation_instance,plan_sha256,command_sha256,
            command_bytes,expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,'ci-terminal-negative-child','ci-terminal-negative-run',
                 19,'terminal-negative',repeat('d',64),
                 encode(public.digest($8::bytea,'sha256'),'hex'),$8,
                 clock_timestamp()+interval '3 minutes')",
    ).bind(not_started).bind(run_id).bind(tenant).bind(&context)
     .bind(&negative_command.keyword_key).bind(run_id.to_string())
     .bind(&parent_run).bind(&negative_bytes)
     .execute(&restricted_operator).await.unwrap();
    sqlx::query("UPDATE pipeline.execution_runs SET status='done' WHERE run_id=$1")
        .bind(run_id).execute(&pg.pool).await
        .expect("terminal transition after completed first effect");
    let restricted_worker = sqlx::PgPool::connect(&worker_url).await.unwrap();
    let terminal_error = sqlx::query_scalar::<_,String>(
        "SELECT pipeline.begin_qdrant_projection_v1($1,$2)",
    ).bind(not_started).bind(&negative_bytes)
     .fetch_one(&restricted_worker).await
     .expect_err("terminal canonical run must not begin a newly admitted Qdrant effect");
    assert_eq!(terminal_error.as_database_error().and_then(|err|err.code()).as_deref(),
        Some("AV001"));
    let mut stale_snapshot = restricted_worker.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *stale_snapshot).await.unwrap();
    let snapshot_error = sqlx::query_scalar::<_,String>(
        "SELECT pipeline.begin_qdrant_projection_v1($1,$2)",
    ).bind(not_started).bind(&negative_bytes)
     .fetch_one(&mut *stale_snapshot).await
     .expect_err("REPEATABLE READ could reuse a pre-terminal snapshot to start a Qdrant RPC");
    assert_eq!(snapshot_error.as_database_error().and_then(|err|err.code()).as_deref(),
        Some("AV001"));
    stale_snapshot.rollback().await.unwrap();
    let negative_attempts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pipeline.qdrant_projection_v1_attempts WHERE admission_id=$1",
    ).bind(not_started).fetch_one(&pg.pool).await.unwrap();
    assert_eq!(negative_attempts,0,"terminal run cannot leave a Qdrant RPC attempt barrier");
    for file in [operator_secret,worker_secret,operator_request,wrong_run] {
        fs::remove_file(file).unwrap();
    }
    eprintln!("CI_QDRANT_V8_NATIVE_TEMPORAL_HISTORY_OPERATOR_RECOVERY_REAL_EFFECT=PASS");
}
