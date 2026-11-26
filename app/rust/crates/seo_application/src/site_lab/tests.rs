use super::*;
use crate::operation_adapter::semantic_extraction_registry;
use operation_protocol::{
    ArtifactTypeId, OperationContract, OperationId, OperationOwnerId, PortId,
};

fn op(id: &str) -> OperationRef {
    OperationRef {
        id: OperationId::from(id),
        revision: 1,
    }
}

fn source(id: &str) -> RecipeSource {
    RecipeSource {
        schema: ArtifactSchema {
            id: ArtifactTypeId::from(id),
            revision: 1,
        },
        provenance: "example:unverified-source".to_owned(),
    }
}

fn external(artifact: &str) -> BTreeMap<PortId, RecipeInput> {
    BTreeMap::from([(
        PortId::from("input"),
        RecipeInput::External {
            artifact: ArtifactId::from(artifact),
        },
    )])
}

fn step(name: &str, operation: &str, inputs: BTreeMap<PortId, RecipeInput>) -> RecipeStep {
    RecipeStep {
        id: OperationInstanceId::from(name),
        operation: op(operation),
        inputs,
        after: BTreeSet::new(),
    }
}

fn recipe() -> SiteRecipe {
    SiteRecipe {
        schema_revision: 1,
        scope: RunScope {
            tenant_id: "lab".to_owned(),
            run_id: "hypothesis-1".to_owned(),
        },
        sources: BTreeMap::from([
            (
                ArtifactId::from("mentions"),
                source("semantic.entity_span_input"),
            ),
            (
                ArtifactId::from("topics"),
                source("semantic.editorial_extraction_input"),
            ),
        ]),
        steps: vec![
            step("extract", "semantic.entity_spans", external("mentions")),
            step(
                "canonicalize",
                "semantic.canonicalize_mentions",
                BTreeMap::from([(
                    PortId::from("input"),
                    RecipeInput::Produced {
                        step: OperationInstanceId::from("extract"),
                        port: PortId::from("output"),
                    },
                )]),
            ),
            step("editorial", "semantic.editorial_topics", external("topics")),
        ],
    }
}

fn catalog() -> ContractCatalog {
    semantic_extraction_registry()
        .expect("real semantic operations are registered")
        .catalog()
        .clone()
}

#[test]
fn registered_tools_are_discoverable_and_never_claim_a_live_provider() {
    let mut registry = catalog();
    let catalog_before = describe_tools(&registry).unwrap();
    assert_eq!(catalog_before.len(), 5);
    assert!(catalog_before
        .iter()
        .all(|tool| tool.readiness == "pure_plan_only"));
    assert!(catalog_before
        .iter()
        .all(|tool| tool.owner == "semantic_extraction"));

    registry
        .register_contract(OperationContract {
            reference: op("future.search_demand"),
            owner: OperationOwnerId::from("future_provider_owner"),
            inputs: BTreeMap::from([(
                PortId::from("input"),
                source("research.query_input").schema,
            )]),
            outputs: BTreeMap::from([(
                PortId::from("output"),
                source("research.query_output").schema,
            )]),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::from([EffectId::from("provider.search")]),
            idempotency: IdempotencyPolicy::Idempotent,
        })
        .unwrap();
    let catalog_after = describe_tools(&registry).unwrap();
    assert_eq!(catalog_after.len(), 6);
    assert!(catalog_after.iter().any(|tool| {
        tool.operation == op("future.search_demand") && tool.readiness == "requires_owner_admission"
    }));

    let external_recipe = SiteRecipe {
        schema_revision: 1,
        scope: recipe().scope,
        sources: BTreeMap::from([(ArtifactId::from("query"), source("research.query_input"))]),
        steps: vec![step("demand", "future.search_demand", external("query"))],
    };
    let preview = preview_recipe(&external_recipe, &registry).unwrap();
    assert_eq!(preview.execution, "not_started");
    assert_eq!(preview.source_authentication, "not_verified");
    assert_eq!(
        preview.steps_requiring_owner_admission,
        vec![OperationInstanceId::from("demand")]
    );
}

