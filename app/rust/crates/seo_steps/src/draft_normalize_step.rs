use contracts::generated::alegria::temporal::v1::{
    DraftAssembleInputPayload, DraftNormalizeInputPayload, DraftNormalizeOutputPayload,
};

pub fn execute(input: &DraftNormalizeInputPayload) -> DraftNormalizeOutputPayload {
    let assembled = crate::draft_assemble_step::execute(&DraftAssembleInputPayload {
        run_id: input.run_id.clone(),
        page_node: input.page_node.clone(),
        page_blueprint: input.page_blueprint.clone(),
        factual_fragments: input.factual_fragments.clone(),
        verified_support: input.verified_support.clone(),
        required_links: input.required_links.clone(),
        section_templates: input.section_templates.clone(),
        llm_candidate: input.llm_candidate.clone(),
        source_context_chunks: Vec::new(),
    });
    let page_brief = assembled.page_brief.clone();
    let draft = assembled.draft.unwrap_or_default();
    let mut content_block_plan = input.content_block_plan.clone().unwrap_or_default();
    if content_block_plan.content_block_plan_key.trim().is_empty() {
        content_block_plan = assembled.content_block_plan.unwrap_or_default();
    }
    DraftNormalizeOutputPayload {
        draft: Some(draft),
        content_block_plan: Some(content_block_plan),
        page_brief,
    }
}

#[cfg(test)]
mod localized_editorial_preflight_tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        LlmDraftCandidate, PageBlueprintState, PageNodeState, SeoVerifiedFactSupportState,
    };

    #[test]
    fn english_url_is_not_hardcoded_into_russian_page_and_metadata_survives_normalize() {
        let quote = "A Schengen visa application form is required.";
        let input = DraftNormalizeInputPayload {
            run_id: "run-ru".to_string(),
            page_node: Some(PageNodeState {
                page_node_key: "page-ru".to_string(),
                canonical_slug: "spain-visa".to_string(),
                canonical_url_path: "/ru/visa/spain/".to_string(),
                ..Default::default()
            }),
            page_blueprint: Some(PageBlueprintState {
                blueprint_key: "blueprint-1".to_string(),
                required_sections: vec!["overview".to_string(), "documents".to_string()],
                ..Default::default()
            }),
            verified_support: vec![SeoVerifiedFactSupportState {
                role_type: "form_required".to_string(),
                support_ref: "rule-form".to_string(),
                fragment_text: quote.to_string(),
                freshness_class: "fresh".to_string(),
                source_url: "https://example.org/government-source".to_string(),
                ..Default::default()
            }],
            llm_candidate: Some(LlmDraftCandidate {
                status: "ready".to_string(),
                body_markdown: "Evidence-backed draft".to_string(),
                seo_title: "Шенгенская виза в Испанию: документы и сроки".to_string(),
                seo_meta_description: "Проверенные сведения о подаче документов и сроках по официальным источникам для заявителей.".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let output = execute(&input);
        let brief = output.page_brief.expect("normalized brief");
        assert_eq!(brief.title, "Шенгенская виза в Испанию: документы и сроки");
        assert!(brief.meta_description.starts_with("Проверенные сведения"));
        let draft = output.draft.expect("normalized draft");
        assert_eq!(
            draft
                .sections
                .iter()
                .find(|s| s.section_role == "overview")
                .map(|s| s.heading.as_str()),
            Some("Обзор")
        );
        assert_eq!(
            draft
                .sections
                .iter()
                .find(|s| s.section_role == "documents")
                .map(|s| s.heading.as_str()),
            Some("Документы")
        );
        assert!(draft
            .sections
            .iter()
            .any(|section| section.body_markdown.contains(quote)));
    }

    #[test]
    fn default_english_page_retains_generic_headings_without_editorial_metadata() {
        let input = DraftNormalizeInputPayload {
            run_id: "run-en".to_string(),
            page_node: Some(PageNodeState {
                canonical_slug: "plant-care".to_string(),
                canonical_url_path: "/en/plant-care/".to_string(),
                ..Default::default()
            }),
            page_blueprint: Some(PageBlueprintState {
                required_sections: vec!["overview".to_string()],
                ..Default::default()
            }),
            llm_candidate: Some(LlmDraftCandidate {
                status: "ready".to_string(),
                body_markdown: "Source-backed draft".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let output = execute(&input);
        let brief = output.page_brief.expect("fallback English brief");
        assert_eq!(brief.title, "Plant Care");
    }
}
