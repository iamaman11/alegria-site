//! Contract-only mixed-owner preflight. This does NOT claim a live PG->Qdrant
//! producer/consumer E2E: their real artifact schemas are incompatible today.
//! The existing native Qdrant Temporal E2E continues to cover real provider I/O.

fn mixed_v6_fixture() -> (
    operation_protocol::OperationDagRunV6,
    operation_protocol::TrustedRouteCatalog,
) {
    use operation_protocol::{
        AdmittedArtifactV1, ArtifactBinding, ContractCatalog, ExecutionPlan, ExternalArtifact,
        Operation, OperationDagRunV6, OperationNode, OwnerCommandWireV1, PinnedOwnerRouteV1,
        PlanHash, RunScope, TrustedRouteCatalog, OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
        OWNER_OPERATION_CHILD_TYPE_V1, PLAN_SCHEMA_REVISION,
    };
    use qdrant_contracts::{QdrantKeywordProjectionContractV2, QDRANT_OWNER_COMMAND_QUEUE_V2};
    use std::collections::{BTreeMap, BTreeSet};
    use truth_domain::review_effect_contract::{
        ReconcileTruthReviewsContractV1, ReconcileTruthReviewsInput,
        TruthReviewScopeArtifactV1,
    };

    let run_id = Uuid::new_v4();
    let scope = RunScope {
        tenant_id: "ci-mixed-typed-tenant".into(),
        run_id: run_id.to_string(),
    };
    let context = format!("ci-mixed-typed-{run_id}");
    let pg_contract = ReconcileTruthReviewsContractV1::contract();
    let qdrant_contract = QdrantKeywordProjectionContractV2::contract();

    let mut contracts = ContractCatalog::default();
    contracts.register::<ReconcileTruthReviewsContractV1>().unwrap();
    contracts.register::<QdrantKeywordProjectionContractV2>().unwrap();
    let external_artifacts = BTreeMap::from([
        (
            "pg_scope".into(),
            ExternalArtifact {
                schema: pg_contract.inputs[&operation_protocol::PortId::from("truth_review_scope")].clone(),
                scope: scope.clone(),
                provenance: "ci-pg-scope-v6".into(),
            }),
        (
            "qdrant_command".into(),
            ExternalArtifact {
                schema: qdrant_contract.inputs[&operation_protocol::PortId::from("keyword_command")].clone(),
                scope: scope.clone(),
                provenance: "ci-qdrant-command-v6".into(),
            }),
    ]);
    let nodes = vec![
        OperationNode {
            instance: "pg_review".into(),
            operation: pg_contract.reference.clone(),
            inputs: BTreeMap::from([(
                "truth_review_scope".into(),
                ArtifactBinding::External { artifact: "pg_scope".into() },
            )]),
            outputs: BTreeMap::from([("review_queue_receipt".into(), "pg_receipt".into())]),
            after: BTreeSet::new(),
            permitted_effects: pg_contract.effects.clone(),
        },
        OperationNode {
            instance: "qdrant_projection".into(),
            operation: qdrant_contract.reference.clone(),
            inputs: BTreeMap::from([(
                "keyword_command".into(),
                ArtifactBinding::External { artifact: "qdrant_command".into() },
            )]),
            outputs: BTreeMap::from([("projection_receipt".into(), "qdrant_receipt".into())]),
            after: BTreeSet::from(["pg_review".into()]),
            permitted_effects: qdrant_contract.effects.clone(),
        },
    ];
    let plan = ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: scope.clone(),
        allowed_effects: pg_contract.effects.union(&qdrant_contract.effects).cloned().collect(),
        external_artifacts,
        pinned_contracts: Vec::new(),
        nodes,
        hash: PlanHash(String::new()),
    }
    .seal(&contracts)
    .unwrap();

    let pg_command = ReconcileTruthReviewsInput {
        admission_id: Uuid::new_v4(),
        run_id,
        scope: scope.clone(),
        context_key: context.clone(),
        operation_instance: "pg_review".into(),
        plan_hash: plan.hash.as_str().into(),
    };
    let qdrant_command = qdrant_contracts::QdrantKeywordProjectionCommandV1 {
        schema_revision: 1,
        admission_id: Uuid::new_v4(),
        tenant_id: scope.tenant_id.clone(),
        context_key: context.clone(),
        keyword_key: format!("ci-mixed-keyword-{run_id}"),
        vector: vec![0.123; 1024],
    };
    let payloads = BTreeMap::from([
        ("pg_scope".into(), serde_json::to_vec(&TruthReviewScopeArtifactV1 {
            run_id, context_key: context,
        }).unwrap()),
        ("qdrant_command".into(), serde_json::to_vec(&qdrant_command).unwrap()),
    ]);
    let admitted = plan.external_artifacts.iter().map(|(id, meta)| (
        id.clone(),
        AdmittedArtifactV1 {
            schema: meta.schema.clone(),
            scope: meta.scope.clone(),
            provenance: meta.provenance.clone(),
            payload: payloads[id].clone(),
        },
    )).collect();

    // Routes originate in trusted runner configuration, never in the plan.
    let mut routes = TrustedRouteCatalog::default();
    for (contract, task_queue, build_id) in [
        (pg_contract.clone(), "alegria-pg-command-v5", "ci-pg-owner-v5"),
        (qdrant_contract.clone(), QDRANT_OWNER_COMMAND_QUEUE_V2, "ci-qdrant-owner-v2"),
    ] {
        let route = PinnedOwnerRouteV1 {
            operation: contract.reference.clone(),
            owner: contract.owner.clone(),
            contract_hash: contract.fingerprint().unwrap(),
            workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
            task_queue: task_queue.into(),
            build_id: build_id.into(),
        };
        routes.register(contract, route).unwrap();
    }
    let owner_commands = BTreeMap::from([
        ("pg_review".into(), OwnerCommandWireV1 {
            operation: pg_contract.reference.clone(),
            owner: pg_contract.owner.clone(),
            contract_hash: pg_contract.fingerprint().unwrap(),
            command_bytes: serde_json::to_vec(&pg_command).unwrap(),
        }),
        ("qdrant_projection".into(), OwnerCommandWireV1 {
            operation: qdrant_contract.reference.clone(),
            owner: qdrant_contract.owner.clone(),
            contract_hash: qdrant_contract.fingerprint().unwrap(),
            command_bytes: serde_json::to_vec(&qdrant_command).unwrap(),
        }),
    ]);
    let pinned_routes = routes
        .trusted_snapshot_for([&pg_contract.reference, &qdrant_contract.reference])
        .unwrap();
    (
        OperationDagRunV6 {
            schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
            plan,
            admitted,
            owner_commands,
            pinned_routes,
        },
        routes,
    )
}

