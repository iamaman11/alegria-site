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

/// Each call is its own sealed, one-node V2 DAG, with one *typed* source DTO.
/// No other operation supplies a prerequisite or runs as a side effect.
/// This exercises the native pure registry, not a Temporal worker/Activity.
fn prove_independent_pure_operation<O: PureExtractionOperation>(
    expected_id: &str,
    input: &O::Input,
) {
    // Prove registration itself needs no neighboring owner: the direct
    // registry contains exactly this one concrete, versioned operation.
    let mut registry = OperationRegistry::default();
    registry.register_pure::<O>().unwrap();
    let contract = O::contract();
    let registered = registry.catalog().contracts().collect::<Vec<_>>();
    assert_eq!(registered.len(), 1);
    assert_eq!(registered[0].reference, contract.reference);
    assert_eq!(contract.reference.id.as_str(), expected_id);
    assert_eq!(contract.reference.revision, 1);
    assert_eq!(
        contract.idempotency,
        operation_protocol::IdempotencyPolicy::Pure
    );
    assert!(contract.effects.is_empty());
    assert!(contract.invalidates.is_empty());

    let (plan, admitted) = plan(
        &registry,
        vec![node::<O>("only-operation", "typed-input", "typed-output")],
        vec![seed::<O>("typed-input", input)],
    );
    assert_eq!(plan.schema_revision, PLAN_SCHEMA_REVISION);
    assert_eq!(plan.nodes.len(), 1);
    assert_eq!(plan.external_artifacts.len(), 1);
    assert!(plan.allowed_effects.is_empty());
    assert!(plan.nodes[0].after.is_empty());
    assert!(plan.nodes[0].permitted_effects.is_empty());
    assert_eq!(plan.nodes[0].operation, contract.reference);

    let standalone = registry.execute(&plan, admitted.clone()).unwrap();
    assert_eq!(
        standalone.order,
        [OperationInstanceId::from("only-operation")]
    );

    let run = PureDagRun {
        plan: plan.clone(),
        admitted: admitted.clone(),
    };
    let receipt = execute_pure_dag("test-run", &run).unwrap();
    assert_eq!(receipt.plan_hash, plan.hash);
    assert_eq!(receipt.order, [OperationInstanceId::from("only-operation")]);
    // The single source and the single produced artifact are the *only*
    // artifacts: there is no hidden acquisition, sibling owner or publisher.
    assert_eq!(receipt.artifacts.len(), 2);
    assert_eq!(
        receipt.artifacts[&ArtifactId::from("typed-output")].payload,
        standalone.artifacts[&ArtifactId::from("typed-output")].payload
    );
    assert_eq!(
        receipt.artifacts[&ArtifactId::from("typed-output")].schema,
        contract.outputs[&PortId::from("output")]
    );
    let decoded: O::Output =
        serde_json::from_slice(&receipt.artifacts[&ArtifactId::from("typed-output")].payload)
            .unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        serde_json::to_value(O::evaluate(input)).unwrap(),
        "{expected_id}: executable registry output differs from the typed owner"
    );

    // The envelope's declared artifact type is pinned to the real owner
    // contract. Changing only the admitted schema must fail before dispatch.
    let mut wrong_schema = admitted.clone();
    wrong_schema
        .get_mut(&ArtifactId::from("typed-input"))
        .unwrap()
        .schema
        .revision += 1;
    assert!(matches!(
        execute_pure_dag(
            "test-run",
            &PureDagRun {
                plan: plan.clone(),
                admitted: wrong_schema,
            }
        ),
        Err(OperationRunError::InvalidArtifactEnvelope(_))
    ));

    // A syntactically valid JSON value of the wrong DTO shape cannot be
    // interpreted as any of these five concrete owner input structs.
    let mut wrong_type = admitted.clone();
    wrong_type
        .get_mut(&ArtifactId::from("typed-input"))
        .unwrap()
        .payload = b"[]".to_vec();
    assert!(matches!(
        execute_pure_dag(
            "test-run",
            &PureDagRun {
                plan: plan.clone(),
                admitted: wrong_type,
            }
        ),
        Err(OperationRunError::InvalidInput { operation, .. })
            if operation == contract.reference
    ));

    // Identical bytes originating from another tenant are not admissible.
    let mut wrong_scope = admitted;
    wrong_scope
        .get_mut(&ArtifactId::from("typed-input"))
        .unwrap()
        .scope
        .tenant_id = "other-tenant".to_owned();
    assert!(matches!(
        execute_pure_dag(
            "test-run",
            &PureDagRun {
                plan,
                admitted: wrong_scope,
            }
        ),
        Err(OperationRunError::InvalidArtifactEnvelope(_))
    ));
}

