fn overlay_generated_sections(
    mut candidate: LlmDraftCandidate,
    request: &LlmDraftRequest,
    generated: GeneratedSections,
) -> LlmDraftCandidate {
    let Some(brief) = request.editorial_brief.as_ref() else {
        return candidate;
    };
    let block_plan = planned_content_blocks(
        &brief.section_templates,
        &brief.verified_support,
        &brief.required_links,
    );
    let title = if generated.seo_title.is_empty() {
        "Expert travel visa guide"
    } else {
        &generated.seo_title
    };
    let mut body_parts = vec![format!("# {title}"), String::new()];
    for section in &mut candidate.sections {
        let Some(block) = block_plan
            .iter()
            .find(|block| block.section_role == section.section_role)
        else {
            continue;
        };
        if block.deterministic_only && section.section_role != "cta_disclaimer" {
            body_parts.push(format!(
                "## {}\n\n{}\n",
                section.heading, section.body_markdown
            ));
            continue;
        }
        if let Some(generated_markdown) = generated.sections.get(&section.section_role) {
            if !generated_markdown.trim().is_empty() {
                section.body_markdown = generated_markdown.trim().to_string();
            }
        }
        body_parts.push(format!(
            "## {}\n\n{}\n",
            section.heading, section.body_markdown
        ));
    }
    for block in &mut candidate.content_blocks {
        if let Some(generated_markdown) = generated.sections.get(&block.section_role) {
            let deterministic_only = block_plan
                .iter()
                .find(|plan| plan.section_role == block.section_role)
                .map(|plan| plan.deterministic_only)
                .unwrap_or(false);
            if (!deterministic_only || block.section_role == "cta_disclaimer")
                && !generated_markdown.trim().is_empty()
            {
                block.markdown = generated_markdown.trim().to_string();
            }
        }
    }
    candidate.body_markdown = body_parts.join("\n");
    candidate.seo_title = generated.seo_title;
    candidate.seo_meta_description = generated.seo_meta_description;
    // Rebuild Article schema with the verified editorial headline downstream,
    // rather than retaining the deterministic English placeholder.
    if !candidate.seo_title.is_empty() {
        candidate.schema_markup_json.clear();
    }
    candidate
}

fn candidate_from_generated_text(
    input: &EditorialDraftGenerateInputPayload,
    provider_key: &str,
    model_key: &str,
    text: &str,
) -> Result<LlmDraftCandidate, DomainError> {
    let request = input
        .request
        .as_ref()
        .ok_or_else(|| DomainError::ValidationFailure {
            message: "editorial draft generation requires LlmDraftRequest".to_string(),
        })?;
    let deterministic = build_deterministic_candidate(input, provider_key, model_key)?;
    let generated = parse_generated_sections(text)?;
    if generated.claims.is_some() {
        if request.output_contract != "llm_draft_candidate@2" {
            return Err(DomainError::ValidationFailure {
                message: "typed claims require explicit editorial contract v2".to_string(),
            });
        }
        return overlay_typed_claims(deterministic, request, generated);
    }
    if request.output_contract == "llm_draft_candidate@2" {
        return Err(DomainError::ValidationFailure {
            message: "typed editorial claims are mandatory for contract v2".to_string(),
        });
    }
    for role in generated.sections.keys() {
        if !deterministic
            .sections
            .iter()
            .any(|section| section.section_role == *role)
        {
            return Err(DomainError::ValidationFailure {
                message: format!("unknown editorial section role from LLM: {role}"),
            });
        }
    }
    Ok(overlay_generated_sections(
        deterministic,
        request,
        generated,
    ))
}

