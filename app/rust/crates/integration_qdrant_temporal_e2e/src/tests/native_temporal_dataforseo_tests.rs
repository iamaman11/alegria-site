// Physical DataForSEO **stub-only** owner proof. The generic V2 parent, a
// separate operator LOGIN, the child Activity LOGIN, PostgreSQL's immutable
// attempt ledger and a real TCP POST/GET server all run in this test. No
// credential or public/paid provider endpoint is available to the worker.

#[cfg(feature = "e2e")]
fn dataforseo_v2_fixture(
    run_id: Uuid,
    tenant: &str,
    keyword: &str,
    build_id: &str,
) -> (
    operation_protocol::OperationDagRunV6,
    operation_protocol::TrustedRouteCatalog,
) {
    use dataforseo_contracts::{
        DataForSeoGoogleOrganicSearchInputV1, DataForSeoGoogleOrganicSearchV1,
        DATAFORSEO_OWNER_QUEUE_V1, INPUT_PORT, OUTPUT_PORT, SCHEMA_REVISION,
    };
    use operation_protocol::{
        AdmittedArtifactV1, ArtifactBinding, ContractCatalog, ExecutionPlan, ExternalArtifact,
        Operation, OperationDagRunV6, OperationNode, OwnerCommandWireV1, PinnedOwnerRouteV1,
        PlanHash, RunScope, TrustedRouteCatalog, OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
        OWNER_OPERATION_CHILD_TYPE_V1, PLAN_SCHEMA_REVISION,
    };
    use std::collections::{BTreeMap, BTreeSet};

    let contract = DataForSeoGoogleOrganicSearchV1::contract();
    let scope = RunScope {
        tenant_id: tenant.to_owned(),
        run_id: run_id.to_string(),
    };
    let input = DataForSeoGoogleOrganicSearchInputV1 {
        schema_revision: SCHEMA_REVISION,
        scope: scope.clone(),
        keyword: keyword.to_owned(),
        language_code: "en".into(),
        location_code: 2840,
        depth: 5,
        max_cost_usd_micros: 100_000,
    };
    let bytes = serde_json::to_vec(&input).unwrap();
    let mut catalog = ContractCatalog::default();
    catalog
        .register::<DataForSeoGoogleOrganicSearchV1>()
        .unwrap();
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: scope.clone(),
        allowed_effects: contract.effects.clone(),
        external_artifacts: BTreeMap::from([(
            "external_serp_query".into(),
            ExternalArtifact {
                schema: contract.inputs[&INPUT_PORT.into()].clone(),
                scope: scope.clone(),
                provenance: "ci-independent-stub-query".into(),
            },
        )]),
        pinned_contracts: Vec::new(),
        nodes: vec![OperationNode {
            instance: "serp_search".into(),
            operation: contract.reference.clone(),
            inputs: BTreeMap::from([(
                INPUT_PORT.into(),
                ArtifactBinding::External {
                    artifact: "external_serp_query".into(),
                },
            )]),
            outputs: BTreeMap::from([(OUTPUT_PORT.into(), "serp_discovery".into())]),
            after: BTreeSet::new(),
            permitted_effects: contract.effects.clone(),
        }],
        hash: PlanHash(String::new()),
    }
    .seal(&catalog)
    .unwrap();
    let route = PinnedOwnerRouteV1 {
        operation: contract.reference.clone(),
        owner: contract.owner.clone(),
        contract_hash: contract.fingerprint().unwrap(),
        workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
        task_queue: DATAFORSEO_OWNER_QUEUE_V1.into(),
        build_id: build_id.into(),
    };
    let mut routes = TrustedRouteCatalog::default();
    routes.register(contract.clone(), route).unwrap();
    let run = OperationDagRunV6 {
        schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
        admitted: BTreeMap::from([(
            "external_serp_query".into(),
            AdmittedArtifactV1 {
                schema: contract.inputs[&INPUT_PORT.into()].clone(),
                scope,
                provenance: "ci-independent-stub-query".into(),
                payload: bytes.clone(),
            },
        )]),
        owner_commands: BTreeMap::from([(
            "serp_search".into(),
            OwnerCommandWireV1 {
                operation: contract.reference.clone(),
                owner: contract.owner.clone(),
                contract_hash: contract.fingerprint().unwrap(),
                command_bytes: bytes,
            },
        )]),
        pinned_routes: routes.trusted_snapshot_for([&contract.reference]).unwrap(),
        plan,
    };
    routes.validate_run_routes(&run).unwrap();
    (run, routes)
}