#[test]
fn installed_pure_registry_contains_exactly_five_named_semantic_operations() {
    let registry = semantic_extraction_registry().unwrap();
    let installed = registry
        .catalog()
        .contracts()
        .map(|contract| contract.reference.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        installed,
        [
            "semantic.canonicalize_mentions",
            "semantic.editorial_topics",
            "semantic.entity_spans",
            "semantic.layer_route",
            "semantic.operational_entities",
        ]
    );
}

#[test]
fn pure_runner_rejects_unknown_owner_input_fields_before_execution() {
    let registry = semantic_extraction_registry().unwrap();
    let (plan, mut admitted) = plan(
        &registry,
        vec![node::<LayerRoute>("only", "source", "output")],
        vec![seed::<LayerRoute>("source", &router_input())],
    );
    let mut raw = serde_json::to_value(router_input()).unwrap();
    raw.as_object_mut()
        .unwrap()
        .insert("ignored_override".into(), serde_json::json!(true));
    admitted
        .get_mut(&ArtifactId::from("source"))
        .unwrap()
        .payload = serde_json::to_vec(&raw).unwrap();
    let err = execute_pure_dag("test-run", &PureDagRun { plan, admitted }).unwrap_err();
    assert!(matches!(
        err,
        OperationRunError::InvalidInput { operation, reason, .. }
            if operation == LayerRoute::contract().reference
                && reason.contains("discarded or altered")
    ));
}

#[test]
fn pure_runner_rejects_duplicate_json_fields_before_typed_owner_decode() {
    let registry = semantic_extraction_registry().unwrap();
    let (plan, mut admitted) = plan(
        &registry,
        vec![node::<LayerRoute>("only", "source", "output")],
        vec![seed::<LayerRoute>("source", &router_input())],
    );
    admitted.get_mut(&ArtifactId::from("source")).unwrap().payload =
        br#"{"section_id":"verified-visa","section_id":"forged", "heading_text":"Docs", "raw_text":"Passport", "source_tier":"official", "block_type":"instruction"}"#.to_vec();
    let err = execute_pure_dag("test-run", &PureDagRun { plan, admitted }).unwrap_err();
    assert!(matches!(
        err,
        OperationRunError::InvalidInput { operation, reason, .. }
            if operation == LayerRoute::contract().reference
                && reason.contains("duplicate operation JSON field")
    ));
}

#[test]
fn historical_v3_serde_ingress_keeps_prior_unknown_field_semantics() {
    let registry = semantic_extraction_registry().unwrap();
    let (plan, mut admitted) = plan(
        &registry,
        vec![node::<LayerRoute>("only", "source", "output")],
        vec![seed::<LayerRoute>("source", &router_input())],
    );
    let mut raw = serde_json::to_value(router_input()).unwrap();
    raw.as_object_mut()
        .unwrap()
        .insert("future_field".into(), serde_json::json!(true));
    admitted
        .get_mut(&ArtifactId::from("source"))
        .unwrap()
        .payload = serde_json::to_vec(&raw).unwrap();

    assert!(matches!(
        execute_pure_dag(
            "test-run",
            &PureDagRun {
                plan: plan.clone(),
                admitted: admitted.clone()
            }
        ),
        Err(OperationRunError::InvalidInput { .. })
    ));
    let replayed = execute_pure_dag_legacy_v3("test-run", &PureDagRun { plan, admitted }).unwrap();
    assert_eq!(replayed.order, [OperationInstanceId::from("only")]);
    assert!(replayed.artifacts.contains_key(&ArtifactId::from("output")));
}

