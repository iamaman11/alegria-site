// Native server proof of the semantic Produced edge using the owner-exported
// operator-installed catalog. External source bytes are explicitly unverified.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn semantic_pure_native_v2_entity_spans_and_canonicalize_produced() {
    use operation_protocol::{
        operation_dag_parent_workflow_id_v2, owner_content_hash, AdmittedArtifactV1,
        ArtifactBinding, ContractCatalog, ExecutionPlan, ExternalArtifact, Operation,
        OperationDagReceiptV1, OperationDagRunV6, OperationNode, OwnerCommandWireV1, PlanHash,
        RunScope, OPERATION_DAG_PARENT_QUEUE_V2, OPERATION_DAG_PARENT_TYPE_V2,
        OPERATION_DAG_RUN_SCHEMA_REVISION_V6, PLAN_SCHEMA_REVISION,
    };
    use seo_domain::extraction::{
        entity_spans,
        operations::{CanonicalizeMentions, EntitySpans, PureExtractionOperation},
    };
    use std::collections::{BTreeMap, BTreeSet};

    let build_id = env::var("GITHUB_SHA").expect("exact-source owner build ID");
    assert_eq!(build_id.len(), 40);
    let owner_bin = binary("INTEGRATION_SEMANTIC_PURE_OWNER_BIN");
    let acquisition_bin = binary("INTEGRATION_ACQUISITION_PURE_OWNER_BIN");
    let mut document = serde_json::json!({"schema_revision": 3, "entries": []});
    for bin in [&owner_bin, &acquisition_bin] {
        let exported = Command::new(bin)
            .arg("--export-trusted-catalog-v3")
            .env("WORKER_BUILD_ID", &build_id)
            .output()
            .expect("owner catalog export");
        assert!(
            exported.status.success(),
            "{}",
            String::from_utf8_lossy(&exported.stderr)
        );
        let mut owner: serde_json::Value = serde_json::from_slice(&exported.stdout).unwrap();
        assert_eq!(owner["schema_revision"], 3);
        document["entries"]
            .as_array_mut()
            .unwrap()
            .append(owner["entries"].as_array_mut().unwrap());
    }
    let catalog = SiteArchitectV2CiRootCatalog::new();
    catalog.install(&document);
    let installed = trusted_owner_catalog::load_trusted_owner_bundle(&catalog.0).unwrap();
    let spans_contract = EntitySpans::contract();
    let canonical_contract = CanonicalizeMentions::contract();
    for contract in [&spans_contract, &canonical_contract] {
        assert_eq!(
            installed
                .routes
                .catalog()
                .contract(&contract.reference)
                .unwrap(),
            contract
        );
    }
    let prepare_ref = operation_protocol::OperationRef {
        id: "acquisition.prepare_entity_span_input".into(),
        revision: 1,
    };
    let prepare_contract = installed
        .routes
        .catalog()
        .contract(&prepare_ref)
        .unwrap()
        .clone();
    assert_eq!(
        prepare_contract.outputs[&"output".into()],
        spans_contract.inputs[&"input".into()]
    );
    let frozen =
        br#"{"section_id":"fees","raw_text":"Visa fee is 80 EUR and processing takes 10 days."}"#
            .to_vec();
    let make_run = |two: bool, acquisition: bool, reject: u8| {
        let scope = RunScope {
            tenant_id: "ci-semantic-unverified".into(),
            run_id: Uuid::new_v4().to_string(),
        };
        let source_schema = if acquisition {
            prepare_contract.inputs[&"input".into()].clone()
        } else {
            spans_contract.inputs[&"input".into()].clone()
        };
        let mut nodes = vec![OperationNode {
            instance: "extract".into(),
            operation: spans_contract.reference.clone(),
            inputs: BTreeMap::from([(
                "input".into(),
                ArtifactBinding::External {
                    artifact: "source".into(),
                },
            )]),
            outputs: BTreeMap::from([("output".into(), "mentions".into())]),
            after: BTreeSet::new(),
            permitted_effects: BTreeSet::new(),
        }];
        if acquisition {
            nodes[0].inputs.insert(
                "input".into(),
                ArtifactBinding::Produced {
                    producer: "prepare".into(),
                    port: "output".into(),
                    artifact: "span_input".into(),
                },
            );
            nodes.insert(
                0,
                OperationNode {
                    instance: "prepare".into(),
                    operation: prepare_ref.clone(),
                    inputs: BTreeMap::from([(
                        "input".into(),
                        ArtifactBinding::External {
                            artifact: "source".into(),
                        },
                    )]),
                    outputs: BTreeMap::from([("output".into(), "span_input".into())]),
                    after: BTreeSet::new(),
                    permitted_effects: BTreeSet::new(),
                },
            );
        }
        if two {
            nodes.push(OperationNode {
                instance: "canonicalize".into(),
                operation: canonical_contract.reference.clone(),
                inputs: BTreeMap::from([(
                    "input".into(),
                    ArtifactBinding::Produced {
                        producer: "extract".into(),
                        port: "output".into(),
                        artifact: "mentions".into(),
                    },
                )]),
                outputs: BTreeMap::from([("output".into(), "canonical".into())]),
                after: BTreeSet::new(),
                permitted_effects: BTreeSet::new(),
            });
        }
        let commands = nodes
            .iter()
            .map(|node| {
                let contract = installed
                    .routes
                    .catalog()
                    .contract(&node.operation)
                    .unwrap();
                (
                    node.instance.clone(),
                    OwnerCommandWireV1 {
                        operation: contract.reference.clone(),
                        owner: contract.owner.clone(),
                        contract_hash: contract.fingerprint().unwrap(),
                        command_bytes: b"{}".to_vec(),
                    },
                )
            })
            .collect();
        let plan = ExecutionPlan {
            schema_revision: PLAN_SCHEMA_REVISION,
            scope: scope.clone(),
            allowed_effects: BTreeSet::new(),
            external_artifacts: BTreeMap::from([(
                "source".into(),
                ExternalArtifact {
                    schema: source_schema.clone(),
                    scope: scope.clone(),
                    provenance: "ci-unverified-source".into(),
                },
            )]),
            pinned_contracts: Vec::new(),
            nodes,
            hash: PlanHash(String::new()),
        }
        .seal(installed.routes.catalog())
        .unwrap();
        let pinned_routes = installed
            .routes
            .trusted_snapshot_for(plan.nodes.iter().map(|n| &n.operation))
            .unwrap();
        OperationDagRunV6 {
            schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
            plan,
            admitted: BTreeMap::from([(
                "source".into(),
                AdmittedArtifactV1 {
                    schema: source_schema,
                    scope,
                    provenance: "ci-unverified-source".into(),
                    payload: if acquisition {
                        serde_json::to_vec(&serde_json::json!({
                        "page": {"url": if reject == 1 { "https://example.org/login" } else { "https://example.org/visa" },
                            "title": if reject == 1 { "Login" } else { "Visa requirements" },
                            "raw_text": if reject == 1 { "Sign in to your account" } else { "Visa requirements and fees" }},
                        "block": {"dom_block_id": "fees", "block_role": if reject == 2 { "navigation" } else { "content_main" },
                            "text": "Visa fee is 80 EUR and processing takes 10 days."}
                    })).unwrap()
                    } else {
                        frozen.clone()
                    },
                },
            )]),
            owner_commands: commands,
            pinned_routes,
        }
    };
    let runs = [
        make_run(false, false, 0),
        make_run(true, false, 0),
        make_run(true, true, 0),
        make_run(true, true, 1),
        make_run(true, true, 2),
    ];
    // A changed consumer revision is incompatible with the real producer's
    // output, even if a plausible Produced edge is authored.
    let mut incompatible = canonical_contract.clone();
    incompatible
        .inputs
        .get_mut(&"input".into())
        .unwrap()
        .revision += 1;
    let mut wrong_catalog = ContractCatalog::default();
    wrong_catalog
        .register_contract(spans_contract.clone())
        .unwrap();
    wrong_catalog.register_contract(incompatible).unwrap();
    let wrong_plan = runs[1].plan.clone().seal(&wrong_catalog).unwrap();
    assert!(matches!(
        wrong_plan.validate(&wrong_catalog),
        Err(operation_protocol::PlanError::ArtifactSchemaMismatch { .. })
    ));

    // Disposable SQL roles independently issue and read the business-run
    // authority. These CI fixtures do not claim a real user grant.
    let pg = PostgresHarness::start()
        .await
        .expect("disposable governance Postgres");
    mixed_v2_install_native_admissions(&pg.pool).await;
    let tenant = "ci-semantic-unverified";
    let context = format!("semantic-ci-{}", Uuid::new_v4());
    sqlx::query("INSERT INTO kb.visa_contexts(context_key,country_code,visa_family,citizenship_code) VALUES ($1,'ES','tourist','BY')")
        .bind(&context).execute(&pg.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO pipeline.tenant_context_ownership(context_key,tenant_id) VALUES ($1,$2)",
    )
    .bind(&context)
    .bind(tenant)
    .execute(&pg.pool)
    .await
    .unwrap();
    let base_roles = mixed_v6_native_owner_roles(&pg, tenant).await;
    let roles = mixed_v2_native_roles(&pg, tenant, &base_roles).await;
    let locators = MixedV2PrivateLocatorFiles::new();
    let reader_dsn = locators.request("semantic-reader-dsn", &roles.read_only_url);
    let pg_bin = binary("INTEGRATION_PG_EFFECT_WORKER_BIN");
    let temporal = TemporalHarness::start()
        .await
        .expect("real Temporal server");
    let _parent = ChildGuard(Some(
        Command::new(binary("INTEGRATION_EFFECT_DAG_WORKER_BIN"))
            .env("ALLOW_OPERATION_DAG_V2_WITH_OWNER_VALIDATION", "true")
            .env(
                "TEMPORAL_ORCHESTRATOR_TASK_QUEUE",
                OPERATION_DAG_PARENT_QUEUE_V2,
            )
            .env("TEMPORAL_OPERATION_DAG_CATALOG_PATH", &catalog.0)
            .env("WORKER_BUILD_ID", &build_id)
            .env("TEMPORAL_URL", &temporal.temporal_url)
            .env("TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .spawn()
            .expect("native parent"),
    ));
    let _owner = ChildGuard(Some(
        Command::new(&owner_bin)
            .env("ALEGRIA_ENABLE_SEMANTIC_PURE_OWNER_V1", "true")
            .env("TEMPORAL_OPERATION_DAG_CATALOG_PATH", &catalog.0)
            .env("WORKER_BUILD_ID", &build_id)
            .env("TEMPORAL_URL", &temporal.temporal_url)
            .env("TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .spawn()
            .expect("native semantic owner"),
    ));
    let _acquisition_owner = ChildGuard(Some(
        Command::new(&acquisition_bin)
            .env("ALEGRIA_ENABLE_ACQUISITION_PURE_OWNER_V1", "true")
            .env("TEMPORAL_OPERATION_DAG_CATALOG_PATH", &catalog.0)
            .env("WORKER_BUILD_ID", &build_id)
            .env("TEMPORAL_URL", &temporal.temporal_url)
            .env("TEMPORAL_NAMESPACE", &temporal.namespace)
            .env_remove("DATABASE_URL")
            .spawn()
            .expect("native acquisition owner"),
    ));
    let client = connect_client(
        &temporal.temporal_url,
        format!("ci-semantic-{}", Uuid::new_v4()),
        &temporal.namespace,
    )
    .await
    .unwrap();
    let mut prior_mentions = None;
    for (index, run) in runs.iter().enumerate() {
        installed.routes.validate_run_routes(run).unwrap();
        let parent = client
            .start_workflow(
                UntypedWorkflow::new(OPERATION_DAG_PARENT_TYPE_V2),
                RawValue::from_value(run, &PayloadConverter::serde_json()),
                WorkflowStartOptions::new(
                    OPERATION_DAG_PARENT_QUEUE_V2,
                    operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id),
                )
                .build(),
            )
            .await
            .expect("start semantic native parent");
        let parent_id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
        let parent_run = parent
            .info()
            .run_id
            .clone()
            .expect("server-issued parent Run ID");
        mixed_v2_authorize_business_root(&roles, run, &context, &parent_id).await;
        let issuer_catalog = mixed_v6_governance_issuer_catalog(run, &installed.routes);
        let request = locators.request(
            "semantic-governance",
            &serde_json::json!({
                "canonical_run_id": run.plan.scope.run_id, "context_key": context,
                "parent_workflow_id": parent_id, "parent_workflow_run_id": parent_run,
                "expires_in_seconds": 600,
            })
            .to_string(),
        );
        let admitted = Command::new(&pg_bin)
            .arg("--operator-admit-dag-run-v2")
            .arg(&request)
            .env(
                "ALEGRIA_ENABLE_OPERATION_DAG_GOVERNANCE_OPERATOR_V2",
                "true",
            )
            .env(
                "OPERATION_DAG_GOVERNANCE_V2_OPERATOR_DATABASE_URL",
                &roles.governance_issuer_url,
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
                &issuer_catalog,
            )
            .env_remove("DATABASE_URL")
            .output()
            .unwrap();
        assert!(
            admitted.status.success(),
            "governance: stdout={} stderr={}",
            String::from_utf8_lossy(&admitted.stdout),
            String::from_utf8_lossy(&admitted.stderr)
        );
        let completed = timeout(
            Duration::from_secs(90),
            parent.get_result(WorkflowGetResultOptions::default()),
        )
        .await
        .expect("semantic native DAG timed out");
        if index >= 3 {
            assert!(
                completed.is_err(),
                "utility/navigation input must be rejected"
            );
            let events = parent
                .fetch_history(WorkflowFetchHistoryOptions::builder().build())
                .into_events()
                .await
                .unwrap();
            let children: Vec<_> = events
                .iter()
                .filter_map(|event| match &event.attributes {
                    Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a)) => {
                        Some(&a.workflow_id)
                    }
                    _ => None,
                })
                .collect();
            assert!(children
                .iter()
                .any(|id| id.starts_with("alegria-owner-validate-v1-")));
            assert!(!children
                .iter()
                .any(|id| id.starts_with("alegria-owner-operation-v2-")));
            assert!(events.iter().any(|event| matches!(
                &event.attributes,
                Some(Attributes::ChildWorkflowExecutionFailedEventAttributes(_))
            )));
            continue;
        }
        let completed = completed.expect("semantic native DAG failed");
        let result: OperationDagReceiptV1 = completed.to_value(&PayloadConverter::serde_json());
        assert_eq!(result.owner_results.len(), index + 1);
        if index == 2 {
            assert_eq!(result.artifacts[&"span_input".into()].payload, frozen);
        }
        let mentions = &result.artifacts[&"mentions".into()].payload;
        let typed: entity_spans::EntitySpanOutput = serde_json::from_slice(mentions).unwrap();
        assert!(!typed.mentions.is_empty());
        if let Some(previous) = &prior_mentions {
            assert_eq!(mentions, previous);
        }
        prior_mentions = Some(mentions.clone());
        let proof = &result.produced_evidence[&"mentions".into()];
        assert_eq!(proof.payload_hash, owner_content_hash(mentions));
        let history = parent
            .fetch_history(WorkflowFetchHistoryOptions::builder().build())
            .into_events()
            .await
            .unwrap();
        let children = history
            .iter()
            .filter_map(|event| match &event.attributes {
                Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a)) => {
                    Some((event.event_id, a.workflow_id.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let validates = children
            .iter()
            .filter(|(_, id)| id.starts_with("alegria-owner-validate-v1-"))
            .collect::<Vec<_>>();
        let executions = children
            .iter()
            .filter(|(_, id)| id.starts_with("alegria-owner-operation-v2-"))
            .collect::<Vec<_>>();
        assert_eq!(validates.len(), index + 1);
        assert_eq!(executions.len(), index + 1);
        assert!(validates.iter().map(|x| x.0).max() < executions.iter().map(|x| x.0).min());
        assert!(executions
            .iter()
            .any(|(_, id)| id == &proof.child_workflow_id));
        if index == 2 {
            for artifact in ["span_input", "mentions"] {
                let produced = &result.produced_evidence[&artifact.into()];
                let producer_start = executions
                    .iter()
                    .find(|(_, id)| id == &produced.child_workflow_id)
                    .unwrap()
                    .0;
                let producer_complete = history
                    .iter()
                    .find_map(|event| match &event.attributes {
                        Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(a))
                            if a.initiated_event_id == producer_start =>
                        {
                            Some(event.event_id)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!(
                    history.iter().any(|event| match &event.attributes {
                        Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(
                            a,
                        )) if event.event_id > producer_complete => {
                            a.input.as_ref().is_some_and(|input| {
                                let wire = String::from_utf8_lossy(&input.payloads[0].data);
                                wire.contains(&produced.payload_hash)
                                    && wire.contains(&produced.child_workflow_id)
                            })
                        }
                        _ => false,
                    }),
                    "consumer must follow producer completion and carry its proof"
                );
            }
        }
        for (_, child_id) in &children {
            let child = client.get_workflow_handle::<UntypedWorkflow>(child_id.clone());
            let server_history = child
                .fetch_history(WorkflowFetchHistoryOptions::builder().build())
                .to_json()
                .await
                .unwrap();
            assert!(
                server_history.len() > 100,
                "inspectable native child History"
            );
            let replay_file = secret(
                "semantic-child-history",
                std::str::from_utf8(&server_history).expect("server History is UTF-8 JSON"),
            );
            let acquisition_child = index == 2
                && (child_id == &result.produced_evidence[&"span_input".into()].child_workflow_id
                    || {
                        let event = history
                            .iter()
                            .find_map(|event| match &event.attributes {
                                Some(
                                    Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(
                                        a,
                                    ),
                                ) if &a.workflow_id == child_id => Some(a),
                                _ => None,
                            })
                            .unwrap();
                        event
                            .task_queue
                            .as_ref()
                            .is_some_and(|q| q.name == "alegria-acquisition-pure-owner-v1")
                    });
            let replay = Command::new(if acquisition_child {
                &acquisition_bin
            } else {
                &owner_bin
            })
            .arg("--replay-history")
            .arg(&replay_file)
            .env("WORKER_BUILD_ID", &build_id)
            .env("TEMPORAL_OPERATION_DAG_CATALOG_PATH", &catalog.0)
            .env_remove("DATABASE_URL")
            .output()
            .expect("replay captured server child History");
            fs::remove_file(&replay_file).unwrap();
            assert!(
                replay.status.success(),
                "semantic replay: {}",
                String::from_utf8_lossy(&replay.stderr)
            );
            assert!(
                String::from_utf8_lossy(&replay.stdout).contains(if acquisition_child {
                    "CI_TEMPORAL_ACQUISITION_PURE_OWNER_REPLAY=PASS"
                } else {
                    "CI_TEMPORAL_SEMANTIC_PURE_OWNER_REPLAY=PASS"
                })
            );
            if index == 1
                && child_id.starts_with("alegria-owner-operation-v2-")
                && child_id != &proof.child_workflow_id
            {
                let input = history
                    .iter()
                    .find_map(|event| match &event.attributes {
                        Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(
                            a,
                        )) if &a.workflow_id == child_id => a.input.as_ref(),
                        _ => None,
                    })
                    .expect("consumer initiated input in server History");
                let wire = String::from_utf8_lossy(&input.payloads[0].data);
                assert!(
                    wire.contains(&proof.payload_hash),
                    "consumer History must carry the actual Produced proof"
                );
                assert!(
                    wire.contains(&proof.child_workflow_id),
                    "consumer History must identify actual producer"
                );
            }
        }
        if index == 1 {
            // Replay an authentic canonical invocation under its genuine child
            // workflow ID as a new *direct* execution. The invocation and key
            // remain valid, but this server run has no reciprocal parent event.
            let producer_input =
                history
                    .iter()
                    .find_map(|event| match &event.attributes {
                        Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(
                            a,
                        )) if a.workflow_id == proof.child_workflow_id => a.input.as_ref(),
                        _ => None,
                    })
                    .expect("real producer invocation");
            let invocation: operation_protocol::OwnerInvocationV1 =
                serde_json::from_slice(&producer_input.payloads[0].data).unwrap();
            let forged = client
                .start_workflow(
                    UntypedWorkflow::new(operation_protocol::OWNER_OPERATION_CHILD_TYPE_V1),
                    RawValue::from_value(&invocation, &PayloadConverter::serde_json()),
                    WorkflowStartOptions::new(
                        "alegria-semantic-pure-owner-v1",
                        proof.child_workflow_id.clone(),
                    )
                    .build(),
                )
                .await
                .expect("start forged direct execution with canonical child ID");
            let denied = timeout(
                Duration::from_secs(90),
                forged.get_result(WorkflowGetResultOptions::default()),
            )
            .await
            .expect("direct execution must not hang");
            assert!(
                denied.is_err(),
                "owner must reject a direct start lacking reciprocal parent History"
            );
            let forged_history = forged
                .fetch_history(WorkflowFetchHistoryOptions::builder().build())
                .into_events()
                .await
                .unwrap();
            assert!(
                forged_history.iter().any(|event| matches!(
                    &event.attributes,
                    Some(Attributes::WorkflowExecutionFailedEventAttributes(_))
                )),
                "real server records rejection"
            );
            assert!(
                forged_history.iter().any(|event| matches!(
                    &event.attributes,
                    Some(Attributes::ActivityTaskFailedEventAttributes(_))
                )),
                "History attestation Activity must reject the direct invocation"
            );
        }
        if index >= 1 {
            let locator = locators.request(
                "semantic-run-read",
                &serde_json::json!({
                    "canonical_run_id": run.plan.scope.run_id,
                })
                .to_string(),
            );
            let read = Command::new(&pg_bin)
                .arg("--operator-read-dag-run-v2")
                .arg(&locator)
                .env("ALEGRIA_ENABLE_OPERATION_DAG_READ_V2", "true")
                .env("OPERATION_DAG_READ_V2_DATABASE_URL_FILE", &reader_dsn)
                .env("OPERATION_DAG_READ_V2_TENANT_ID", tenant)
                .env("OPERATION_DAG_READ_V2_TEMPORAL_URL", &temporal.temporal_url)
                .env(
                    "OPERATION_DAG_READ_V2_TEMPORAL_NAMESPACE",
                    &temporal.namespace,
                )
                .env(
                    "OPERATION_DAG_READ_V2_TRUSTED_CATALOG_FILE",
                    &issuer_catalog,
                )
                .env_remove("DATABASE_URL")
                .output()
                .unwrap();
            assert!(
                read.status.success(),
                "read by run ID: stdout={} stderr={}",
                String::from_utf8_lossy(&read.stdout),
                String::from_utf8_lossy(&read.stderr)
            );
            let view: serde_json::Value = serde_json::from_slice(&read.stdout).unwrap();
            assert_eq!(view["status"], "temporal_completed_owner_unverified");
            assert_eq!(view["canonical_run_id"], run.plan.scope.run_id);
            assert_eq!(view["owner_result_count"], index + 1);
            let trace: operation_protocol::OperationDagReadTraceV1 =
                serde_json::from_value(view["trace"].clone()).unwrap();
            assert!(trace.has_valid_shape(index + 1));
            if index == 2 {
                assert_eq!(
                    trace
                        .steps
                        .iter()
                        .map(|step| step.operation.id.as_str())
                        .collect::<Vec<_>>(),
                    [
                        "acquisition.prepare_entity_span_input",
                        "semantic.entity_spans",
                        "semantic.canonicalize_mentions"
                    ]
                );
                for (step, expected_queue) in trace.steps.iter().zip([
                    "alegria-acquisition-pure-owner-v1",
                    "alegria-semantic-pure-owner-v1",
                    "alegria-semantic-pure-owner-v1",
                ]) {
                    let start = history
                        .iter()
                        .find_map(|event| match &event.attributes {
                            Some(
                                Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(a),
                            ) if a.workflow_id == step.child_workflow_id => Some(a),
                            _ => None,
                        })
                        .unwrap();
                    assert_eq!(start.task_queue.as_ref().unwrap().name, expected_queue);
                }
                assert_eq!(trace.steps[1].input_steps, [0]);
                assert_eq!(trace.steps[2].input_steps, [1]);
                let proof = &result.produced_evidence[&"span_input".into()];
                assert_eq!(proof.payload_hash, owner_content_hash(&frozen));
                let consumer = history
                    .iter()
                    .find_map(|event| match &event.attributes {
                        Some(Attributes::StartChildWorkflowExecutionInitiatedEventAttributes(
                            a,
                        )) if a.workflow_id
                            == result.produced_evidence[&"mentions".into()].child_workflow_id =>
                        {
                            a.input.as_ref()
                        }
                        _ => None,
                    })
                    .unwrap();
                let wire = String::from_utf8_lossy(&consumer.payloads[0].data);
                assert!(
                    wire.contains(&proof.payload_hash) && wire.contains(&proof.child_workflow_id)
                );
            }
            let expected = CanonicalizeMentions::evaluate(&typed);
            assert_eq!(
                result.artifacts[&"canonical".into()].payload,
                serde_json::to_vec(&expected).unwrap()
            );
            let producer_start = executions
                .iter()
                .find(|(_, id)| id == &proof.child_workflow_id)
                .unwrap()
                .0;
            let producer_complete = history
                .iter()
                .find_map(|event| match &event.attributes {
                    Some(Attributes::ChildWorkflowExecutionCompletedEventAttributes(a))
                        if a.initiated_event_id == producer_start =>
                    {
                        Some(event.event_id)
                    }
                    _ => None,
                })
                .expect("producer completion in real parent History");
            let consumer_start = executions
                .iter()
                .find(|(_, id)| {
                    index == 1 && id != &proof.child_workflow_id
                        || index == 2
                            && id != &proof.child_workflow_id
                            && id
                                != &result.produced_evidence[&"span_input".into()].child_workflow_id
                })
                .unwrap()
                .0;
            assert!(
                producer_complete < consumer_start,
                "Produced consumer follows completed producer"
            );
        }
    }
}
