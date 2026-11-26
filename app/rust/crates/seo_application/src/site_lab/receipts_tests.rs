use super::*;
use operation_protocol::{
    canonical_owner_result_hash, owner_child_workflow_id_v2, owner_content_hash,
    AdmittedArtifactV1, ArtifactTypeId, OperationContract, OperationId, OperationOwnerId,
    OwnerResultV1, ProducedArtifactEvidenceV1,
};

fn ref_id(id: &str) -> OperationRef {
    OperationRef {
        id: OperationId::from(id),
        revision: 1,
    }
}

fn schema(id: &str) -> ArtifactSchema {
    ArtifactSchema {
        id: ArtifactTypeId::from(id),
        revision: 1,
    }
}

fn catalog() -> ContractCatalog {
    let mut catalog = ContractCatalog::default();
    for (id, input, output) in [
        ("lab.extract", "lab.raw_reviews", "lab.demand_signals"),
        ("lab.analyze", "lab.demand_signals", "lab.research_findings"),
    ] {
        catalog
            .register_contract(OperationContract {
                reference: ref_id(id),
                owner: OperationOwnerId::from("test_owner"),
                inputs: BTreeMap::from([(PortId::from("input"), schema(input))]),
                outputs: BTreeMap::from([(PortId::from("output"), schema(output))]),
                invalidates: BTreeSet::new(),
                effects: BTreeSet::new(),
                idempotency: IdempotencyPolicy::Pure,
            })
            .unwrap();
    }
    catalog
}

fn plan(run_id: &str) -> ExecutionPlan {
    let recipe = SiteRecipe {
        schema_revision: 1,
        scope: RunScope {
            tenant_id: "tenant-test".to_owned(),
            run_id: run_id.to_owned(),
        },
        sources: BTreeMap::from([(
            ArtifactId::from("reviews"),
            RecipeSource {
                schema: schema("lab.raw_reviews"),
                provenance: "frozen-fixture:reviews-v1".to_owned(),
            },
        )]),
        steps: vec![
            RecipeStep {
                id: OperationInstanceId::from("extract"),
                operation: ref_id("lab.extract"),
                inputs: BTreeMap::from([(
                    PortId::from("input"),
                    RecipeInput::External {
                        artifact: ArtifactId::from("reviews"),
                    },
                )]),
                after: BTreeSet::new(),
            },
            RecipeStep {
                id: OperationInstanceId::from("analyze"),
                operation: ref_id("lab.analyze"),
                inputs: BTreeMap::from([(
                    PortId::from("input"),
                    RecipeInput::Produced {
                        step: OperationInstanceId::from("extract"),
                        port: PortId::from("output"),
                    },
                )]),
                after: BTreeSet::new(),
            },
        ],
    };
    compile_recipe(&recipe, &catalog()).unwrap()
}

fn fixture_receipt(
    plan: &ExecutionPlan,
    marker: &str,
    external_bytes: &[u8],
) -> OperationDagReceiptV1 {
    let registry = catalog();
    let order = plan.validate(&registry).unwrap().execution_order().to_vec();
    let mut artifacts = BTreeMap::new();
    for (id, source) in &plan.external_artifacts {
        artifacts.insert(
            id.clone(),
            AdmittedArtifactV1 {
                schema: source.schema.clone(),
                scope: source.scope.clone(),
                provenance: source.provenance.clone(),
                payload: external_bytes.to_vec(),
            },
        );
    }
    let mut owner_results = BTreeMap::new();
    let mut produced_evidence = BTreeMap::new();
    for instance in &order {
        let node = plan.nodes.iter().find(|node| &node.instance == instance).unwrap();
        let contract = registry.contract(&node.operation).unwrap();
        let key = owner_content_hash(format!("{marker}-{instance:?}").as_bytes());
        let mut outputs = BTreeMap::new();
        for (port, artifact_id) in &node.outputs {
            let output = AdmittedArtifactV1 {
                schema: contract.outputs[port].clone(),
                scope: plan.scope.clone(),
                provenance: format!("fixture-result:{marker}"),
                payload: format!("{marker}:{}", instance.as_str()).into_bytes(),
            };
            artifacts.insert(artifact_id.clone(), output.clone());
            outputs.insert(port.clone(), output);
        }
        let result = OwnerResultV1 {
            dispatch_execution_key: key.clone(),
            outputs,
            receipt_bytes: format!("receipt-{marker}-{}", instance.as_str()).into_bytes(),
        };
        for (port, artifact_id) in &node.outputs {
            produced_evidence.insert(
                artifact_id.clone(),
                ProducedArtifactEvidenceV1 {
                    producer: instance.clone(),
                    port: port.clone(),
                    artifact_id: artifact_id.clone(),
                    dispatch_execution_key: key.clone(),
                    child_workflow_id: owner_child_workflow_id_v2(&key),
                    result_hash: canonical_owner_result_hash(&result).unwrap(),
                    receipt_hash: owner_content_hash(&result.receipt_bytes),
                    payload_hash: owner_content_hash(&result.outputs[port].payload),
                },
            );
        }
        owner_results.insert(instance.clone(), result);
    }
    OperationDagReceiptV1 {
        plan_hash: plan.hash.clone(),
        order,
        artifacts,
        produced_evidence,
        owner_results,
    }
}

