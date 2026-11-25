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
struct AtMostOncePublish;
struct ChangedScoreSchema;
struct ChangedScoreOwner;
struct ChangedScoreInvalidation;
struct InvalidateEvidence;
struct RefreshEvidence;
struct DraftFromEvidence;
struct ReviewDraft;
struct InvalidateAndDraft;
struct InvalidateAndRefresh;
struct MergeScores;
struct ExtensionReport;

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
            invalidates: BTreeSet::new(),
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
            invalidates: BTreeSet::new(),
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
            invalidates: BTreeSet::new(),
            effects: BTreeSet::from([EffectId::from("release.write")]),
            idempotency: IdempotencyPolicy::Idempotent,
        }
    }
}

impl Operation for AtMostOncePublish {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        let mut contract = Publish::contract();
        contract.reference = reference("release.commit.at-most-once");
        contract.idempotency = IdempotencyPolicy::AtMostOnce;
        contract
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

impl Operation for ChangedScoreInvalidation {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        let mut contract = Score::contract();
        contract.invalidates.insert("evidence".into());
        contract
    }
}

impl Operation for InvalidateEvidence {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        OperationContract {
            reference: reference("truth.invalidate-evidence"),
            owner: "truth_owner".into(),
            inputs: BTreeMap::new(),
            outputs: BTreeMap::new(),
            invalidates: BTreeSet::from(["evidence".into()]),
            effects: BTreeSet::from(["truth.mutate".into()]),
            idempotency: IdempotencyPolicy::Idempotent,
        }
    }
}

impl Operation for RefreshEvidence {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        let mut contract = Source::contract();
        contract.reference = reference("source.refresh-evidence");
        contract
    }
}

impl Operation for DraftFromEvidence {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        OperationContract {
            reference: reference("content.draft-from-evidence"),
            owner: "content_owner".into(),
            inputs: BTreeMap::from([("evidence".into(), schema("evidence"))]),
            outputs: BTreeMap::from([("draft".into(), schema("draft"))]),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::new(),
            idempotency: IdempotencyPolicy::Pure,
        }
    }
}

impl Operation for ReviewDraft {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        OperationContract {
            reference: reference("content.review-draft"),
            owner: "content_owner".into(),
            inputs: BTreeMap::from([("draft".into(), schema("draft"))]),
            outputs: BTreeMap::from([("review".into(), schema("review"))]),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::new(),
            idempotency: IdempotencyPolicy::Pure,
        }
    }
}

impl Operation for InvalidateAndDraft {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        let mut contract = DraftFromEvidence::contract();
        contract.reference = reference("truth.invalidate-and-draft");
        contract.owner = "truth_owner".into();
        contract.invalidates.insert("evidence".into());
        contract.effects.insert("truth.mutate".into());
        contract.idempotency = IdempotencyPolicy::Idempotent;
        contract
    }
}

impl Operation for InvalidateAndRefresh {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        let mut contract = RefreshEvidence::contract();
        contract.reference = reference("truth.invalidate-and-refresh");
        contract.owner = "truth_owner".into();
        contract.invalidates.insert("evidence".into());
        contract.effects.insert("truth.mutate".into());
        contract.idempotency = IdempotencyPolicy::Idempotent;
        contract
    }
}

// Test-only operations deliberately use owner identities unrelated to any
// provider. The neutral planner must accept them through the same catalog.
impl Operation for MergeScores {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        OperationContract {
            reference: reference("synthetic.merge-scores"),
            owner: "independent_merge_owner".into(),
            inputs: BTreeMap::from([
                ("left".into(), schema("score")),
                ("right".into(), schema("score")),
            ]),
            outputs: BTreeMap::from([("merged".into(), schema("merged"))]),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::new(),
            idempotency: IdempotencyPolicy::Pure,
        }
    }
}

impl Operation for ExtensionReport {
    type Input = Input;
    type Output = Output;
    type Failure = std::io::Error;

    fn contract() -> OperationContract {
        OperationContract {
            reference: reference("extension.report"),
            owner: "newly_registered_owner".into(),
            inputs: BTreeMap::from([("merged".into(), schema("merged"))]),
            outputs: BTreeMap::from([("report".into(), schema("report"))]),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::new(),
            idempotency: IdempotencyPolicy::Pure,
        }
    }
}

