// Three different independently built pure owners in one genuine native
// Temporal V2 parent. This proves a selectable multi-owner DAG, not a forged
// Produced bridge: the three current owner-defined DTOs have different artifact
// schemas and are correctly forbidden from consuming each other's output.
// User-authored External inputs here are explicitly unverified (no source or
// SQL user-grant attestation is claimed by this direct CI workflow).

#[cfg(feature = "e2e")]
fn mixed_pure_native_v2_case(
    build_id: &str,
    reverse_nodes: bool,
) -> (
    operation_protocol::OperationDagRunV6,
    operation_protocol::TrustedRouteCatalog,
) {
    use acquisition::operations::ClassifySourcePage;
    use operation_protocol::{
        AdmittedArtifactV1, ArtifactBinding, ContractCatalog, ExecutionPlan, ExternalArtifact,
        Operation, OperationDagRunV6, OperationNode, OwnerCommandWireV1, PinnedOwnerRouteV1,
        PlanHash, RunScope, TrustedRouteCatalog, OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
        OWNER_OPERATION_CHILD_TYPE_V1, PLAN_SCHEMA_REVISION,
    };
    use seo_steps::owner_pure_operations::ExtractProceduralRules;
    use std::collections::{BTreeMap, BTreeSet};
    use truth_domain::operations::JudgeCompletenessV1;

    let scope = RunScope {
        tenant_id: "ci-mixed-three-pure-owners".into(),
        run_id: Uuid::new_v4().to_string(),
    };
    let owners = [
        (
            "acquisition",
            ClassifySourcePage::contract(),
            "alegria-acquisition-pure-owner-v1",
            br#"{"url":"https://ci-user-input.example.test","title":"Unverified directory","raw_text":"Unverified page text"}"#.to_vec(),
        ),
        (
            "seo",
            ExtractProceduralRules::contract(),
            "alegria-seo-steps-pure-owner-v1",
            br#"{"section_id":"section-1","raw_text":"user-supplied EUR 80","mentions":[{"raw_text":"80 EUR","entity_type":"fee","has_numeric":true,"is_central":true,"confidence":0.9}]}"#.to_vec(),
        ),
        (
            "truth",
            JudgeCompletenessV1::contract(),
            "alegria-truth-pure-owner-v1",
            br#"{"section_id":"section-1","raw_text":"user-supplied fee EUR 25","source_numeric_tokens":["25"],"extracted_numeric_tokens":[],"extracted_rule_keys":[]}"#.to_vec(),
        ),
    ];
    let mut contracts = ContractCatalog::default();
    let mut routes = TrustedRouteCatalog::default();
    let mut external_artifacts = BTreeMap::new();
    let mut admitted = BTreeMap::new();
    let mut nodes = Vec::new();
    let mut owner_commands = BTreeMap::new();

    for (instance, contract, queue, payload) in owners {
        contracts.register_contract(contract.clone()).unwrap();
        routes.register(
            contract.clone(),
            PinnedOwnerRouteV1 {
                operation: contract.reference.clone(),
                owner: contract.owner.clone(),
                contract_hash: contract.fingerprint().unwrap(),
                workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
                task_queue: queue.into(),
                build_id: build_id.into(),
            },
        ).unwrap();
        let source_id = format!("{instance}_source");
        let result_id = format!("{instance}_result");
        let input_schema = contract.inputs[&"input".into()].clone();
        let provenance = "ci-user-supplied-unverified-dto".to_owned();
        external_artifacts.insert(source_id.as_str().into(), ExternalArtifact {
            schema: input_schema.clone(),
            scope: scope.clone(),
            provenance: provenance.clone(),
        });
        admitted.insert(source_id.as_str().into(), AdmittedArtifactV1 {
            schema: input_schema,
            scope: scope.clone(),
            provenance,
            payload,
        });
        nodes.push(OperationNode {
            instance: instance.into(),
            operation: contract.reference.clone(),
            inputs: BTreeMap::from([(
                "input".into(),
                ArtifactBinding::External { artifact: source_id.as_str().into() },
            )]),
            outputs: BTreeMap::from([("output".into(), result_id.as_str().into())]),
            after: BTreeSet::new(),
            permitted_effects: BTreeSet::new(),
        });
        owner_commands.insert(instance.into(), OwnerCommandWireV1 {
            operation: contract.reference.clone(),
            owner: contract.owner.clone(),
            contract_hash: contract.fingerprint().unwrap(),
            command_bytes: b"{}".to_vec(),
        });
    }
    if reverse_nodes {
        nodes.reverse();
    }
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope,
        allowed_effects: BTreeSet::new(),
        external_artifacts,
        pinned_contracts: Vec::new(),
        nodes,
        hash: PlanHash(String::new()),
    }.seal(&contracts).expect("three independently selectable zero-effect owner operations");
    let pinned_routes = routes.trusted_snapshot_for(
        plan.nodes.iter().map(|node| &node.operation),
    ).unwrap();
    let run = OperationDagRunV6 {
        schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
        plan,
        admitted,
        owner_commands,
        pinned_routes,
    };
    routes.validate_run_routes(&run).unwrap();
    (run, routes)
}

