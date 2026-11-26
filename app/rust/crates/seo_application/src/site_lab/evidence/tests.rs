use super::*;

fn hypothesis(key: &str) -> ConsumerNeedHypothesisV1 {
    ConsumerNeedHypothesisV1 {
        need_key: key.into(),
        proposed_site_change: "Show a transparent cost breakdown".into(),
        expected_benefit: "Reduce confusion during comparison".into(),
    }
}

fn observation(
    id: &str,
    key: &str,
    kind: NeedEvidenceRepresentationV1,
    origin: &str,
    document: &str,
    original_bytes: &[u8],
) -> ConsumerNeedObservationV1 {
    let excerpt = format!("synthetic observed text for {id}");
    ConsumerNeedObservationV1 {
        observation_id: id.into(),
        need_key: key.into(),
        asserted_need: "Compare final costs".into(),
        context: "Synthetic test fixture, not real customer testimony".into(),
        excerpt_hash: owner_content_hash(excerpt.as_bytes()),
        excerpt,
        source: NeedEvidenceSourceV1 {
            source_id: format!("synthetic/{id}"),
            tenant_id: "tenant-a".into(),
            source_run_id: "source-run".into(),
            retrieval_channel: "synthetic-test".into(),
            representation: kind,
            primary_origin_id: origin.into(),
            primary_document_id: document.into(),
            provenance_locator: format!("synthetic://fixture/{id}"),
            document_payload_hash: owner_content_hash(original_bytes),
        },
    }
}

fn batch(observations: Vec<ConsumerNeedObservationV1>) -> ConsumerNeedEvidenceBatchV1 {
    ConsumerNeedEvidenceBatchV1 {
        schema_revision: NEED_EVIDENCE_SCHEMA_REVISION_V1,
        scope: RunScope {
            tenant_id: "tenant-a".into(),
            run_id: "research-one".into(),
        },
        observations,
        hypotheses: vec![hypothesis("cost-transparency")],
    }
}

#[test]
fn independent_primary_publishers_produce_only_unattested_candidate() {
    let first = observation(
        "forum-post",
        "cost-transparency",
        NeedEvidenceRepresentationV1::PrimaryDocument,
        "forum-author-a",
        "forum/topic/1",
        b"synthetic original forum post",
    );
    let second = observation(
        "review",
        "cost-transparency",
        NeedEvidenceRepresentationV1::PrimaryDocument,
        "independent-review-author-b",
        "review/2",
        b"synthetic independent review",
    );
    let preview = preview_consumer_need_evidence(&batch(vec![second, first])).unwrap();
    assert_eq!(preview.status, "structural_only_unattested");
    assert_eq!(preview.assessments.len(), 1);
    let assessment = &preview.assessments[0];
    assert_eq!(
        assessment.status,
        ConsumerNeedEvidenceStatusV1::MultiSourceCandidateUnattested
    );
    assert_eq!(
        assessment.structurally_independent_observation_ids,
        vec!["forum-post".to_owned(), "review".to_owned()]
    );
    assert_eq!(assessment.source_authentication, "not_checked");
    assert_eq!(assessment.owner_receipt_attestation, "not_checked");
    assert_eq!(assessment.execution, "not_started");
    assert_eq!(assessment.observations.len(), 2);
    assert_eq!(
        assessment.observations[0].document_payload_hash,
        owner_content_hash(b"synthetic original forum post")
    );
}

#[test]
fn serp_snippet_and_destination_page_never_count_as_two_sources() {
    let search_snippet = observation(
        "serp-snippet",
        "cost-transparency",
        NeedEvidenceRepresentationV1::SearchSnippet,
        "publisher-a",
        "canonical-destination/a",
        b"synthetic canonical destination",
    );
    let destination = observation(
        "destination",
        "cost-transparency",
        NeedEvidenceRepresentationV1::PrimaryDocument,
        "publisher-a",
        "canonical-destination/a",
        b"synthetic canonical destination",
    );
    let assessment = preview_consumer_need_evidence(&batch(vec![search_snippet, destination]))
        .unwrap()
        .assessments
        .remove(0);
    assert_eq!(
        assessment.status,
        ConsumerNeedEvidenceStatusV1::InsufficientEvidence
    );
    assert!(assessment
        .structurally_independent_observation_ids
        .is_empty());
    assert_eq!(assessment.observations.len(), 2);
}

#[test]
fn same_publisher_or_copied_document_does_not_create_independence() {
    let first = observation(
        "a",
        "cost-transparency",
        NeedEvidenceRepresentationV1::PrimaryDocument,
        "one-publisher",
        "one-publisher/page-a",
        b"one source content",
    );
    let same_publisher = observation(
        "b",
        "cost-transparency",
        NeedEvidenceRepresentationV1::PrimaryDocument,
        "one-publisher",
        "one-publisher/page-b",
        b"different content, same original publisher",
    );
    let result =
        preview_consumer_need_evidence(&batch(vec![first.clone(), same_publisher])).unwrap();
    assert_eq!(
        result.assessments[0].status,
        ConsumerNeedEvidenceStatusV1::InsufficientEvidence
    );

    let copied_bytes = observation(
        "mirror-claim",
        "cost-transparency",
        NeedEvidenceRepresentationV1::PrimaryDocument,
        "different-publisher-claim",
        "mirror/page-c",
        b"one source content",
    );
    let result = preview_consumer_need_evidence(&batch(vec![first, copied_bytes])).unwrap();
    assert_eq!(
        result.assessments[0].status,
        ConsumerNeedEvidenceStatusV1::InsufficientEvidence
    );
}

