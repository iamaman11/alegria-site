use super::*;

#[derive(Debug, Serialize, Deserialize)]
struct Input {
    evidence: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Output {
    result: String,
}

struct Source;
struct Score;
struct Publish;
struct ChangedScoreSchema;
struct ChangedScoreOwner;

fn schema(name: &str) -> ArtifactSchema {
    ArtifactSchema {
        id: name.into(),
        revision: 1,
    }
}

fn reference(name: &str) -> OperationRef {
    OperationRef {
        id: name.into(),
        revision: 1,
    }
}

impl Operation for Source {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        OperationContract {
            reference: reference("source.collect"),
            owner: "source_owner".into(),
            inputs: BTreeMap::new(),
            outputs: BTreeMap::from([(PortId::from("evidence"), schema("evidence"))]),
            effects: BTreeSet::new(),
            idempotency: IdempotencyPolicy::Pure,
        }
    }
}

impl Operation for Score {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        OperationContract {
            reference: reference("content.score"),
            owner: "scoring_owner".into(),
            inputs: BTreeMap::from([(PortId::from("evidence"), schema("evidence"))]),
            outputs: BTreeMap::from([(PortId::from("score"), schema("score"))]),
            effects: BTreeSet::new(),
            idempotency: IdempotencyPolicy::Pure,
        }
    }
}

impl Operation for Publish {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        OperationContract {
            reference: reference("release.commit"),
            owner: "publication_owner".into(),
            inputs: BTreeMap::from([(PortId::from("score"), schema("score"))]),
            outputs: BTreeMap::from([(PortId::from("release"), schema("release"))]),
            effects: BTreeSet::from([EffectId::from("release.write")]),
            idempotency: IdempotencyPolicy::Idempotent,
        }
    }
}

impl Operation for ChangedScoreSchema {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        let mut contract = Score::contract();
        contract.outputs.insert(
            "score".into(),
            ArtifactSchema {
                id: "score".into(),
                revision: 2,
            },
        );
        contract
    }
}

impl Operation for ChangedScoreOwner {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        let mut contract = Score::contract();
        contract.owner = "unexpected_owner".into();
        contract
    }
}

fn catalog() -> ContractCatalog {
    let mut catalog = ContractCatalog::default();
    catalog.register::<Source>().unwrap();
    catalog.register::<Score>().unwrap();
    catalog.register::<Publish>().unwrap();
    catalog
}

fn scope() -> RunScope {
    RunScope {
        tenant_id: "tenant-a".to_owned(),
        run_id: "run-1".to_owned(),
    }
}

fn external() -> BTreeMap<ArtifactId, ExternalArtifact> {
    BTreeMap::from([(
        ArtifactId::from("external-evidence"),
        ExternalArtifact {
            schema: schema("evidence"),
            scope: scope(),
            provenance: "verified-raw-source/hash:v1".to_owned(),
        },
    )])
}