#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn mixed_pure_native_v2_three_distinct_owners_and_incompatible_produced_rejection() {
    use acquisition::operation_owner_schema_json::acquisition_owner_schemas;
    use acquisition::source_relevance::PageUtilityClassifierOutput;
    use operation_protocol::{
        operation_dag_parent_workflow_id_v2, ArtifactBinding, OperationDagReceiptV1,
        OPERATION_DAG_PARENT_QUEUE_V2, OPERATION_DAG_PARENT_TYPE_V2,
    };
    use seo_steps::operation_owner_schema_json::seo_steps_owner_schemas;
    use seo_steps::owner_pure_operations::ProceduralRulesOutputV1;
    use truth_domain::operation_owner_schema_json::truth_pure_operation_owner_schemas;
    use truth_domain::operations::CompletenessOutputV1;

    let build_id = env::var("GITHUB_SHA").expect("source-pinned native owner builds");
    assert_eq!(build_id.len(), 40);
    let cases = [
        mixed_pure_native_v2_case(&build_id, false),
        mixed_pure_native_v2_case(&build_id, true),
    ];
    let schemas = [
        acquisition_owner_schemas().unwrap(),
        seo_steps_owner_schemas().unwrap(),
        truth_pure_operation_owner_schemas().unwrap(),
    ];
    let (first_run, first_routes) = &cases[0];
    let mut entries = Vec::new();
    for route in &first_run.pinned_routes {
        let contract = first_routes.catalog().contract(&route.operation).unwrap();
        let pin = schemas.iter().find_map(|owned| owned.get(&route.operation))
            .expect("exact-source owner must issue its DTO schema");
        pin.validate(contract).unwrap();
        entries.push(serde_json::json!({
            "contract": contract,
            "route": route,
            "parameter_schema": {"type":"object","additionalProperties":false},
            "owner_schema": pin,
        }));
    }
    let catalog = SiteArchitectV2CiRootCatalog::new();
    catalog.install(&serde_json::json!({"schema_revision":3,"entries":entries}));

    // A syntactically plausible connection must fail *before* any owner:
    // SEO procedural output is NOT the Truth completeness input DTO.
    let mut incompatible = first_run.plan.clone();
    let truth = incompatible.nodes.iter_mut()
        .find(|node| node.instance.as_str() == "truth").unwrap();
    truth.inputs.insert("input".into(), ArtifactBinding::Produced {
        producer: "seo".into(),
        port: "output".into(),
        artifact: "seo_result".into(),
    });
    let incompatible = incompatible.seal(first_routes.catalog()).unwrap();
    assert!(
        matches!(
            incompatible.validate(first_routes.catalog()),
            Err(operation_protocol::PlanError::ArtifactSchemaMismatch { .. })
        ),
        "mismatched output/input DTO schemas must be rejected before Start"
    );

    let temporal = TemporalHarness::start().await.expect("real disposable native Temporal server");
    let _parent = ChildGuard(Some(Command::new(binary("INTEGRATION_EFFECT_DAG_WORKER_BIN"))
        .env("ALLOW_OPERATION_DAG_V2_WITH_OWNER_VALIDATION", "true")
        .env("TEMPORAL_ORCHESTRATOR_TASK_QUEUE", OPERATION_DAG_PARENT_QUEUE_V2)
        .env("TEMPORAL_OPERATION_DAG_CATALOG_PATH", &catalog.0)
        .env("WORKER_BUILD_ID", &build_id)
        .env("TEMPORAL_URL", &temporal.temporal_url)
        .env("TEMPORAL_NAMESPACE", &temporal.namespace)
        .env_remove("DATABASE_URL")
        .spawn().expect("generic provider-free V2 coordinator")));
    let owner_binaries = [
        ("INTEGRATION_ACQUISITION_PURE_OWNER_BIN", "ALEGRIA_ENABLE_ACQUISITION_PURE_OWNER_V1"),
        ("INTEGRATION_SEO_STEPS_PURE_OWNER_BIN", "ALEGRIA_ENABLE_SEO_STEPS_PURE_OWNER_V1"),
        ("INTEGRATION_TRUTH_PURE_OWNER_BIN", "ALEGRIA_ENABLE_TRUTH_PURE_OWNER_V1"),
    ];
    let mut _owners = Vec::new();
    for (binary_env, enable_env) in owner_binaries {
        _owners.push(ChildGuard(Some(Command::new(binary(binary_env))
            .env(enable_env, "true")
            .env("WORKER_BUILD_ID", &build_id)
            .env("TEMPORAL_URL", &temporal.temporal_url)
            .env("TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .spawn().expect("independent native pure owner worker"))));
    }
    let client = connect_client(
        &temporal.temporal_url,
        format!("ci-mixed-pure-v2-{}", Uuid::new_v4()),
        &temporal.namespace,
    ).await.unwrap();
    let mut previous_outputs: Option<[Vec<u8>; 3]> = None;
    for (run, routes) in &cases {
        routes.validate_run_routes(run).unwrap();
        let parent_id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
        let parent = client.start_workflow(
            UntypedWorkflow::new(OPERATION_DAG_PARENT_TYPE_V2),
            RawValue::from_value(run, &PayloadConverter::serde_json()),
            WorkflowStartOptions::new(OPERATION_DAG_PARENT_QUEUE_V2, parent_id).build(),
        ).await.expect("three independently installed owners share one Temporal DAG");
        let completed = timeout(Duration::from_secs(90),
            parent.get_result(WorkflowGetResultOptions::default())).await
            .expect("three-owner V2 DAG result timed out")
            .expect("one native pure child failed");
        let receipt: OperationDagReceiptV1 = completed.to_value(&PayloadConverter::serde_json());
        assert_eq!(receipt.order.len(), 3);
        assert_eq!(receipt.owner_results.len(), 3);
        assert_eq!(receipt.produced_evidence.len(), 3);
        assert_eq!(receipt.artifacts.len(), 6);
        let classified: PageUtilityClassifierOutput = serde_json::from_slice(
            &receipt.artifacts[&"acquisition_result".into()].payload).unwrap();
        assert!(!classified.allow_structural_extraction);
        let seo: ProceduralRulesOutputV1 = serde_json::from_slice(
            &receipt.artifacts[&"seo_result".into()].payload).unwrap();
        assert_eq!(seo.rules.len(), 1);
        let truth: CompletenessOutputV1 = serde_json::from_slice(
            &receipt.artifacts[&"truth_result".into()].payload).unwrap();
        assert!(truth.needs_hitl);
        let outputs = [
            receipt.artifacts[&"acquisition_result".into()].payload.clone(),
            receipt.artifacts[&"seo_result".into()].payload.clone(),
            receipt.artifacts[&"truth_result".into()].payload.clone(),
        ];
        if let Some(previous) = &previous_outputs {
            assert_eq!(&outputs, previous,
                "the same three typed pure decisions must be order-independent");
        }
        previous_outputs = Some(outputs);
        let history = parent.fetch_history(WorkflowFetchHistoryOptions::builder().build())
            .into_events().await.expect("real three-owner parent History");
        let mut validations = Vec::new();
        let mut operations = Vec::new();
        for event in &history {
            if let Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a)) =
                &event.attributes {
                if a.workflow_id.starts_with("alegria-owner-validate-v1-") {
                    validations.push(event.event_id);
                } else if a.workflow_id.starts_with("alegria-owner-operation-v2-") {
                    operations.push(event.event_id);
                }
            }
        }
        assert_eq!(validations.len(), 3, "exactly one ValidateOnly per pure owner");
        assert_eq!(operations.len(), 3, "exactly one execution Child per pure owner");
        assert!(validations.iter().max() < operations.iter().min(),
            "all owner-local validation must precede any executable owner Child");
    }
}
