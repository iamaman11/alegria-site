use super::*;
use operation_protocol::{ExternalArtifact, OperationNode, PlanHash, PLAN_SCHEMA_REVISION};
use seo_domain::canonical_mapping;
use seo_domain::extraction::{editorial_topics, entity_spans, layer_routing, operational_entities};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

type Seed = (ArtifactId, ExternalArtifact, ArtifactValue);

fn scope() -> RunScope {
    RunScope {
        tenant_id: "test-tenant".to_owned(),
        run_id: "test-run".to_owned(),
    }
}

fn seed<O: Operation>(id: &str, input: &O::Input) -> Seed {
    let schema = O::contract()
        .inputs
        .get(&PortId::from("input"))
        .expect("one typed input")
        .clone();
    let provenance = format!("verified:source-section/{id}");
    (
        id.into(),
        ExternalArtifact {
            schema: schema.clone(),
            scope: scope(),
            provenance: provenance.clone(),
        },
        ArtifactValue {
            schema,
            scope: scope(),
            provenance,
            payload: serde_json::to_vec(input).unwrap(),
        },
    )
}

fn node<O: Operation>(instance: &str, input: &str, output: &str) -> OperationNode {
    OperationNode {
        instance: instance.into(),
        operation: O::contract().reference,
        inputs: BTreeMap::from([(
            "input".into(),
            ArtifactBinding::External {
                artifact: input.into(),
            },
        )]),
        outputs: BTreeMap::from([("output".into(), output.into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    }
}

fn plan(
    registry: &OperationRegistry,
    nodes: Vec<OperationNode>,
    seeds: Vec<Seed>,
) -> (ExecutionPlan, BTreeMap<ArtifactId, ArtifactValue>) {
    let mut external_artifacts = BTreeMap::new();
    let mut admitted = BTreeMap::new();
    for (id, declared, value) in seeds {
        external_artifacts.insert(id.clone(), declared);
        admitted.insert(id, value);
    }
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: scope(),
        allowed_effects: BTreeSet::new(),
        external_artifacts,
        pinned_contracts: Vec::new(),
        nodes,
        hash: PlanHash(String::new()),
    }
    .seal(registry.catalog())
    .unwrap();
    (plan, admitted)
}

fn router_input() -> layer_routing::LayerRouterInput {
    layer_routing::LayerRouterInput {
        section_id: "verified-visa".into(),
        heading_text: "Документы и сбор".into(),
        raw_text: concat!(
            "Паспорт и страховка обязательны. Сбор 80 EUR, ",
            "срок 15 дней. Почему отказ?"
        )
        .into(),
        source_tier: "official".into(),
        block_type: "instruction".into(),
    }
}

fn entity_input() -> entity_spans::EntitySpanInput {
    entity_spans::EntitySpanInput {
        section_id: "verified-visa".into(),
        raw_text: router_input().raw_text,
    }
}

fn editorial_input() -> editorial_topics::EditorialExtractionInput {
    editorial_topics::EditorialExtractionInput {
        section_id: "verified-visa".into(),
        raw_text: router_input().raw_text,
    }
}

fn operational_input() -> operational_entities::OperationalExtractionInput {
    operational_entities::OperationalExtractionInput {
        section_id: "verified-operations".into(),
        raw_text: concat!(
            "График работы и выходные. ",
            "Запись по предварительному appointment."
        )
        .into(),
    }
}

fn result<T: serde::de::DeserializeOwned>(result: &ExecutionResult, id: &str) -> T {
    serde_json::from_slice(&result.artifacts.get(&ArtifactId::from(id)).unwrap().payload).unwrap()
}

#[test]
fn real_pure_owners_declare_no_truth_invalidation_under_plan_revision_two() {
    assert_eq!(PLAN_SCHEMA_REVISION, 2);
    for contract in [
        LayerRoute::contract(),
        EntitySpans::contract(),
        CanonicalizeMentions::contract(),
        EditorialTopics::contract(),
        OperationalEntities::contract(),
    ] {
        assert!(contract.invalidates.is_empty());
        assert!(contract.effects.is_empty());
        assert_eq!(
            contract.idempotency,
            operation_protocol::IdempotencyPolicy::Pure
        );
    }
}

#[test]
fn real_operations_execute_in_a_composable_order_without_a_site_pipeline() {
    let registry = semantic_extraction_registry().unwrap();
    let mut entity = node::<EntitySpans>("entity", "entity-input", "entity-output");
    entity.after.insert("route".into());
    let nodes = vec![
        node::<EditorialTopics>("editorial", "editorial-input", "editorial-output"),
        entity,
        node::<LayerRoute>("route", "route-input", "route-output"),
    ];
    let (plan, admitted) = plan(
        &registry,
        nodes,
        vec![
            seed::<LayerRoute>("route-input", &router_input()),
            seed::<EntitySpans>("entity-input", &entity_input()),
            seed::<EditorialTopics>("editorial-input", &editorial_input()),
        ],
    );
    let executed = registry.execute(&plan, admitted).unwrap();
    let order = executed
        .order
        .iter()
        .map(|instance| instance.as_str())
        .collect::<Vec<_>>();
    assert_eq!(order, ["editorial", "route", "entity"]);

    let routed: layer_routing::LayerRouterOutput = result(&executed, "route-output");
    assert_eq!(routed.primary_layer, "procedural");
    let spans: entity_spans::EntitySpanOutput = result(&executed, "entity-output");
    assert!(spans
        .mentions
        .iter()
        .any(|mention| { mention.entity_type == "fee" && mention.raw_text == "80 EUR" }));
    let topics: editorial_topics::EditorialExtractionOutput = result(&executed, "editorial-output");
    assert!(topics
        .topics
        .iter()
        .any(|topic| topic.topic_type == "faq_topic"));
}

#[test]
fn subset_scenario_runs_editorial_without_required_acquisition_or_publish() {
    let registry = semantic_extraction_registry().unwrap();
    let (plan, admitted) = plan(
        &registry,
        vec![node::<EditorialTopics>(
            "faq",
            "editorial-input",
            "faq-output",
        )],
        vec![seed::<EditorialTopics>(
            "editorial-input",
            &editorial_input(),
        )],
    );
    let executed = registry.execute(&plan, admitted).unwrap();
    assert_eq!(executed.order, [OperationInstanceId::from("faq")]);
    let topics: editorial_topics::EditorialExtractionOutput = result(&executed, "faq-output");
    assert!(topics
        .topics
        .iter()
        .any(|topic| topic.topic_type == "faq_topic"));
    assert!(!executed.artifacts.contains_key(&"route-output".into()));
}

#[test]
fn repeated_real_operation_is_not_a_retry_and_new_owner_binding_works() {
    let registry = semantic_extraction_registry().unwrap();
    let nodes = vec![
        node::<EntitySpans>("mentions-2", "entity-input", "mentions-output-2"),
        node::<OperationalEntities>("operations", "operations-input", "operations-output"),
        node::<EntitySpans>("mentions-1", "entity-input", "mentions-output-1"),
    ];
    let (plan, admitted) = plan(
        &registry,
        nodes,
        vec![
            seed::<EntitySpans>("entity-input", &entity_input()),
            seed::<OperationalEntities>("operations-input", &operational_input()),
        ],
    );
    let checked = plan.validate(registry.catalog()).unwrap();
    let first_key = checked
        .execution_key(&"mentions-1".into(), "same-input")
        .unwrap();
    let retry_key = checked
        .execution_key(&"mentions-1".into(), "same-input")
        .unwrap();
    let second_key = checked
        .execution_key(&"mentions-2".into(), "same-input")
        .unwrap();
    assert_eq!(first_key, retry_key);
    assert_ne!(first_key, second_key);

    let executed = registry.execute(&plan, admitted).unwrap();
    let first: entity_spans::EntitySpanOutput = result(&executed, "mentions-output-1");
    let second: entity_spans::EntitySpanOutput = result(&executed, "mentions-output-2");
    assert_eq!(first.mentions.len(), second.mentions.len());
    assert!(!first.mentions.is_empty());
    let operations: operational_entities::OperationalExtractionOutput =
        result(&executed, "operations-output");
    assert!(operations
        .entities
        .iter()
        .any(|entity| { entity.entity_kind == "office_schedule" }));
}

#[test]
fn real_entity_output_is_consumed_by_canonical_mapping_through_a_dag_edge() {
    let registry = semantic_extraction_registry().unwrap();
    let canonical = OperationNode {
        instance: "canonical".into(),
        operation: CanonicalizeMentions::contract().reference,
        inputs: BTreeMap::from([(
            "input".into(),
            ArtifactBinding::Produced {
                producer: "entities".into(),
                port: "output".into(),
                artifact: "entity-output".into(),
            },
        )]),
        outputs: BTreeMap::from([("output".into(), "canonical-output".into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    };
    let (plan, admitted) = plan(
        &registry,
        vec![
            canonical,
            node::<EntitySpans>("entities", "entity-input", "entity-output"),
        ],
        vec![seed::<EntitySpans>("entity-input", &entity_input())],
    );
    let checked = plan.validate(registry.catalog()).unwrap();
    assert_eq!(
        checked.execution_order(),
        [
            OperationInstanceId::from("entities"),
            OperationInstanceId::from("canonical")
        ]
    );
    let executed = registry.execute(&plan, admitted).unwrap();
    let mapping: canonical_mapping::CanonicalMappingOutput = result(&executed, "canonical-output");
    assert!(mapping.mappings.iter().any(|item| {
        item.raw_text == "80 EUR" && item.canonical_key.as_deref() == Some("consular_fee")
    }));
    assert!(mapping.mappings.iter().any(|item| {
        item.raw_text == "15 дней" && item.canonical_key.as_deref() == Some("processing_time")
    }));
}

#[test]
fn typed_validation_blocks_wrong_schema_or_scope_before_operation_execution() {
    let mut registry = semantic_extraction_registry().unwrap();
    let (plan, admitted) = plan(
        &registry,
        vec![node::<EntitySpans>("mentions", "input", "output")],
        vec![seed::<EntitySpans>("input", &entity_input())],
    );
    let invocations = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::clone(&invocations);
    registry.handlers.insert(
        EntitySpans::contract().reference,
        Box::new(move |_inputs| {
            recorded.fetch_add(1, Ordering::SeqCst);
            Ok(BTreeMap::new())
        }),
    );

    let mut wrong_schema = plan.clone();
    wrong_schema
        .external_artifacts
        .get_mut(&"input".into())
        .unwrap()
        .schema
        .revision = 9;
    wrong_schema = wrong_schema.seal(registry.catalog()).unwrap();
    assert!(matches!(
        registry.execute(&wrong_schema, admitted.clone()),
        Err(OperationRunError::InvalidPlan(
            PlanError::ArtifactSchemaMismatch { .. }
        ))
    ));
    assert_eq!(invocations.load(Ordering::SeqCst), 0);

    let mut wrong_scope = admitted;
    wrong_scope
        .get_mut(&"input".into())
        .unwrap()
        .scope
        .tenant_id = "another-tenant".into();
    assert!(matches!(
        registry.execute(&plan, wrong_scope),
        Err(OperationRunError::InvalidArtifactEnvelope(_))
    ));
    assert_eq!(invocations.load(Ordering::SeqCst), 0);
}

#[test]
fn malformed_typed_payload_fails_closed_at_the_only_json_boundary() {
    let registry = semantic_extraction_registry().unwrap();
    let (plan, mut admitted) = plan(
        &registry,
        vec![node::<EntitySpans>("mentions", "input", "output")],
        vec![seed::<EntitySpans>("input", &entity_input())],
    );
    admitted.get_mut(&"input".into()).unwrap().payload = b"[]".to_vec();
    assert!(matches!(
        registry.execute(&plan, admitted),
        Err(OperationRunError::InvalidInput { .. })
    ));
}

#[test]
fn durable_pure_dag_executes_real_operations_with_immutable_payloads() {
    let registry = semantic_extraction_registry().unwrap();
    let (plan, admitted) = plan(
        &registry,
        vec![
            node::<EditorialTopics>("faq", "editorial-input", "faq-output"),
            node::<EntitySpans>("spans", "entity-input", "spans-output"),
        ],
        vec![
            seed::<EditorialTopics>("editorial-input", &editorial_input()),
            seed::<EntitySpans>("entity-input", &entity_input()),
        ],
    );
    let wire = serde_json::to_vec(&PureDagRun { plan, admitted }).unwrap();
    let recorded: PureDagRun = serde_json::from_slice(&wire).unwrap();
    let receipt = execute_pure_dag("test-run", &recorded).unwrap();
    assert_eq!(receipt.plan_hash, recorded.plan.hash);
    assert_eq!(receipt.order.len(), 2);
    let encoded_receipt = serde_json::to_vec(&receipt).unwrap();
    let replayed: PureDagReceipt = serde_json::from_slice(&encoded_receipt).unwrap();
    assert_eq!(serde_json::to_vec(&replayed).unwrap(), encoded_receipt);
    let faq: editorial_topics::EditorialExtractionOutput =
        serde_json::from_slice(&replayed.artifacts[&ArtifactId::from("faq-output")].payload)
            .unwrap();
    assert!(faq
        .topics
        .iter()
        .any(|topic| topic.topic_type == "faq_topic"));
}

#[test]
fn pure_dag_rejects_changed_or_effectful_input_without_repinning() {
    let registry = semantic_extraction_registry().unwrap();
    let (plan, admitted) = plan(
        &registry,
        vec![node::<EntitySpans>("spans", "entity-input", "spans-output")],
        vec![seed::<EntitySpans>("entity-input", &entity_input())],
    );
    let original = PureDagRun { plan, admitted };
    assert!(matches!(
        execute_pure_dag("another-temporal-workflow", &original),
        Err(OperationRunError::TemporalRunIdMismatch)
    ));

    let mut altered = original.clone();
    altered.plan.nodes[0].instance = "different-instance".into();
    assert!(matches!(
        execute_pure_dag("test-run", &altered),
        Err(OperationRunError::InvalidPlan(PlanError::PlanHashMismatch))
    ));

    let mut effects = original;
    effects
        .plan
        .allowed_effects
        .insert("publication.write".into());
    effects.plan = effects.plan.seal(registry.catalog()).unwrap();
    assert!(matches!(
        execute_pure_dag("test-run", &effects),
        Err(OperationRunError::EffectfulPlanEnvelope)
    ));
}

#[test]
fn pure_dag_rejects_empty_success_before_any_domain_execution() {
    let registry = semantic_extraction_registry().unwrap();
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: scope(),
        allowed_effects: BTreeSet::new(),
        external_artifacts: BTreeMap::new(),
        pinned_contracts: Vec::new(),
        nodes: Vec::new(),
        hash: PlanHash(String::new()),
    }
    .seal(registry.catalog())
    .unwrap();
    assert!(matches!(
        execute_pure_dag(
            "test-run",
            &PureDagRun {
                plan,
                admitted: BTreeMap::new()
            }
        ),
        Err(OperationRunError::EmptyPurePlan)
    ));
}

#[test]
fn missing_late_executor_is_rejected_before_any_pure_operation_runs() {
    let mut registry = semantic_extraction_registry().unwrap();
    let (plan, admitted) = plan(
        &registry,
        vec![
            node::<EditorialTopics>("a-first", "editorial-input", "faq-output"),
            node::<EntitySpans>("z-later", "entity-input", "spans-output"),
        ],
        vec![
            seed::<EditorialTopics>("editorial-input", &editorial_input()),
            seed::<EntitySpans>("entity-input", &entity_input()),
        ],
    );
    let invocations = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::clone(&invocations);
    registry.handlers.insert(
        EditorialTopics::contract().reference,
        Box::new(move |_inputs| {
            recorded.fetch_add(1, Ordering::SeqCst);
            Ok(BTreeMap::new())
        }),
    );
    registry.handlers.remove(&EntitySpans::contract().reference);
    assert!(matches!(
        registry.execute(&plan, admitted),
        Err(OperationRunError::MissingExecutor(_))
    ));
    assert_eq!(invocations.load(Ordering::SeqCst), 0);
}

#[test]
fn pure_dag_enforces_exact_json_input_limit_without_serialization_allocation() {
    // A JSON string includes the opening and closing quotation marks. The
    // counting writer accepts exactly the limit and fails at the first excess
    // byte, without allocating the entire serialized representation.
    enforce_json_budget(&"abcd", 6, "input_bytes").unwrap();
    assert!(matches!(
        enforce_json_budget(&"abcde", 6, "input_bytes"),
        Err(OperationRunError::ResourceBudgetExceeded {
            resource: "input_bytes",
            limit: 6
        })
    ));

    let registry = semantic_extraction_registry().unwrap();
    let (plan, mut admitted) = plan(
        &registry,
        vec![node::<EntitySpans>("spans", "input", "output")],
        vec![seed::<EntitySpans>("input", &entity_input())],
    );
    admitted.get_mut(&"input".into()).unwrap().payload = vec![b'x'; PURE_DAG_MAX_INPUT_BYTES];
    // Even though this payload is not a valid typed EntitySpans input, the
    // wire budget must be rejected before any operation deserialization.
    assert!(matches!(
        execute_pure_dag("test-run", &PureDagRun { plan, admitted }),
        Err(OperationRunError::ResourceBudgetExceeded {
            resource: "input_bytes",
            limit: PURE_DAG_MAX_INPUT_BYTES
        })
    ));
}

#[test]
fn pure_dag_node_budget_rejects_oversize_before_executor_validation() {
    let registry = semantic_extraction_registry().unwrap();
    let nodes = (0..PURE_DAG_MAX_NODES)
        .map(|index| {
            node::<EntitySpans>(
                &format!("node-{index}"),
                "input",
                &format!("output-{index}"),
            )
        })
        .collect();
    let (plan, admitted) = plan(
        &registry,
        nodes,
        vec![seed::<EntitySpans>("input", &entity_input())],
    );
    let mut run = PureDagRun { plan, admitted };
    enforce_pure_dag_input_budget(&run).unwrap();
    run.plan
        .nodes
        .push(node::<EntitySpans>("node-extra", "input", "output-extra"));
    // The mutated manifest also has a stale hash. A too-large manifest must
    // be rejected first without ever reaching the operation executor.
    assert!(matches!(
        execute_pure_dag("test-run", &run),
        Err(OperationRunError::ResourceBudgetExceeded {
            resource: "operation_nodes",
            limit: PURE_DAG_MAX_NODES
        })
    ));
}

#[test]
fn pure_dag_result_budget_is_measured_before_temporal_history_return() {
    let registry = semantic_extraction_registry().unwrap();
    let (plan, admitted) = plan(
        &registry,
        vec![node::<EntitySpans>("spans", "input", "output")],
        vec![seed::<EntitySpans>("input", &entity_input())],
    );
    let receipt = execute_pure_dag("test-run", &PureDagRun { plan, admitted }).unwrap();
    let exact = serde_json::to_vec(&receipt).unwrap().len();
    enforce_json_budget(&receipt, exact, "result_bytes").unwrap();
    assert!(matches!(
        enforce_json_budget(&receipt, exact - 1, "result_bytes"),
        Err(OperationRunError::ResourceBudgetExceeded {
            resource: "result_bytes",
            ..
        })
    ));
}
