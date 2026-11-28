//! Regression boundary for the proposed parallel Temporal parent.
//!
//! This is NOT proof of cross-owner ValidateOnly admission. Until a real
//! owner-signed pre-effect protocol is implemented, V2 must categorically
//! refuse effectful or non-Pure contracts before scheduling any child.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    operation_dag_parent_workflow_id_v2, ArtifactSchema, ArtifactTypeId, EffectId, ExecutionPlan,
    IdempotencyPolicy, OperationContract, OperationDagRunV6, OperationId, OperationInstanceId,
    OperationNode, OperationOwnerId, OperationRef, OwnerCommandWireV1, PinnedOwnerRouteV1,
    PlanHash, RunScope, TrustedRouteCatalog, OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
    OWNER_OPERATION_CHILD_TYPE_V1, PLAN_SCHEMA_REVISION,
};
use seo_application::operation_adapter::{GenericDagError, OperationDagMachine};

fn prepared_single_owner(
    effects: BTreeSet<EffectId>,
    idempotency: IdempotencyPolicy,
) -> (OperationDagRunV6, TrustedRouteCatalog) {
    let scope = RunScope {
        tenant_id: "ci-validate-only-tenant".into(),
        run_id: "ci-validate-only-run".into(),
    };
    let contract = OperationContract {
        reference: OperationRef {
            id: OperationId::from("test.future-owner"),
            revision: 1,
        },
        owner: OperationOwnerId::from("test.future-owner"),
        inputs: BTreeMap::new(),
        outputs: BTreeMap::from([(
            "result".into(),
            ArtifactSchema {
                id: ArtifactTypeId::from("test.future-result"),
                revision: 1,
            },
        )]),
        invalidates: BTreeSet::new(),
        effects: effects.clone(),
        idempotency,
    };
    let fingerprint = contract.fingerprint().expect("typed contract fingerprint");
    let mut routes = TrustedRouteCatalog::default();
    routes
        .register(
            contract.clone(),
            PinnedOwnerRouteV1 {
                operation: contract.reference.clone(),
                owner: contract.owner.clone(),
                contract_hash: fingerprint.clone(),
                workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
                task_queue: "ci-isolated-owner".into(),
                build_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            },
        )
        .expect("trusted test route");
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope,
        allowed_effects: effects.clone(),
        external_artifacts: BTreeMap::new(),
        pinned_contracts: Vec::new(),
        nodes: vec![OperationNode {
            instance: "owner-step".into(),
            operation: contract.reference.clone(),
            inputs: BTreeMap::new(),
            outputs: BTreeMap::from([("result".into(), "result-artifact".into())]),
            after: BTreeSet::new(),
            permitted_effects: effects,
        }],
        hash: PlanHash::from("unsealed"),
    }
    .seal(routes.catalog())
    .expect("valid static plan without owner-side ValidateOnly evidence");
    let pinned_routes = routes
        .trusted_snapshot_for([&contract.reference])
        .expect("trusted owner route snapshot");
    (
        OperationDagRunV6 {
            schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
            plan,
            admitted: BTreeMap::new(),
            owner_commands: BTreeMap::from([(
                OperationInstanceId::from("owner-step"),
                OwnerCommandWireV1 {
                    operation: contract.reference,
                    owner: contract.owner,
                    contract_hash: fingerprint,
                    // Opaque nonempty bytes are not a proof of valid business DTO.
                    command_bytes: b"{not independently validated}".to_vec(),
                },
            )]),
            pinned_routes,
        },
        routes,
    )
}

#[test]
fn parallel_v2_rejects_effectful_owner_without_validate_only_admission() {
    let (run, routes) = prepared_single_owner(
        BTreeSet::from([EffectId::from("provider.write")]),
        IdempotencyPolicy::Idempotent,
    );
    let workflow_id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
    assert!(matches!(
        OperationDagMachine::new_v2(&workflow_id, run, routes),
        Err(GenericDagError::UnadmittedV2Effect(instance))
            if instance == OperationInstanceId::from("owner-step")
    ));
}

#[test]
fn parallel_v2_does_not_treat_empty_effects_as_pure_validation() {
    let (run, routes) = prepared_single_owner(BTreeSet::new(), IdempotencyPolicy::AtMostOnce);
    let workflow_id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
    assert!(matches!(
        OperationDagMachine::new_v2(&workflow_id, run, routes),
        Err(GenericDagError::UnadmittedV2Effect(_))
    ));
}

#[test]
fn parallel_v2_preserves_valid_pure_owner_path() {
    let (run, routes) = prepared_single_owner(BTreeSet::new(), IdempotencyPolicy::Pure);
    let workflow_id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
    assert!(OperationDagMachine::new_v2(&workflow_id, run, routes).is_ok());
}
