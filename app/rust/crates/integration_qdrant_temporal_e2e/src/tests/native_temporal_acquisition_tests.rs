// Exact-source physical proof for both independently selectable Acquisition
// pure operations. This opt-in CI-only suite uses a disposable native Temporal
// server, the real generic V2 parent, root-installed V3 structural catalog,
// independent Acquisition ValidateOnly and owner Child workflows, and SDK
// replay of captured server History. No crawler/SQL/provider effects.

#[cfg(feature = "e2e")]
fn acquisition_native_v2_case(
    contract: operation_protocol::OperationContract,
    payload: Vec<u8>,
    build_id: &str,
) -> (
    operation_protocol::OperationDagRunV6,
    operation_protocol::TrustedRouteCatalog,
) {
    use operation_protocol::{
        AdmittedArtifactV1, ArtifactBinding, ContractCatalog, ExecutionPlan, ExternalArtifact,
        OperationDagRunV6, OperationNode, OwnerCommandWireV1, PinnedOwnerRouteV1, PlanHash,
        RunScope, TrustedRouteCatalog, OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
        OWNER_OPERATION_CHILD_TYPE_V1, PLAN_SCHEMA_REVISION,
    };
    use std::collections::{BTreeMap, BTreeSet};

    let scope = RunScope {
        tenant_id: "ci-native-acquisition-owner".to_owned(),
        run_id: Uuid::new_v4().to_string(),
    };
    let input_schema = contract.inputs[&"input".into()].clone();
    let mut contracts = ContractCatalog::default();
    contracts.register_contract(contract.clone()).unwrap();
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: scope.clone(),
        allowed_effects: BTreeSet::new(),
        external_artifacts: BTreeMap::from([(
            "source".into(),
            ExternalArtifact {
                schema: input_schema.clone(),
                scope: scope.clone(),
                provenance: "ci-unattested-external-source".into(),
            },
        )]),
        pinned_contracts: Vec::new(),
        nodes: vec![OperationNode {
            instance: "one_step".into(),
            operation: contract.reference.clone(),
            inputs: BTreeMap::from([(
                "input".into(),
                ArtifactBinding::External {
                    artifact: "source".into(),
                },
            )]),
            outputs: BTreeMap::from([("output".into(), "result".into())]),
            after: BTreeSet::new(),
            permitted_effects: BTreeSet::new(),
        }],
        hash: PlanHash(String::new()),
    }
    .seal(&contracts)
    .unwrap();
    let mut routes = TrustedRouteCatalog::default();
    routes
        .register(
            contract.clone(),
            PinnedOwnerRouteV1 {
                operation: contract.reference.clone(),
                owner: contract.owner.clone(),
                contract_hash: contract.fingerprint().unwrap(),
                workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
                task_queue: "alegria-acquisition-pure-owner-v1".into(),
                build_id: build_id.into(),
            },
        )
        .unwrap();
    let run = OperationDagRunV6 {
        schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
        admitted: BTreeMap::from([(
            "source".into(),
            AdmittedArtifactV1 {
                schema: input_schema,
                scope,
                provenance: "ci-unattested-external-source".into(),
                payload,
            },
        )]),
        owner_commands: BTreeMap::from([(
            "one_step".into(),
            OwnerCommandWireV1 {
                operation: contract.reference.clone(),
                owner: contract.owner.clone(),
                contract_hash: contract.fingerprint().unwrap(),
                command_bytes: b"{}".to_vec(),
            },
        )]),
        pinned_routes: routes.trusted_snapshot_for([&contract.reference]).unwrap(),
        plan,
    };
    routes.validate_run_routes(&run).unwrap();
    (run, routes)
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn acquisition_native_v2_both_independent_one_step_operations_and_replay() {
    use acquisition::operation_owner_schema_json::acquisition_owner_schemas;
    use acquisition::operations::{
        ClassifySourcePage, ClassifySourcePageInput, DomBlockRelevanceInput, EvaluateDomBlocks,
        StrictDomBlockInput,
    };
    use acquisition::source_relevance::{
        BlockRole, DomBlockRelevanceOutput, PageUtilityClassifierOutput,
    };
    use operation_protocol::{
        operation_dag_parent_workflow_id_v2, Operation, OperationDagReceiptV1,
        OPERATION_DAG_PARENT_QUEUE_V2, OPERATION_DAG_PARENT_TYPE_V2,
    };

    let build_id = env::var("GITHUB_SHA").expect("real worker binary must pin exact CI SHA");
    assert_eq!(build_id.len(), 40);
    let schemas = acquisition_owner_schemas().unwrap();
    assert_eq!(schemas.len(), 2);
    let cases = [
        acquisition_native_v2_case(
            ClassifySourcePage::contract(),
            serde_json::to_vec(&ClassifySourcePageInput {
                url: "https://example.test/directory".into(),
                title: "Public directory".into(),
                raw_text: "All countries".into(),
            })
            .unwrap(),
            &build_id,
        ),
        acquisition_native_v2_case(
            EvaluateDomBlocks::contract(),
            serde_json::to_vec(&DomBlockRelevanceInput {
                blocks: vec![StrictDomBlockInput {
                    dom_block_id: "main".into(),
                    block_role: BlockRole::ContentMain,
                    text: "Fee requirements, documents and appointments".into(),
                }],
            })
            .unwrap(),
            &build_id,
        ),
    ];
    // This production catalog format binds exact owner-issued structural
    // Input/Output/Failure DTO descriptors, not author-supplied JSON schemas.
    let entries = cases
        .iter()
        .flat_map(|(run, routes)| run.pinned_routes.iter().map(move |route| (route, routes)))
        .map(|(route, routes)| {
            let contract = routes.catalog().contract(&route.operation).unwrap();
            let pin = &schemas[&route.operation];
            pin.validate(contract).unwrap();
            serde_json::json!({
                "contract": contract,
                "route": route,
                "parameter_schema": {"type":"object","additionalProperties":false},
                "owner_schema": pin,
            })
        })
        .collect::<Vec<_>>();
    let catalog = SiteLabV2CiRootCatalog::new();
    catalog.install(&serde_json::json!({"schema_revision":3,"entries":entries}));
    let installed = trusted_owner_catalog::load_trusted_owner_bundle(&catalog.0).unwrap();
    assert_eq!(installed.routes.catalog().contracts().count(), 2);
    for (run, _) in &cases {
        let contract = &run.pinned_routes[0].operation;
        installed.verify_caller_schema(contract, &schemas[contract]).unwrap();
    }

    let temporal = TemporalHarness::start()
        .await
        .expect("real disposable Temporal server for Acquisition owner");
    let parent_bin = binary("INTEGRATION_EFFECT_DAG_WORKER_BIN");
    let acquisition_bin = binary("INTEGRATION_ACQUISITION_PURE_OWNER_BIN");
    let _parent = ChildGuard(Some(
        Command::new(&parent_bin)
            .env("ALLOW_OPERATION_DAG_V2_WITH_OWNER_VALIDATION", "true")
            .env("TEMPORAL_ORCHESTRATOR_TASK_QUEUE", OPERATION_DAG_PARENT_QUEUE_V2)
            .env("TEMPORAL_OPERATION_DAG_CATALOG_PATH", &catalog.0)
            .env("WORKER_BUILD_ID", &build_id)
            .env("TEMPORAL_URL", &temporal.temporal_url)
            .env("TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .spawn()
            .expect("provider-free generic V2 Temporal parent"),
    ));
    let _owner = ChildGuard(Some(
        Command::new(&acquisition_bin)
            .env("ALEGRIA_ENABLE_ACQUISITION_PURE_OWNER_V1", "true")
            .env("WORKER_BUILD_ID", &build_id)
            .env("TEMPORAL_URL", &temporal.temporal_url)
            .env("TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .spawn()
            .expect("independent native Acquisition ValidateOnly/Child worker"),
    ));
    let client = connect_client(
        &temporal.temporal_url,
        format!("ci-acquisition-native-v2-{}", Uuid::new_v4()),
        &temporal.namespace,
    )
    .await
    .unwrap();

    for (index, (run, _)) in cases.iter().enumerate() {
        let parent_id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
        let parent = client
            .start_workflow(
                UntypedWorkflow::new(OPERATION_DAG_PARENT_TYPE_V2),
                RawValue::from_value(run, &PayloadConverter::serde_json()),
                WorkflowStartOptions::new(OPERATION_DAG_PARENT_QUEUE_V2, parent_id).build(),
            )
            .await
            .expect("native owner-neutral V2 Start");
        let completed = timeout(
            Duration::from_secs(90),
            parent.get_result(WorkflowGetResultOptions::default()),
        )
        .await
        .expect("Acquisition owner Child did not complete")
        .expect("Acquisition owner Child failed");
        let result: OperationDagReceiptV1 = completed.to_value(&PayloadConverter::serde_json());
        assert_eq!(result.order, vec!["one_step".into()]);
        assert_eq!(result.owner_results.len(), 1);
        assert_eq!(result.artifacts[&"result".into()].scope, run.plan.scope);
        assert_eq!(result.produced_evidence.len(), 1);
        let payload = &result.artifacts[&"result".into()].payload;
        if index == 0 {
            let classification: PageUtilityClassifierOutput =
                serde_json::from_slice(payload).unwrap();
            assert!(!classification.allow_structural_extraction);
        } else {
            let blocks: DomBlockRelevanceOutput = serde_json::from_slice(payload).unwrap();
            assert_eq!(blocks.blocks.len(), 1);
            // Raw positive block scoring is not a page-level extraction grant.
            assert!(blocks.blocks[0].allow_extraction);
        }
        let history = parent
            .fetch_history(WorkflowFetchHistoryOptions::builder().build())
            .into_events()
            .await
            .expect("genuine Temporal parent History");
        let validations = history.iter().filter_map(|event| match &event.attributes {
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                if a.workflow_id.starts_with("alegria-owner-validate-v1-") =>
            {
                Some(event.event_id)
            }
            _ => None,
        }).collect::<Vec<_>>();
        let operations = history.iter().filter_map(|event| match &event.attributes {
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                if a.workflow_id.starts_with("alegria-owner-operation-v2-") =>
            {
                Some((event.event_id, a.workflow_id.as_str()))
            }
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(validations.len(), 1);
        assert_eq!(operations.len(), 1);
        assert!(validations[0] < operations[0].0);
        let child = client.get_workflow_handle::<UntypedWorkflow>(operations[0].1.to_owned());
        let server_history = child
            .fetch_history(WorkflowFetchHistoryOptions::builder().build())
            .to_json()
            .await
            .expect("server-issued Acquisition Child History for native SDK replay");
        assert!(server_history.len() > 100);
        let replay_file = secret("acquisition-owner-server-history", "{}");
        fs::write(&replay_file, server_history).unwrap();
        let replay = Command::new(&acquisition_bin)
            .arg("--replay-history")
            .arg(&replay_file)
            .env("WORKER_BUILD_ID", &build_id)
            .env_remove("DATABASE_URL")
            .output()
            .expect("run exact owner binary SDK History replay");
        fs::remove_file(&replay_file).unwrap();
        assert!(replay.status.success(), "{}", String::from_utf8_lossy(&replay.stderr));
        assert!(String::from_utf8_lossy(&replay.stdout)
            .contains("CI_TEMPORAL_ACQUISITION_PURE_OWNER_REPLAY=PASS"));
    }

    // Negative owner proof: an unknown top-level DTO field is rejected by
    // the real ValidateOnly Child before the first executable owner Child.
    let (invalid, _) = acquisition_native_v2_case(
        ClassifySourcePage::contract(),
        br#"{"url":"https://example.test","title":"Content","raw_text":"ok","owner_override":true}"#.to_vec(),
        &build_id,
    );
    let invalid_id = operation_dag_parent_workflow_id_v2(&invalid.plan.scope.run_id);
    let denied = client.start_workflow(
        UntypedWorkflow::new(OPERATION_DAG_PARENT_TYPE_V2),
        RawValue::from_value(&invalid, &PayloadConverter::serde_json()),
        WorkflowStartOptions::new(OPERATION_DAG_PARENT_QUEUE_V2, invalid_id).build(),
    ).await.expect("start invalid native Acquisition DTO on disposable Temporal");
    let rejected = timeout(
        Duration::from_secs(90),
        denied.get_result(WorkflowGetResultOptions::default()),
    ).await.expect("invalid DTO must be denied, not remain running");
    assert!(rejected.is_err(), "unknown field must fail in owner ValidateOnly");
    let denied_history = denied.fetch_history(
        WorkflowFetchHistoryOptions::builder().build()
    ).into_events().await.unwrap();
    assert_eq!(denied_history.iter().filter(|event| matches!(
        &event.attributes,
        Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
            if a.workflow_id.starts_with("alegria-owner-validate-v1-")
    )).count(), 1);
    assert_eq!(denied_history.iter().filter(|event| matches!(
        &event.attributes,
        Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
            if a.workflow_id.starts_with("alegria-owner-operation-v2-")
    )).count(), 0, "invalid DTO must never reach executable owner Child");
}

/// An operator-only Start needs TWO authorities absent from the direct
/// Temporal Worker E2E above: the already existing independently issued
/// business root and a separately persisted source-issuer record. Neither
/// the private intent nor a user-authored provenance string can mint one.
///
/// This test deliberately simulates issuance from a tenant-attributed raw
/// snapshot; it does not claim a production authenticated HTML-to-DTO
/// transformer, a user session, or published siteArchitect Start capability.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn acquisition_native_v2_operator_refuses_unattested_and_starts_with_source_issuer() {
    use acquisition::operation_owner_schema_json::acquisition_owner_schemas;
    use acquisition::operations::{ClassifySourcePage, ClassifySourcePageInput};
    use acquisition::source_relevance::PageUtilityClassifierOutput;
    use operation_protocol::{
        operation_dag_parent_workflow_id_v2, owner_content_hash, Operation,
        OperationDagReceiptV1, PlanHash, SiteLabExpectedStartV1,
        SITE_LAB_START_PREFLIGHT_SCHEMA_REVISION_V1,
        OPERATION_DAG_PARENT_QUEUE_V2,
    };

    #[derive(serde::Serialize)]
    struct ExactSourceStartIntent<'a> {
        schema_revision: u16,
        run: &'a operation_protocol::OperationDagRunV6,
        context_key: &'a str,
        expected: &'a SiteLabExpectedStartV1,
        expires_at_unix_seconds: u64,
    }
    fn expected_for(run: &operation_protocol::OperationDagRunV6) -> SiteLabExpectedStartV1 {
        SiteLabExpectedStartV1 {
            schema_revision: SITE_LAB_START_PREFLIGHT_SCHEMA_REVISION_V1,
            plan_hash: run.plan.hash.clone(),
            command_hashes: run
                .owner_commands
                .iter()
                .map(|(id, wire)| (id.clone(), owner_content_hash(&wire.command_bytes)))
                .collect(),
            source_payload_hashes: run
                .admitted
                .iter()
                .map(|(id, source)| (id.clone(), owner_content_hash(&source.payload)))
                .collect(),
        }
    }

    let build_id = env::var("GITHUB_SHA").expect("exact binary SHA for native source Start");
    assert_eq!(build_id.len(), 40);
    let tenant = "ci-native-acquisition-owner";
    let context = format!("acq-{}", Uuid::new_v4().simple());
    let record_id = Uuid::new_v4();
    let provenance = format!("source-record-v1:{record_id}");
    let payload = serde_json::to_vec(&ClassifySourcePageInput {
        url: "https://ci-native-source.example.test/directory".into(),
        title: "Captured directory page".into(),
        raw_text: "All countries".into(),
    })
    .unwrap();
    let (mut run, routes) =
        acquisition_native_v2_case(ClassifySourcePage::contract(), payload.clone(), &build_id);
    run.plan
        .external_artifacts
        .get_mut(&"source".into())
        .unwrap()
        .provenance = provenance.clone();
    run.admitted.get_mut(&"source".into()).unwrap().provenance = provenance;
    run.plan.hash = PlanHash(String::new());
    run.plan = run.plan.clone().seal(routes.catalog()).unwrap();
    routes.validate_run_routes(&run).unwrap();
    assert_eq!(run.plan.scope.tenant_id, tenant);

    // Exact root-owned structural catalog: no user-supplied schema or Task
    // Queue may acquire source-issuer/owner authority.
    let contract = routes
        .catalog()
        .contract(&run.pinned_routes[0].operation)
        .unwrap();
    let schemas = acquisition_owner_schemas().unwrap();
    let owner_pin = &schemas[&contract.reference];
    owner_pin.validate(contract).unwrap();
    let catalog = SiteLabV2CiRootCatalog::new();
    catalog.install(&serde_json::json!({
        "schema_revision":3,
        "entries":[{
            "contract":contract,
            "route":run.pinned_routes[0],
            "parameter_schema":{"type":"object","additionalProperties":false},
            "owner_schema":owner_pin
        }]
    }));
    let installed = trusted_owner_catalog::load_trusted_owner_bundle(&catalog.0).unwrap();
    installed
        .verify_caller_schema(&contract.reference, owner_pin)
        .unwrap();

    let pg = PostgresHarness::start()
        .await
        .expect("disposable PostgreSQL source and business issuers");
    mixed_v2_install_native_admissions(&pg.pool).await;
    sqlx::raw_sql(include_str!(
        "../../../../../db/migrations/20261127_0002_operation_dag_external_source_attestation_v1.sql"
    ))
    .execute(&pg.pool)
    .await
    .expect("install independent source attestation migration on disposable PG");

    // The raw snapshot's content digest is independent of the transformed
    // typed JSON payload digest; this test never equates the two.
    let raw_html = "<html><title>Captured directory page</title>All countries</html>";
    let raw_hash = owner_content_hash(raw_html.as_bytes());
    sqlx::query(
        "INSERT INTO kb.visa_contexts(context_key,country_code,visa_family,citizenship_code)
         VALUES ($1,'ES','tourist','BY')",
    )
    .bind(&context)
    .execute(&pg.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id) VALUES ($1,$2)",
    )
    .bind(&context)
    .bind(tenant)
    .execute(&pg.pool)
    .await
    .unwrap();
    let source_page_id: i64 = sqlx::query_scalar(
        "INSERT INTO raw.pages(url,domain,dtype,tenant_id,raw_html,content_hash)
         VALUES ($1,'ci-native-source.example.test','government',$2,$3,$4)
         RETURNING id",
    )
    .bind("https://ci-native-source.example.test/directory")
    .bind(tenant)
    .bind(raw_html)
    .bind(&raw_hash)
    .fetch_one(&pg.pool)
    .await
    .unwrap();

    let nonce = Uuid::new_v4().simple().to_string();
    let business_role = format!("acq_business_{nonce}");
    let source_role = format!("acq_source_{nonce}");
    let reader_role = format!("acq_start_reader_{nonce}");
    const PASSWORD: &str = "disposable_acquisition_source_only";
    // These identifiers are exact ASCII prefixes + generated hex and only
    // reach the container owned by this CI test. Role powers are disjoint.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {business_role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {source_role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         CREATE ROLE {reader_role} LOGIN NOINHERIT PASSWORD '{PASSWORD}';
         GRANT CONNECT ON DATABASE alegria TO {business_role},{source_role},{reader_role};
         GRANT USAGE ON SCHEMA pipeline TO {business_role},{source_role},{reader_role};
         GRANT USAGE ON SCHEMA raw TO {source_role};
         GRANT INSERT ON pipeline.operation_dag_business_runs_v1 TO {business_role};
         GRANT INSERT,UPDATE(revoked_at)
             ON pipeline.operation_dag_external_source_attestations_v1 TO {source_role};
         GRANT SELECT ON raw.pages TO {source_role};
         GRANT SELECT ON pipeline.operation_dag_business_runs_v1,
             pipeline.operation_dag_external_source_attestations_v1 TO {reader_role};"
    )))
    .execute(&pg.pool)
    .await
    .expect("provision independently credentialed business/source/reader roles");
    for role in [&business_role, &source_role, &reader_role] {
        sqlx::query(
            "INSERT INTO pipeline.tenant_role_ownership(role_name,tenant_id) VALUES ($1::name,$2)",
        )
        .bind(role)
        .bind(tenant)
        .execute(&pg.pool)
        .await
        .unwrap();
    }
    let restricted_url = |role: &str| {
        let prefix = "postgres://postgres:postgres_password@";
        assert!(pg.database_url.starts_with(prefix));
        pg.database_url
            .replacen(prefix, &format!("postgres://{role}:{PASSWORD}@"), 1)
    };
    let business_pool = sqlx::PgPool::connect(&restricted_url(&business_role))
        .await
        .unwrap();
    let source_pool = sqlx::PgPool::connect(&restricted_url(&source_role))
        .await
        .unwrap();
    let reader_pool = sqlx::PgPool::connect(&restricted_url(&reader_role))
        .await
        .unwrap();

    let temporal = TemporalHarness::start()
        .await
        .expect("disposable native Temporal for authorized Acquisition Start");
    let parent_bin = binary("INTEGRATION_EFFECT_DAG_WORKER_BIN");
    let acquisition_bin = binary("INTEGRATION_ACQUISITION_PURE_OWNER_BIN");
    let pg_operator_bin = binary("INTEGRATION_PG_EFFECT_WORKER_BIN");
    let _parent = ChildGuard(Some(
        Command::new(&parent_bin)
            .env("ALLOW_OPERATION_DAG_V2_WITH_OWNER_VALIDATION", "true")
            .env("TEMPORAL_ORCHESTRATOR_TASK_QUEUE", OPERATION_DAG_PARENT_QUEUE_V2)
            .env("TEMPORAL_OPERATION_DAG_CATALOG_PATH", &catalog.0)
            .env("WORKER_BUILD_ID", &build_id)
            .env("TEMPORAL_URL", &temporal.temporal_url)
            .env("TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .spawn()
            .expect("native V2 generic parent"),
    ));
    let _owner = ChildGuard(Some(
        Command::new(&acquisition_bin)
            .env("ALEGRIA_ENABLE_ACQUISITION_PURE_OWNER_V1", "true")
            .env("WORKER_BUILD_ID", &build_id)
            .env("TEMPORAL_URL", &temporal.temporal_url)
            .env("TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .spawn()
            .expect("native pure Acquisition owner"),
    ));
    let client = connect_client(
        &temporal.temporal_url,
        format!("ci-acquisition-trusted-source-{}", Uuid::new_v4()),
        &temporal.namespace,
    )
    .await
    .unwrap();

    let private = MixedV2PrivateLocatorFiles::new();
    let reader_dsn = private.request("acq-start-reader-dsn", &restricted_url(&reader_role));
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    let make_intent = |candidate: &operation_protocol::OperationDagRunV6, label: &str| {
        let expected = expected_for(candidate);
        private.request(
            label,
            &serde_json::to_string(&ExactSourceStartIntent {
                schema_revision: 1,
                run: candidate,
                context_key: &context,
                expected: &expected,
                expires_at_unix_seconds: deadline,
            })
            .unwrap(),
        )
    };
    let invoke = |request: &std::path::Path| {
        Command::new(&pg_operator_bin)
            .arg("--operator-start-single-v2")
            .arg(request)
            .env("ALEGRIA_ENABLE_SITE_LAB_SINGLE_V2_START", "true")
            .env("SITE_LAB_SINGLE_V2_TENANT_ID", tenant)
            .env("SITE_LAB_SINGLE_V2_DATABASE_URL_FILE", &reader_dsn)
            .env("SITE_LAB_SINGLE_V2_TEMPORAL_URL", &temporal.temporal_url)
            .env("SITE_LAB_SINGLE_V2_TEMPORAL_NAMESPACE", &temporal.namespace)
            .env("SITE_LAB_SINGLE_V2_TRUSTED_CATALOG_FILE", &catalog.0)
            .env_remove("DATABASE_URL")
            .output()
            .expect("native exact-source single-step operator Start")
    };
    let parent_id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
    let intent_file = make_intent(&run, "acquisition-source-authorized-intent");
    let without_business = invoke(&intent_file);
    assert!(!without_business.status.success());
    assert!(client
        .get_workflow_handle::<UntypedWorkflow>(parent_id.clone())
        .describe(WorkflowDescribeOptions::builder().build())
        .await
        .is_err());

    sqlx::query(
        "INSERT INTO pipeline.operation_dag_business_runs_v1
           (run_id,tenant_id,context_key,plan_hash,parent_workflow_id,expires_at)
         VALUES ($1,$2,$3,$4,$5,clock_timestamp()+interval '1 hour')",
    )
    .bind(Uuid::parse_str(&run.plan.scope.run_id).unwrap())
    .bind(tenant)
    .bind(&context)
    .bind(run.plan.hash.as_str())
    .bind(&parent_id)
    .execute(&business_pool)
    .await
    .expect("independent tenant business issuer");
    let without_source = invoke(&intent_file);
    assert!(!without_source.status.success());
    let without_source_json: serde_json::Value =
        serde_json::from_slice(&without_source.stdout).unwrap();
    assert_eq!(
        without_source_json["code"],
        "independent_external_source_attestation_unavailable"
    );
    assert!(client
        .get_workflow_handle::<UntypedWorkflow>(parent_id.clone())
        .describe(WorkflowDescribeOptions::builder().build())
        .await
        .is_err());

    let source_schema = &run.admitted[&"source".into()].schema;
    sqlx::query(
        "INSERT INTO pipeline.operation_dag_external_source_attestations_v1
           (record_id,run_id,tenant_id,context_key,plan_hash,
            artifact_id,artifact_schema,payload_hash,
            source_page_id,source_page_content_hash,expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,clock_timestamp()+interval '5 minutes')",
    )
    .bind(record_id)
    .bind(Uuid::parse_str(&run.plan.scope.run_id).unwrap())
    .bind(tenant)
    .bind(&context)
    .bind(run.plan.hash.as_str())
    .bind("source")
    .bind(sqlx::types::Json(source_schema))
    .bind(owner_content_hash(&payload))
    .bind(source_page_id)
    .bind(raw_hash)
    .execute(&source_pool)
    .await
    .expect("separately authenticated raw-snapshot source issuer");
    let cannot_mint_business: bool = sqlx::query_scalar(
        "SELECT has_table_privilege(current_user,
            'pipeline.operation_dag_business_runs_v1','INSERT')",
    )
    .fetch_one(&source_pool)
    .await
    .unwrap();
    assert!(!cannot_mint_business);
    let cannot_mint_source: bool = sqlx::query_scalar(
        "SELECT has_table_privilege(current_user,
            'pipeline.operation_dag_external_source_attestations_v1','INSERT')",
    )
    .fetch_one(&reader_pool)
    .await
    .unwrap();
    assert!(!cannot_mint_source);

    // An attacker may know the exact record UUID and create an internally
    // consistent *new expected hash*. The independently persisted source
    // digest must still reject the modified payload before Temporal Start.
    let mut forged = run.clone();
    forged
        .admitted
        .get_mut(&"source".into())
        .unwrap()
        .payload
        .push(b' ');
    let forged_response = invoke(&make_intent(&forged, "acquisition-source-forged-payload"));
    assert!(!forged_response.status.success());
    let forged_json: serde_json::Value =
        serde_json::from_slice(&forged_response.stdout).unwrap();
    assert_eq!(
        forged_json["code"],
        "independent_external_source_attestation_unavailable"
    );
    assert!(client
        .get_workflow_handle::<UntypedWorkflow>(parent_id.clone())
        .describe(WorkflowDescribeOptions::builder().build())
        .await
        .is_err());

    let successful = invoke(&intent_file);
    assert!(
        successful.status.success(),
        "trusted source operator Start failed: stdout={} stderr={}",
        String::from_utf8_lossy(&successful.stdout),
        String::from_utf8_lossy(&successful.stderr)
    );
    let started: serde_json::Value = serde_json::from_slice(&successful.stdout).unwrap();
    assert_eq!(started["status"], "started_business_root_verified_owner_unadmitted");
    assert_eq!(
        started["source_authentication"],
        "source_record_independently_issued_hash_verified_at_start"
    );
    assert_eq!(started["parent_workflow_id"], parent_id);
    assert_eq!(started["owner_admission"], "not_issued_by_start");
    assert!(!invoke(&intent_file).status.success(), "duplicate Start must fail");

    let parent = client.get_workflow_handle::<UntypedWorkflow>(parent_id);
    let done = timeout(
        Duration::from_secs(90),
        parent.get_result(WorkflowGetResultOptions::default()),
    )
    .await
    .expect("native source-verified Acquisition result timed out")
    .expect("native source-verified Acquisition failed");
    let receipt: OperationDagReceiptV1 = done.to_value(&PayloadConverter::serde_json());
    assert_eq!(receipt.order, vec!["one_step".into()]);
    let classified: PageUtilityClassifierOutput =
        serde_json::from_slice(&receipt.artifacts[&"result".into()].payload).unwrap();
    assert!(!classified.allow_structural_extraction);
    let history = parent
        .fetch_history(WorkflowFetchHistoryOptions::builder().build())
        .into_events()
        .await
        .unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|e| matches!(
                &e.attributes,
                Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                    if a.workflow_id.starts_with("alegria-owner-validate-v1-")
            ))
            .count(),
        1
    );
    assert_eq!(
        history
            .iter()
            .filter(|e| matches!(
                &e.attributes,
                Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                    if a.workflow_id.starts_with("alegria-owner-operation-v2-")
            ))
            .count(),
        1
    );
    business_pool.close().await;
    source_pool.close().await;
    reader_pool.close().await;
}
