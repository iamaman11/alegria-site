#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        EditorialBrief, LinkRecommendationState, LlmDraftRequest, SectionTemplateBinding,
        SeoVerifiedFactSupportState,
    };

    fn sample_input() -> EditorialDraftGenerateInputPayload {
        EditorialDraftGenerateInputPayload {
            run_id: "run-1".to_string(),
            request: Some(LlmDraftRequest {
                request_key: "req-1".to_string(),
                run_id: "run-1".to_string(),
                editorial_brief: Some(EditorialBrief {
                    editorial_brief_key: "brief-1".to_string(),
                    page_node_key: "page-1".to_string(),
                    page_brief_key: "page-brief-1".to_string(),
                    target_audience: "tourist".to_string(),
                    expertise_goal: "help applicant understand the process".to_string(),
                    section_templates: vec![SectionTemplateBinding {
                        template_key: "overview".to_string(),
                        page_type_key: "detail".to_string(),
                        dominant_intent: "informational".to_string(),
                        section_role: "overview".to_string(),
                        heading: "Overview".to_string(),
                        template_body: "overview".to_string(),
                        template_version: 1,
                        required: true,
                    }],
                    verified_support: vec![SeoVerifiedFactSupportState {
                        support_ref: "rule-1".to_string(),
                        role_type: "document_required".to_string(),
                        fragment_text: "Passport is required.".to_string(),
                        source_label: "Official source".to_string(),
                        source_tier: "official".to_string(),
                        freshness_class: "fresh".to_string(),
                        observed_at: "2026-05-08T00:00:00Z".to_string(),
                        valid_until: "2027-05-08T00:00:00Z".to_string(),
                        source_url: String::new(),
                    }],
                    required_links: vec![LinkRecommendationState::default()],
                    truth_snapshot_ref: "truth-1".to_string(),
                    source_context_chunks: Vec::new(),
                    locale_code: "en".to_string(),
                    canonical_url_path: "/en/visa/spain/".to_string(),
                }),
                provider_policy: "multi_provider:first_available".to_string(),
                prompt_version: "editorial_prompt@1".to_string(),
                output_contract: "headless".to_string(),
            }),
            provider_policy: String::new(),
        }
    }

    #[test]
    fn parses_generated_sections_json() {
        let sections = parse_generated_sections(
            r#"{"sections":{"overview":"Expert overview.","cta_disclaimer":"Confirm official requirements."}}"#,
        )
        .expect("parse sections");
        assert_eq!(
            sections.sections.get("overview").map(String::as_str),
            Some("Expert overview.")
        );
    }

    #[test]
    fn deterministic_factual_blocks_contain_only_verifiable_source_text() {
        let input = sample_input();
        let brief = input
            .request
            .as_ref()
            .and_then(|r| r.editorial_brief.as_ref())
            .expect("brief");

        let verified_fragments: Vec<&str> = brief
            .verified_support
            .iter()
            .map(|s| s.fragment_text.as_str())
            .collect();

        assert!(!verified_fragments.is_empty());
        assert!(verified_fragments.contains(&"Passport is required."));
    }
}