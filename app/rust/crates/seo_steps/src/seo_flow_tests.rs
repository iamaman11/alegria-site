use contracts::generated::alegria::temporal::v1::{
    ClaimLedgerEntry, CmsPublishInputPayload, DraftAssembleInputPayload,
    DraftNormalizeInputPayload, DraftQaInputPayload, IaBuildInputPayload,
    LinkRecommendInputPayload, LlmDraftCandidate, OpportunityBuildInputPayload,
    RebuildDetectInputPayload, SeoScopePayload, SeoVerifiedFactSupportState,
    SerpIngestInputPayload, SerpNormalizeInputPayload,
};

fn scope() -> SeoScopePayload {
    let scope_signature = primitives::seo::scope_signature(&[
        ("market", "alegria-site"),
        ("locale", "ru-RU"),
        ("country_code", "ES"),
        ("visa_type", "tourist"),
        ("applicant_profile", "standard"),
    ]);
    SeoScopePayload {
        market: "alegria-site".to_string(),
        locale: "ru-RU".to_string(),
        country_code: "ES".to_string(),
        visa_type: "tourist".to_string(),
        applicant_profile: "standard".to_string(),
        raw_scope_tuple: String::new(),
        scope_signature,
    }
}

fn support(
    fragment_text: &str,
    support_ref: &str,
    role_type: &str,
    source_label: &str,
) -> SeoVerifiedFactSupportState {
    SeoVerifiedFactSupportState {
        fragment_text: fragment_text.to_string(),
        support_ref: support_ref.to_string(),
        role_type: role_type.to_string(),
        source_label: source_label.to_string(),
        source_tier: "official".to_string(),
        freshness_class: "fresh".to_string(),
        observed_at: "2026-05-06T00:00:00Z".to_string(),
        valid_until: "2027-05-06T00:00:00Z".to_string(),
    }
}

fn llm_candidate(
    page_node_key: &str,
    request_key: &str,
    verified_support: &[SeoVerifiedFactSupportState],
) -> LlmDraftCandidate {
    LlmDraftCandidate {
        candidate_key: primitives::seo::seo_artifact_key(
            "llm_candidate",
            &[page_node_key, "fixture"],
        ),
        request_key: request_key.to_string(),
        provider_key: "deterministic_fixture".to_string(),
        model_key: "source_backed_template@1".to_string(),
        prompt_version: "seo_editorial_prompt@1".to_string(),
        body_markdown: "# Expert travel visa guide\n\nSource-backed editorial preview.".to_string(),
        sections: Vec::new(),
        claim_ledger: verified_support
            .iter()
            .map(|support| ClaimLedgerEntry {
                claim_key: primitives::seo::seo_artifact_key(
                    "claim",
                    &[page_node_key, &support.role_type, &support.support_ref],
                ),
                fragment_text: support.fragment_text.clone(),
                claim_kind: "factual".to_string(),
                traceability_label: "verified_fact".to_string(),
                support_refs: vec![support.support_ref.clone()],
                validation_verdict: "supported".to_string(),
                source_section_key: support.role_type.clone(),
            })
            .collect(),
        content_blocks: Vec::new(),
        faq_json: String::new(),
        schema_markup_json: String::new(),
        status: "ready".to_string(),
    }
}