#[test]
fn mixed_v6_typed_owner_preflight_rejects_cross_tenant_and_fake_produced_dataflow() {
    use operation_protocol::{ArtifactBinding, PlanError, RouteCatalogError};
    use seo_application::operation_adapter::{GenericDagError, OperationDagMachine};

    let (run, routes) = mixed_v6_fixture();
    let workflow_id = run.plan.scope.run_id.clone();
    // Legitimate but LIMITED "after" plan: neither owner consumes the other's
    // output, and the trust-anchored routes are not caller-configurable.
    run.plan.validate(routes.catalog()).expect("genuine owner contracts");
    routes.validate_run_routes(&run).expect("exact owner/queue route snapshot");
    let machine = OperationDagMachine::new(&workflow_id, run.clone(), routes.clone())
        .expect("generic Temporal owner plan accepts two independently routed operations");
    assert_eq!(machine.next_instance().unwrap().as_str(), "pg_review");

    // Even the later node's cross-tenant inputs fail before any effects.
    let mut cross_tenant = run.plan.clone();
    cross_tenant.external_artifacts.get_mut(&"qdrant_command".into())
        .unwrap().scope.tenant_id = "another-tenant".into();
    let cross_tenant = cross_tenant.seal(routes.catalog()).unwrap();
    assert!(matches!(
        cross_tenant.validate(routes.catalog()),
        Err(PlanError::CrossScopeArtifact(artifact)) if artifact.as_str() == "qdrant_command"
    ));
    let mut wrong_scope = run.clone();
    wrong_scope.plan = cross_tenant;
    assert!(matches!(
        OperationDagMachine::new(&workflow_id, wrong_scope, routes.clone()),
        Err(GenericDagError::Plan(PlanError::CrossScopeArtifact(artifact)))
            if artifact.as_str() == "qdrant_command"
    ), "generic runner must reject a later node's foreign artifact before first child");

    // The mutable payload envelope is checked, too: changing only admitted
    // data may not bypass the separately pinned run scope.
    let mut foreign_admission = run.clone();
    foreign_admission.admitted.get_mut(&"qdrant_command".into()).unwrap()
        .scope.tenant_id = "another-tenant".into();
    assert!(matches!(
        OperationDagMachine::new(&workflow_id, foreign_admission, routes.clone()),
        Err(GenericDagError::InvalidArtifact(artifact))
            if artifact.as_str() == "qdrant_command"
    ));

    // No semantic conversion exists: PG Truth receipt is NOT a Qdrant command.
    let mut incompatible = run.plan.clone();
    incompatible.nodes.iter_mut()
        .find(|node| node.instance.as_str() == "qdrant_projection")
        .unwrap().inputs.insert(
            "keyword_command".into(),
            ArtifactBinding::Produced {
                producer: "pg_review".into(),
                port: "review_queue_receipt".into(),
                artifact: "pg_receipt".into(),
            },
        );
    let incompatible = incompatible.seal(routes.catalog()).unwrap();
    assert!(matches!(
        incompatible.validate(routes.catalog()),
        Err(PlanError::ArtifactSchemaMismatch { instance, port, artifact })
            if instance.as_str() == "qdrant_projection"
            && port.as_str() == "keyword_command"
            && artifact.as_str() == "pg_receipt"
    ), "never reinterpret PG receipt as Qdrant command to fake DAG dataflow");
    let mut false_dataflow = run.clone();
    false_dataflow.plan = incompatible;
    assert!(matches!(
        OperationDagMachine::new(&workflow_id, false_dataflow, routes.clone()),
        Err(GenericDagError::Plan(PlanError::ArtifactSchemaMismatch { instance, port, artifact }))
            if instance.as_str() == "qdrant_projection"
            && port.as_str() == "keyword_command"
            && artifact.as_str() == "pg_receipt"
    ));

    let mut forged_route = run.clone();
    forged_route.pinned_routes.last_mut().unwrap().task_queue = "attacker-queue".into();
    assert!(matches!(
        routes.validate_run_routes(&forged_route),
        Err(RouteCatalogError::UntrustedRouteSnapshot)
    ));
    assert!(matches!(
        OperationDagMachine::new(&workflow_id, forged_route, routes),
        Err(GenericDagError::Route(RouteCatalogError::UntrustedRouteSnapshot))
    ));
}
