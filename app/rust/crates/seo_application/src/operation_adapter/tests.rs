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
