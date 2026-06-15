fn build_deterministic_candidate(
    input: &EditorialDraftGenerateInputPayload,
    provider_key: &str,
    model_key: &str,
) -> Result<LlmDraftCandidate, DomainError> {
    let Some(request) = input.request.as_ref() else {
        return Err(DomainError::ValidationFailure {
            message: "editorial draft generation requires LlmDraftRequest".to_string(),
        });
    };
    let Some(brief) = request.editorial_brief.as_ref() else {
        return Err(DomainError::ValidationFailure {
            message: "editorial draft generation requires EditorialBrief".to_string(),
        });
    };
    let mut markdown = String::new();
    let mut sections = Vec::new();
    let mut claim_ledger = Vec::new();
    let mut content_blocks = Vec::new();
    let page_node_key = &brief.page_node_key;
    markdown.push_str("# Expert travel visa guide\n\n");
    let block_plan = planned_content_blocks(
        &brief.section_templates,
        &brief.verified_support,
        &brief.required_links,
    );
    for block in &block_plan {
        let supports = supports_for_role(&brief.verified_support, &block.section_role);
        let heading = block.heading.clone();
        let mut body = String::new();
        if block.section_role == "related_pages" {
            for link in &brief.required_links {
                body.push_str("- ");
                body.push_str(&link.link_role);
                body.push_str(" -> ");
                body.push_str(&link.target_page_key);
                body.push('\n');
            }
        } else if block.section_role == "cta_disclaimer" {
            body.push_str("This draft is ready for human editorial review. Confirm current official requirements before publication.\n");
        } else {
            body.push_str(if block.deterministic_only {
                "Verified source-backed structured content:\n"
            } else {
                "Based on verified source-backed evidence:\n"
            });
            for support in supports {
                body.push_str("- ");
                body.push_str(support.fragment_text.trim());
                body.push('\n');
                let claim_key = primitives::seo::seo_artifact_key(
                    "claim",
                    &[page_node_key, &block.section_role, &support.support_ref],
                );
                claim_ledger.push(ClaimLedgerEntry {
                    claim_key,
                    fragment_text: support.fragment_text.clone(),
                    claim_kind: if factual_role(&block.section_role) {
                        "factual".to_string()
                    } else {
                        "editorial".to_string()
                    },
                    traceability_label: support_traceability_label(&block.section_role).to_string(),
                    support_refs: vec![support.support_ref.clone()],
                    validation_verdict: "supported".to_string(),
                    source_section_key: block.section_role.clone(),
                });
            }
        }
        let section_key = primitives::seo::seo_artifact_key(
            "draft_section",
            &[page_node_key, &block.section_role],
        );
        markdown.push_str("## ");
        markdown.push_str(&heading);
        markdown.push_str("\n\n");
        markdown.push_str(&body);
        markdown.push('\n');
        sections.push(SeoDraftSectionState {
            section_key: section_key.clone(),
            section_role: block.section_role.clone(),
            heading: heading.clone(),
            body_markdown: body.clone(),
            required: block.required,
            traceability_label: support_traceability_label(&block.section_role).to_string(),
            support_refs: claim_ledger
                .iter()
                .filter(|claim| claim.source_section_key == block.section_role)
                .flat_map(|claim| claim.support_refs.clone())
                .collect(),
            template_key: brief
                .section_templates
                .iter()
                .find(|template| template.section_role == block.section_role)
                .map(|template| template.template_key.clone())
                .unwrap_or_default(),
        });
        content_blocks.push(RenderedContentBlock {
            block_key: primitives::seo::seo_artifact_key(
                "content_block",
                &[page_node_key, &block.section_role, "llm"],
            ),
            block_type: block.block_type.clone(),
            section_role: block.section_role.clone(),
            heading,
            markdown: body,
            support_refs: claim_ledger
                .iter()
                .filter(|claim| claim.source_section_key == block.section_role)
                .flat_map(|claim| claim.support_refs.clone())
                .collect(),
            traceability_label: support_traceability_label(&block.section_role).to_string(),
            required: block.required,
        });
    }
    let faq_items = brief
        .verified_support
        .iter()
        .take(6)
        .map(|support| {
            serde_json::json!({
                "@type": "Question",
                "name": "What does the verified source say?",
                "acceptedAnswer": {
                    "@type": "Answer",
                    "text": support.fragment_text,
                }
            })
        })
        .collect::<Vec<_>>();
    let faq_json = serde_json::to_string(&faq_items).unwrap_or_else(|_| "[]".to_string());
    let schema_markup_json = serde_json::to_string(&serde_json::json!({
        "@context": "https://schema.org",
        "@graph": [
            {
                "@type": "Article",
                "headline": "Expert travel visa guide",
            },
            {
                "@type": "FAQPage",
                "mainEntity": faq_items,
            }
        ]
    }))
    .unwrap_or_else(|_| "{}".to_string());
    Ok(LlmDraftCandidate {
        candidate_key: primitives::seo::seo_artifact_key(
            "llm_draft_candidate",
            &[&request.request_key, provider_key, model_key],
        ),
        request_key: request.request_key.clone(),
        provider_key: provider_key.to_string(),
        model_key: model_key.to_string(),
        prompt_version: if request.prompt_version.trim().is_empty() {
            DETERMINISTIC_PROMPT_VERSION.to_string()
        } else {
            request.prompt_version.clone()
        },
        body_markdown: markdown,
        sections,
        claim_ledger,
        content_blocks,
        faq_json,
        schema_markup_json,
        status: "ready".to_string(),
    })
}
