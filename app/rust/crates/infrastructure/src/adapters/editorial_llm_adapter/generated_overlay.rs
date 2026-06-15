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
    let mut body_parts = vec!["# Expert travel visa guide".to_string(), String::new()];
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
    Ok(overlay_generated_sections(
        deterministic,
        request,
        generated,
    ))
}
