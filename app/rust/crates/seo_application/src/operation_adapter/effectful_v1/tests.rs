use super::*;

use operation_protocol::{
    ArtifactSchema, EffectId, ExternalArtifact, OperationNode, PlanHash, RunScope,
    PLAN_SCHEMA_REVISION,
};
use seo_domain::extraction::operations::{EditorialTopics, LayerRoute};
use seo_domain::extraction::{editorial_topics, layer_routing};
use truth_domain::review_effect_contract::{ReconcileTruthReviewsContractV1, OUTPUT_PORT};
use uuid::Uuid;

const RUN_ID: u128 = 0x1234567890abcdef1234567890abcdef;
const ADMISSION_ID: u128 = 0x77777777777777777777777777777777;

fn scope() -> RunScope {
    RunScope {
        tenant_id: "claimed-tenant-not-authority".to_owned(),
        run_id: Uuid::from_u128(RUN_ID).to_string(),
    }
}

fn external(
    id: &str,
    schema: ArtifactSchema,
    payload: Vec<u8>,
) -> (ArtifactId, ExternalArtifact, ArtifactValue) {
    let provenance = format!("declared-only:not-trusted/{id}");
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
            payload,
        },
    )
}

fn route_seed() -> (ArtifactId, ExternalArtifact, ArtifactValue) {
    let input = layer_routing::LayerRouterInput {
        section_id: "verified-visa".to_owned(),
        heading_text: "Documents".to_owned(),
        raw_text: "Passport and insurance required. Fee 80 EUR.".to_owned(),
        source_tier: "official".to_owned(),
        block_type: "instruction".to_owned(),
    };
    external(
        "route_input",
        LayerRoute::contract().inputs[&PortId::from("input")].clone(),
        serde_json::to_vec(&input).unwrap(),
    )
}

fn editorial_seed() -> (ArtifactId, ExternalArtifact, ArtifactValue) {
    let input = editorial_topics::EditorialExtractionInput {
        section_id: "verified-visa".to_owned(),
        raw_text: "Passport and insurance required. Fee 80 EUR.".to_owned(),
    };
    external(
        "editorial_input",
        EditorialTopics::contract().inputs[&PortId::from("input")].clone(),
        serde_json::to_vec(&input).unwrap(),
    )
}

fn review_seed() -> (ArtifactId, ExternalArtifact, ArtifactValue) {
    external(
        "review_scope",
        ReconcileTruthReviewsContractV1::contract().inputs[&PortId::from(INPUT_PORT)].clone(),
        serde_json::to_vec(&TruthReviewScopeArtifactV1 {
            run_id: Uuid::from_u128(RUN_ID),
            context_key: "canonical-context".to_owned(),
        })
        .unwrap(),
    )
}

