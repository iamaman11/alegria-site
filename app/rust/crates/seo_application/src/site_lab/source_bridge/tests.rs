use super::*;
use dataforseo_contracts::{DataForSeoOrganicHitV1, DataForSeoSourceStatusV1};

fn scope() -> RunScope {
    RunScope {
        tenant_id: "tenant-a".into(),
        run_id: "123e4567-e89b-42d3-a456-426614174000".into(),
    }
}

fn serp_input() -> DataForSeoGoogleOrganicSearchInputV1 {
    DataForSeoGoogleOrganicSearchInputV1 {
        schema_revision: 1,
        scope: scope(),
        keyword: "kitchen cost reviews".into(),
        language_code: "en".into(),
        location_code: 2840,
        depth: 10,
        max_cost_usd_micros: 100_000,
    }
}

fn organic_discovery(
    input: &DataForSeoGoogleOrganicSearchInputV1,
) -> DataForSeoGoogleOrganicSearchOutputV1 {
    DataForSeoGoogleOrganicSearchOutputV1 {
        schema_revision: 1,
        source_status: DataForSeoSourceStatusV1::SerpDiscoveryOnly,
        provider_task_id: "synthetic-task-1".into(),
        provider_response_hash: owner_content_hash(b"synthetic provider JSON"),
        input_hash: owner_content_hash(&serde_json::to_vec(input).unwrap()),
        provider_reported_cost_usd_micros: Some(100),
        organic_results: vec![
            DataForSeoOrganicHitV1 {
                rank: 1,
                title: "Synthetic public page".into(),
                source_url: "https://shop.example.org/reviews".into(),
                domain_norm: "shop.example.org".into(),
                snippet: "just discovery".into(),
            },
            DataForSeoOrganicHitV1 {
                rank: 2,
                title: "Forbidden hostname".into(),
                source_url: "https://localhost.internal/admin".into(),
                domain_norm: "localhost.internal".into(),
                snippet: "not allowed".into(),
            },
            DataForSeoOrganicHitV1 {
                rank: 3,
                title: "Duplicate page".into(),
                source_url: "https://shop.example.org/reviews".into(),
                domain_norm: "shop.example.org".into(),
                snippet: "same page".into(),
            },
        ],
    }
}

fn fetch_request(url: &str) -> WebFetchRequestV1 {
    WebFetchRequestV1 {
        tenant_id: "tenant-a".into(),
        source_url: url.into(),
        allowed_hosts: [
            "forum.example.org".to_owned(),
            "reviews.example.net".to_owned(),
            "shop.example.org".to_owned(),
        ]
        .into(),
        max_response_bytes: 4096,
        max_redirects: 0,
        timeout_seconds: 10,
    }
}

fn snapshot(url: &str, raw: &[u8], key: &str) -> WebPageSnapshotV1 {
    WebPageSnapshotV1 {
        tenant_id: "tenant-a".into(),
        snapshot_key: key.into(),
        source_url: url.into(),
        final_url: url.into(),
        content_hash: owner_content_hash(raw),
        captured_at_utc: "2026-11-27T12:00:00Z".into(),
        body_bytes: raw.len().try_into().unwrap(),
        robots_evidence_hash: owner_content_hash(b"robots synthetic"),
        redirect_chain_hash: owner_content_hash(b"none"),
    }
}

fn finding(id: &str, excerpt: &str) -> PublicPageFindingV1 {
    PublicPageFindingV1 {
        observation_id: id.into(),
        need_key: "cost-transparency".into(),
        asserted_need: "Understand the final price".into(),
        context: "Synthetic test data; no claim about real customers".into(),
        excerpt: excerpt.into(),
    }
}

fn evidence_page<'a>(
    request: &'a WebFetchRequestV1,
    snapshot: &'a WebPageSnapshotV1,
    document_bytes: &'a [u8],
    findings: &'a [PublicPageFindingV1],
) -> PublicPageEvidenceInputV1<'a> {
    PublicPageEvidenceInputV1 {
        request,
        snapshot,
        document_bytes,
        source_run_id: "source-run",
        findings,
    }
}

fn hypothesis() -> ConsumerNeedHypothesisV1 {
    ConsumerNeedHypothesisV1 {
        need_key: "cost-transparency".into(),
        proposed_site_change: "Show a detailed quote".into(),
        expected_benefit: "Make costs easier to compare".into(),
    }
}