#[cfg(feature = "e2e")]
struct DataForSeoNativeRoles {
    operator_secret: PathBuf,
    worker_secret: PathBuf,
    tenant: String,
}

#[cfg(feature = "e2e")]
impl Drop for DataForSeoNativeRoles {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.operator_secret);
        let _ = fs::remove_file(&self.worker_secret);
    }
}

#[cfg(feature = "e2e")]
async fn dataforseo_native_roles(pg: &PostgresHarness, tenant: &str) -> DataForSeoNativeRoles {
    let nonce = Uuid::new_v4().simple().to_string();
    let issuer = format!("dfs_issuer_{nonce}");
    let worker = format!("dfs_worker_{nonce}");
    const PASSWORD: &str = "disposable_dataforseo_owner_password";
    // Only fixed ASCII prefixes and UUID hex enter these disposable DDL names.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {issuer} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {worker} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {issuer},{worker};
         GRANT USAGE ON SCHEMA pipeline TO {issuer},{worker};
         GRANT SELECT ON pipeline.operation_dag_business_runs_v1 TO {issuer};
         GRANT INSERT ON pipeline.dataforseo_owner_admissions_v1 TO {issuer};
         GRANT EXECUTE ON FUNCTION pipeline.operation_dag_governance_matches_v2(
             UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BOOLEAN) TO {issuer};
         GRANT EXECUTE ON FUNCTION pipeline.lookup_dataforseo_owner_admission_v1(
             TEXT,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,BIGINT),
             pipeline.begin_dataforseo_owner_v1(UUID,BYTEA,TEXT,BIGINT),
             pipeline.record_stub_dataforseo_task_v1(UUID,BYTEA,TEXT,TEXT),
             pipeline.lookup_stub_dataforseo_task_v1(UUID,BYTEA,TEXT),
             pipeline.confirm_dataforseo_owner_v1(UUID,BYTEA,TEXT,TEXT,BYTEA,BYTEA,BIGINT),
             pipeline.read_confirmed_dataforseo_owner_receipt_v1(UUID,BYTEA,TEXT)
           TO {worker};"
    )))
    .execute(&pg.pool)
    .await
    .expect("create two independent DataForSEO LOGINs");
    for role in [&issuer, &worker] {
        sqlx::query(
            "INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id) VALUES ($1::name,$2)",
        )
        .bind(role)
        .bind(tenant)
        .execute(&pg.pool)
        .await
        .unwrap();
    }
    let url = |role: &str| {
        let prefix = "postgres://postgres:postgres_password@";
        assert!(pg.database_url.starts_with(prefix));
        pg.database_url
            .replacen(prefix, &format!("postgres://{role}:{PASSWORD}@"), 1)
    };
    DataForSeoNativeRoles {
        operator_secret: secret("ci-dataforseo-independent-issuer", &url(&issuer)),
        worker_secret: secret("ci-dataforseo-effect-worker", &url(&worker)),
        tenant: tenant.into(),
    }
}

#[cfg(feature = "e2e")]
struct DataForSeoHttpFixture {
    endpoint: String,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    allow_resume: std::sync::Arc<std::sync::atomic::AtomicBool>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(feature = "e2e")]
