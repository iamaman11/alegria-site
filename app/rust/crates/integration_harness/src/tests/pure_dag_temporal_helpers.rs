// Real pure domain operations, composed independently from any default site
// workflow. These are not a new fixed business-operation catalog; they are
// typed test manifests used by the already-running Temporal integration test.
#[cfg(feature = "e2e")]
fn pure_dag_e2e_case(case: &str, run_id: &str) -> seo_application::operation_adapter::PureDagRun {
    use operation_protocol::{
        ArtifactBinding, ArtifactId, ArtifactSchema, ExecutionPlan, ExternalArtifact, Operation,
        OperationNode, PlanHash, PortId, RunScope, PLAN_SCHEMA_REVISION,
    };
    use seo_application::operation_adapter::{
        semantic_extraction_registry, ArtifactValue, PureDagRun,
    };
    use seo_domain::extraction::operations::{
        CanonicalizeMentions, EditorialTopics, EntitySpans, LayerRoute, OperationalEntities,
    };
    use seo_domain::extraction::{
        editorial_topics, entity_spans, layer_routing, operational_entities,
    };

    fn seed<O: Operation>(
        id: &str,
        input: &O::Input,
        scope: &RunScope,
    ) -> (ArtifactId, ExternalArtifact, ArtifactValue) {
        let schema: ArtifactSchema = O::contract().inputs[&PortId::from("input")].clone();
        let provenance = format!("test-only-temporal-history/{id}");
        (
            id.into(),
            ExternalArtifact {
                schema: schema.clone(),
                scope: scope.clone(),
                provenance: provenance.clone(),
            },
            ArtifactValue {
                schema,
                scope: scope.clone(),
                provenance,
                payload: serde_json::to_vec(input).unwrap(),
            },
        )
    }

    fn external_node<O: Operation>(id: &str, input: &str, output: &str) -> OperationNode {
        OperationNode {
            instance: id.into(),
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

    let scope = RunScope {
        tenant_id: "pure-dag-fixture-tenant".into(),
        run_id: run_id.into(),
    };
    let text = concat!(
        "Паспорт и страховка обязательны. ",
        "Сбор 80 EUR, срок 15 дней. Почему отказ?"
    );
    let route = layer_routing::LayerRouterInput {
        section_id: "verified-visa".into(),
        heading_text: "Документы и сбор".into(),
        raw_text: text.into(),
        source_tier: "official".into(),
        block_type: "instruction".into(),
    };
    let entities = entity_spans::EntitySpanInput {
        section_id: "verified-visa".into(),
        raw_text: text.into(),
    };
    let editorial = editorial_topics::EditorialExtractionInput {
        section_id: "verified-visa".into(),
        raw_text: text.into(),
    };
    let operations = operational_entities::OperationalExtractionInput {
        section_id: "office-hours".into(),
        raw_text: "График работы и запись по предварительному appointment".into(),
    };

    let (nodes, seeds) = match case {
        // One independent real operation with no acquisition/site default.
        "route-only" => (
            vec![external_node::<LayerRoute>("route", "route-in", "route-out")],
            vec![seed::<LayerRoute>("route-in", &route, &scope)],
        ),
        "entity-only" => (
            vec![external_node::<EntitySpans>("entities", "entities-in", "entity-out")],
            vec![seed::<EntitySpans>("entities-in", &entities, &scope)],
        ),
        // Actual produced artifact, not merely an `after` ordering edge.
        "entity-to-canonical" => (
            vec![
                OperationNode {
                    instance: "canonical".into(),
                    operation: CanonicalizeMentions::contract().reference,
                    inputs: BTreeMap::from([(
                        "input".into(),
                        ArtifactBinding::Produced {
                            producer: "entities".into(),
                            port: "output".into(),
                            artifact: "entity-out".into(),
                        },
                    )]),
                    outputs: BTreeMap::from([("output".into(), "canonical-out".into())]),
                    after: BTreeSet::new(),
                    permitted_effects: BTreeSet::new(),
                },
                external_node::<EntitySpans>("entities", "entities-in", "entity-out"),
            ],
            vec![seed::<EntitySpans>("entities-in", &entities, &scope)],
        ),
        // Different subset with two independent owners and an explicit order
        // that need not follow source acquisition or publication phases.
        "independent-editorial-operations" => (
            vec![
                external_node::<OperationalEntities>(
                    "operations",
                    "operations-in",
                    "operations-out",
                ),
                external_node::<EditorialTopics>("editorial", "editorial-in", "editorial-out"),
            ],
            vec![
                seed::<EditorialTopics>("editorial-in", &editorial, &scope),
                seed::<OperationalEntities>("operations-in", &operations, &scope),
            ],
        ),
        other => panic!("unexpected pure DAG Temporal case: {other}"),
    };
    let mut external_artifacts = BTreeMap::new();
    let mut admitted = BTreeMap::new();
    for (id, envelope, payload) in seeds {
        external_artifacts.insert(id.clone(), envelope);
        admitted.insert(id, payload);
    }
    let catalog = semantic_extraction_registry().unwrap();
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope,
        allowed_effects: BTreeSet::new(),
        external_artifacts,
        pinned_contracts: Vec::new(),
        nodes,
        hash: PlanHash(String::new()),
    }
    .seal(catalog.catalog())
    .unwrap();
    PureDagRun { plan, admitted }
}
