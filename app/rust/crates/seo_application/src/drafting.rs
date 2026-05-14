use contracts::generated::alegria::temporal::v1::{
    ContentContractValidateInputPayload, ContentContractValidateOutputPayload,
    DraftAssembleInputPayload, DraftAssembleOutputPayload, DraftNormalizeInputPayload,
    DraftNormalizeOutputPayload, DraftQaInputPayload, DraftQaOutputPayload,
    EditorialDraftGenerateInputPayload, EditorialDraftGenerateOutputPayload,
};
use primitives::errors::DomainError;
use seo_ports::{
    DraftRepository, EditorialGenerationPort, SectionTemplateRepository, SourceContextRepository,
};

fn draft_source_context_query(input: &DraftAssembleInputPayload) -> String {
    let page_node = input.page_node.clone().unwrap_or_default();
    let page_blueprint = input.page_blueprint.clone().unwrap_or_default();
    format!(
        "{} {} {} {} {}",
        page_node.canonical_url_path,
        page_node.canonical_slug,
        page_blueprint.page_type_key,
        page_blueprint.dominant_intent,
        input
            .verified_support
            .iter()
            .take(8)
            .map(|support| support.fragment_text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    )
}

pub async fn run_draft_assemble<
    R: DraftRepository + SectionTemplateRepository + SourceContextRepository,
>(
    repo: &R,
    input: &DraftAssembleInputPayload,
) -> Result<DraftAssembleOutputPayload, DomainError> {
    let mut enriched_input = input.clone();
    if enriched_input.section_templates.is_empty() {
        if let Some(blueprint) = enriched_input.page_blueprint.as_ref() {
            enriched_input.section_templates = repo
                .load_section_templates(&blueprint.page_type_key, &blueprint.dominant_intent)
                .await?;
        }
    }
    if enriched_input.source_context_chunks.is_empty() {
        enriched_input.source_context_chunks = repo
            .load_source_context_chunks(&draft_source_context_query(&enriched_input), 12)
            .await?;
    }
    let output = seo_steps::draft_assemble_step::execute(&enriched_input);
    repo.persist_draft_assemble_output(&output).await?;
    Ok(output)
}

pub async fn run_editorial_draft_generate<P: EditorialGenerationPort>(
    port: &P,
    input: &EditorialDraftGenerateInputPayload,
) -> Result<EditorialDraftGenerateOutputPayload, DomainError> {
    port.generate_editorial_draft(input).await
}

pub async fn run_draft_normalize<R: DraftRepository>(
    repo: &R,
    input: &DraftNormalizeInputPayload,
) -> Result<DraftNormalizeOutputPayload, DomainError> {
    let output = seo_steps::draft_normalize_step::execute(input);
    repo.persist_draft_normalize_output(input, &output).await?;
    Ok(output)
}

pub async fn run_content_contract_validate<R: DraftRepository>(
    repo: &R,
    input: &ContentContractValidateInputPayload,
) -> Result<ContentContractValidateOutputPayload, DomainError> {
    let output = seo_steps::content_contract_validate_step::execute(input);
    repo.persist_content_contract_validate_output(input, &output)
        .await?;
    Ok(output)
}

pub async fn run_draft_qa<R: DraftRepository>(
    repo: &R,
    input: &DraftQaInputPayload,
) -> Result<DraftQaOutputPayload, DomainError> {
    let output = seo_steps::draft_qa_step::execute(input);
    repo.persist_draft_qa_output(input, &output).await?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use contracts::generated::alegria::temporal::v1::{
        DraftState, EditorialBrief, EditorialDraftGenerateOutputPayload, LlmDraftCandidate,
        LlmDraftRequest, PageBlueprintState, PageNodeState, SectionTemplateBinding,
    };
    use seo_ports::{
        DraftRepository, EditorialGenerationPort, SectionTemplateRepository,
        SourceContextRepository,
    };

    #[derive(Default)]
    struct FakeDraftRepo;

    #[async_trait]
    impl SectionTemplateRepository for FakeDraftRepo {
        async fn load_section_templates(
            &self,
            page_type_key: &str,
            dominant_intent: &str,
        ) -> Result<Vec<SectionTemplateBinding>, DomainError> {
            Ok(vec![SectionTemplateBinding {
                template_key: "tmpl:overview".to_string(),
                page_type_key: page_type_key.to_string(),
                dominant_intent: dominant_intent.to_string(),
                section_role: "overview".to_string(),
                heading: "Overview".to_string(),
                template_body: "overview template".to_string(),
                template_version: 1,
                required: true,
            }])
        }
    }

    #[async_trait]
    impl DraftRepository for FakeDraftRepo {
        async fn persist_draft_assemble_output(
            &self,
            _output: &DraftAssembleOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_draft_normalize_output(
            &self,
            _input: &DraftNormalizeInputPayload,
            _output: &DraftNormalizeOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_content_contract_validate_output(
            &self,
            _input: &ContentContractValidateInputPayload,
            _output: &ContentContractValidateOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }

        async fn persist_draft_qa_output(
            &self,
            _input: &DraftQaInputPayload,
            _output: &DraftQaOutputPayload,
        ) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[async_trait]
    impl SourceContextRepository for FakeDraftRepo {
        async fn load_source_context_chunks(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<
            Vec<contracts::generated::alegria::temporal::v1::SourceContextChunkState>,
            DomainError,
        > {
            Ok(Vec::new())
        }
    }

    struct FakeEditorialPort;

    #[async_trait]
    impl EditorialGenerationPort for FakeEditorialPort {
        async fn generate_editorial_draft(
            &self,
            input: &EditorialDraftGenerateInputPayload,
        ) -> Result<EditorialDraftGenerateOutputPayload, DomainError> {
            Ok(EditorialDraftGenerateOutputPayload {
                provider_key: "fake".to_string(),
                model_key: "fake-model".to_string(),
                status: "selected".to_string(),
                candidate: Some(LlmDraftCandidate {
                    candidate_key: "cand-1".to_string(),
                    request_key: input
                        .request
                        .as_ref()
                        .map(|request| request.request_key.clone())
                        .unwrap_or_default(),
                    provider_key: "fake".to_string(),
                    model_key: "fake-model".to_string(),
                    prompt_version: "prompt@1".to_string(),
                    body_markdown: "draft".to_string(),
                    sections: Vec::new(),
                    claim_ledger: Vec::new(),
                    content_blocks: Vec::new(),
                    faq_json: "[]".to_string(),
                    schema_markup_json: "{}".to_string(),
                    status: "selected".to_string(),
                }),
            })
        }
    }

    #[tokio::test]
    async fn drafting_assemble_loads_templates_without_sql_adapter() {
        let repo = FakeDraftRepo;
        let output = run_draft_assemble(
            &repo,
            &DraftAssembleInputPayload {
                run_id: "run-1".to_string(),
                page_node: Some(PageNodeState {
                    page_node_key: "page-1".to_string(),
                    canonical_slug: "spain-visa".to_string(),
                    ..Default::default()
                }),
                page_blueprint: Some(PageBlueprintState {
                    blueprint_key: "bp-1".to_string(),
                    page_type_key: "detail".to_string(),
                    dominant_intent: "requirements".to_string(),
                    ..Default::default()
                }),
                verified_support: Vec::new(),
                required_links: Vec::new(),
                section_templates: Vec::new(),
                factual_fragments: Vec::new(),
                source_context_chunks: Vec::new(),
                llm_candidate: None,
            },
        )
        .await
        .unwrap();
        assert!(output.content_block_plan.is_some());
    }

    #[tokio::test]
    async fn drafting_editorial_generation_uses_port() {
        let port = FakeEditorialPort;
        let output = run_editorial_draft_generate(
            &port,
            &EditorialDraftGenerateInputPayload {
                run_id: "run-1".to_string(),
                request: Some(LlmDraftRequest {
                    request_key: "req-1".to_string(),
                    run_id: "run-1".to_string(),
                    editorial_brief: Some(EditorialBrief {
                        editorial_brief_key: "brief-1".to_string(),
                        page_node_key: "page-1".to_string(),
                        page_brief_key: "page-brief-1".to_string(),
                        target_audience: "tourist".to_string(),
                        expertise_goal: "goal".to_string(),
                        section_templates: Vec::new(),
                        verified_support: Vec::new(),
                        required_links: Vec::new(),
                        truth_snapshot_ref: "truth-1".to_string(),
                        source_context_chunks: Vec::new(),
                    }),
                    provider_policy: "multi_provider:first_available".to_string(),
                    prompt_version: "prompt@1".to_string(),
                    output_contract: "json".to_string(),
                }),
                provider_policy: "multi_provider:first_available".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(output.provider_key, "fake");
        assert!(output.candidate.is_some());
    }

    #[tokio::test]
    async fn drafting_qa_persists_without_sql_adapter() {
        let repo = FakeDraftRepo;
        let output = run_draft_qa(
            &repo,
            &DraftQaInputPayload {
                run_id: "run-1".to_string(),
                draft: Some(DraftState::default()),
                supported_fragments: Vec::new(),
                required_links: Vec::new(),
            },
        )
        .await
        .unwrap();
        assert!(!output.verdict.is_empty());
    }
}