impl DataForSeoHttpFixture {
    fn start() -> Self {
        use std::io::{Read, Write};
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let logged = requests.clone();
        let allow_resume = Arc::new(AtomicBool::new(false));
        let release = allow_resume.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut get_count = 0;
            while !stopping.load(Ordering::SeqCst) {
                let (mut socket, address) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(15));
                        continue;
                    }
                    Err(error) => panic!("loopback fixture accept: {error}"),
                };
                assert!(address.ip().is_loopback());
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut raw = Vec::new();
                let mut buf = [0_u8; 2048];
                let header_end = loop {
                    let n = socket.read(&mut buf).unwrap();
                    assert!(
                        n > 0 && raw.len() + n < 8192,
                        "bounded fixture HTTP request"
                    );
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(pos) = raw.windows(4).position(|part| part == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let length = String::from_utf8_lossy(&raw[..header_end])
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|size| size.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                while raw.len() - header_end < length {
                    let n = socket.read(&mut buf).unwrap();
                    assert!(n > 0 && raw.len() + n < 8192);
                    raw.extend_from_slice(&buf[..n]);
                }
                let request = String::from_utf8(raw).unwrap();
                logged.lock().unwrap().push(request.clone());
                let path = request.lines().next().unwrap_or("");
                if path.starts_with("POST /v3/serp/google/organic/task_post ") {
                    if request.contains("ambiguous owner task") {
                        // Remote server accepted the connection/request but
                        // the response is lost: the owner must NEVER re-POST.
                        continue;
                    }
                    let body = r#"{"status_code":20000,"tasks":[{"status_code":20100,"id":"stub-serp-71"}]}"#;
                    write_http_fixture_response(&mut socket, "200 OK", body);
                } else if path
                    .starts_with("GET /v3/serp/google/organic/task_get/advanced/stub-serp-71 ")
                {
                    get_count += 1;
                    if get_count == 1 {
                        // Hold the GET while the runner kills the physical
                        // Worker. No ack/receipt can be recorded this way.
                        while !release.load(Ordering::SeqCst) && !stopping.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        continue;
                    }
                    let body = r#"{"status_code":20000,"tasks":[{"status_code":20000,"result":[{"items":[{"type":"organic","rank_absolute":1,"title":"Offline fixture","url":"https://example.org/offline","description":"Test only"}]}]}]}"#;
                    write_http_fixture_response(&mut socket, "200 OK", body);
                } else {
                    write_http_fixture_response(&mut socket, "404 Not Found", "{}");
                }
            }
            fn write_http_fixture_response(
                stream: &mut std::net::TcpStream,
                status: &str,
                body: &str,
            ) {
                let bytes = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(bytes.as_bytes()).unwrap();
            }
        });
        Self {
            endpoint,
            requests,
            allow_resume,
            stop,
            thread: Some(thread),
        }
    }

    fn count(&self, prefix: &str, contains: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.starts_with(prefix) && request.contains(contains))
            .count()
    }
}

