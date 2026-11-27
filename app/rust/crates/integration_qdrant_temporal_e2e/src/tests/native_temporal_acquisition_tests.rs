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