#[test]
fn full_receipt_is_consistent_but_never_claims_server_attestation() {
    let plan = plan("run-1");
    let receipt = fixture_receipt(&plan, "v1", b"same-frozen-source");
    let report = inspect_execution(&plan, &receipt, &catalog()).unwrap();
    assert_eq!(report.status, "self_consistent_not_attested");
    assert_eq!(report.steps.len(), 2);
    assert_eq!(report.artifacts.len(), 3);
    assert_eq!(report.temporal_history_attestation, "not_checked");
    assert_eq!(report.owner_receipt_attestation, "not_checked");
    assert_eq!(report.source_authentication, "not_checked");
    assert_eq!(report.artifacts[0].origin, "produced");
}

#[test]
fn mismatched_result_artifact_evidence_and_history_identity_fail_closed() {
    let plan = plan("run-1");
    let base = fixture_receipt(&plan, "v1", b"source");
    let id = ArtifactId::from("generated/extract/output");
    let mut altered = base.clone();
    altered.artifacts.get_mut(&id).unwrap().payload.push(0);
    assert!(inspect_execution(&plan, &altered, &catalog()).is_err());

    let mut altered = base.clone();
    altered.produced_evidence.get_mut(&id).unwrap().result_hash = "0".repeat(64);
    assert!(inspect_execution(&plan, &altered, &catalog()).is_err());

    let mut altered = base.clone();
    altered.produced_evidence.get_mut(&id).unwrap().child_workflow_id =
        "alegria-owner-operation-v1-wrong-parent".to_owned();
    assert!(inspect_execution(&plan, &altered, &catalog()).is_err());

    let mut altered = base.clone();
    altered.order.reverse();
    assert!(inspect_execution(&plan, &altered, &catalog()).is_err());

    let mut altered = base.clone();
    altered.owner_results.remove(&OperationInstanceId::from("analyze"));
    assert!(inspect_execution(&plan, &altered, &catalog()).is_err());

    let mut altered = base;
    altered.produced_evidence.remove(&id);
    assert!(inspect_execution(&plan, &altered, &catalog()).is_err());
}

#[test]
fn results_compare_only_across_two_distinct_runs_with_identical_source_bytes() {
    let first_plan = plan("hypothesis-a");
    let second_plan = plan("hypothesis-b");
    let first = fixture_receipt(&first_plan, "a", b"frozen-reviews");
    let second = fixture_receipt(&second_plan, "b", b"frozen-reviews");
    let report = compare_executions(
        &first_plan,
        &first,
        &second_plan,
        &second,
        &catalog(),
    )
    .unwrap();
    assert_eq!(report.common_output_count, 2);
    assert_eq!(report.changes.len(), 2);
    assert_eq!(report.seo_measurement, "not_performed");

    let different_snapshot = fixture_receipt(&second_plan, "b", b"different-snapshot");
    assert!(matches!(
        compare_executions(
            &first_plan,
            &first,
            &second_plan,
            &different_snapshot,
            &catalog()
        ),
        Err(InspectionError::DifferentSources)
    ));
    assert!(matches!(
        compare_executions(&first_plan, &first, &first_plan, &first, &catalog()),
        Err(InspectionError::SameRun)
    ));
}

#[test]
fn forged_contract_and_unknown_provider_cannot_be_silently_resealed() {
    let plan = plan("run-1");
    let receipt = fixture_receipt(&plan, "v1", b"source");
    assert!(matches!(
        inspect_execution(&plan, &receipt, &ContractCatalog::default()),
        Err(InspectionError::Plan(_))
    ));
    let mut tampered = plan;
    tampered.hash = PlanHash("0".repeat(64));
    assert!(matches!(
        inspect_execution(&tampered, &receipt, &catalog()),
        Err(InspectionError::Plan(_))
    ));
}