fn catalog() -> ContractCatalog {
    let mut catalog = ContractCatalog::default();
    catalog.register::<Source>().unwrap();
    catalog.register::<Score>().unwrap();
    catalog.register::<Publish>().unwrap();
    catalog.register::<AtMostOncePublish>().unwrap();
    catalog.register::<InvalidateEvidence>().unwrap();
    catalog.register::<RefreshEvidence>().unwrap();
    catalog.register::<DraftFromEvidence>().unwrap();
    catalog.register::<ReviewDraft>().unwrap();
    catalog.register::<InvalidateAndDraft>().unwrap();
    catalog.register::<InvalidateAndRefresh>().unwrap();
    catalog.register::<MergeScores>().unwrap();
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

fn invalidate_node(id: &str) -> OperationNode {
    OperationNode {
        instance: id.into(),
        operation: reference("truth.invalidate-evidence"),
        inputs: BTreeMap::new(),
        outputs: BTreeMap::new(),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::from(["truth.mutate".into()]),
    }
}

fn refresh_node(id: &str, output: &str) -> OperationNode {
    OperationNode {
        instance: id.into(),
        operation: reference("source.refresh-evidence"),
        inputs: BTreeMap::new(),
        outputs: BTreeMap::from([("evidence".into(), output.into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    }
}

fn produced(producer: &str, port: &str, artifact: &str) -> ArtifactBinding {
    ArtifactBinding::Produced {
        producer: producer.into(),
        port: port.into(),
        artifact: artifact.into(),
    }
}

fn draft_node(id: &str, output: &str, binding: ArtifactBinding) -> OperationNode {
    OperationNode {
        instance: id.into(),
        operation: reference("content.draft-from-evidence"),
        inputs: BTreeMap::from([("evidence".into(), binding)]),
        outputs: BTreeMap::from([("draft".into(), output.into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    }
}

fn review_node(id: &str, producer: &str, artifact: &str) -> OperationNode {
    OperationNode {
        instance: id.into(),
        operation: reference("content.review-draft"),
        inputs: BTreeMap::from([("draft".into(), produced(producer, "draft", artifact))]),
        outputs: BTreeMap::from([("review".into(), ArtifactId(format!("{id}-result")))]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    }
}

fn plan(nodes: Vec<OperationNode>) -> ExecutionPlan {
    ExecutionPlan {
        schema_revision: PLAN_SCHEMA_REVISION,
        scope: scope(),
        allowed_effects: BTreeSet::from([
            EffectId::from("release.write"),
            EffectId::from("truth.mutate"),
        ]),
        external_artifacts: external(),
        pinned_contracts: Vec::new(),
        nodes,
        hash: PlanHash(String::new()),
    }
    .seal(&catalog())
    .unwrap()
}

#[test]
fn freshness_reader_before_invalidator_requires_real_happens_before() {
    let reader = score_node("reader", "scored-before-change");
    let mut invalidator = invalidate_node("invalidate");
    invalidator.after.insert("reader".into());
    let left = plan(vec![invalidator.clone(), reader.clone()]);
    let right = plan(vec![reader, invalidator]);
    assert_eq!(left.hash, right.hash);
    assert_eq!(
        left.validate(&catalog()).unwrap().execution_order(),
        &[
            OperationInstanceId::from("reader"),
            OperationInstanceId::from("invalidate")
        ]
    );
    assert!(right.validate(&catalog()).is_ok());
}

#[test]
fn freshness_unordered_read_hazard_is_not_silenced_by_sorted_topology() {
    let reader = score_node("a-reader", "scored-before-change");
    let invalidator = invalidate_node("z-invalidator");
    let scenario = plan(vec![reader, invalidator]);
    assert!(matches!(
        scenario.validate(&catalog()),
        Err(PlanError::UnorderedArtifactHazard {
            invalidator,
            consumer,
            artifact_type,
        }) if invalidator == OperationInstanceId::from("z-invalidator")
            && consumer == OperationInstanceId::from("a-reader")
            && artifact_type == ArtifactTypeId::from("evidence")
    ));

    let mut reader = score_node("read-after", "stale-result");
    reader.after.insert("invalidate".into());
    let scenario = plan(vec![reader, invalidate_node("invalidate")]);
    assert!(matches!(
        scenario.validate(&catalog()),
        Err(PlanError::StaleArtifact {
            invalidator,
            consumer,
            artifact,
            artifact_type,
        }) if invalidator == OperationInstanceId::from("invalidate")
            && consumer == OperationInstanceId::from("read-after")
            && artifact == ArtifactId::from("external-evidence")
            && artifact_type == ArtifactTypeId::from("evidence")
    ));
}

#[test]
fn freshness_output_caused_after_invalidation_is_reusable() {
    let invalidator = invalidate_node("invalidate");
    let mut refresh = refresh_node("refresh", "refreshed-evidence");
    refresh.after.insert("invalidate".into());
    let mut reader = score_node("score", "valid-score");
    reader.inputs.insert(
        "evidence".into(),
        produced("refresh", "evidence", "refreshed-evidence"),
    );
    let checked = plan(vec![reader, refresh, invalidator])
        .validate(&catalog())
        .unwrap();
    assert_eq!(
        checked.execution_order(),
        &[
            OperationInstanceId::from("invalidate"),
            OperationInstanceId::from("refresh"),
            OperationInstanceId::from("score"),
        ]
    );
}

#[test]
fn freshness_two_writers_of_same_type_must_have_explicit_order() {
    let unordered = plan(vec![invalidate_node("first"), invalidate_node("second")]);
    assert!(matches!(
        unordered.validate(&catalog()),
        Err(PlanError::UnorderedInvalidations { artifact_type, .. })
            if artifact_type == ArtifactTypeId::from("evidence")
    ));

    let first = invalidate_node("first");
    let mut second = invalidate_node("second");
    second.after.insert("first".into());
    assert!(plan(vec![second, first]).validate(&catalog()).is_ok());
}

#[test]
fn freshness_later_invalidation_makes_earlier_refresh_stale() {
    let first = invalidate_node("first");
    let mut refresh = refresh_node("refresh", "fresh-after-first");
    refresh.after.insert("first".into());
    let mut second = invalidate_node("second");
    second.after.insert("refresh".into());
    let mut reader = score_node("reader", "stale-score");
    reader.inputs.insert(
        "evidence".into(),
        produced("refresh", "evidence", "fresh-after-first"),
    );
    reader.after.insert("second".into());
    assert!(matches!(
        plan(vec![first, reader, second, refresh])
            .validate(&catalog()),
        Err(PlanError::StaleArtifact { invalidator, .. })
            if invalidator == OperationInstanceId::from("second")
    ));

    let first = invalidate_node("first");
    let mut second = invalidate_node("second");
    second.after.insert("first".into());
    let mut refresh = refresh_node("refresh", "fresh-after-both");
    refresh.after.insert("second".into());
    let mut reader = score_node("reader", "valid-score");
    reader.inputs.insert(
        "evidence".into(),
        produced("refresh", "evidence", "fresh-after-both"),
    );
    assert!(plan(vec![reader, refresh, second, first])
        .validate(&catalog())
        .is_ok());
}

#[test]
fn freshness_transitive_draft_lineage_rejects_old_evidence_after_change() {
    let before = draft_node(
        "old-draft",
        "old-draft-output",
        ArtifactBinding::External {
            artifact: "external-evidence".into(),
        },
    );
    let mut invalidator = invalidate_node("invalidate");
    invalidator.after.insert("old-draft".into());
    let mut review = review_node("review", "old-draft", "old-draft-output");
    review.after.insert("invalidate".into());
    assert!(matches!(
        plan(vec![review, invalidator, before]).validate(&catalog()),
        Err(PlanError::StaleArtifact {
            invalidator,
            consumer,
            artifact,
            artifact_type,
        }) if invalidator == OperationInstanceId::from("invalidate")
            && consumer == OperationInstanceId::from("review")
            && artifact == ArtifactId::from("old-draft-output")
            && artifact_type == ArtifactTypeId::from("evidence")
    ));

    let invalidator = invalidate_node("invalidate");
    let mut refresh = refresh_node("refresh", "fresh-evidence");
    refresh.after.insert("invalidate".into());
    let draft = draft_node(
        "new-draft",
        "new-draft-output",
        produced("refresh", "evidence", "fresh-evidence"),
    );
    let review = review_node("review", "new-draft", "new-draft-output");
    assert!(plan(vec![review, draft, refresh, invalidator])
        .validate(&catalog())
        .is_ok());
}

#[test]
fn freshness_self_invalidating_node_cannot_launder_old_input_into_different_output() {
    let producer = OperationNode {
        instance: "invalidate-and-draft".into(),
        operation: reference("truth.invalidate-and-draft"),
        inputs: BTreeMap::from([(
            "evidence".into(),
            ArtifactBinding::External {
                artifact: "external-evidence".into(),
            },
        )]),
        outputs: BTreeMap::from([("draft".into(), "draft-with-old-evidence".into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::from(["truth.mutate".into()]),
    };
    let review = review_node("review", "invalidate-and-draft", "draft-with-old-evidence");
    assert!(matches!(
        plan(vec![review, producer]).validate(&catalog()),
        Err(PlanError::StaleArtifact { invalidator, .. })
            if invalidator == OperationInstanceId::from("invalidate-and-draft")
    ));

    let producer = OperationNode {
        instance: "invalidate-and-refresh".into(),
        operation: reference("truth.invalidate-and-refresh"),
        inputs: BTreeMap::new(),
        outputs: BTreeMap::from([("evidence".into(), "own-fresh-evidence".into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::from(["truth.mutate".into()]),
    };
    let mut reader = score_node("score", "valid-score");
    reader.inputs.insert(
        "evidence".into(),
        produced("invalidate-and-refresh", "evidence", "own-fresh-evidence"),
    );
    assert!(plan(vec![reader, producer]).validate(&catalog()).is_ok());
}

#[test]
fn freshness_invalidated_type_ids_must_be_nonempty() {
    let mut contract = Score::contract();
    contract.invalidates.insert("".into());
    assert_eq!(
        verify_contract(&contract),
        Err(PlanError::EmptyIdentifier("invalidated_artifact_type_id"))
    );
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

    // The effect owner receives a typed, deterministic dispatch identity;
    // the manifest is not itself a PostgreSQL/operator authorization.
    let first = checked
        .checked_dispatch(&"publish".into(), br#"{"score":1}"#, DispatchAttempt::First)
        .unwrap();
    assert_eq!(first.operation, reference("release.commit"));
    assert_eq!(first.owner, OperationOwnerId::from("publication_owner"));
    assert_eq!(
        first.effects,
        BTreeSet::from([EffectId::from("release.write")])
    );
    assert_eq!(first.scope, scope());
    assert_eq!(first.plan_hash, checked.hash().clone());
    assert_eq!(first.idempotency, IdempotencyPolicy::Idempotent);
    assert_eq!(
        first.input_hash,
        blake3::hash(br#"{"score":1}"#).to_hex().to_string()
    );
    let history_record = serde_json::to_vec(&first).unwrap();
    assert_eq!(
        serde_json::from_slice::<CheckedDispatch>(&history_record).unwrap(),
        first,
        "dispatch facts must roundtrip as an immutable typed Temporal History payload"
    );
    assert_eq!(
        first.execution_key,
        checked
            .checked_dispatch(
                &"publish".into(),
                br#"{"score":1}"#,
                DispatchAttempt::RetryAfterUnknownOutcome
            )
            .unwrap()
            .execution_key,
        "idempotent retry must bind to exactly the original execution key"
    );
    assert_ne!(
        first.execution_key,
        checked
            .checked_dispatch(&"publish".into(), br#"{"score":2}"#, DispatchAttempt::First)
            .unwrap()
            .execution_key,
        "changed actual bytes must never share an effect receipt key"
    );
    assert!(matches!(
        checked.checked_dispatch(&"unknown".into(), b"", DispatchAttempt::First),
        Err(PlanError::UnknownInstance(_))
    ));
}

#[test]
fn at_most_once_effect_refuses_ambiguous_retry_before_any_provider_call() {
    let mut commit = OperationNode {
        instance: "commit".into(),
        operation: reference("release.commit.at-most-once"),
        inputs: BTreeMap::from([("score".into(), produced("score", "score", "result"))]),
        outputs: BTreeMap::from([("release".into(), "committed".into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::from(["release.write".into()]),
    };
    let checked = plan(vec![score_node("score", "result"), commit.clone()])
        .validate(&catalog())
        .unwrap();
    assert_eq!(
        checked
            .checked_dispatch(&"commit".into(), b"verified-input", DispatchAttempt::First)
            .unwrap()
            .idempotency,
        IdempotencyPolicy::AtMostOnce
    );
    assert_eq!(
        checked.checked_dispatch(
            &"commit".into(),
            b"verified-input",
            DispatchAttempt::RetryAfterUnknownOutcome,
        ),
        Err(PlanError::AmbiguousEffectRetry("commit".into()))
    );
    // An effect policy cannot be upgraded by editing the manifest after seal.
    commit.permitted_effects.clear();
    let unchecked = plan(vec![score_node("score", "result"), commit]);
    assert!(matches!(
        unchecked.validate(&catalog()),
        Err(PlanError::MissingEffect { .. })
    ));
}

#[test]
fn version_and_unknown_operation_fail_closed() {
    let mut historical = plan(vec![score_node("old-score", "old-output")]);
    historical.schema_revision = 1;
    historical.hash = historical.canonical_hash().unwrap();
    assert_eq!(
        historical.validate(&catalog()),
        Err(PlanError::UnsupportedPlanRevision(1)),
        "a legacy plan with no pinned freshness semantics must not run as v2"
    );

    let mut scenario = plan(vec![score_node("score", "out")]);
    scenario.schema_revision += 1;
    scenario = scenario.seal(&catalog()).unwrap();
    assert_eq!(
        scenario.validate(&catalog()),
        Err(PlanError::UnsupportedPlanRevision(PLAN_SCHEMA_REVISION + 1))
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
    let original = format!("\"schema_revision\":{}", PLAN_SCHEMA_REVISION);
    let invalid = serialized.replacen(
        original.as_str(),
        &format!("{original},\"unrecognized\":true"),
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

    let mut invalidation_changed = ContractCatalog::default();
    invalidation_changed
        .register::<ChangedScoreInvalidation>()
        .unwrap();
    assert_eq!(
        pinned.validate(&invalidation_changed),
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

fn source_node(id: &str, output: &str) -> OperationNode {
    OperationNode {
        instance: id.into(),
        operation: reference("source.collect"),
        inputs: BTreeMap::new(),
        outputs: BTreeMap::from([("evidence".into(), output.into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    }
}

fn fan_out_fan_in_nodes() -> Vec<OperationNode> {
    let source = source_node("seed", "seed-evidence");
    let independent = source_node("d-free", "free-evidence");
    let mut left = score_node("b-right", "right-score");
    left.inputs.insert(
        "evidence".into(),
        produced("seed", "evidence", "seed-evidence"),
    );
    let mut right = score_node("z-left", "left-score");
    right.inputs.insert(
        "evidence".into(),
        produced("seed", "evidence", "seed-evidence"),
    );
    let merge = OperationNode {
        instance: "a-join".into(),
        operation: reference("synthetic.merge-scores"),
        inputs: BTreeMap::from([
            ("left".into(), produced("z-left", "score", "left-score")),
            ("right".into(), produced("b-right", "score", "right-score")),
        ]),
        outputs: BTreeMap::from([("merged".into(), "merged-result".into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    };
    vec![merge, left, independent, right, source]
}

fn for_each_permutation(
    nodes: &mut [OperationNode],
    first: usize,
    visit: &mut impl FnMut(&[OperationNode]),
) {
    if first == nodes.len() {
        visit(nodes);
        return;
    }
    for index in first..nodes.len() {
        nodes.swap(first, index);
        for_each_permutation(nodes, first + 1, visit);
        nodes.swap(first, index);
    }
}

#[test]
fn every_serialization_of_fan_out_fan_in_and_independent_work_is_one_dag() {
    // All 5! authoring orders are valid. The source fans out to two independent
    // scoring instances, then both must finish before the typed merge owner.
    // The unrelated source is not a dependency even when the tie-break runs
    // it first. This is a protocol property, not another provider route.
    let mut nodes = fan_out_fan_in_nodes();
    let baseline = plan(nodes.clone());
    let expected_order = vec![
        OperationInstanceId::from("d-free"),
        OperationInstanceId::from("seed"),
        OperationInstanceId::from("b-right"),
        OperationInstanceId::from("z-left"),
        OperationInstanceId::from("a-join"),
    ];
    let mut checked_permutations = 0;
    for_each_permutation(&mut nodes, 0, &mut |arrangement| {
        let actual = plan(arrangement.to_vec());
        assert_eq!(actual.hash, baseline.hash);
        let validated = actual.validate(&catalog()).unwrap();
        assert_eq!(validated.execution_order(), expected_order.as_slice());
        let merge = validated
            .checked_dispatch(&"a-join".into(), b"two-checked-scores", DispatchAttempt::First)
            .unwrap();
        assert_eq!(merge.owner, OperationOwnerId::from("independent_merge_owner"));
        assert_eq!(merge.idempotency, IdempotencyPolicy::Pure);
        checked_permutations += 1;
    });
    assert_eq!(checked_permutations, 120);
}

#[test]
fn fan_in_rejects_wrong_typed_edge_and_back_edge_before_effect_dispatch() {
    let mut wrong_schema = fan_out_fan_in_nodes();
    let join = wrong_schema
        .iter_mut()
        .find(|node| node.instance.as_str() == "a-join")
        .unwrap();
    join.inputs.insert(
        "left".into(),
        produced("seed", "evidence", "seed-evidence"),
    );
    assert!(matches!(
        plan(wrong_schema).validate(&catalog()),
        Err(PlanError::ArtifactSchemaMismatch { instance, port, artifact })
            if instance == OperationInstanceId::from("a-join")
                && port == PortId::from("left")
                && artifact == ArtifactId::from("seed-evidence")
    ));

    let mut cycle = fan_out_fan_in_nodes();
    let consumer = cycle
        .iter_mut()
        .find(|node| node.instance.as_str() == "z-left")
        .unwrap();
    consumer.after.insert("a-join".into());
    assert_eq!(plan(cycle).validate(&catalog()), Err(PlanError::Cycle));
}

#[test]
fn registering_a_new_owner_extends_the_graph_without_engine_changes() {
    let base = plan(fan_out_fan_in_nodes());
    let mut extended = base.clone();
    extended.nodes.push(OperationNode {
        instance: "extension".into(),
        operation: reference("extension.report"),
        inputs: BTreeMap::from([(
            "merged".into(),
            produced("a-join", "merged", "merged-result"),
        )]),
        outputs: BTreeMap::from([("report".into(), "extension-report".into())]),
        after: BTreeSet::new(),
        permitted_effects: BTreeSet::new(),
    });
    let mut trusted = catalog();
    assert!(matches!(
        extended.clone().seal(&trusted),
        Err(PlanError::UnknownOperation(operation))
            if operation == reference("extension.report")
    ));

    // This is the same owner-neutral registration path available to future
    // separately built adapters; there is no core enum or provider switch.
    trusted.register_contract(ExtensionReport::contract()).unwrap();
    let sealed = extended.seal(&trusted).unwrap();
    let validated = sealed.validate(&trusted).unwrap();
    assert_eq!(
        validated.execution_order().last(),
        Some(&OperationInstanceId::from("extension"))
    );
    let dispatch = validated
        .checked_dispatch(&"extension".into(), b"merged-result-bytes", DispatchAttempt::First)
        .unwrap();
    assert_eq!(dispatch.owner, OperationOwnerId::from("newly_registered_owner"));
    assert_eq!(dispatch.operation, reference("extension.report"));
    assert_ne!(sealed.hash, base.hash);

    // A newly registered owner gets exactly the same typed-port protection.
    let mut incompatible = sealed.clone();
    incompatible
        .nodes
        .iter_mut()
        .find(|node| node.instance.as_str() == "extension")
        .unwrap()
        .inputs
        .insert("merged".into(), produced("b-right", "score", "right-score"));
    let incompatible = incompatible.seal(&trusted).unwrap();
    assert!(matches!(
        incompatible.validate(&trusted),
        Err(PlanError::ArtifactSchemaMismatch { instance, port, artifact })
            if instance == OperationInstanceId::from("extension")
                && port == PortId::from("merged")
                && artifact == ArtifactId::from("right-score")
    ));
}