#[test]
fn serp_results_are_review_candidates_only_and_never_network_authority() {
    let input = serp_input();
    let output = organic_discovery(&input);
    let policy = SerpFetchPolicyV1 {
        requested_allowed_hosts: ["shop.example.org".to_owned()].into(),
        max_response_bytes: 2048,
        max_redirects: 0,
        timeout_seconds: 10,
    };
    let candidates = preview_serp_fetch_candidates_v1(&input, &output, &policy).unwrap();
    assert_eq!(candidates.len(), 3);
    assert_eq!(
        candidates[0].status,
        SerpFetchCandidateStatusV1::RequiresIndependentOwnerAdmission
    );
    assert_eq!(
        candidates[0].proposed_request.as_ref().unwrap().tenant_id,
        "tenant-a"
    );
    assert_eq!(
        candidates[1].status,
        SerpFetchCandidateStatusV1::RejectedStaticPolicy
    );
    assert!(candidates[1].proposed_request.is_none());
    assert_eq!(candidates[2].status, SerpFetchCandidateStatusV1::DuplicateUrl);
    assert!(candidates[2].proposed_request.is_none());
}

#[test]
fn serp_provider_hash_input_mismatch_fails_closed_before_candidate_generation() {
    let input = serp_input();
    let mut output = organic_discovery(&input);
    output.input_hash = owner_content_hash(b"another run");
    assert!(matches!(
        preview_serp_fetch_candidates_v1(
            &input,
            &output,
            &SerpFetchPolicyV1 {
                requested_allowed_hosts: ["shop.example.org".to_owned()].into(),
                max_response_bytes: 2048,
                max_redirects: 0,
                timeout_seconds: 10,
            }
        ),
        Err(ResearchSourceBridgeErrorV1::InvalidDiscovery)
    ));
}

#[test]
fn stub_only_discovery_cannot_be_promoted_to_real_fetch_candidates() {
    let input = serp_input();
    let mut output = organic_discovery(&input);
    output.source_status = DataForSeoSourceStatusV1::StubFixtureOnly;
    let result = preview_serp_fetch_candidates_v1(
        &input,
        &output,
        &SerpFetchPolicyV1 {
            requested_allowed_hosts: ["shop.example.org".to_owned()].into(),
            max_response_bytes: 2048,
            max_redirects: 0,
            timeout_seconds: 10,
        },
    );
    assert!(matches!(result, Err(ResearchSourceBridgeErrorV1::InvalidDiscovery)));
}

#[test]
fn two_exact_snapshots_generate_only_unattested_multi_source_candidate() {
    let first_bytes = b"<p>My quote had hidden costs.</p>";
    let second_bytes = b"<p>We could not compare kitchen materials.</p>";
    let first_request = fetch_request("https://forum.example.org/thread/one");
    let second_request = fetch_request("https://reviews.example.net/item/two");
    let first_snapshot = snapshot(&first_request.source_url, first_bytes, "one");
    let second_snapshot = snapshot(&second_request.source_url, second_bytes, "two");
    let first = [finding("forum-one", "My quote had hidden costs.")];
    let second = [finding("review-two", "We could not compare kitchen materials.")];
    let pages = [
        PublicPageEvidenceInputV1 {
            request: &first_request,
            snapshot: &first_snapshot,
            document_bytes: first_bytes,
            source_run_id: "first-run",
            findings: &first,
        },
        PublicPageEvidenceInputV1 {
            request: &second_request,
            snapshot: &second_snapshot,
            document_bytes: second_bytes,
            source_run_id: "second-run",
            findings: &second,
        },
    ];
    let preview = bind_public_page_findings_v1(&scope(), &pages, vec![hypothesis()]).unwrap();
    assert_eq!(preview.status, "structural_only_unattested");
    assert_eq!(preview.assessments[0].owner_receipt_attestation, "not_checked");
    assert_eq!(preview.assessments[0].source_authentication, "not_checked");
    assert_eq!(
        preview.assessments[0].status,
        super::super::evidence::ConsumerNeedEvidenceStatusV1::MultiSourceCandidateUnattested
    );
    assert_eq!(preview.assessments[0].observations.len(), 2);
    assert!(preview.assessments[0].observations.iter().all(|item| {
        item.provenance_locator.starts_with("web-snapshot:")
            && !item.provenance_locator.contains("https://")
    }));
}