#[cfg(feature = "e2e")]
impl Drop for DataForSeoHttpFixture {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        self.allow_resume.store(true, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            handle.join().expect("stub loopback fixture must terminate");
        }
    }
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn dataforseo_native_v2_temporal_sql_http_restart_and_ambiguous_post() {
    use dataforseo_contracts::{DataForSeoGoogleOrganicSearchOutputV1, DataForSeoSourceStatusV1};
    use operation_protocol::{
        operation_dag_parent_workflow_id_v2, OperationDagReceiptV1, OPERATION_DAG_PARENT_QUEUE_V2,
        OPERATION_DAG_PARENT_TYPE_V2,
    };
    use std::sync::atomic::Ordering;

    let pg = PostgresHarness::start()
        .await
        .expect("physical DataForSEO PostgreSQL");
    mixed_v2_install_native_admissions(&pg.pool).await;
    sqlx::raw_sql(include_str!(
        "../../../../../db/migrations/20261127_0001_dataforseo_owner_v1_admission.sql"
    ))
    .execute(&pg.pool)
    .await
    .expect("DataForSEO isolated stub-only SQL admission migration");
    let temporal = TemporalHarness::start()
        .await
        .expect("physical DataForSEO Temporal server");
    let stub = DataForSeoHttpFixture::start();
    let parent_bin = binary("INTEGRATION_EFFECT_DAG_WORKER_BIN");
    let pg_bin = binary("INTEGRATION_PG_EFFECT_WORKER_BIN");
    let dataforseo_bin = binary("INTEGRATION_DATAFORSEO_EFFECT_WORKER_BIN");
    let build_id = env::var("GITHUB_SHA").expect("exact SHA of physical owner binaries");
    assert_eq!(build_id.len(), 40);
    let tenant = "ci-dataforseo-native-tenant";
    let success_id = Uuid::new_v4();
    let ambiguous_id = Uuid::new_v4();
    let success_context = format!("ci-dataforseo-success-{success_id}");
    let ambiguous_context = format!("ci-dataforseo-ambiguous-{ambiguous_id}");
    // The business-run issuer derives tenant authority from the existing
    // canonical context registry, not from the requested Temporal plan.
    for context in [&success_context, &ambiguous_context] {
        sqlx::query(
            "INSERT INTO kb.visa_contexts(context_key,country_code,visa_family,citizenship_code)
             VALUES ($1,'US','tourist','BY')",
        )
        .bind(context)
        .execute(&pg.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id) VALUES ($1,$2)",
        )
        .bind(context)
        .bind(tenant)
        .execute(&pg.pool)
        .await
        .unwrap();
    }
    let (success, success_routes) =
        dataforseo_v2_fixture(success_id, tenant, "reliable owner task", &build_id);
    let (ambiguous, ambiguous_routes) =
        dataforseo_v2_fixture(ambiguous_id, tenant, "ambiguous owner task", &build_id);
    let catalog = mixed_v2_install_ci_trusted_catalog(&[
        (&success, &success_routes),
        (&ambiguous, &ambiguous_routes),
    ]);
    let catalogs = [
        mixed_v6_governance_issuer_catalog(&success, &success_routes),
        mixed_v6_governance_issuer_catalog(&ambiguous, &ambiguous_routes),
    ];
    let base_roles = mixed_v6_native_owner_roles(&pg, tenant).await;
    let roots = mixed_v2_native_roles(&pg, tenant, &base_roles).await;
    let roles = dataforseo_native_roles(&pg, tenant).await;
    let locators = MixedV2PrivateLocatorFiles::new();
    let _parent_guard = ChildGuard(Some(
        Command::new(&parent_bin)
            .env("ALLOW_OPERATION_DAG_V2_WITH_OWNER_VALIDATION", "true")
            .env(
                "TEMPORAL_ORCHESTRATOR_TASK_QUEUE",
                OPERATION_DAG_PARENT_QUEUE_V2,
            )
            .env("TEMPORAL_OPERATION_DAG_CATALOG_PATH", &catalog)
            .env("WORKER_BUILD_ID", &build_id)
            .env("TEMPORAL_URL", &temporal.temporal_url)
            .env("TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .spawn()
            .expect("generic owner-neutral V2 Temporal parent"),
    ));
    let start_worker = || {
        Command::new(&dataforseo_bin)
            .env("ALEGRIA_ENABLE_DATAFORSEO_STUB_OWNER_V1", "true")
            .env("DATAFORSEO_STUB_BASE_URL", &stub.endpoint)
            .env("DATAFORSEO_OWNER_DATABASE_URL_FILE", &roles.worker_secret)
            .env("DATAFORSEO_OWNER_TENANT_ID", &roles.tenant)
            .env("TEMPORAL_URL", &temporal.temporal_url)
            .env("TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .spawn()
            .expect("independent DataForSEO Temporal Child/Activity worker")
    };
    let mut worker = ChildGuard(Some(start_worker()));
    let client = connect_client(
        &temporal.temporal_url,
        format!("ci-dataforseo-{}", Uuid::new_v4()),
        &temporal.namespace,
    )
    .await
    .expect("Temporal client");

    for (case, business_id, run, context, issuer_catalog) in [
        (
            "success",
            success_id,
            &success,
            &success_context,
            &catalogs[0],
        ),
        (
            "ambiguous",
            ambiguous_id,
            &ambiguous,
            &ambiguous_context,
            &catalogs[1],
        ),
    ] {
        let admission_id = Uuid::new_v4();
        let parent_id = operation_dag_parent_workflow_id_v2(&business_id.to_string());
        let parent = client
            .start_workflow(
                UntypedWorkflow::new(OPERATION_DAG_PARENT_TYPE_V2),
                RawValue::from_value(run, &PayloadConverter::serde_json()),
                WorkflowStartOptions::new(OPERATION_DAG_PARENT_QUEUE_V2, parent_id.clone()).build(),
            )
            .await
            .expect("start genuine V2 DataForSEO DAG parent");
        let parent_run_id = parent
            .info()
            .run_id
            .clone()
            .expect("server-issued parent run ID");
        mixed_v2_authorize_business_root(&roots, run, context, &parent_id).await;
        let root_locator = locators.request(
            "dataforseo-governance",
            &serde_json::json!({
                "canonical_run_id": business_id, "context_key": context,
                "parent_workflow_id": parent_id, "parent_workflow_run_id": parent_run_id,
                "expires_in_seconds": 600
            })
            .to_string(),
        );
        let governance = Command::new(&pg_bin)
            .arg("--operator-admit-dag-run-v2")
            .arg(&root_locator)
            .env(
                "ALEGRIA_ENABLE_OPERATION_DAG_GOVERNANCE_OPERATOR_V2",
                "true",
            )
            .env(
                "OPERATION_DAG_GOVERNANCE_V2_OPERATOR_DATABASE_URL",
                &roots.governance_issuer_url,
            )
            .env("OPERATION_DAG_GOVERNANCE_TENANT_ID", tenant)
            .env(
                "OPERATION_DAG_GOVERNANCE_TEMPORAL_URL",
                &temporal.temporal_url,
            )
            .env(
                "OPERATION_DAG_GOVERNANCE_TEMPORAL_NAMESPACE",
                &temporal.namespace,
            )
            .env(
                "OPERATION_DAG_GOVERNANCE_TRUSTED_CATALOG_FILE",
                issuer_catalog,
            )
            .env_remove("DATABASE_URL")
            .output()
            .unwrap();
        assert!(
            governance.status.success(),
            "{case}: business/History root: {}",
            String::from_utf8_lossy(&governance.stderr)
        );
        let (child_id, child_run_id) = timeout(Duration::from_secs(75), async {
            loop {
                let history = parent
                    .fetch_history(WorkflowFetchHistoryOptions::builder().build())
                    .into_events()
                    .await
                    .unwrap();
                let validated = history
                    .iter()
                    .filter_map(|event| {
                        if let Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(_)) =
                            &event.attributes
                        {
                            Some(event.event_id)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                let effect = history.iter().find_map(|event| {
                    let Some(Attributes::ChildWorkflowExecutionStartedEventAttributes(attrs)) =
                        &event.attributes
                    else {
                        return None;
                    };
                    let execution = attrs.workflow_execution.as_ref()?;
                    execution
                        .workflow_id
                        .starts_with("alegria-owner-operation-v2-")
                        .then_some((
                            execution.workflow_id.clone(),
                            execution.run_id.clone(),
                            event.event_id,
                        ))
                });
                if let Some((id, run_id, event)) = effect {
                    assert_eq!(
                        validated.len(),
                        1,
                        "{case}: one actual typed ValidateOnly Child must complete"
                    );
                    assert!(
                        validated[0] < event,
                        "{case}: owner effect started before ValidateOnly result"
                    );
                    break (id, run_id);
                }
                sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .expect("DataForSEO V2 ValidateOnly and effect Child not observed");

        let make_locator = |child_run: String| {
            locators.request(
                "dataforseo-child",
                &serde_json::json!({
                    "admission_id": admission_id, "canonical_run_id": business_id,
                    "parent_workflow_id": parent_id, "parent_workflow_run_id": parent_run_id,
                    "child_workflow_id": child_id, "child_workflow_run_id": child_run,
                    "expires_in_seconds": 300
                })
                .to_string(),
            )
        };
        let request = make_locator(child_run_id.clone());
        let issuer = |path: &PathBuf, operator_tenant: &str| {
            Command::new(&dataforseo_bin)
                .arg("--issue-stub-admission")
                .arg(path)
                .env("ALEGRIA_ENABLE_DATAFORSEO_STUB_OPERATOR_V1", "true")
                .env("DATAFORSEO_OPERATOR_TRUSTED_BUILD_ID", &build_id)
                .env(
                    "DATAFORSEO_OPERATOR_DATABASE_URL_FILE",
                    &roles.operator_secret,
                )
                .env("DATAFORSEO_OPERATOR_TENANT_ID", operator_tenant)
                .env("DATAFORSEO_OPERATOR_TEMPORAL_URL", &temporal.temporal_url)
                .env(
                    "DATAFORSEO_OPERATOR_TEMPORAL_NAMESPACE",
                    &temporal.namespace,
                )
                .env_remove("DATABASE_URL")
                .output()
                .unwrap()
        };
        let forged = make_locator(Uuid::new_v4().to_string());
        assert!(
            !issuer(&forged, tenant).status.success(),
            "{case}: arbitrary server Child Run ID must not issue admission"
        );
        assert!(
            !issuer(&request, "foreign-unowned-tenant").status.success(),
            "{case}: cross-tenant issuer must not adopt owner authority"
        );
        let before: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pipeline.dataforseo_owner_admissions_v1 WHERE admission_id=$1",
        )
        .bind(admission_id)
        .fetch_one(&pg.pool)
        .await
        .unwrap();
        assert_eq!(before, 0, "{case}: rejected issuer never minted authority");
        let case_keyword = if case == "success" {
            "reliable owner task"
        } else {
            "ambiguous owner task"
        };
        assert_eq!(
            stub.count("POST ", case_keyword),
            0,
            "{case}: no HTTP POST without independent SQL admission"
        );
        let issued = issuer(&request, tenant);
        assert!(
            issued.status.success(),
            "{case}: independent operator: stdout={} stderr={}",
            String::from_utf8_lossy(&issued.stdout),
            String::from_utf8_lossy(&issued.stderr)
        );
        if case == "success" {
            // The accepted POST has already created an immutable task row
            // when the first GET arrives. Kill the real worker before an ACK.
            timeout(Duration::from_secs(70), async {
                loop {
                    if stub.count("GET ", "stub-serp-71") == 1 {
                        break;
                    }
                    sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .expect("DataForSEO stub never attempted its first GET");
            let posted: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pipeline.dataforseo_owner_posted_tasks_v1
                 WHERE admission_id=$1 AND provider_task_id='stub-serp-71'",
            )
            .bind(admission_id)
            .fetch_one(&pg.pool)
            .await
            .unwrap();
            assert_eq!(
                posted, 1,
                "accepted POST task marker must commit before first GET"
            );
            let mut stopped = worker.0.take().unwrap();
            stopped
                .kill()
                .expect("kill physical DataForSEO worker during pending GET");
            stopped.wait().expect("reap killed DataForSEO worker");
            stub.allow_resume.store(true, Ordering::SeqCst);
            worker.0 = Some(start_worker());
            let completion = timeout(
                Duration::from_secs(110),
                parent.get_result(WorkflowGetResultOptions::default()),
            )
            .await
            .expect("DataForSEO GET-only restart did not complete")
            .expect("DataForSEO restarted Temporal parent failed");
            let receipt: OperationDagReceiptV1 =
                completion.to_value(&PayloadConverter::serde_json());
            assert_eq!(receipt.order.len(), 1);
            assert_eq!(receipt.owner_results.len(), 1);
            let payload = &receipt.artifacts[&"serp_discovery".into()].payload;
            let typed: DataForSeoGoogleOrganicSearchOutputV1 =
                serde_json::from_slice(payload).expect("real owner typed discovery output");
            assert_eq!(
                typed.source_status,
                DataForSeoSourceStatusV1::StubFixtureOnly
            );
            assert_eq!(typed.provider_task_id, "stub-serp-71");
            assert_eq!(typed.provider_reported_cost_usd_micros, Some(0));
            assert_eq!(typed.organic_results.len(), 1);
            let row: (i64, i64, i64, Vec<u8>, Vec<u8>) = sqlx::query_as(
                "SELECT (SELECT count(*) FROM pipeline.dataforseo_owner_attempts_v1
                         WHERE admission_id=$1),
                        (SELECT count(*) FROM pipeline.dataforseo_owner_posted_tasks_v1
                         WHERE admission_id=$1),
                        (SELECT count(*) FROM pipeline.dataforseo_owner_receipts_v1
                         WHERE admission_id=$1),
                        result_bytes,receipt_bytes
                   FROM pipeline.dataforseo_owner_receipts_v1 WHERE admission_id=$1",
            )
            .bind(admission_id)
            .fetch_one(&pg.pool)
            .await
            .unwrap();
            assert_eq!((row.0, row.1, row.2), (1, 1, 1));
            assert_eq!(
                &row.3, payload,
                "Temporal typed payload matches immutable SQL bytes"
            );
            let stored: operation_protocol::OwnerResultV1 = serde_json::from_slice(&row.4).unwrap();
            assert_eq!(stored.receipt_bytes, *payload);
            assert_eq!(stub.count("POST ", "reliable owner task"), 1);
            assert!(stub.count("GET ", "stub-serp-71") >= 2);
            eprintln!("CI_DATAFORSEO_NATIVE_TEMPORAL_SQL_HTTP_RESTART_GET_ONLY=PASS");
        } else {
            let failed = timeout(
                Duration::from_secs(90),
                parent.get_result(WorkflowGetResultOptions::default()),
            )
            .await
            .expect("ambiguous DataForSEO POST must terminate");
            assert!(
                failed.is_err(),
                "lost POST ACK cannot mint a fake completed discovery"
            );
            let rows: (i64, i64, i64) = sqlx::query_as(
                "SELECT (SELECT count(*) FROM pipeline.dataforseo_owner_attempts_v1
                         WHERE admission_id=$1),
                        (SELECT count(*) FROM pipeline.dataforseo_owner_posted_tasks_v1
                         WHERE admission_id=$1),
                        (SELECT count(*) FROM pipeline.dataforseo_owner_receipts_v1
                         WHERE admission_id=$1)",
            )
            .bind(admission_id)
            .fetch_one(&pg.pool)
            .await
            .unwrap();
            assert_eq!(
                rows,
                (1, 0, 0),
                "lost POST reply retains immutable uncertain claim"
            );
            // A newly launched Activity would observe 'uncertain', not
            // 'started': the database cannot authorize a second paid-like
            // POST even if a transport or Worker replays this exact command.
            let worker_pool =
                sqlx::PgPool::connect(&fs::read_to_string(&roles.worker_secret).unwrap())
                    .await
                    .expect("independently restricted worker SQL connection");
            let dispatch_key: String = sqlx::query_scalar(
                "SELECT dispatch_execution_key FROM pipeline.dataforseo_owner_admissions_v1
                 WHERE admission_id=$1",
            )
            .bind(admission_id)
            .fetch_one(&pg.pool)
            .await
            .unwrap();
            let claim: String =
                sqlx::query_scalar("SELECT pipeline.begin_dataforseo_owner_v1($1,$2,$3,$4)")
                    .bind(admission_id)
                    .bind(&run.owner_commands[&"serp_search".into()].command_bytes)
                    .bind(dispatch_key)
                    .bind(100_000_i64)
                    .fetch_one(&worker_pool)
                    .await
                    .unwrap();
            assert_eq!(
                claim, "uncertain",
                "exact re-entry must not mint a second POST"
            );
            worker_pool.close().await;
            assert_eq!(stub.count("POST ", "ambiguous owner task"), 1);
            eprintln!("CI_DATAFORSEO_NATIVE_AMBIGUOUS_POST_FAIL_CLOSED=PASS");
        }
    }
}