fn route_node(instance: &str) -> OperationNode {
    OperationNode {
        instance: instance.into(),
        operation: LayerRoute::contract().reference,
        inputs: BTreeMap::from([(
            "input".into(),
            ArtifactBinding::External {
                artifact: "route_input".into(),
            },
        )]),
        outputs: BTreeMap::from([(
            "output".into(),
            format!("{instance}_output").as_str().into(),
        )]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    }
}

fn editorial_node(instance: &str) -> OperationNode {
    OperationNode {
        instance: instance.into(),
        operation: EditorialTopics::contract().reference,
        inputs: BTreeMap::from([(
            "input".into(),
            ArtifactBinding::External {
                artifact: "editorial_input".into(),
            },
        )]),
        outputs: BTreeMap::from([(
            "output".into(),
            format!("{instance}_output").as_str().into(),
        )]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    }
}

fn review_node(instance: &str) -> OperationNode {
    OperationNode {
        instance: instance.into(),
        operation: ReconcileTruthReviewsContractV1::contract().reference,
        inputs: BTreeMap::from([(
            INPUT_PORT.into(),
            ArtifactBinding::External {
                artifact: "review_scope".into(),
            },
        )]),
        outputs: BTreeMap::from([(
            OUTPUT_PORT.into(),
            format!("{instance}_receipt").as_str().into(),
        )]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::from([EffectId::from(
            truth_domain::review_effect_contract::EFFECT_ID,
        )]),
    }
}

fn run(nodes: Vec<OperationNode>) -> EffectDagRunV1 {
    let registry = {
        let mut registry = semantic_extraction_registry().unwrap();
        registry
            .catalog
            .register::<ReconcileTruthReviewsContractV1>()
            .unwrap();
        registry
    };
    let effect_instances: Vec<_> = nodes
        .iter()
        .filter(|node| !node.permitted_effects.is_empty())
        .map(|node| node.instance.clone())
        .collect();
    let all_seeds = [route_seed(), editorial_seed(), review_seed()];
    let used_input_ids: BTreeSet<_> = nodes
        .iter()
        .flat_map(|node| node.inputs.values())
        .map(|input| match input {
            ArtifactBinding::External { artifact } | ArtifactBinding::Produced { artifact, .. } => {
                artifact.clone()
            }
        })
        .collect();
    let mut external_artifacts = BTreeMap::new();
    let mut admitted = BTreeMap::new();
    for (id, declared, payload) in all_seeds {
        if used_input_ids.contains(&id) {
            external_artifacts.insert(id.clone(), declared);
            admitted.insert(id, payload);
        }
    }
    let allowed_effects = if effect_instances.is_empty() {
        BTreeSet::new()
    } else {
        BTreeSet::from([EffectId::from(
            truth_domain::review_effect_contract::EFFECT_ID,
        )])
    };
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: scope(),
        allowed_effects,
        external_artifacts,
        pinned_contracts: Vec::new(),
        nodes,
        hash: PlanHash(String::new()),
    }
    .seal(registry.catalog())
    .unwrap();
    let owner_commands = effect_instances
        .into_iter()
        .map(|instance| {
            (
                instance.clone(),
                OwnerCommandV1::PgTruthReviewReconcileV1(ReconcileTruthReviewsInput {
                    admission_id: Uuid::from_u128(ADMISSION_ID),
                    run_id: Uuid::from_u128(RUN_ID),
                    scope: scope(),
                    context_key: "canonical-context".to_owned(),
                    operation_instance: instance.as_str().to_owned(),
                    plan_hash: plan.hash.as_str().to_owned(),
                }),
            )
        })
        .collect();
    EffectDagRunV1 {
        plan,
        admitted,
        owner_commands,
    }
}

fn no_durable_effects(run: &EffectDagRunV1) {
    let preflight = preflight_effect_dag_v1(&scope().run_id, run).unwrap();
    assert!(!preflight.effect_instances().is_empty());
    assert!(matches!(
        execute_checked_dag_v1(&scope().run_id, run),
        Err(EffectDagError::NoDurableOwnerBridge(_))
    ));
}

#[test]
fn three_real_distinct_pure_effect_dag_topologies_preflight_but_never_start_effects() {
    // (1) Extract a route, then reconcile the external verified review scope.
    let route = route_node("route_first");
    let mut review = review_node("review_after_route");
    review.after.insert(route.instance.clone());
    let first = run(vec![review, route]); // intentionally reverse the input order
    let first_checked = preflight_effect_dag_v1(&scope().run_id, &first).unwrap();
    assert!(matches!(
        &first_checked.steps[0],
        PreparedDagStepV1::Pure { .. }
    ));
    assert!(matches!(
        &first_checked.steps[1],
        PreparedDagStepV1::Owner { .. }
    ));
    no_durable_effects(&first);

    // (2) Effect before an independently sourced editorial extraction.
    let review = review_node("review_first");
    let mut editorial = editorial_node("editorial_after_review");
    editorial.after.insert(review.instance.clone());
    let second = run(vec![editorial, review]);
    let second_checked = preflight_effect_dag_v1(&scope().run_id, &second).unwrap();
    assert!(matches!(
        &second_checked.steps[0],
        PreparedDagStepV1::Owner { .. }
    ));
    assert!(matches!(
        &second_checked.steps[1],
        PreparedDagStepV1::Pure { .. }
    ));
    no_durable_effects(&second);

    // (3) Two independent real extraction branches converge on a review
    // effect with ordering-only edges, not a fictitious fixed site pipeline.
    let route = route_node("route_branch");
    let editorial = editorial_node("editorial_branch");
    let mut review = review_node("review_after_branches");
    review.after.insert(route.instance.clone());
    review.after.insert(editorial.instance.clone());
    let third = run(vec![review, editorial, route]);
    let third_checked = preflight_effect_dag_v1(&scope().run_id, &third).unwrap();
    assert_eq!(third_checked.steps.len(), 3);
    assert!(matches!(
        &third_checked.steps[2],
        PreparedDagStepV1::Owner { .. }
    ));
    no_durable_effects(&third);
}

#[test]
fn changed_actual_external_bytes_change_checked_dispatch_key_not_operator_authority() {
    let original = run(vec![review_node("review")]);
    let prepared = preflight_effect_dag_v1(&scope().run_id, &original).unwrap();
    let old_key = match &prepared.steps[0] {
        PreparedDagStepV1::Owner { checked, .. } => checked.execution_key.clone(),
        _ => panic!("expected owner"),
    };
    let mut changed = original.clone();
    let value = changed
        .admitted
        .get_mut(&ArtifactId::from("review_scope"))
        .unwrap();
    value.payload =
        format!("  {} ", String::from_utf8(value.payload.clone()).unwrap()).into_bytes();
    let prepared = preflight_effect_dag_v1(&scope().run_id, &changed).unwrap();
    let new_key = match &prepared.steps[0] {
        PreparedDagStepV1::Owner { checked, .. } => checked.execution_key.clone(),
        _ => panic!("expected owner"),
    };
    assert_ne!(old_key, new_key);
    no_durable_effects(&changed);
}

#[test]
fn malformed_owner_command_and_cross_scope_are_rejected_before_any_activity() {
    let valid = run(vec![review_node("review")]);
    let mut cross_run = valid.clone();
    let OwnerCommandV1::PgTruthReviewReconcileV1(input) = cross_run
        .owner_commands
        .get_mut(&OperationInstanceId::from("review"))
        .unwrap();
    input.run_id = Uuid::from_u128(RUN_ID + 1);
    assert!(matches!(
        preflight_effect_dag_v1(&scope().run_id, &cross_run),
        Err(EffectDagError::InvalidOwnerCommand {
            reason: "cross-run or cross-tenant command scope",
            ..
        })
    ));

    let mut stale_plan = valid.clone();
    let OwnerCommandV1::PgTruthReviewReconcileV1(input) = stale_plan
        .owner_commands
        .get_mut(&OperationInstanceId::from("review"))
        .unwrap();
    input.plan_hash = "forged-contract-fingerprint".into();
    assert!(matches!(
        preflight_effect_dag_v1(&scope().run_id, &stale_plan),
        Err(EffectDagError::InvalidOwnerCommand {
            reason: "instance or plan fingerprint mismatch",
            ..
        })
    ));

    let mut mutated_source = valid.clone();
    mutated_source
        .admitted
        .get_mut(&ArtifactId::from("review_scope"))
        .unwrap()
        .payload = serde_json::to_vec(&TruthReviewScopeArtifactV1 {
        run_id: Uuid::from_u128(RUN_ID),
        context_key: "different-source".into(),
    })
    .unwrap();
    assert!(matches!(
        preflight_effect_dag_v1(&scope().run_id, &mutated_source),
        Err(EffectDagError::InvalidOwnerCommand {
            reason: "owner input artifact and command disagree",
            ..
        })
    ));

    let mut extra_command = valid.clone();
    let unplanned_command =
        extra_command.owner_commands[&OperationInstanceId::from("review")].clone();
    extra_command
        .owner_commands
        .insert("unplanned".into(), unplanned_command);
    assert!(matches!(
        preflight_effect_dag_v1(&scope().run_id, &extra_command),
        Err(EffectDagError::UnexpectedOwnerCommand(_))
    ));

    let mut missing_command = valid;
    missing_command.owner_commands.clear();
    assert!(matches!(
        preflight_effect_dag_v1(&scope().run_id, &missing_command),
        Err(EffectDagError::MissingOwnerCommand(_))
    ));
}

#[test]
fn malformed_pure_payload_cannot_run_before_effect_admission_failure() {
    // The pure route stage appears before the owner effect in the graph. Its
    // input is malformed, but no domain operation is invoked in this path:
    // the *entire* prepared mixed DAG is rejected before execution begins.
    let mut review = review_node("review_after_pure");
    review.after.insert(OperationInstanceId::from("route"));
    let mut run = run(vec![route_node("route"), review]);
    run.admitted
        .get_mut(&ArtifactId::from("route_input"))
        .unwrap()
        .payload = b"not a valid layer routing input".to_vec();
    assert!(preflight_effect_dag_v1(&scope().run_id, &run).is_ok());
    assert!(matches!(
        execute_checked_dag_v1(&scope().run_id, &run),
        Err(EffectDagError::NoDurableOwnerBridge(_))
    ));
}

#[test]
fn malformed_owner_scope_payload_fails_closed_in_preflight() {
    let mut run = run(vec![review_node("review")]);
    run.admitted
        .get_mut(&ArtifactId::from("review_scope"))
        .unwrap()
        .payload = b"{\"run_id\":\"fake\",\"context_key\":\"canonical-context\"}".to_vec();
    assert!(matches!(
        preflight_effect_dag_v1(&scope().run_id, &run),
        Err(EffectDagError::InvalidReviewScope { .. })
    ));
}

#[test]
fn malicious_plan_changes_rejected_without_repining_and_no_bypass_via_pure_dag() {
    let valid = run(vec![route_node("first"), review_node("effect")]);
    let mut unapproved = valid.clone();
    unapproved.plan.nodes[1].permitted_effects.clear();
    assert!(preflight_effect_dag_v1(&scope().run_id, &unapproved).is_err());

    let mut changed_contract = valid.clone();
    changed_contract.plan.pinned_contracts.clear();
    assert!(preflight_effect_dag_v1(&scope().run_id, &changed_contract).is_err());

    let mut foreign_artifact = valid.clone();
    foreign_artifact
        .admitted
        .get_mut(&ArtifactId::from("review_scope"))
        .unwrap()
        .scope
        .tenant_id = "another-tenant".into();
    assert!(matches!(
        preflight_effect_dag_v1(&scope().run_id, &foreign_artifact),
        Err(EffectDagError::Run(
            OperationRunError::InvalidArtifactEnvelope(_)
        ))
    ));

    assert!(execute_pure_dag(
        &scope().run_id,
        &PureDagRun {
            plan: valid.plan,
            admitted: valid.admitted,
        }
    )
    .is_err());
}

#[test]
fn multiple_effect_instances_have_distinct_dispatch_keys_but_no_admitted_child_provenance() {
    let first = review_node("review_a");
    let mut second = review_node("review_b");
    second.after.insert(first.instance.clone());
    let run = run(vec![second, first]);
    let prepared = preflight_effect_dag_v1(&scope().run_id, &run).unwrap();
    let keys: Vec<_> = prepared
        .steps
        .iter()
        .filter_map(|step| match step {
            PreparedDagStepV1::Owner { checked, .. } => Some(checked.execution_key.clone()),
            PreparedDagStepV1::Pure { .. } => None,
        })
        .collect();
    assert_eq!(keys.len(), 2);
    assert_ne!(keys[0], keys[1]);
    no_durable_effects(&run);
}

#[test]
fn pure_only_plan_is_executable_through_checked_workflow_without_owner_bridge() {
    let run = run(vec![route_node("route")]);
    assert!(matches!(
        execute_checked_dag_v1(&scope().run_id, &run),
        Ok(CheckedDagOutcomeV1::Pure(_))
    ));
}