#[test]
fn mirrored_documents_and_model_summaries_never_count_as_primary_sources() {
    let primary = observation(
        "primary",
        "cost-transparency",
        NeedEvidenceRepresentationV1::PrimaryDocument,
        "publisher-a",
        "publisher-a/original",
        b"original",
    );
    let mirrored = observation(
        "mirror",
        "cost-transparency",
        NeedEvidenceRepresentationV1::MirroredDocument,
        "claimed-mirror-b",
        "mirror-b/copy",
        b"mirror",
    );
    let summary = observation(
        "model-summary",
        "cost-transparency",
        NeedEvidenceRepresentationV1::DerivedSummary,
        "model-c",
        "model-c/summary",
        b"summary",
    );
    let result = preview_consumer_need_evidence(&batch(vec![primary, mirrored, summary])).unwrap();
    assert_eq!(
        result.assessments[0].status,
        ConsumerNeedEvidenceStatusV1::InsufficientEvidence
    );
    assert_eq!(result.assessments[0].observations.len(), 3);
}

#[test]
fn conflicting_document_origin_and_cross_tenant_source_fail_closed() {
    let first = observation(
        "first",
        "cost-transparency",
        NeedEvidenceRepresentationV1::SearchSnippet,
        "original-a",
        "same-canonical-document",
        b"document",
    );
    let second = observation(
        "second",
        "cost-transparency",
        NeedEvidenceRepresentationV1::PrimaryDocument,
        "claimed-original-b",
        "same-canonical-document",
        b"document",
    );
    assert!(matches!(
        preview_consumer_need_evidence(&batch(vec![first.clone(), second])),
        Err(ConsumerNeedEvidenceError::ConflictingPrimaryDocument)
    ));

    let mut wrong_tenant = first;
    wrong_tenant.source.tenant_id = "tenant-b".into();
    assert!(matches!(
        preview_consumer_need_evidence(&batch(vec![wrong_tenant])),
        Err(ConsumerNeedEvidenceError::ScopeMismatch)
    ));
}

#[test]
fn same_source_id_cannot_claim_two_publishers_documents_or_snapshots() {
    let first = observation(
        "source-a",
        "cost-transparency",
        NeedEvidenceRepresentationV1::PrimaryDocument,
        "original-author",
        "original/document",
        b"original source bytes",
    );
    let mut forged = observation(
        "source-b",
        "cost-transparency",
        NeedEvidenceRepresentationV1::PrimaryDocument,
        "unrelated-author",
        "unrelated/document",
        b"different source bytes",
    );
    forged.source.source_id = first.source.source_id.clone();
    assert!(matches!(
        preview_consumer_need_evidence(&batch(vec![first.clone(), forged])),
        Err(ConsumerNeedEvidenceError::ConflictingPrimaryDocument)
    ));

    let mut same_id_new_snapshot = first.clone();
    same_id_new_snapshot.observation_id = "new-excerpt".into();
    same_id_new_snapshot.source.document_payload_hash = owner_content_hash(b"updated bytes");
    assert!(matches!(
        preview_consumer_need_evidence(&batch(vec![first, same_id_new_snapshot])),
        Err(ConsumerNeedEvidenceError::ConflictingPrimaryDocument)
    ));
}

#[test]
fn duplicate_observations_and_invalid_excerpt_hash_fail_closed() {
    let first = observation(
        "same-id",
        "cost-transparency",
        NeedEvidenceRepresentationV1::PrimaryDocument,
        "a",
        "a/1",
        b"primary",
    );
    assert!(matches!(
        preview_consumer_need_evidence(&batch(vec![first.clone(), first.clone()])),
        Err(ConsumerNeedEvidenceError::DuplicateObservation)
    ));
    let mut forged_excerpt = first;
    forged_excerpt.excerpt.push('!');
    assert!(matches!(
        preview_consumer_need_evidence(&batch(vec![forged_excerpt])),
        Err(ConsumerNeedEvidenceError::InvalidHash)
    ));
}

#[test]
fn missing_evidence_remains_insufficient_and_does_not_invent_observations() {
    let preview = preview_consumer_need_evidence(&batch(Vec::new())).unwrap();
    assert_eq!(
        preview.assessments[0].status,
        ConsumerNeedEvidenceStatusV1::InsufficientEvidence
    );
    assert!(preview.assessments[0].observations.is_empty());
    assert!(preview.assessments[0]
        .structurally_independent_observation_ids
        .is_empty());
}
