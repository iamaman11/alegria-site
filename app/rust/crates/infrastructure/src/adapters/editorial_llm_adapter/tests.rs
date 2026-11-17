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
        let mut input = sample_input();
        let brief = input
            .request
            .as_mut()
            .and_then(|request| request.editorial_brief.as_mut())
            .expect("sample editorial brief");
        brief.section_templates.push(SectionTemplateBinding {
            template_key: "fees".to_string(),
            page_type_key: "detail".to_string(),
            dominant_intent: "informational".to_string(),
            section_role: "fees".to_string(),
            heading: "Fees".to_string(),
            template_body: String::new(),
            template_version: 1,
            required: true,
        });
        brief.verified_support.push(SeoVerifiedFactSupportState {
            support_ref: "rule-fee".to_string(),
            role_type: "fee_item".to_string(),
            fragment_text: "An official source specifies the application fee.".to_string(),
            source_label: "Official source".to_string(),
            source_tier: "official".to_string(),
            freshness_class: "fresh".to_string(),
            ..Default::default()
        });

        let candidate = build_deterministic_candidate(
            &input,
            "deterministic_fixture",
            "source_backed_template@1",
        )
        .expect("deterministic candidate");
        let fee = candidate
            .content_blocks
            .iter()
            .find(|block| block.section_role == "fees")
            .expect("fees content block");
        assert_eq!(
            fee.markdown.trim(),
            "- An official source specifies the application fee."
        );
        assert_eq!(fee.support_refs, vec!["rule-fee"]);
        assert!(!candidate.content_blocks.iter().any(|block| block
            .markdown
            .contains("Verified source-backed structured content")));
        assert!(candidate
            .claim_ledger
            .iter()
            .any(|claim| claim.support_refs == ["rule-fee"]
                && claim.fragment_text == "An official source specifies the application fee."));
    }

    #[test]
    fn russian_source_link_block_contains_only_real_verified_source_urls() {
        let mut input = sample_input();
        let brief = input
            .request
            .as_mut()
            .unwrap()
            .editorial_brief
            .as_mut()
            .unwrap();
        brief.locale_code = "ru".to_string();
        brief.canonical_url_path = "/ru/visa/spain/".to_string();
        brief.required_links.clear();
        brief.section_templates.push(SectionTemplateBinding {
            section_role: "related_pages".to_string(),
            heading: "Источники".to_string(),
            required: true,
            ..Default::default()
        });
        brief.verified_support[0].source_url =
            "https://home-affairs.ec.europa.eu/policies/schengen/visa-policy/applying-schengen-visa_en".to_string();
        let candidate = build_deterministic_candidate(
            &input,
            "deterministic_fixture",
            "source_backed_template@1",
        )
        .unwrap();
        let related = candidate
            .content_blocks
            .iter()
            .find(|block| block.section_role == "related_pages")
            .unwrap();
        assert!(related.markdown.contains(
            "https://home-affairs.ec.europa.eu/policies/schengen/visa-policy/applying-schengen-visa_en"
        ));
        assert!(!related.markdown.contains("No mandatory related pages"));
        assert!(!related.markdown.contains(" -> "));
    }

    #[test]
    fn russian_fee_block_preserves_exact_quote_and_official_source_url() {
        let mut input = sample_input();
        let brief = input
            .request
            .as_mut()
            .unwrap()
            .editorial_brief
            .as_mut()
            .unwrap();
        brief.locale_code = "ru".to_string();
        brief.canonical_url_path = "/ru/visa/spain/".to_string();
        brief.required_links.clear();
        brief.section_templates.push(SectionTemplateBinding {
            section_role: "fees".to_string(),
            required: true,
            ..Default::default()
        });
        brief.verified_support.push(SeoVerifiedFactSupportState {
            role_type: "fee_item".to_string(),
            fragment_text: "For a specific category, the visa fee is EUR 35.".to_string(),
            source_url: "https://eur-lex.europa.eu/legal-content/EN/TXT/".to_string(),
            support_ref: "fee-confirmed".to_string(),
            ..Default::default()
        });
        let candidate = build_deterministic_candidate(
            &input,
            "deterministic_fixture",
            "source_backed_template@1",
        )
        .unwrap();
        let fee = candidate
            .content_blocks
            .iter()
            .find(|block| block.section_role == "fees")
            .unwrap();
        assert!(fee
            .markdown
            .contains("For a specific category, the visa fee is EUR 35."));
        assert!(fee
            .markdown
            .contains("https://eur-lex.europa.eu/legal-content/EN/TXT/"));
        assert!(fee.markdown.contains("Консульский сбор"));
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
