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
                    }],
                    required_links: vec![LinkRecommendationState::default()],
                    truth_snapshot_ref: "truth-1".to_string(),
                    source_context_chunks: Vec::new(),
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

    #[tokio::test]
    async fn deterministic_fallback_is_used_when_no_provider_is_configured() {
        // Ensure no external provider is considered configured for this fallback test
        std::env::set_var("VERTEX_GEMINI_ENABLED", "false");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("ANTHROPIC_API_KEY");
        std::env::remove_var("GEMINI_API_KEY");
        std::env::remove_var("GOOGLE_API_KEY");
        std::env::remove_var("SEO_LLM_LOCAL_ENDPOINT");

        let input = sample_input();
        let output = generate_editorial_draft(&input)
            .await
            .expect("fallback output");
        assert_eq!(output.provider_key, "deterministic_fixture");
        assert!(output.candidate.is_some());
    }
}