#[test]
fn seo_steps_form_publish_ready_pipeline() {
    let run_id = "8dcb91a6-7b3c-44ad-b00f-22922dd4590a".to_string();
    let scope = scope();
    let ingest = crate::serp_ingest_step::execute(&SerpIngestInputPayload {
        run_id: run_id.clone(),
        query_batch_key: primitives::seo::seo_artifact_key(
            "query_batch",
            &[&run_id, &scope.scope_signature],
        ),
        scope: Some(scope.clone()),
        queries: vec![
            "Spain tourist visa requirements".to_string(),
            "spain tourist visa cost".to_string(),
            "spain tourist visa cost".to_string(),
        ],
    });
    assert_eq!(ingest.query_count, 3);
    assert_eq!(ingest.status, "done");

    let serp = crate::serp_normalize_step::execute(&SerpNormalizeInputPayload {
        run_id: run_id.clone(),
        query_batch_key: ingest.query_batch_key,
        scope: Some(scope.clone()),
        queries: vec![
            "Spain tourist visa requirements".to_string(),
            "spain tourist visa cost".to_string(),
            "spain tourist visa cost".to_string(),
        ],
    });
    assert_eq!(serp.normalized_query_count, 2);
    assert_eq!(serp.serp_patterns.len(), 2);

    let opportunities = crate::opportunity_build_step::execute(&OpportunityBuildInputPayload {
        run_id: run_id.clone(),
        scope: Some(scope.clone()),
        serp_patterns: serp.serp_patterns,
    });
    assert_eq!(opportunities.keyword_clusters.len(), 2);
    assert!(opportunities.content_gaps.is_empty());

    let ia = crate::ia_build_step::execute(&IaBuildInputPayload {
        run_id: run_id.clone(),
        scope: Some(scope),
        keyword_clusters: opportunities.keyword_clusters,
    });
    assert_eq!(ia.page_nodes.len(), 4);
    assert!(ia
        .page_nodes
        .iter()
        .any(|page| page.page_type_key == "hub_page"));
    assert!(ia.page_nodes.iter().any(|page| {
        page.page_type_key == "country_hub_page"
            && page.canonical_url_path == "/ru/visa/spain/"
            && page.hierarchy_depth == 2
    }));
    assert!(ia.page_nodes.iter().any(|page| {
        page.page_type_key == "hub_page"
            && page.canonical_url_path == "/ru/visa/spain/tourist/"
            && page.hierarchy_depth == 3
            && !page.parent_page_node_key.is_empty()
    }));
    assert!(ia.cannibalization_conflicts.is_empty());

    let links = crate::link_recommend_step::execute(&LinkRecommendInputPayload {
        run_id: run_id.clone(),
        page_nodes: ia.page_nodes.clone(),
        max_links_per_page: 1,
    });
    assert!(links.link_recommendations.len() >= 4);
    assert!(links
        .link_recommendations
        .iter()
        .any(|link| link.required_flag && link.link_role == "hub_child"));
    assert!(links
        .link_recommendations
        .iter()
        .any(|link| link.required_flag && link.link_role == "child_hub"));

    let page_node = ia
        .page_nodes
        .iter()
        .find(|page| page.page_type_key == "hub_page")
        .cloned()
        .expect("hub page");
    let page_blueprint = ia
        .page_blueprints
        .iter()
        .find(|blueprint| blueprint.blueprint_key == page_node.blueprint_key)
        .cloned()
        .expect("blueprint for page node");
    let fragments = Vec::new();
    let verified_support = vec![
        support(
            "Required document: passport.",
            "rule:passport",
            "document_required",
            "Official consular checklist",
        ),
        support(
            "Fee item: 80.00 EUR for visa fee.",
            "rule:fee",
            "fee_item",
            "Official consular fee table",
        ),
        support(
            "Processing timeline: decision takes 15 days.",
            "rule:timeline",
            "timeline_item",
            "Official consular timeline",
        ),
        support(
            "Where to apply: consulate via appointment.",
            "rule:where",
            "where_to_apply",
            "Official consular route",
        ),
        support(
            "Eligibility rule: standard tourist applicant.",
            "rule:eligibility",
            "eligibility_rule",
            "Official consular rule",
        ),
        support(
            "Application step 1: submit the application.",
            "rule:step",
            "step",
            "Official consular route",
        ),
    ];
    let required_links: Vec<_> = links
        .link_recommendations
        .iter()
        .filter(|link| link.required_flag && link.source_page_key == page_node.page_node_key)
        .cloned()
        .collect();
    let draft = crate::draft_assemble_step::execute(&DraftAssembleInputPayload {
        run_id: run_id.clone(),
        page_node: Some(page_node.clone()),
        page_blueprint: Some(page_blueprint.clone()),
        factual_fragments: fragments.clone(),
        verified_support: verified_support.clone(),
        required_links: required_links.clone(),
        section_templates: Vec::new(),
        llm_candidate: None,
        source_context_chunks: Vec::new(),
    });
    assert!(draft.page_brief.is_some());
    assert!(draft.draft.is_none());
    assert!(draft.content_block_plan.is_some());
    let request_key = draft.llm_request.clone().expect("llm request").request_key;

    let draft_normalized = crate::draft_normalize_step::execute(&DraftNormalizeInputPayload {
        run_id: run_id.clone(),
        page_node: Some(page_node.clone()),
        page_blueprint: Some(page_blueprint.clone()),
        factual_fragments: fragments.clone(),
        verified_support: verified_support.clone(),
        required_links: required_links.clone(),
        section_templates: draft
            .editorial_brief
            .clone()
            .expect("editorial brief")
            .section_templates,
        content_block_plan: draft.content_block_plan.clone(),
        llm_candidate: Some(llm_candidate(
            &page_node.page_node_key,
            &request_key,
            &verified_support,
        )),
    });
    assert!(draft_normalized.draft.is_some());
    assert!(draft_normalized
        .draft
        .as_ref()
        .expect("draft")
        .traceability_entries
        .iter()
        .all(|entry| entry.validation_verdict != "blocked"));
    assert!(!draft_normalized
        .draft
        .as_ref()
        .expect("draft")
        .claim_ledger
        .is_empty());
    assert!(!draft_normalized
        .draft
        .as_ref()
        .expect("draft")
        .content_blocks
        .is_empty());
    assert_ne!(
        draft_normalized
            .draft
            .as_ref()
            .expect("draft")
            .schema_markup_json,
        "{}"
    );

    let draft_state = draft_normalized.draft.clone();
    let qa = crate::draft_qa_step::execute(&DraftQaInputPayload {
        run_id: run_id.clone(),
        draft: draft_state.clone(),
        supported_fragments: fragments,
        required_links,
    });
    assert_eq!(qa.verdict, "publish_ready");
    assert!(qa.blocking_reasons.is_empty());

    let draft_without_links = crate::draft_assemble_step::execute(&DraftAssembleInputPayload {
        run_id: run_id.clone(),
        page_node: Some(page_node.clone()),
        page_blueprint: Some(page_blueprint.clone()),
        factual_fragments: Vec::new(),
        verified_support: verified_support.clone(),
        required_links: Vec::new(),
        section_templates: Vec::new(),
        llm_candidate: None,
        source_context_chunks: Vec::new(),
    });
    let draft_without_links = crate::draft_normalize_step::execute(&DraftNormalizeInputPayload {
        run_id: run_id.clone(),
        page_node: Some(page_node.clone()),
        page_blueprint: Some(page_blueprint),
        factual_fragments: Vec::new(),
        verified_support: verified_support.clone(),
        required_links: Vec::new(),
        section_templates: draft_without_links
            .editorial_brief
            .clone()
            .expect("editorial brief")
            .section_templates,
        content_block_plan: draft_without_links.content_block_plan.clone(),
        llm_candidate: Some(llm_candidate(
            &page_node.page_node_key,
            &request_key,
            &verified_support,
        )),
    });
    let qa_without_links = crate::draft_qa_step::execute(&DraftQaInputPayload {
        run_id: run_id.clone(),
        draft: draft_without_links.draft,
        supported_fragments: Vec::new(),
        required_links: Vec::new(),
    });
    assert_eq!(qa_without_links.verdict, "publish_ready");
    assert!(!qa_without_links
        .blocking_reasons
        .contains(&"missing_required_internal_links".to_string()));

    let mut cms_draft = draft_state.expect("draft state");
    cms_draft.qa_verdict = qa.verdict;
    let cms = crate::cms_publish_step::execute(&CmsPublishInputPayload {
        run_id: run_id.clone(),
        page_node: Some(page_node),
        draft: Some(cms_draft),
        actor_role: "seo_system".to_string(),
        publish_mode: "request_review".to_string(),
        approval_decision: None,
    });
    assert_eq!(cms.verdict, "review_requested");
    assert!(cms.blocking_reasons.is_empty());

    let rebuild = crate::rebuild_detect_step::execute(&RebuildDetectInputPayload {
        run_id,
        changed_truth_keys: vec!["verified.rule_instance:fee".to_string()],
        page_nodes: ia.page_nodes,
    });
    assert_eq!(rebuild.verdict, "rebuild_required");
    assert_eq!(rebuild.impacted_page_node_keys.len(), 4);
}