#[test]
fn historical_v3_serde_ingress_keeps_prior_duplicate_key_semantics() {
    let registry = semantic_extraction_registry().unwrap();
    let (plan, mut admitted) = plan(
        &registry,
        vec![node::<LayerRoute>("only", "source", "output")],
        vec![seed::<LayerRoute>("source", &router_input())],
    );
    admitted.get_mut(&ArtifactId::from("source")).unwrap().payload =
        br#"{"section_id":"verified-visa","section_id":"prior-last-wins", "heading_text":"Docs", "raw_text":"Passport", "source_tier":"official", "block_type":"instruction"}"#.to_vec();

    assert!(matches!(
        execute_pure_dag(
            "test-run",
            &PureDagRun {
                plan: plan.clone(),
                admitted: admitted.clone()
            }
        ),
        Err(OperationRunError::InvalidInput { .. })
    ));
    let replayed = execute_pure_dag_legacy_v3("test-run", &PureDagRun { plan, admitted }).unwrap();
    assert_eq!(replayed.order, [OperationInstanceId::from("only")]);
}

#[test]
fn historical_v3_and_strict_v4_agree_on_lossless_typed_input() {
    let registry = semantic_extraction_registry().unwrap();
    let (plan, admitted) = plan(
        &registry,
        vec![node::<LayerRoute>("only", "source", "output")],
        vec![seed::<LayerRoute>("source", &router_input())],
    );
    let run = PureDagRun { plan, admitted };
    let strict = execute_pure_dag("test-run", &run).unwrap();
    let legacy = execute_pure_dag_legacy_v3("test-run", &run).unwrap();
    assert_eq!(strict.plan_hash, legacy.plan_hash);
    assert_eq!(strict.order, legacy.order);
    assert_eq!(
        strict.artifacts.keys().collect::<Vec<_>>(),
        legacy.artifacts.keys().collect::<Vec<_>>()
    );
    for (id, value) in strict.artifacts {
        let previous = &legacy.artifacts[&id];
        assert_eq!(value.schema, previous.schema);
        assert_eq!(value.scope, previous.scope);
        assert_eq!(value.provenance, previous.provenance);
        assert_eq!(value.payload, previous.payload);
    }
}

#[test]
fn layer_route_executes_as_an_independent_typed_pure_dag() {
    prove_independent_pure_operation::<LayerRoute>("semantic.layer_route", &router_input());
}

#[test]
fn entity_spans_executes_as_an_independent_typed_pure_dag() {
    prove_independent_pure_operation::<EntitySpans>("semantic.entity_spans", &entity_input());
}

#[test]
fn canonicalize_mentions_executes_as_an_independent_typed_pure_dag() {
    // Its input is the actual typed EntitySpanOutput DTO, admitted directly
    // as an independent source. The EntitySpans operation is *not* scheduled.
    let typed_input = entity_spans::execute(&entity_input());
    prove_independent_pure_operation::<CanonicalizeMentions>(
        "semantic.canonicalize_mentions",
        &typed_input,
    );
}

#[test]
fn editorial_topics_executes_as_an_independent_typed_pure_dag() {
    prove_independent_pure_operation::<EditorialTopics>(
        "semantic.editorial_topics",
        &editorial_input(),
    );
}

#[test]
fn operational_entities_executes_as_an_independent_typed_pure_dag() {
    prove_independent_pure_operation::<OperationalEntities>(
        "semantic.operational_entities",
        &operational_input(),
    );
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