#[test]
fn changed_bytes_fake_excerpt_or_foreign_tenant_cannot_be_bound() {
    let source_bytes = b"<article>Actual price discussion</article>";
    let request = fetch_request("https://forum.example.org/price");
    let snapshot = snapshot(&request.source_url, source_bytes, "synthetic-one");
    let good = [finding("one", "Actual price discussion")];
    let wrong = [finding("one", "Invented customer quote")];
    assert!(matches!(
        bind_public_page_findings_v1(
            &scope(),
            &[evidence_page(&request, &snapshot, b"fake", &good)],
            vec![hypothesis()]
        ),
        Err(ResearchSourceBridgeErrorV1::SnapshotMismatch)
    ));
    assert!(matches!(
        bind_public_page_findings_v1(
            &scope(),
            &[evidence_page(&request, &snapshot, source_bytes, &wrong)],
            vec![hypothesis()]
        ),
        Err(ResearchSourceBridgeErrorV1::ExcerptNotInSnapshot)
    ));
    let mut foreign = snapshot.clone();
    foreign.tenant_id = "other-tenant".into();
    assert!(matches!(
        bind_public_page_findings_v1(
            &scope(),
            &[PublicPageEvidenceInputV1 {
                request: &request,
                snapshot: &foreign,
                document_bytes: source_bytes,
                source_run_id: "source-run",
                findings: &good,
            }],
            vec![hypothesis()]
        ),
        Err(ResearchSourceBridgeErrorV1::ScopeMismatch)
    ));
    let mut missing_robots_proof = snapshot.clone();
    missing_robots_proof.robots_evidence_hash = "missing".into();
    assert!(matches!(
        bind_public_page_findings_v1(
            &scope(),
            &[evidence_page(
                &request,
                &missing_robots_proof,
                source_bytes,
                &good
            )],
            vec![hypothesis()]
        ),
        Err(ResearchSourceBridgeErrorV1::SnapshotMismatch)
    ));
}

#[test]
fn forged_redirect_to_disallowed_host_is_not_evidence() {
    let bytes = b"trusted source text";
    let request = fetch_request("https://forum.example.org/one");
    let mut snapshot = snapshot(&request.source_url, bytes, "one");
    snapshot.final_url = "https://evil.example.com/exfil".into();
    let findings = [finding("one", "trusted source text")];
    assert!(matches!(
        bind_public_page_findings_v1(
            &scope(),
            &[PublicPageEvidenceInputV1 {
                request: &request,
                snapshot: &snapshot,
                document_bytes: bytes,
                source_run_id: "source-run",
                findings: &findings,
            }],
            vec![hypothesis()]
        ),
        Err(ResearchSourceBridgeErrorV1::InvalidSourceUrl)
    ));
}

#[test]
fn same_publisher_pages_never_produce_two_independent_sources() {
    let bytes_a = b"<p>Custom cupboards are expensive</p>";
    let bytes_b = b"<p>I requested a detailed invoice</p>";
    let request_a = fetch_request("https://forum.example.org/a");
    let request_b = fetch_request("https://forum.example.org/b");
    let snapshot_a = snapshot(&request_a.source_url, bytes_a, "a");
    let snapshot_b = snapshot(&request_b.source_url, bytes_b, "b");
    let a = [finding("a", "Custom cupboards are expensive")];
    let b = [finding("b", "I requested a detailed invoice")];
    let preview = bind_public_page_findings_v1(
        &scope(),
        &[
            PublicPageEvidenceInputV1 {
                request: &request_a,
                snapshot: &snapshot_a,
                document_bytes: bytes_a,
                source_run_id: "source-run",
                findings: &a,
            },
            PublicPageEvidenceInputV1 {
                request: &request_b,
                snapshot: &snapshot_b,
                document_bytes: bytes_b,
                source_run_id: "source-run",
                findings: &b,
            },
        ],
        vec![hypothesis()],
    )
    .unwrap();
    assert_eq!(
        preview.assessments[0].status,
        super::super::evidence::ConsumerNeedEvidenceStatusV1::InsufficientEvidence
    );
}
