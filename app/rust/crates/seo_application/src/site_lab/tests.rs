use super::*;
use crate::operation_adapter::semantic_extraction_registry;
use operation_protocol::{
    ArtifactTypeId, OperationContract, OperationId, OperationOwnerId, PortId,
};
use serde_json::json;

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
        parameters: RecipeParameters::default(),
        after: BTreeSet::new(),
    }
}

fn recipe() -> SiteRecipe {
    SiteRecipe {
        schema_revision: 2,
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

#[test]
fn atomic_recipe_changes_are_typed_and_preserve_existing_steps() {
    let original = recipe();
    let revised = revise_recipe(&original, &[
        RecipeChange::SetAfter {
            step: OperationInstanceId::from("editorial"),
            after: BTreeSet::from([OperationInstanceId::from("canonicalize")]),
        },
        RecipeChange::SetInput {
            step: OperationInstanceId::from("canonicalize"),
            port: PortId::from("input"),
            input: RecipeInput::Produced {
                step: OperationInstanceId::from("extract"),
                port: PortId::from("output"),
            },
        },
    ]).unwrap();
    assert_ne!(revised, original);
    assert_eq!(original.steps[2].after.len(), 0);
    assert_eq!(preview_recipe(&revised, &catalog()).unwrap().execution, "not_started");

    let rejected = revise_recipe(&original, &[
        RecipeChange::RemoveStep { step: OperationInstanceId::from("extract") },
        RecipeChange::SetParameters {
            step: OperationInstanceId::from("missing"),
            parameters: RecipeParameters::default(),
        },
    ]);
    assert!(matches!(rejected, Err(RecipeEditError::UnknownStep(_))));
    assert_eq!(original.steps.len(), 3);
}

#[test]
fn removing_a_producer_does_not_rewire_or_fake_a_valid_graph() {
    let draft = revise_recipe(&recipe(), &[
        RecipeChange::RemoveStep { step: OperationInstanceId::from("extract") },
    ]).unwrap();
    assert!(matches!(
        preview_recipe(&draft, &catalog()),
        Err(SiteLabError::InvalidPlan(_))
    ));
}

fn catalog() -> SiteLabCatalog {
    installed_semantic_lab_catalog(
        semantic_extraction_registry()
            .expect("real semantic operations are registered")
            .catalog()
            .clone(),
    )
    .unwrap()
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
    let contract = registry
        .catalog()
        .contract(&op("future.search_demand"))
        .unwrap();
    let fingerprint = contract.fingerprint().unwrap();
    registry
        .register_remote(
            &op("future.search_demand"),
            &fingerprint,
            json!({"type":"object", "properties":{"region":{"type":"string"}}}),
        )
        .unwrap();
    let catalog_after = describe_tools(&registry).unwrap();
    assert_eq!(catalog_after.len(), 6);
    assert!(catalog_after.iter().any(|tool| {
        tool.operation == op("future.search_demand") && tool.readiness == "requires_owner_admission"
    }));

    let external_recipe = SiteRecipe {
        schema_revision: 2,
        scope: recipe().scope,
        sources: BTreeMap::from([(ArtifactId::from("query"), source("research.query_input"))]),
        steps: vec![step("demand", "future.search_demand", external("query"))],
    };
    let preview = preview_recipe(&external_recipe, &registry).unwrap();
    assert_eq!(preview.status, "pending_owner_parameter_validation");
    assert_eq!(preview.execution, "not_started");
    assert_eq!(preview.source_authentication, "not_verified");
    assert_eq!(
        preview.steps_requiring_owner_parameter_validation,
        vec![OperationInstanceId::from("demand")]
    );
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
    assert_eq!(first_preview.recipe_hash, second_preview.recipe_hash);
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
    assert!(!diff.same_recipe);
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
    invalid_revision.schema_revision = 3;
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HybridSearchParameters {
    mode: HybridSearchMode,
    top_k: std::num::NonZeroU16,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HybridSearchMode {
    Dense,
    Hybrid,
    Sparse,
}

#[derive(Debug, Serialize, Deserialize)]
struct LenientOwnerParameters {
    top_k: u16,
}

#[test]
fn owner_dto_cannot_silently_discard_unknown_author_settings() {
    let mut installed = catalog();
    let operation = op("future.lenient_owner");
    installed
        .register_contract(OperationContract {
            reference: operation.clone(),
            owner: OperationOwnerId::from("future_owner"),
            inputs: BTreeMap::from([(
                PortId::from("input"),
                source("semantic.entity_span_input").schema,
            )]),
            outputs: BTreeMap::from([(
                PortId::from("output"),
                source("semantic.entity_span_output").schema,
            )]),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::new(),
            idempotency: IdempotencyPolicy::Pure,
        })
        .unwrap();
    let fingerprint = installed
        .catalog()
        .contract(&operation)
        .unwrap()
        .fingerprint()
        .unwrap();
    installed
        .register_local::<LenientOwnerParameters>(
            &operation,
            &fingerprint,
            json!({"type": "object", "properties": {"top_k": {"type": "integer"}}}),
        )
        .unwrap();
    let mut authoring = recipe();
    authoring.steps = vec![step(
        "lenient",
        "future.lenient_owner",
        external("mentions"),
    )];
    authoring.steps[0].parameters = json!({"top_k": 10, "future_advanced_mode": "needed"}).into();
    let problem = preview_recipe(&authoring, &installed)
        .unwrap_err()
        .problem();
    assert_eq!(problem.code, "operation_parameters_invalid");
    assert_eq!(problem.step, Some(OperationInstanceId::from("lenient")));
    assert!(!problem.message.contains("future_advanced_mode"));
}

#[test]
fn legacy_recipes_remain_valid_without_parameters_but_require_v2_for_parameters() {
    let registry = catalog();
    let current = recipe();
    let current_hash = preview_recipe(&current, &registry).unwrap().recipe_hash;
    let mut legacy = current;
    legacy.schema_revision = 1;
    let legacy_hash = preview_recipe(&legacy, &registry).unwrap().recipe_hash;
    assert_eq!(current_hash, legacy_hash);

    legacy.steps[0].parameters = json!({"unexpected": "value"}).into();
    assert_eq!(
        preview_recipe(&legacy, &registry)
            .unwrap_err()
            .problem()
            .code,
        "recipe_revision_unsupported"
    );
}

#[test]
fn installed_rust_parameters_are_typed_and_change_recipe_identity() {
    let mut registry = catalog();
    let operation = op("qdrant.hybrid_search");
    registry
        .register_contract(OperationContract {
            reference: operation.clone(),
            owner: OperationOwnerId::from("qdrant"),
            inputs: BTreeMap::from([(
                PortId::from("input"),
                source("semantic.entity_span_input").schema,
            )]),
            outputs: BTreeMap::from([(
                PortId::from("output"),
                source("semantic.entity_span_output").schema,
            )]),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::from([EffectId::from("qdrant.search")]),
            idempotency: IdempotencyPolicy::Idempotent,
        })
        .unwrap();
    let fingerprint = registry
        .catalog()
        .contract(&operation)
        .unwrap()
        .fingerprint()
        .unwrap();
    registry
        .register_local::<HybridSearchParameters>(
            &operation,
            &fingerprint,
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["mode", "top_k"],
                "properties": {
                    "mode": {"type": "string", "enum": ["dense", "hybrid", "sparse"]},
                    "top_k": {"type": "integer", "minimum": 1, "maximum": 65535}
                }
            }),
        )
        .unwrap();
    let mut first = recipe();
    first.steps = vec![step("search", "qdrant.hybrid_search", external("mentions"))];
    first.steps[0].parameters = json!({"mode": "hybrid", "top_k": 20}).into();
    let first_preview = preview_recipe(&first, &registry).unwrap();
    assert_eq!(
        first_preview
            .steps_requiring_owner_parameter_validation
            .len(),
        0
    );
    assert_eq!(first_preview.steps_requiring_owner_admission.len(), 1);

    let mut second = first.clone();
    second.steps[0].parameters = json!({"mode": "hybrid", "top_k": 40}).into();
    let diff = diff_recipes(&first, &second, &registry).unwrap();
    assert!(diff.same_plan);
    assert!(!diff.same_recipe);
    assert_ne!(diff.first.parameter_hashes, diff.second.parameter_hashes);

    second.steps[0].parameters = json!({"mode": "hybrid", "top_k": "40"}).into();
    let type_problem = preview_recipe(&second, &registry).unwrap_err().problem();
    assert_eq!(type_problem.code, "operation_parameters_invalid");
    assert_eq!(type_problem.step, Some(OperationInstanceId::from("search")));
    assert!(!type_problem.message.contains("40"));
    second.steps[0].parameters = json!({"mode": "hybrid", "top_k": 0}).into();
    assert_eq!(
        preview_recipe(&second, &registry)
            .unwrap_err()
            .problem()
            .code,
        "operation_parameters_invalid"
    );
    second.steps[0].parameters = json!({"mode": "unknown", "top_k": 40}).into();
    assert_eq!(
        preview_recipe(&second, &registry)
            .unwrap_err()
            .problem()
            .code,
        "operation_parameters_invalid"
    );
    second.steps[0].parameters =
        json!({"mode": "hybrid", "top_k": 40, "api_key": "should-never-be-in-recipe"}).into();
    assert_eq!(
        preview_recipe(&second, &registry)
            .unwrap_err()
            .problem()
            .code,
        "operation_parameters_invalid"
    );
}

#[test]
fn pure_operations_reject_extra_settings_and_untrusted_schema_replacement() {
    let registry = catalog();
    let mut recipe = recipe();
    recipe.steps[0].parameters = json!({"top_k": 10}).into();
    let extra_problem = preview_recipe(&recipe, &registry).unwrap_err().problem();
    assert_eq!(extra_problem.code, "operation_parameters_invalid");
    assert_eq!(
        extra_problem.step,
        Some(OperationInstanceId::from("extract"))
    );
    let mut registry = catalog();
    let operation = op("semantic.entity_spans");
    let fingerprint = registry
        .catalog()
        .contract(&operation)
        .unwrap()
        .fingerprint()
        .unwrap();
    assert!(matches!(
        registry.register_remote(&operation, &fingerprint, json!({"type":"object"})),
        Err(ParameterError::Duplicate)
    ));
    assert!(matches!(
        registry.register_remote(
            &op("semantic.editorial_topics"),
            &operation_protocol::ContractHash("0".repeat(64)),
            json!({"type":"object"})
        ),
        Err(ParameterError::MismatchedContract)
    ));
}

#[test]
fn missing_schema_fails_closed_and_schema_changes_invalidate_recipe_identity() {
    let operation = op("future.provider_read");
    let contract = OperationContract {
        reference: operation.clone(),
        owner: OperationOwnerId::from("future_provider_owner"),
        inputs: BTreeMap::from([(
            PortId::from("input"),
            source("semantic.entity_span_input").schema,
        )]),
        outputs: BTreeMap::from([(
            PortId::from("output"),
            source("semantic.entity_span_output").schema,
        )]),
        invalidates: BTreeSet::new(),
        effects: BTreeSet::from([EffectId::from("provider.read")]),
        idempotency: IdempotencyPolicy::Idempotent,
    };
    let mut first_catalog = catalog();
    first_catalog.register_contract(contract.clone()).unwrap();
    let mut first = recipe();
    first.steps = vec![step("remote", "future.provider_read", external("mentions"))];
    let missing_problem = preview_recipe(&first, &first_catalog)
        .unwrap_err()
        .problem();
    assert_eq!(missing_problem.code, "operation_parameter_schema_missing");
    assert_eq!(
        missing_problem.step,
        Some(OperationInstanceId::from("remote"))
    );
    let tool = describe_tools(&first_catalog).unwrap();
    assert_eq!(
        tool.iter()
            .find(|tool| tool.operation == operation)
            .unwrap()
            .readiness,
        "missing_parameter_schema"
    );

    let fingerprint = contract.fingerprint().unwrap();
    first_catalog
        .register_remote(
            &operation,
            &fingerprint,
            json!({"type": "object", "properties": {"mode": {"type": "string"}}}),
        )
        .unwrap();
    let first_preview = preview_recipe(&first, &first_catalog).unwrap();

    let mut next_catalog = catalog();
    next_catalog.register_contract(contract).unwrap();
    next_catalog
        .register_remote(
            &operation,
            &fingerprint,
            json!({"type": "object", "properties": {"mode": {"type": "integer"}}}),
        )
        .unwrap();
    let next_preview = preview_recipe(&first, &next_catalog).unwrap();
    assert_eq!(first_preview.plan_hash, next_preview.plan_hash);
    assert_ne!(first_preview.recipe_hash, next_preview.recipe_hash);
    assert_ne!(
        first_preview.parameter_hashes,
        next_preview.parameter_hashes
    );
}