fn overlay_typed_claims(
    mut candidate: LlmDraftCandidate,
    request: &LlmDraftRequest,
    mut generated: GeneratedSections,
) -> Result<LlmDraftCandidate, DomainError> {
    use contracts::generated::alegria::temporal::v1::EditorialClaimProposal;
    use runtime_models::seo_blocks::{render_editorial_claim, validate_editorial_claim};
    use std::collections::BTreeSet;

    let fail = |message: String| DomainError::ValidationFailure { message };
    let brief = request
        .editorial_brief
        .as_ref()
        .ok_or_else(|| fail("missing brief".into()))?;
    let mut claims = generated
        .claims
        .take()
        .ok_or_else(|| fail("missing claims".into()))?;
    let mut ledger = Vec::new();
    for section in &mut candidate.sections {
        let role = &section.section_role;
        if !factual_role(role) {
            section.body_markdown = match role.as_str() {
                "cta_disclaimer" => {
                    runtime_models::seo_blocks::editorial_cta(&brief.locale_code).to_string()
                }
                "related_pages" => runtime_models::seo_blocks::render_verified_source_list(
                    &brief.verified_support,
                    &brief.locale_code,
                ),
                _ => return Err(fail(format!("unsupported non-factual section {role}"))),
            };
            continue;
        }
        let proposed = claims
            .remove(role)
            .ok_or_else(|| fail(format!("missing factual claims for {role}")))?;
        let supports = supports_for_role(&brief.verified_support, role);
        let expected = supports
            .iter()
            .map(|s| s.support_ref.as_str())
            .collect::<BTreeSet<_>>();
        let mut seen = BTreeSet::new();
        let mut text = Vec::new();
        let mut refs = Vec::new();
        for item in proposed {
            if !seen.insert(item.support_ref.clone()) {
                return Err(fail(format!(
                    "duplicate editorial support reference for {role}"
                )));
            }
            let source = supports
                .iter()
                .find(|s| s.support_ref == item.support_ref)
                .ok_or_else(|| {
                    fail(format!(
                        "unknown or inapplicable support reference for {role}"
                    ))
                })?;
            let claim = ClaimLedgerEntry {
                claim_key: primitives::seo::seo_artifact_key(
                    "claim",
                    &[&brief.page_node_key, role, &source.support_ref],
                ),
                fragment_text: source.fragment_text.clone(),
                claim_kind: "factual".to_string(),
                traceability_label: support_traceability_label(role).to_string(),
                support_refs: vec![source.support_ref.clone()],
                validation_verdict: "supported".to_string(),
                source_section_key: role.clone(),
                editorial_proposal: Some(EditorialClaimProposal {
                    text: item.text,
                    question: item.question,
                    source_url: source.source_url.clone(),
                }),
            };
            validate_editorial_claim(&claim, &brief.verified_support).map_err(|err| {
                fail(format!(
                    "editorial claim {role} failed evidence binding: {err:?}"
                ))
            })?;
            text.push(
                render_editorial_claim(&claim, &brief.locale_code).map_err(|err| {
                    fail(format!("editorial claim {role} failed rendering: {err:?}"))
                })?,
            );
            refs.push(source.support_ref.clone());
            ledger.push(claim);
        }
        if !matches!(role.as_str(), "overview" | "faq")
            && seen.iter().map(String::as_str).collect::<BTreeSet<_>>() != expected
        {
            return Err(fail(format!(
                "incomplete verified support coverage for {role}"
            )));
        }
        section.body_markdown = text.join("\n\n");
        section.support_refs = refs;
    }
    if !claims.is_empty() {
        return Err(fail(
            "unexpected or non-factual section in claim envelope".into(),
        ));
    }
    for block in &mut candidate.content_blocks {
        let section = candidate
            .sections
            .iter()
            .find(|s| s.section_role == block.section_role)
            .ok_or_else(|| fail("content block without section".into()))?;
        block.markdown = section.body_markdown.clone();
        block.support_refs = section.support_refs.clone();
    }
    candidate.claim_ledger = ledger;
    candidate.prompt_version = "seo_editorial_prompt@2".to_string();
    candidate.seo_title = generated.seo_title;
    candidate.seo_meta_description = generated.seo_meta_description;
    candidate.body_markdown = format!(
        "# {}\n\n{}",
        candidate.seo_title,
        candidate
            .sections
            .iter()
            .map(|s| format!("## {}\n\n{}\n", s.heading, s.body_markdown))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let faq = candidate
        .claim_ledger
        .iter()
        .filter(|claim| claim.source_section_key == "faq")
        .filter_map(|claim| claim.editorial_proposal.as_ref())
        .map(|proposal| {
            json!({"@type": "Question", "name": proposal.question,
            "acceptedAnswer": {"@type": "Answer", "text": proposal.text}})
        })
        .collect::<Vec<_>>();
    candidate.faq_json = serde_json::to_string(&faq)
        .map_err(|err| fail(format!("FAQ serialization failed: {err}")))?;
    candidate.schema_markup_json.clear();
    Ok(candidate)
}

#[cfg(test)]
mod typed_claim_overlay_tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        EditorialBrief, SectionTemplateBinding, SeoVerifiedFactSupportState,
    };

    fn input() -> EditorialDraftGenerateInputPayload {
        EditorialDraftGenerateInputPayload {
            request: Some(LlmDraftRequest {
                output_contract: "llm_draft_candidate@2".into(),
                editorial_brief: Some(EditorialBrief {
                    locale_code: "ru".into(),
                    page_node_key: "test-page".into(),
                    section_templates: vec![SectionTemplateBinding {
                        section_role: "timing".into(),
                        heading: "Сроки".into(),
                        required: true,
                        ..Default::default()
                    }],
                    verified_support: vec![SeoVerifiedFactSupportState {
                        support_ref: "rule1".into(),
                        role_type: "timeline_item".into(),
                        freshness_class: "fresh".into(),
                        fragment_text: "The decision normally takes 15 days.".into(),
                        source_url: "https://example.org/official".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn envelope(text: &str, reference: &str) -> String {
        json!({"metadata":{"title":"Сроки рассмотрения заявления","description":"Официальные сведения о порядке и сроках рассмотрения заявлений."},
            "claims":{"timing":[{"support_ref":reference,"text":text}]}}).to_string()
    }

    #[test]
    fn v2_builds_localized_blocks_and_preserves_exact_original() {
        let out = candidate_from_generated_text(
            &input(),
            "test",
            "test",
            &envelope("Обычно решение принимается за 15 дней.", "rule1"),
        )
        .unwrap();
        assert!(out
            .body_markdown
            .contains("Обычно решение принимается за 15 дней."));
        assert!(!out.body_markdown.contains("The decision normally takes"));
        assert_eq!(
            out.claim_ledger[0].fragment_text,
            "The decision normally takes 15 days."
        );
        assert!(out.claim_ledger[0].editorial_proposal.is_some());
        assert!(out.content_blocks[0]
            .markdown
            .contains("https://example.org/official"));
    }

    #[test]
    fn v2_rejects_legacy_unbound_text_unknown_refs_and_numeric_drift() {
        for body in [
            r#"{"sections":{"timing":"Some text."}}"#.to_string(),
            envelope("Обычно решение принимается за 15 дней.", "other"),
            envelope("Обычно решение принимается за 3 дня.", "rule1"),
        ] {
            assert!(candidate_from_generated_text(&input(), "test", "test", &body).is_err());
        }
    }
}
