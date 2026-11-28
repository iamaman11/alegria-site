// Exact-source, opt-in physical Temporal proof for three independently
// selectable pure SEO actions: independent ValidateOnly, owner Child,
// immutable typed receipts, captured server History replay, and denial of a
// malformed input before the executable Child. No SQL, HTTP or provider IO.

#[cfg(feature = "e2e")]
fn seo_steps_native_v2_case(
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
        tenant_id: "ci-native-seo-steps-owner".to_owned(),
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
                provenance: "ci-unattested-external-seo-source".into(),
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
    routes.register(
        contract.clone(),
        PinnedOwnerRouteV1 {
            operation: contract.reference.clone(),
            owner: contract.owner.clone(),
            contract_hash: contract.fingerprint().unwrap(),
            workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
            task_queue: "alegria-seo-steps-pure-owner-v1".into(),
            build_id: build_id.into(),
        },
    ).unwrap();
    let run = OperationDagRunV6 {
        schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
        admitted: BTreeMap::from([(
            "source".into(),
            AdmittedArtifactV1 {
                schema: input_schema,
                scope,
                provenance: "ci-unattested-external-seo-source".into(),
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
async fn seo_steps_native_v2_three_independent_one_step_operations_and_replay() {
    use operation_protocol::{
        operation_dag_parent_workflow_id_v2, Operation, OperationDagReceiptV1,
        OPERATION_DAG_PARENT_QUEUE_V2, OPERATION_DAG_PARENT_TYPE_V2,
    };
    use seo_steps::operation_owner_schema_json::seo_steps_owner_schemas;
    use seo_steps::owner_pure_operations::{
        BuildSeoOpportunities, ExtractProceduralRules, NormalizeSerpQueries,
        OpportunityBuildOutputV1, ProceduralRulesOutputV1, SerpQueryNormalizeOutputV1,
    };

    let build_id = env::var("GITHUB_SHA").expect("real worker binary must pin exact CI SHA");
    assert_eq!(build_id.len(), 40);
    let schemas = seo_steps_owner_schemas().unwrap();
    assert_eq!(schemas.len(), 3);
    let cases = [
        seo_steps_native_v2_case(
            ExtractProceduralRules::contract(),
            br#"{"section_id":"section-1","raw_text":"unverified context","mentions":[{"raw_text":"80 EUR","entity_type":"fee","has_numeric":true,"is_central":true,"confidence":0.9}]}"#.to_vec(),
            &build_id,
        ),
        seo_steps_native_v2_case(
            NormalizeSerpQueries::contract(),
            br#"{"run_id":"seo-run","queries":["spain visa fee"]}"#.to_vec(),
            &build_id,
        ),
        seo_steps_native_v2_case(
            BuildSeoOpportunities::contract(),
            br#"{"serp_patterns":[{"serp_pattern_key":"p1","query_batch_key":"q1","scope_signature":"","query":"spain visa fee","pattern_type":"query_intent","dominant_intent":"informational","reliability_score":0.75,"evidence_ref":"q1","status":"active"}]}"#.to_vec(),
            &build_id,
        ),
    ];
    let entries = cases.iter()
        .flat_map(|(run, routes)| run.pinned_routes.iter().map(move |route| (route, routes)))
        .map(|(route, routes)| {
            let contract = routes.catalog().contract(&route.operation).unwrap();
            let pinned = &schemas[&route.operation];
            pinned.validate(contract).unwrap();
            serde_json::json!({
                "contract": contract,
                "route": route,
                "parameter_schema": {"type":"object","additionalProperties":false},
                "owner_schema": pinned,
            })
        }).collect::<Vec<_>>();
    let catalog = SiteArchitectV2CiRootCatalog::new();
    catalog.install(&serde_json::json!({"schema_revision":3,"entries":entries}));
    let installed = trusted_owner_catalog::load_trusted_owner_bundle(&catalog.0).unwrap();
    assert_eq!(installed.routes.catalog().contracts().count(), 3);

    let temporal = TemporalHarness::start().await.expect("disposable native Temporal server");
    let parent_bin = binary("INTEGRATION_EFFECT_DAG_WORKER_BIN");
    let seo_bin = binary("INTEGRATION_SEO_STEPS_PURE_OWNER_BIN");
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
            .expect("provider-free generic Temporal parent"),
    ));
    let _owner = ChildGuard(Some(
        Command::new(&seo_bin)
            .env("ALEGRIA_ENABLE_SEO_STEPS_PURE_OWNER_V1", "true")
            .env("WORKER_BUILD_ID", &build_id)
            .env("TEMPORAL_URL", &temporal.temporal_url)
            .env("TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .spawn()
            .expect("native SEO steps pure owner"),
    ));
    let client = connect_client(
        &temporal.temporal_url,
        format!("ci-seo-steps-native-v2-{}", Uuid::new_v4()),
        &temporal.namespace,
    ).await.unwrap();

    for (index, (run, _)) in cases.iter().enumerate() {
        let parent_id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
        let parent = client.start_workflow(
            UntypedWorkflow::new(OPERATION_DAG_PARENT_TYPE_V2),
            RawValue::from_value(run, &PayloadConverter::serde_json()),
            WorkflowStartOptions::new(OPERATION_DAG_PARENT_QUEUE_V2, parent_id).build(),
        ).await.expect("native single-operation Temporal Start");
        let completed = timeout(
            Duration::from_secs(90),
            parent.get_result(WorkflowGetResultOptions::default()),
        ).await.expect("SEO owner Child timeout").expect("SEO owner Child failed");
        let result: OperationDagReceiptV1 = completed.to_value(&PayloadConverter::serde_json());
        assert_eq!(result.order, vec!["one_step".into()]);
        assert_eq!(result.owner_results.len(), 1);
        assert_eq!(result.produced_evidence.len(), 1);
        assert_eq!(result.artifacts[&"result".into()].scope, run.plan.scope);
        let payload = &result.artifacts[&"result".into()].payload;
        match index {
            0 => assert_eq!(
                serde_json::from_slice::<ProceduralRulesOutputV1>(payload).unwrap().rules.len(),
                1,
            ),
            1 => assert_eq!(
                serde_json::from_slice::<SerpQueryNormalizeOutputV1>(payload).unwrap()
                    .normalized_query_count,
                1,
            ),
            2 => assert_eq!(
                serde_json::from_slice::<OpportunityBuildOutputV1>(payload).unwrap()
                    .keyword_clusters.len(),
                1,
            ),
            _ => unreachable!(),
        }
        let history = parent.fetch_history(
            WorkflowFetchHistoryOptions::builder().build()
        ).into_events().await.expect("actual parent History");
        let validations = history.iter().filter(|event| matches!(
            &event.attributes,
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                if a.workflow_id.starts_with("alegria-owner-validate-v1-")
        )).count();
        let operations = history.iter().filter_map(|event| match &event.attributes {
            Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a))
                if a.workflow_id.starts_with("alegria-owner-operation-v2-") =>
                    Some(a.workflow_id.as_str()),
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(validations, 1);
        assert_eq!(operations.len(), 1);
        let child = client.get_workflow_handle::<UntypedWorkflow>(operations[0].to_owned());
        let server_history = child.fetch_history(
            WorkflowFetchHistoryOptions::builder().build()
        ).to_json().await.expect("server-issued SEO Child History");
        assert!(server_history.len() > 100);
        let replay_file = secret("seo-steps-pure-owner-server-history", "{}");
        fs::write(&replay_file, server_history).unwrap();
        let replay = Command::new(&seo_bin)
            .arg("--replay-history")
            .arg(&replay_file)
            .env("WORKER_BUILD_ID", &build_id)
            .env_remove("DATABASE_URL")
            .output()
            .expect("run exact SEO worker replay");
        fs::remove_file(&replay_file).unwrap();
        assert!(replay.status.success(), "{}", String::from_utf8_lossy(&replay.stderr));
        assert!(String::from_utf8_lossy(&replay.stdout)
            .contains("CI_TEMPORAL_SEO_STEPS_PURE_OWNER_REPLAY=PASS"));
    }

    // The owner rejects a semantic (not merely structural) invalid run ID
    // while only the validation child has been started.
    let (invalid, _) = seo_steps_native_v2_case(
        NormalizeSerpQueries::contract(),
        br#"{"run_id":"   ","queries":[]}"#.to_vec(),
        &build_id,
    );
    let invalid_id = operation_dag_parent_workflow_id_v2(&invalid.plan.scope.run_id);
    let denied = client.start_workflow(
        UntypedWorkflow::new(OPERATION_DAG_PARENT_TYPE_V2),
        RawValue::from_value(&invalid, &PayloadConverter::serde_json()),
        WorkflowStartOptions::new(OPERATION_DAG_PARENT_QUEUE_V2, invalid_id).build(),
    ).await.expect("start invalid native SEO operation");
    let rejected = timeout(
        Duration::from_secs(90),
        denied.get_result(WorkflowGetResultOptions::default()),
    ).await.expect("invalid DTO must complete with refusal");
    assert!(rejected.is_err(), "domain failure must be denied in owner ValidateOnly");
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
    )).count(), 0, "invalid domain input must never reach executable owner Child");
}