#[test]
fn independent_steps_have_one_stable_plan_hash() {
    let registry = catalog();
    let first = recipe();
    let mut shuffled = first.clone();
    shuffled.steps.reverse();
    let first_preview = preview_recipe(&first, &registry).unwrap();
    let second_preview = preview_recipe(&shuffled, &registry).unwrap();
    assert_eq!(first_preview.plan_hash, second_preview.plan_hash);
    assert_eq!(first_preview.ordered_steps, second_preview.ordered_steps);
    assert_eq!(first_preview.steps_requiring_owner_admission.len(), 0);
    assert_eq!(first_preview.execution, "not_started");
    assert_eq!(first_preview.source_count, 2);
}

#[test]
fn editing_a_recipe_changes_a_hypothesis_without_a_rebuild() {
    let registry = catalog();
    let original = recipe();
    let mut alternative = original.clone();
    alternative.steps[2]
        .after
        .insert(OperationInstanceId::from("canonicalize"));
    let diff = diff_recipes(&original, &alternative, &registry).unwrap();
    assert_eq!(diff.status, "structural_comparison_only");
    assert!(!diff.same_plan);
    assert!(diff.same_declared_sources);
    assert!(diff.note.contains("not measured"));
}

#[test]
fn invalid_ports_and_missing_producers_produce_actionable_errors() {
    let registry = catalog();
    let mut wrong_type = recipe();
    wrong_type.steps[1].inputs = external("topics");
    assert_eq!(
        preview_recipe(&wrong_type, &registry)
            .unwrap_err()
            .problem()
            .code,
        "incompatible_ports"
    );

    let mut missing = recipe();
    missing.steps[1].inputs.insert(
        PortId::from("input"),
        RecipeInput::Produced {
            step: OperationInstanceId::from("unregistered_producer"),
            port: PortId::from("output"),
        },
    );
    let problem = preview_recipe(&missing, &registry).unwrap_err().problem();
    assert!(matches!(problem.code, "missing_output" | "unknown_step"));
}

#[test]
fn cycles_and_undeclared_sources_fail_closed() {
    let registry = catalog();
    let mut cyclic = recipe();
    cyclic.steps[0]
        .after
        .insert(OperationInstanceId::from("canonicalize"));
    assert_eq!(
        preview_recipe(&cyclic, &registry)
            .unwrap_err()
            .problem()
            .code,
        "recipe_cycle"
    );
    let mut unknown = recipe();
    unknown.sources.remove(&ArtifactId::from("mentions"));
    assert_eq!(
        preview_recipe(&unknown, &registry)
            .unwrap_err()
            .problem()
            .code,
        "missing_source"
    );
}

#[test]
fn revision_provenance_and_unregistered_operations_are_explicit() {
    let registry = catalog();
    let mut invalid_revision = recipe();
    invalid_revision.schema_revision = 2;
    assert_eq!(
        preview_recipe(&invalid_revision, &registry)
            .unwrap_err()
            .problem()
            .code,
        "recipe_revision_unsupported"
    );
    let mut untrusted = recipe();
    untrusted.steps[0].operation = op("user.manifest.fake_provider");
    assert_eq!(
        preview_recipe(&untrusted, &registry)
            .unwrap_err()
            .problem()
            .code,
        "operation_unavailable"
    );
    let mut provenance = recipe();
    provenance
        .sources
        .get_mut(&ArtifactId::from("mentions"))
        .unwrap()
        .provenance = "  ".to_owned();
    assert_eq!(
        preview_recipe(&provenance, &registry)
            .unwrap_err()
            .problem()
            .code,
        "missing_provenance"
    );
}

#[test]
fn duplicate_instances_and_generated_output_collisions_fail_closed() {
    let registry = catalog();
    let mut duplicate = recipe();
    duplicate.steps.push(duplicate.steps[0].clone());
    assert!(matches!(
        preview_recipe(&duplicate, &registry).unwrap_err(),
        SiteLabError::InvalidPlan(PlanError::DuplicateInstance(_))
    ));

    let mut collision = recipe();
    collision.sources.insert(
        ArtifactId::from("generated/extract/output"),
        source("semantic.entity_span_output"),
    );
    assert!(matches!(
        preview_recipe(&collision, &registry).unwrap_err(),
        SiteLabError::InvalidPlan(PlanError::AmbiguousArtifact(_))
    ));
}