fn score_node(id: &str, output: &str) -> OperationNode {
    OperationNode {
        instance: id.into(),
        operation: reference("content.score"),
        inputs: BTreeMap::from([(
            PortId::from("evidence"),
            ArtifactBinding::External {
                artifact: "external-evidence".into(),
            },
        )]),
        outputs: BTreeMap::from([(PortId::from("score"), output.into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    }
}

fn plan(nodes: Vec<OperationNode>) -> ExecutionPlan {
    ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: scope(),
        allowed_effects: BTreeSet::from([EffectId::from("release.write")]),
        external_artifacts: external(),
        pinned_contracts: Vec::new(),
        nodes,
        hash: PlanHash(String::new()),
    }
    .seal(&catalog())
    .unwrap()
}

#[test]
fn two_identical_operations_with_identical_input_are_distinct_instances() {
    let scenario = plan(vec![
        score_node("score-1", "score-1-output"),
        score_node("score-2", "score-2-output"),
    ]);
    let checked = scenario.validate(&catalog()).unwrap();
    assert_eq!(
        checked.execution_order,
        [
            OperationInstanceId::from("score-1"),
            OperationInstanceId::from("score-2")
        ]
    );
    let once = checked
        .execution_key(&"score-1".into(), "same-input-bytes")
        .unwrap();
    let retry = checked
        .execution_key(&"score-1".into(), "same-input-bytes")
        .unwrap();
    let other = checked
        .execution_key(&"score-2".into(), "same-input-bytes")
        .unwrap();
    assert_eq!(once, retry, "retry must use the original idempotency key");
    assert_ne!(
        once, other,
        "intentional second node must not be deduplicated"
    );
    assert!(matches!(
        checked.execution_key(&"unknown".into(), "same-input-bytes"),
        Err(PlanError::UnknownInstance(_))
    ));
}

#[test]
fn canonical_plan_hash_and_schedule_ignore_unrelated_node_order() {
    let first = score_node("z", "out-z");
    let second = score_node("a", "out-a");
    let left = plan(vec![first.clone(), second.clone()]);
    let right = plan(vec![second, first]);
    assert_eq!(left.hash, right.hash);
    assert_eq!(
        left.validate(&catalog()).unwrap(),
        right.validate(&catalog()).unwrap()
    );
    assert_eq!(
        left.validate(&catalog()).unwrap().execution_order,
        [
            OperationInstanceId::from("a"),
            OperationInstanceId::from("z")
        ]
    );
}

#[test]
fn produced_artifact_binding_forms_a_real_multi_operation_dag() {
    let source = OperationNode {
        instance: "source".into(),
        operation: reference("source.collect"),
        inputs: BTreeMap::new(),
        outputs: BTreeMap::from([(
            PortId::from("evidence"),
            ArtifactId::from("generated-evidence"),
        )]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    };
    let mut score = score_node("score", "generated-score");
    score.inputs.insert(
        "evidence".into(),
        ArtifactBinding::Produced {
            producer: "source".into(),
            port: "evidence".into(),
            artifact: "generated-evidence".into(),
        },
    );
    let publish = OperationNode {
        instance: "publish".into(),
        operation: reference("release.commit"),
        inputs: BTreeMap::from([(
            "score".into(),
            ArtifactBinding::Produced {
                producer: "score".into(),
                port: "score".into(),
                artifact: "generated-score".into(),
            },
        )]),
        outputs: BTreeMap::from([("release".into(), "generated-release".into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::from([EffectId::from("release.write")]),
    };
    let checked = plan(vec![publish, score, source])
        .validate(&catalog())
        .unwrap();
    assert_eq!(
        checked.execution_order,
        [
            OperationInstanceId::from("source"),
            OperationInstanceId::from("score"),
            OperationInstanceId::from("publish")
        ]
    );
}

#[test]
fn version_and_unknown_operation_fail_closed() {
    let mut scenario = plan(vec![score_node("score", "out")]);
    scenario.schema_revision += 1;
    scenario = scenario.seal(&catalog()).unwrap();
    assert_eq!(
        scenario.validate(&catalog()),
        Err(PlanError::UnsupportedPlanRevision(2))
    );

    let mut scenario = plan(vec![score_node("score", "out")]);
    scenario.nodes[0].operation.revision = 2;
    scenario.hash = scenario.canonical_hash().unwrap();
    assert!(matches!(
        scenario.validate(&catalog()),
        Err(PlanError::UnknownOperation(_))
    ));

    let mut scenario = plan(vec![score_node("score", "out")]);
    scenario.nodes[0].operation.id = "not.registered".into();
    scenario.hash = scenario.canonical_hash().unwrap();
    assert!(matches!(
        scenario.validate(&catalog()),
        Err(PlanError::UnknownOperation(_))
    ));
}

#[test]
fn missing_or_wrong_schema_is_rejected_before_runner_can_schedule_io() {
    let mut scenario = plan(vec![score_node("score", "out")]);
    scenario.external_artifacts.clear();
    scenario = scenario.seal(&catalog()).unwrap();
    let side_effects = std::sync::atomic::AtomicUsize::new(0);
    assert!(matches!(
        scenario.validate(&catalog()),
        Err(PlanError::MissingExternalArtifact(_))
    ));
    assert_eq!(side_effects.load(std::sync::atomic::Ordering::Relaxed), 0);

    let mut scenario = plan(vec![score_node("score", "out")]);
    scenario
        .external_artifacts
        .get_mut(&"external-evidence".into())
        .unwrap()
        .schema
        .revision += 1;
    scenario = scenario.seal(&catalog()).unwrap();
    assert!(matches!(
        scenario.validate(&catalog()),
        Err(PlanError::ArtifactSchemaMismatch { .. })
    ));

    let mut scenario = plan(vec![score_node("score", "out")]);
    scenario.nodes[0].inputs.clear();
    scenario = scenario.seal(&catalog()).unwrap();
    assert!(matches!(
        scenario.validate(&catalog()),
        Err(PlanError::MissingPort { .. })
    ));
}

#[test]
fn scope_and_provenance_are_not_interchangeable() {
    let mut scenario = plan(vec![score_node("score", "out")]);
    scenario
        .external_artifacts
        .get_mut(&"external-evidence".into())
        .unwrap()
        .scope
        .tenant_id = "tenant-b".to_owned();
    scenario = scenario.seal(&catalog()).unwrap();
    assert!(matches!(
        scenario.validate(&catalog()),
        Err(PlanError::CrossScopeArtifact(_))
    ));

    let mut scenario = plan(vec![score_node("score", "out")]);
    scenario
        .external_artifacts
        .get_mut(&"external-evidence".into())
        .unwrap()
        .provenance
        .clear();
    scenario = scenario.seal(&catalog()).unwrap();
    assert!(matches!(
        scenario.validate(&catalog()),
        Err(PlanError::MissingProvenance(_))
    ));
}

#[test]
fn effect_policies_and_plan_hash_reject_unapproved_execution() {
    let mut scenario = plan(vec![score_node("score", "out")]);
    scenario.nodes[0]
        .permitted_effects
        .insert("release.write".into());
    scenario = scenario.seal(&catalog()).unwrap();
    assert!(matches!(
        scenario.validate(&catalog()),
        Err(PlanError::ForbiddenEffect { .. })
    ));

    let mut scenario = plan(vec![score_node("score", "out")]);
    scenario.hash = "forged-hash".into();
    assert_eq!(
        scenario.validate(&catalog()),
        Err(PlanError::PlanHashMismatch)
    );
}

#[test]
fn duplicate_instance_ambiguous_artifact_and_cycle_fail_closed() {
    let dup = plan(vec![
        score_node("score", "out-a"),
        score_node("score", "out-b"),
    ]);
    assert!(matches!(
        dup.validate(&catalog()),
        Err(PlanError::DuplicateInstance(_))
    ));

    let conflict = plan(vec![
        score_node("a", "same-out"),
        score_node("b", "same-out"),
    ]);
    assert!(matches!(
        conflict.validate(&catalog()),
        Err(PlanError::AmbiguousArtifact(_))
    ));

    let mut a = score_node("a", "out-a");
    let mut b = score_node("b", "out-b");
    a.after.insert("b".into());
    b.after.insert("a".into());
    let scenario = plan(vec![a, b]);
    assert_eq!(scenario.validate(&catalog()), Err(PlanError::Cycle));
}

#[test]
fn binding_must_name_the_real_producer_port_and_artifact() {
    let mut score = score_node("score", "out");
    score.inputs.insert(
        "evidence".into(),
        ArtifactBinding::Produced {
            producer: "missing".into(),
            port: "evidence".into(),
            artifact: "bad".into(),
        },
    );
    assert!(matches!(
        plan(vec![score]).validate(&catalog()),
        Err(PlanError::UnknownInstance(_))
    ));

    let source = OperationNode {
        instance: "source".into(),
        operation: reference("source.collect"),
        inputs: BTreeMap::new(),
        outputs: BTreeMap::from([("evidence".into(), "real-id".into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    };
    let mut score = score_node("score", "out");
    score.inputs.insert(
        "evidence".into(),
        ArtifactBinding::Produced {
            producer: "source".into(),
            port: "evidence".into(),
            artifact: "different-id".into(),
        },
    );
    assert!(matches!(
        plan(vec![source, score]).validate(&catalog()),
        Err(PlanError::MissingProducedArtifact { .. })
    ));
}

#[test]
fn unknown_manifest_fields_and_duplicate_contract_are_rejected() {
    let serialized = serde_json::to_string(&plan(vec![score_node("score", "out")])).unwrap();
    let invalid = serialized.replacen(
        "\"schema_revision\":1",
        "\"schema_revision\":1,\"unrecognized\":true",
        1,
    );
    assert!(serde_json::from_str::<ExecutionPlan>(&invalid).is_err());

    let mut catalog = catalog();
    assert!(matches!(
        catalog.register::<Score>(),
        Err(PlanError::DuplicateContract(_))
    ));
}

#[test]
fn old_plan_rejects_same_revision_contract_schema_or_owner_drift() {
    let pinned = plan(vec![score_node("score", "out")]);
    let reference = reference("content.score");
    assert!(pinned
        .pinned_contracts
        .iter()
        .any(|contract| contract.reference == reference));

    let mut schema_changed = ContractCatalog::default();
    schema_changed.register::<ChangedScoreSchema>().unwrap();
    assert_eq!(
        pinned.validate(&schema_changed),
        Err(PlanError::ContractFingerprintMismatch(reference.clone()))
    );

    let mut owner_changed = ContractCatalog::default();
    owner_changed.register::<ChangedScoreOwner>().unwrap();
    assert_eq!(
        pinned.validate(&owner_changed),
        Err(PlanError::ContractFingerprintMismatch(reference.clone()))
    );

    let mut unpinned = pinned.clone();
    unpinned.pinned_contracts.clear();
    unpinned.hash = unpinned.canonical_hash().unwrap();
    assert_eq!(
        unpinned.validate(&catalog()),
        Err(PlanError::ContractNotPinned(reference))
    );
}

#[test]
fn pinned_contracts_use_json_safe_roundtrip_and_canonical_sorting() {
    let source = OperationNode {
        instance: "source".into(),
        operation: reference("source.collect"),
        inputs: BTreeMap::new(),
        outputs: BTreeMap::from([("evidence".into(), "source-output".into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    };
    let mut scenario = plan(vec![
        score_node("score-2", "out-2"),
        score_node("score-1", "out-1"),
        source,
    ]);
    assert_eq!(scenario.pinned_contracts.len(), 2);
    let serialized = serde_json::to_string(&scenario).unwrap();
    let restored: ExecutionPlan = serde_json::from_str(&serialized).unwrap();
    assert_eq!(restored, scenario);
    assert_eq!(
        restored.validate(&catalog()).unwrap().hash(),
        &scenario.hash
    );

    scenario.pinned_contracts.reverse();
    assert_eq!(scenario.canonical_hash().unwrap(), restored.hash);
    assert!(scenario.validate(&catalog()).is_ok());

    let duplicate = scenario.pinned_contracts[0].clone();
    scenario.pinned_contracts.push(duplicate);
    scenario.hash = scenario.canonical_hash().unwrap();
    assert!(matches!(
        scenario.validate(&catalog()),
        Err(PlanError::DuplicatePinnedContract(_))
    ));

    let mut extra = restored.clone();
    extra.pinned_contracts.push(PinnedContract {
        reference: reference("unused.contract"),
        fingerprint: "unused-hash".into(),
    });
    extra.hash = extra.canonical_hash().unwrap();
    assert!(matches!(
        extra.validate(&catalog()),
        Err(PlanError::UnexpectedPinnedContract(_))
    ));
}
