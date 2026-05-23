use std::collections::BTreeSet;

use crate::pii_redaction_prepass_step::{
    sanitize_candidate, sanitize_source_context_chunks, sanitize_support_bundle, sanitize_text,
};
use crate::seo_step_support::artifact_key;
use contracts::generated::alegria::temporal::v1::{
    ClaimLedgerEntry, ContentBlockPlanItemState, ContentBlockPlanState, DraftAssembleInputPayload,
    DraftAssembleOutputPayload, DraftState, EditorialBrief, LlmDraftRequest, PageBriefState,
    RenderedContentBlock, SectionTemplateBinding, SeoDraftSectionState, SeoTraceabilityEntryState,
    SeoVerifiedFactSupportState,
};
use runtime_models::seo_blocks::{
    default_section_roles, factual_role, heading_for_role, mandatory_section_roles,
    planned_content_blocks,
    support_traceability_label, supports_for_role,
};

// Draft assembly must emit traceability_entries with verified_fact / verified_summary
// labels so downstream QA and persistence can enforce support_refs coverage.

fn title_from_slug(slug: &str) -> String {
    let title = slug
        .split(['-', '/'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        "Expert Guide".to_string()
    } else {
        title
    }
}

fn template_bindings(input: &DraftAssembleInputPayload) -> Vec<SectionTemplateBinding> {
    if !input.section_templates.is_empty() {
        return input.section_templates.clone();
    }
    let page_type = input
        .page_blueprint
        .as_ref()
        .map(|blueprint| blueprint.page_type_key.clone())
        .unwrap_or_else(|| "detail_page".to_string());
    let intent = input
        .page_blueprint
        .as_ref()
        .map(|blueprint| blueprint.dominant_intent.clone())
        .unwrap_or_else(|| "informational".to_string());
    let required_roles = input
        .page_blueprint
        .as_ref()
        .map(|blueprint| {
            if blueprint.required_sections.is_empty() {
                mandatory_section_roles()
                    .iter()
                    .map(|role| (*role).to_string())
                    .collect::<BTreeSet<_>>()
            } else {
                blueprint
                    .required_sections
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
            }
        })
        .unwrap_or_else(|| {
            mandatory_section_roles()
                .iter()
                .map(|role| (*role).to_string())
                .collect::<BTreeSet<_>>()
        });
    default_section_roles()
        .iter()
        .map(|role| SectionTemplateBinding {
            template_key: artifact_key("section_template", &[&page_type, &intent, role, "1"]),
            page_type_key: page_type.clone(),
            dominant_intent: intent.clone(),
            section_role: (*role).to_string(),
            heading: heading_for_role(role).to_string(),
            template_body: format!(
                "Write the {role} section using only verified support refs and neutral expert tone."
            ),
            template_version: 1,
            required: required_roles.contains(*role),
        })
        .collect()
}

fn section_roles(templates: &[SectionTemplateBinding]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut roles = Vec::new();
    for role in default_section_roles() {
        if seen.insert((*role).to_string()) {
            roles.push((*role).to_string());
        }
    }
    for template in templates {
        if seen.insert(template.section_role.clone()) {
            roles.push(template.section_role.clone());
        }
    }
    roles
}

fn traceability_from_support(
    page_node_key: &str,
    section_role: &str,
    support: &SeoVerifiedFactSupportState,
) -> SeoTraceabilityEntryState {
    SeoTraceabilityEntryState {
        fragment_key: artifact_key(
            "draft_fragment",
            &[page_node_key, section_role, &support.support_ref],
        ),
        fragment_text: support.fragment_text.clone(),
        fragment_kind: if factual_role(section_role) {
            "factual".to_string()
        } else {
            "editorial".to_string()
        },
        traceability_label: support_traceability_label(section_role).to_string(),
        support_refs: vec![support.support_ref.clone()],
        validation_verdict: "supported".to_string(),
    }
}

fn claim_from_traceability(
    entry: &SeoTraceabilityEntryState,
    section_key: &str,
) -> ClaimLedgerEntry {
    ClaimLedgerEntry {
        claim_key: artifact_key("claim", &[&entry.fragment_key, section_key]),
        fragment_text: entry.fragment_text.clone(),
        claim_kind: entry.fragment_kind.clone(),
        traceability_label: entry.traceability_label.clone(),
        support_refs: entry.support_refs.clone(),
        validation_verdict: entry.validation_verdict.clone(),
        source_section_key: section_key.to_string(),
    }
}

fn deterministic_section_body(
    page_node_key: &str,
    section_role: &str,
    section_support: &[&SeoVerifiedFactSupportState],
    required_links: &[contracts::generated::alegria::temporal::v1::LinkRecommendationState],
    traceability_entries: &mut Vec<SeoTraceabilityEntryState>,
    claim_ledger: &mut Vec<ClaimLedgerEntry>,
) -> String {
    if section_role == "related_pages" {
        if required_links.is_empty() {
            return "No mandatory related pages are required for this scope.\n".to_string();
        }
        let mut body = String::new();
        for link in required_links {
            body.push_str("- Recommended internal link: ");
            body.push_str(&link.link_role);
            body.push_str(" -> ");
            body.push_str(&link.target_page_key);
            body.push('\n');
        }
        return body;
    }
    if section_role == "cta_disclaimer" {
        return "Use this page as an expert editorial draft for human review. Confirm current official requirements before advising a traveler.\n".to_string();
    }
    if section_support.is_empty() {
        traceability_entries.push(SeoTraceabilityEntryState {
            fragment_key: artifact_key(
                "draft_fragment",
                &[page_node_key, section_role, "unsupported"],
            ),
            fragment_text: format!(
                "Missing verified support for {}.",
                heading_for_role(section_role)
            ),
            fragment_kind: "factual".to_string(),
            traceability_label: "unsupported_factual_fragment".to_string(),
            support_refs: Vec::new(),
            validation_verdict: "blocked".to_string(),
        });
        return format!("[unsupported:{section_role}]\n");
    }

    match section_role {
        "documents" => {
            let mut body = "The verified checklist currently requires:\n".to_string();
            for support in section_support {
                body.push_str("- ");
                body.push_str(support.fragment_text.trim());
                body.push('\n');
                let trace = traceability_from_support(page_node_key, section_role, support);
                claim_ledger.push(claim_from_traceability(&trace, section_role));
                traceability_entries.push(trace);
            }
            body
        }
        "fees" => {
            let mut body = "| Fee item | Verified source |\n|---|---|\n".to_string();
            for support in section_support {
                body.push('|');
                body.push_str(support.fragment_text.trim());
                body.push('|');
                body.push_str(if support.source_label.trim().is_empty() {
                    &support.support_ref
                } else {
                    support.source_label.trim()
                });
                body.push_str("|\n");
                let trace = traceability_from_support(page_node_key, section_role, support);
                claim_ledger.push(claim_from_traceability(&trace, section_role));
                traceability_entries.push(trace);
            }
            body
        }
        "faq" => {
            let mut body = String::new();
            for support in section_support.iter().take(6) {
                body.push_str("### What does the verified source say?\n\n");
                body.push_str(support.fragment_text.trim());
                body.push_str("\n\n");
                let trace = traceability_from_support(page_node_key, section_role, support);
                claim_ledger.push(claim_from_traceability(&trace, section_role));
                traceability_entries.push(trace);
            }
            body
        }
        _ => {
            let mut body =
                "This section is assembled from verified source-backed fragments:\n".to_string();
            for support in section_support {
                body.push_str("- ");
                body.push_str(support.fragment_text.trim());
                body.push('\n');
                let trace = traceability_from_support(page_node_key, section_role, support);
                claim_ledger.push(claim_from_traceability(&trace, section_role));
                traceability_entries.push(trace);
            }
            body
        }
    }
}

fn supported_refs_for_section(
    support: &[SeoVerifiedFactSupportState],
    section_role: &str,
) -> Vec<String> {
    supports_for_role(support, section_role)
        .iter()
        .map(|entry| entry.support_ref.clone())
        .collect()
}

fn build_fallback_draft_parts(
    page_node_key: &str,
    title: &str,
    support: &[SeoVerifiedFactSupportState],
    required_links: &[contracts::generated::alegria::temporal::v1::LinkRecommendationState],
    templates: &[SectionTemplateBinding],
) -> (
    String,
    Vec<SeoDraftSectionState>,
    Vec<SeoTraceabilityEntryState>,
    Vec<ClaimLedgerEntry>,
    Vec<RenderedContentBlock>,
) {
    let mut body = format!("# {title}\n\n");
    let mut sections = Vec::new();
    let mut traceability_entries = Vec::new();
    let mut claim_ledger = Vec::new();
    let mut content_blocks = Vec::new();
    let block_plan = planned_content_blocks(templates, support, required_links);

    for block in &block_plan {
        let section_role = block.section_role.clone();
        let heading = block.heading.clone();
        let section_support = supports_for_role(support, &section_role);
        let section_body = deterministic_section_body(
            page_node_key,
            &section_role,
            &section_support,
            required_links,
            &mut traceability_entries,
            &mut claim_ledger,
        );
        let section_key = artifact_key("draft_section", &[page_node_key, &section_role]);
        body.push_str("## ");
        body.push_str(&heading);
        body.push_str("\n\n");
        body.push_str(&section_body);
        body.push('\n');
        sections.push(SeoDraftSectionState {
            section_key: section_key.clone(),
            section_role: section_role.clone(),
            heading: heading.clone(),
            body_markdown: section_body.clone(),
            required: block.required,
            traceability_label: if factual_role(&section_role) {
                support_traceability_label(&section_role).to_string()
            } else {
                "business_copy".to_string()
            },
            support_refs: supported_refs_for_section(support, &section_role),
            template_key: templates
                .iter()
                .find(|template| template.section_role == section_role)
                .map(|template| template.template_key.clone())
                .unwrap_or_default(),
        });
        content_blocks.push(RenderedContentBlock {
            block_key: artifact_key("content_block", &[page_node_key, &section_role, "1"]),
            block_type: block.block_type.clone(),
            section_role: section_role.clone(),
            heading,
            markdown: section_body,
            support_refs: supported_refs_for_section(support, &section_role),
            traceability_label: if factual_role(&section_role) {
                support_traceability_label(&section_role).to_string()
            } else {
                "business_copy".to_string()
            },
            required: block.required,
        });
    }

    (
        body,
        sections,
        traceability_entries,
        claim_ledger,
        content_blocks,
    )
}

fn traceability_from_claims(claims: &[ClaimLedgerEntry]) -> Vec<SeoTraceabilityEntryState> {
    claims
        .iter()
        .map(|claim| SeoTraceabilityEntryState {
            fragment_key: claim.claim_key.clone(),
            fragment_text: claim.fragment_text.clone(),
            fragment_kind: claim.claim_kind.clone(),
            traceability_label: claim.traceability_label.clone(),
            support_refs: claim.support_refs.clone(),
            validation_verdict: claim.validation_verdict.clone(),
        })
        .collect()
}

fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn default_schema(title: &str, path: &str, faq_json: &str) -> String {
    format!(
        "{{\"@context\":\"https://schema.org\",\"@graph\":[{{\"@type\":\"Article\",\"headline\":\"{}\",\"url\":\"{}\"}},{{\"@type\":\"FAQPage\",\"mainEntity\":{}}}]}}",
        json_escape(title),
        json_escape(path),
        if faq_json.trim().is_empty() {
            "[]"
        } else {
            faq_json.trim()
        }
    )
}

fn faq_json_from_support(support: &[SeoVerifiedFactSupportState]) -> String {
    let items = support
        .iter()
        .take(6)
        .map(|entry| {
            format!(
                "{{\"@type\":\"Question\",\"name\":\"What does the verified source say?\",\"acceptedAnswer\":{{\"@type\":\"Answer\",\"text\":\"{}\"}}}}",
                json_escape(&entry.fragment_text)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{items}]")
}

fn build_content_block_plan(
    page_node_key: &str,
    page_brief_key: &str,
    templates: &[SectionTemplateBinding],
    support: &[SeoVerifiedFactSupportState],
    required_links: &[contracts::generated::alegria::temporal::v1::LinkRecommendationState],
) -> ContentBlockPlanState {
    let items = planned_content_blocks(templates, support, required_links)
        .into_iter()
        .map(|item| ContentBlockPlanItemState {
            plan_item_key: artifact_key(
                "content_block_plan_item",
                &[page_node_key, &item.section_role],
            ),
            section_role: item.section_role.clone(),
            block_type: item.block_type.clone(),
            heading: item.heading.clone(),
            required: item.required,
            deterministic_only: item.deterministic_only,
            support_refs: supported_refs_for_section(support, &item.section_role),
            template_key: templates
                .iter()
                .find(|template| template.section_role == item.section_role)
                .map(|template| template.template_key.clone())
                .unwrap_or_default(),
        })
        .collect();
    ContentBlockPlanState {
        content_block_plan_key: artifact_key(
            "content_block_plan",
            &[page_node_key, page_brief_key, "seo_content_block_plan@1"],
        ),
        page_node_key: page_node_key.to_string(),
        page_brief_key: page_brief_key.to_string(),
        items,
        plan_version: "seo_content_block_plan@1".to_string(),
    }
}

pub fn execute(input: &DraftAssembleInputPayload) -> DraftAssembleOutputPayload {
    let page_node = input.page_node.clone().unwrap_or_default();
    let blueprint = input.page_blueprint.clone().unwrap_or_default();
    let safe_title = title_from_slug(&page_node.canonical_slug);
    let templates = template_bindings(input);
    let sanitized_support = sanitize_support_bundle(&input.verified_support);
    let sanitized_source_context_chunks =
        sanitize_source_context_chunks(&input.source_context_chunks);
    let page_brief_key = artifact_key(
        "page_brief",
        &[&page_node.page_node_key, &blueprint.blueprint_key, "1"],
    );
    let editorial_brief = EditorialBrief {
        editorial_brief_key: artifact_key(
            "editorial_brief",
            &[&page_node.page_node_key, &page_brief_key, "1"],
        ),
        page_node_key: page_node.page_node_key.clone(),
        page_brief_key: page_brief_key.clone(),
        target_audience: "visa/tourism applicants".to_string(),
        expertise_goal:
            "Produce a source-backed expert travel/visa page without introducing unsourced claims."
                .to_string(),
        section_templates: templates.clone(),
        verified_support: sanitized_support.clone(),
        required_links: input.required_links.clone(),
        truth_snapshot_ref: input.run_id.clone(),
        source_context_chunks: sanitized_source_context_chunks,
    };
    let llm_request = LlmDraftRequest {
        request_key: artifact_key(
            "llm_draft_request",
            &[
                &page_node.page_node_key,
                &page_brief_key,
                "seo_editorial_prompt@1",
            ],
        ),
        run_id: input.run_id.clone(),
        editorial_brief: Some(editorial_brief.clone()),
        provider_policy: "multi_provider:first_available".to_string(),
        prompt_version: "seo_editorial_prompt@1".to_string(),
        output_contract: "llm_draft_candidate@1".to_string(),
    };
    let content_block_plan = build_content_block_plan(
        &page_node.page_node_key,
        &page_brief_key,
        &templates,
        &sanitized_support,
        &input.required_links,
    );

    let fallback = build_fallback_draft_parts(
        &page_node.page_node_key,
        &safe_title,
        &sanitized_support,
        &input.required_links,
        &templates,
    );
    let candidate = input.llm_candidate.as_ref().map(sanitize_candidate);
    let has_candidate = candidate
        .as_ref()
        .map(|value| value.status == "ready" && !value.body_markdown.trim().is_empty())
        .unwrap_or(false);

    let (
        body,
        sections,
        traceability_entries,
        claim_ledger,
        content_blocks,
        faq_json,
        schema_markup_json,
        llm_provider_key,
        llm_model_key,
        generation_request_key,
    ) = if has_candidate {
        let candidate = candidate.as_ref().unwrap();
        (
            candidate.body_markdown.clone(),
            if candidate.sections.is_empty() {
                fallback.1
            } else {
                candidate.sections.clone()
            },
            traceability_from_claims(&candidate.claim_ledger),
            candidate.claim_ledger.clone(),
            if candidate.content_blocks.is_empty() {
                fallback.4
            } else {
                candidate.content_blocks.clone()
            },
            if candidate.faq_json.trim().is_empty() {
                faq_json_from_support(&sanitized_support)
            } else {
                candidate.faq_json.clone()
            },
            if candidate.schema_markup_json.trim().is_empty() {
                default_schema(
                    &safe_title,
                    &page_node.canonical_url_path,
                    &candidate.faq_json,
                )
            } else {
                candidate.schema_markup_json.clone()
            },
            candidate.provider_key.clone(),
            candidate.model_key.clone(),
            candidate.request_key.clone(),
        )
    } else {
        let faq_json = faq_json_from_support(&sanitized_support);
        (
            fallback.0,
            fallback.1,
            fallback.2,
            fallback.3,
            fallback.4,
            faq_json.clone(),
            default_schema(&safe_title, &page_node.canonical_url_path, &faq_json),
            "deterministic_fallback".to_string(),
            "source_backed_template@1".to_string(),
            llm_request.request_key.clone(),
        )
    };

    let mut traceability_entries = traceability_entries;
    for fragment in &input.factual_fragments {
        if !fragment.trim().is_empty()
            && !input
                .verified_support
                .iter()
                .any(|support| support.fragment_text.trim() == fragment.trim())
        {
            traceability_entries.push(SeoTraceabilityEntryState {
                fragment_key: artifact_key(
                    "draft_fragment",
                    &[&page_node.page_node_key, "manual", fragment.trim()],
                ),
                fragment_text: sanitize_text(fragment.trim()),
                fragment_kind: "factual".to_string(),
                traceability_label: "unsupported_factual_fragment".to_string(),
                support_refs: Vec::new(),
                validation_verdict: "blocked".to_string(),
            });
        }
    }

    for chunk in &input.source_context_chunks {
        let policy = chunk.usage_policy.to_ascii_lowercase();
        if policy.contains("redistribution_allowed=false")
            || policy.contains("non_redistributable")
            || policy.contains("no redistribution")
        {
            traceability_entries.push(SeoTraceabilityEntryState {
                fragment_key: artifact_key(
                    "draft_fragment",
                    &[&page_node.page_node_key, "license", &chunk.chunk_key],
                ),
                fragment_text: "Source usage policy forbids redistribution.".to_string(),
                fragment_kind: "policy".to_string(),
                traceability_label: "restricted_redistribution_source".to_string(),
                support_refs: Vec::new(),
                validation_verdict: "blocked".to_string(),
            });
        }
    }

    let draft = has_candidate.then_some(DraftState {
        page_draft_key: artifact_key(
            "page_draft",
            &[
                &page_node.page_node_key,
                &page_brief_key,
                "1",
                "seo_draft_assemble@3",
            ],
        ),
        page_node_key: page_node.page_node_key.clone(),
        page_brief_key: page_brief_key.clone(),
        draft_revision: 1,
        body_markdown: body,
        qa_verdict: "not_run".to_string(),
        truth_snapshot_ref: input.run_id.clone(),
        sections,
        traceability_entries,
        required_links: input.required_links.clone(),
        schema_markup_json,
        claim_ledger,
        content_blocks,
        faq_json,
        llm_provider_key,
        llm_model_key,
        generation_request_key,
    });
    DraftAssembleOutputPayload {
        page_brief: Some(PageBriefState {
            page_brief_key,
            page_node_key: page_node.page_node_key,
            blueprint_key: blueprint.blueprint_key,
            title: safe_title,
            meta_description: "Expert guide assembled from verified source-backed facts."
                .to_string(),
            brief_version: 1,
            status: "ready".to_string(),
            scope_signature: page_node.scope_signature,
            target_audience: "visa/tourism applicants".to_string(),
            goal: "Answer the dominant visa/tourism intent with verified facts and human review."
                .to_string(),
            required_sections: section_roles(&templates),
            metadata_obligations: vec![
                "title".to_string(),
                "meta_description".to_string(),
                "canonical_url".to_string(),
                "h1".to_string(),
                "schema_markup".to_string(),
                "claim_ledger".to_string(),
                "content_blocks".to_string(),
            ],
            required_links: input.required_links.clone(),
            truth_snapshot_ref: input.run_id.clone(),
        }),
        draft,
        editorial_brief: Some(editorial_brief),
        llm_request: Some(llm_request),
        content_block_plan: Some(content_block_plan),
    }
}
