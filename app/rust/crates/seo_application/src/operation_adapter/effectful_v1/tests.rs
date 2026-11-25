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
        .enumerate()
        .map(|(ordinal, instance)| {
            (
                instance.clone(),
                OwnerCommandV1::PgTruthReviewReconcileV1(ReconcileTruthReviewsInput {
                    admission_id: Uuid::from_u128(ADMISSION_ID + ordinal as u128),
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
    // V1 manifest preflight remains history-compatible and accepts an
    // equivalent JSON representation, but V4 must match PostgreSQL's exact
    // canonical typed artifact bytes before any child Workflow can start.
    assert!(matches!(
        super::super::MixedDagMachineV4::new(&scope().run_id, changed),
        Err(EffectDagError::NonCanonicalOwnerInput(id))
            if id == ArtifactId::from("review_scope")
    ));
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
    // The new V4 parent has the stronger deterministic safety property:
    // dry-run all pure computations *before* a child Workflow is scheduled.
    assert!(matches!(
        super::super::MixedDagMachineV4::new(&scope().run_id, run),
        Err(EffectDagError::Run(OperationRunError::InvalidInput { .. }))
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

#[test]
fn v4_native_child_invocation_roundtrips_exact_plan_derived_dispatch() {
    let mut review = review_node("review_after_route");
    review.after.insert(OperationInstanceId::from("route"));
    let run = run(vec![review, route_node("route")]);
    let mut machine = super::super::MixedDagMachineV4::new(&scope().run_id, run.clone()).unwrap();
    machine.advance_pure().unwrap();
    let invocation = machine
        .next_owner_child_invocation(operation_protocol::TemporalExecutionIdentityV4 {
            workflow_id: scope().run_id,
            run_id: "actual-server-parent-temporal-run-id".into(),
        })
        .unwrap();
    invocation.verify_integrity().unwrap();
    assert_eq!(
        invocation.schema_revision,
        operation_protocol::EFFECT_CHILD_SCHEMA_REVISION
    );
    assert_eq!(invocation.checked_dispatch.plan_hash, run.plan.hash);
    assert_eq!(
        invocation.checked_dispatch.instance.as_str(),
        "review_after_route"
    );
    let typed: ReconcileTruthReviewsInput =
        serde_json::from_slice(&invocation.owner_command_bytes).unwrap();
    assert_eq!(typed.operation_instance, "review_after_route");
    assert_eq!(typed.admission_id, Uuid::from_u128(ADMISSION_ID));

    let mut malicious = invocation;
    malicious.bound_input.payload.push(b' ');
    assert!(malicious.verify_integrity().is_err());
}

#[test]
fn v4_interpreter_rejects_forged_pg_receipt_and_unfinished_graph() {
    let run = run(vec![review_node("review")]);
    let mut machine = super::super::MixedDagMachineV4::new(&scope().run_id, run).unwrap();
    let forged = truth_domain::review_effect_contract::ReconcileTruthReviewsReceipt {
        receipt_id: Uuid::from_u128(4),
        admission_id: Uuid::from_u128(ADMISSION_ID),
        execution_key: "attacker-chosen-key".into(),
        run_id: Uuid::from_u128(RUN_ID),
        context_key: "canonical-context".into(),
        candidate_set_hash: "claimed-source".into(),
        created_review_tasks: 1,
        open_review_tasks: 1,
    };
    assert!(matches!(
        machine.advance_owner_receipt(forged),
        Err(EffectDagError::InvalidOwnerReceipt(_))
    ));
    assert!(matches!(
        machine.finish(),
        Err(EffectDagError::IncompleteDag)
    ));
}

#[test]
fn v4_typed_child_result_boundary_rejects_wrong_version_and_dispatch_key() {
    let run = run(vec![review_node("review")]);
    let prepared = preflight_effect_dag_v1(&scope().run_id, &run).unwrap();
    let checked = match &prepared.steps[0] {
        PreparedDagStepV1::Owner { checked, .. } => checked,
        _ => panic!("expected PG owner step"),
    };
    let receipt = truth_domain::review_effect_contract::ReconcileTruthReviewsReceipt {
        receipt_id: Uuid::from_u128(9),
        admission_id: Uuid::from_u128(ADMISSION_ID),
        execution_key: checked.execution_key.clone(),
        run_id: Uuid::from_u128(RUN_ID),
        context_key: "canonical-context".into(),
        candidate_set_hash: "snapshot-hash".into(),
        created_review_tasks: 0,
        open_review_tasks: 2,
    };
    let mut child_result = operation_protocol::EffectChildResultV4 {
        schema_revision: operation_protocol::EFFECT_CHILD_SCHEMA_REVISION,
        dispatch_execution_key: checked.execution_key.clone(),
        pg_receipt: receipt.clone(),
    };
    let wire = serde_json::to_string(&child_result).unwrap();
    assert_eq!(
        decode_owner_child_result_v4(&wire, checked).unwrap(),
        receipt
    );
    child_result.schema_revision = 3;
    assert!(
        decode_owner_child_result_v4(&serde_json::to_string(&child_result).unwrap(), checked,)
            .is_err()
    );
    child_result.schema_revision = operation_protocol::EFFECT_CHILD_SCHEMA_REVISION;
    child_result.dispatch_execution_key = "forged-dispatch-key".into();
    assert!(
        decode_owner_child_result_v4(&serde_json::to_string(&child_result).unwrap(), checked,)
            .is_err()
    );
}

#[test]
fn v4_interpreter_completes_only_after_exact_typed_receipt_for_each_effect() {
    let first = review_node("review_a");
    let mut second = review_node("review_b");
    second.after.insert(first.instance.clone());
    let run = run(vec![second, first]);
    let mut machine = super::super::MixedDagMachineV4::new(&scope().run_id, run).unwrap();
    let mut keys = Vec::new();
    for (ordinal, instance) in ["review_a", "review_b"].iter().enumerate() {
        let invocation = machine
            .next_owner_child_invocation(operation_protocol::TemporalExecutionIdentityV4 {
                workflow_id: scope().run_id,
                run_id: "server-parent-run-id".into(),
            })
            .unwrap();
        invocation.verify_integrity().unwrap();
        assert_eq!(invocation.checked_dispatch.instance.as_str(), *instance);
        keys.push(invocation.checked_dispatch.execution_key.clone());
        let command: ReconcileTruthReviewsInput =
            serde_json::from_slice(&invocation.owner_command_bytes).unwrap();
        // Unit-only typed receipt fixture: a real Temporal parent obtains
        // this exclusively from the admitted PG child Workflow completion.
        let receipt = truth_domain::review_effect_contract::ReconcileTruthReviewsReceipt {
            receipt_id: Uuid::from_u128(ordinal as u128 + 10),
            admission_id: command.admission_id,
            execution_key: invocation.checked_dispatch.execution_key,
            run_id: command.run_id,
            context_key: command.context_key,
            candidate_set_hash: format!("synthetic-unit-snapshot-{ordinal}"),
            created_review_tasks: 1,
            open_review_tasks: 1,
        };
        machine.advance_owner_receipt(receipt).unwrap();
    }
    assert_ne!(keys[0], keys[1]);
    let finished = machine.finish().unwrap();
    assert_eq!(finished.owner_receipts.len(), 2);
    assert_eq!(finished.order.len(), 2);
    assert_eq!(finished.artifacts.len(), 3); // one admitted scope + two PG receipts
}

#[test]
fn v5_unordered_same_context_effects_fail_before_any_child_invocation() {
    let manifest = run(vec![review_node("review_a"), review_node("review_b")]);
    let denied = super::super::MixedDagMachineV5::new(&scope().run_id, manifest);
    // The canonical plan validator already detects unordered invalidations
    // of `truth.review_queue_snapshot`, before the V5 owner-scope fence.
    // Keep this stronger first-line fail-closed rejection as the expected
    // outcome; the V5 fence additionally guards future non-invalidation
    // owners that share a canonical PostgreSQL write scope.
    assert!(matches!(
        denied,
        Err(EffectDagError::Plan(
            PlanError::UnorderedInvalidations { .. }
        )) | Err(EffectDagError::ConflictingOwnerWriteScope { .. })
    ));
}

#[test]
fn v5_duplicate_admission_id_and_different_context_fail_whole_dag_preflight() {
    let first = review_node("review_a");
    let mut second = review_node("review_b");
    second.after.insert(first.instance.clone());
    let original = run(vec![second, first]);

    let mut repeated = original.clone();
    let OwnerCommandV1::PgTruthReviewReconcileV1(first_command) =
        &repeated.owner_commands[&OperationInstanceId::from("review_a")];
    let repeated_admission = first_command.admission_id;
    let OwnerCommandV1::PgTruthReviewReconcileV1(second_command) = repeated
        .owner_commands
        .get_mut(&OperationInstanceId::from("review_b"))
        .unwrap();
    second_command.admission_id = repeated_admission;
    assert!(matches!(
        super::super::MixedDagMachineV5::new(&scope().run_id, repeated),
        Err(EffectDagError::DuplicateOwnerAdmission(_))
    ));

    let mut mismatched = original;
    let distinct_context = "different-canonical-context";
    let distinct_artifact_id: ArtifactId = "distinct_review_scope".into();
    let source = mismatched.admitted[&ArtifactId::from("review_scope")].clone();
    let mut distinct_source = source;
    distinct_source.provenance = "declared-only:distinct-review-source".into();
    distinct_source.payload = serde_json::to_vec(&TruthReviewScopeArtifactV1 {
        run_id: Uuid::from_u128(RUN_ID),
        context_key: distinct_context.into(),
    })
    .unwrap();
    mismatched.plan.external_artifacts.insert(
        distinct_artifact_id.clone(),
        ExternalArtifact {
            schema: distinct_source.schema.clone(),
            scope: distinct_source.scope.clone(),
            provenance: distinct_source.provenance.clone(),
        },
    );
    mismatched
        .admitted
        .insert(distinct_artifact_id.clone(), distinct_source);
    let second_node = mismatched
        .plan
        .nodes
        .iter_mut()
        .find(|node| node.instance == OperationInstanceId::from("review_b"))
        .unwrap();
    second_node.inputs.insert(
        PortId::from(INPUT_PORT),
        ArtifactBinding::External {
            artifact: distinct_artifact_id,
        },
    );
    let mut catalog = semantic_extraction_registry().unwrap();
    catalog
        .catalog
        .register::<ReconcileTruthReviewsContractV1>()
        .unwrap();
    mismatched.plan = mismatched.plan.clone().seal(&catalog.catalog).unwrap();
    let new_hash = mismatched.plan.hash.as_str().to_owned();
    for command in mismatched.owner_commands.values_mut() {
        let OwnerCommandV1::PgTruthReviewReconcileV1(value) = command;
        value.plan_hash = new_hash.clone();
    }
    let OwnerCommandV1::PgTruthReviewReconcileV1(second_command) = mismatched
        .owner_commands
        .get_mut(&OperationInstanceId::from("review_b"))
        .unwrap();
    second_command.context_key = distinct_context.into();
    // Both owner commands now have individually valid typed source envelopes.
    // Nonetheless a canonical execution_run owns only one context_key, so
    // V5 must reject BEFORE the first effect instead of failing on child 2.
    assert!(matches!(
        super::super::MixedDagMachineV5::new(&scope().run_id, mismatched),
        Err(EffectDagError::MixedOwnerContext(_))
    ));
}

#[test]
fn v5_ordered_pg_children_use_distinct_dispatches_and_strict_fanin() {
    use super::super::{MixedDagMachineV5, NativeEffectRouteV5};

    let first = review_node("review_a");
    let mut second = review_node("review_b");
    second.after.insert(first.instance.clone());
    let mut machine = MixedDagMachineV5::new(&scope().run_id, run(vec![second, first])).unwrap();
    let parent = operation_protocol::TemporalExecutionIdentityV4 {
        workflow_id: scope().run_id,
        run_id: "true-temporal-parent-run-id".into(),
    };
    let mut keys = Vec::new();
    for expected in ["review_a", "review_b"] {
        let ready = machine.next_independent_children(parent.clone()).unwrap();
        // These effects write one canonical Truth context and therefore MUST
        // not fan out despite being separate admitted operation instances.
        assert_eq!(ready.len(), 1);
        let child = &ready[0];
        assert_eq!(child.route, NativeEffectRouteV5::PgTruthReviewV4);
        assert_eq!(child.checked.instance.as_str(), expected);
        child.invocation.verify_integrity().unwrap();
        keys.push(child.checked.execution_key.clone());
        let command: ReconcileTruthReviewsInput =
            serde_json::from_slice(&child.invocation.owner_command_bytes).unwrap();
        machine
            .advance_owner_receipt(
                truth_domain::review_effect_contract::ReconcileTruthReviewsReceipt {
                    receipt_id: Uuid::new_v4(),
                    admission_id: command.admission_id,
                    execution_key: child.checked.execution_key.clone(),
                    run_id: command.run_id,
                    context_key: command.context_key,
                    candidate_set_hash: "typed-unit-fixture-not-a-real-pg-hash".into(),
                    created_review_tasks: 0,
                    open_review_tasks: 0,
                },
            )
            .unwrap();
    }
    assert_ne!(keys[0], keys[1]);
    let result = machine.finish().unwrap();
    assert_eq!(
        result.schema_revision,
        super::super::ADMITTED_DAG_RECEIPT_SCHEMA_V5
    );
    assert_eq!(result.result.owner_receipts.len(), 2);
    assert_eq!(result.result.artifacts.len(), 3);
}
