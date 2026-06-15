use contracts::generated::alegria::temporal::v1::{
    ContentContractValidateInputPayload, ContentContractValidateOutputPayload,
    DraftAssembleInputPayload, DraftAssembleOutputPayload, DraftNormalizeInputPayload,
    DraftNormalizeOutputPayload, DraftQaInputPayload, DraftQaOutputPayload,
    EditorialDraftGenerateInputPayload, EditorialDraftGenerateOutputPayload,
    SeoTraceabilityEntryState,
};
use primitives::errors::DomainError;
use seo_ports::{
    DraftRepository, EditorialGenerationPort, GraphCapabilityPort, SectionTemplateRepository,
    SourceContextRepository,
};

use crate::seo_runtime;
use std::collections::HashSet;

async fn ensure_graph_required_for_phase<R: GraphCapabilityPort>(
    repo: &R,
    phase: &str,
) -> Result<(), DomainError> {
    repo.ensure_graph_contract(phase)
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!(
                "graph capability contract failed for drafting phase `{phase}`: {err}"
            ),
        })
}

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
    R: DraftRepository + SectionTemplateRepository + SourceContextRepository + GraphCapabilityPort,
>(
    repo: &R,
    input: &DraftAssembleInputPayload,
) -> Result<DraftAssembleOutputPayload, DomainError> {
    ensure_graph_required_for_phase(repo, "draft_assemble").await?;
    seo_runtime::truth_admissibility_gate(
        &input.verified_support,
        input
            .page_node
            .as_ref()
            .map(|node| node.page_node_key.as_str())
            .unwrap_or(input.run_id.as_str()),
        "draft_assemble",
    )?;
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
    repo.persist_draft_assemble_output(&input.run_id, &output)
        .await?;
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

pub async fn run_draft_qa<R: DraftRepository + GraphCapabilityPort>(
    repo: &R,
    input: &DraftQaInputPayload,
) -> Result<DraftQaOutputPayload, DomainError>
where
    R: SourceContextRepository,
{
    ensure_graph_required_for_phase(repo, "draft_qa").await?;
    let mut output = seo_steps::draft_qa_step::execute(input);
    append_retrieval_diagnostics(repo, input, &mut output).await?;
    repo.persist_draft_qa_output(input, &output).await?;
    Ok(output)
}

fn trimmed_excerpt(value: &str, limit: usize) -> String {
    value
        .split_whitespace()
        .take(limit)
        .collect::<Vec<_>>()
        .join(" ")
}

fn jaccard_tokens(lhs: &str, rhs: &str) -> f64 {
    let tokenize = |text: &str| {
        text.split_whitespace()
            .map(|token| {
                token
                    .trim_matches(|ch: char| !ch.is_alphanumeric())
                    .to_ascii_lowercase()
            })
            .filter(|token| token.len() > 2)
            .collect::<HashSet<_>>()
    };
    let left = tokenize(lhs);
    let right = tokenize(rhs);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(&right).count() as f64;
    let union = left.union(&right).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn semantic_trace(
    fragment_key: String,
    fragment_text: String,
    traceability_label: &str,
    support_refs: Vec<String>,
) -> SeoTraceabilityEntryState {
    SeoTraceabilityEntryState {
        fragment_key,
        fragment_text,
        fragment_kind: "diagnostic".to_string(),
        traceability_label: traceability_label.to_string(),
        support_refs,
        validation_verdict: "diagnostic".to_string(),
    }
}

async fn append_retrieval_diagnostics<R: SourceContextRepository>(
    repo: &R,
    input: &DraftQaInputPayload,
    output: &mut DraftQaOutputPayload,
) -> Result<(), DomainError> {
    let Some(draft) = input.draft.as_ref() else {
        return Ok(());
    };

    let mut diagnostics = Vec::<SeoTraceabilityEntryState>::new();

    // Unsupported-claim retrieval diagnostics.
    for entry in draft.traceability_entries.iter().filter(|entry| {
        entry.traceability_label == "unsupported_factual_fragment"
            || (entry.fragment_kind == "factual" && entry.support_refs.is_empty())
    }) {
        if entry.fragment_text.trim().is_empty() {
            continue;
        }
        let query = trimmed_excerpt(&entry.fragment_text, 48);
        let neighbors = repo.load_source_context_chunks(&query, 3).await?;
        if neighbors.is_empty() {
            continue;
        }
        let refs = neighbors
            .iter()
            .map(|chunk| format!("qdrant://{}/{}", chunk.section_type, chunk.chunk_key))
            .collect::<Vec<_>>();
        diagnostics.push(semantic_trace(
            format!("diag:unsupported:{}", entry.fragment_key),
            entry.fragment_text.clone(),
            "qa_unsupported_claim_retrieval",
            refs,
        ));
    }

    // Semantic duplication diagnostics between assembled sections.
    let sections = &draft.sections;
    for i in 0..sections.len() {
        for j in (i + 1)..sections.len() {
            let left = sections[i].body_markdown.trim();
            let right = sections[j].body_markdown.trim();
            if left.is_empty() || right.is_empty() {
                continue;
            }
            let similarity = jaccard_tokens(left, right);
            if similarity < 0.88 {
                continue;
            }
            diagnostics.push(semantic_trace(
                format!(
                    "diag:dup:{}:{}",
                    sections[i].section_role, sections[j].section_role
                ),
                format!(
                    "{} <-> {}",
                    sections[i].section_role, sections[j].section_role
                ),
                "qa_semantic_duplication",
                Vec::new(),
            ));
        }
    }

    // Plagiarism/boilerplate leakage diagnostics.
    if !draft.body_markdown.trim().is_empty() {
        let query = trimmed_excerpt(&draft.body_markdown, 64);
        let neighbors = repo.load_source_context_chunks(&query, 4).await?;
        for chunk in neighbors {
            let excerpt = trimmed_excerpt(&chunk.content_md, 40);
            if excerpt.is_empty() {
                continue;
            }
            let similarity = jaccard_tokens(&draft.body_markdown, &chunk.content_md);
            if similarity < 0.72 {
                continue;
            }
            diagnostics.push(semantic_trace(
                format!("diag:leakage:{}", chunk.chunk_key),
                excerpt,
                "qa_boilerplate_or_plagiarism_risk",
                vec![format!(
                    "qdrant://{}/{}",
                    chunk.section_type, chunk.chunk_key
                )],
            ));
        }
    }

    if diagnostics.is_empty() {
        return Ok(());
    }

    let mut seen = output
        .traceability_entries
        .iter()
        .map(|entry| entry.fragment_key.clone())
        .collect::<HashSet<_>>();
    for diagnostic in diagnostics {
        if seen.insert(diagnostic.fragment_key.clone()) {
            output.traceability_entries.push(diagnostic);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use contracts::generated::alegria::temporal::v1::{
        DraftState, EditorialBrief, EditorialDraftGenerateOutputPayload, LlmDraftCandidate,
        LlmDraftRequest, PageBlueprintState, PageNodeState, SectionTemplateBinding,
        SeoDraftSectionState, SeoVerifiedFactSupportState, SourceContextChunkState,
    };
    use seo_ports::{
        DraftRepository, EditorialGenerationPort, SectionTemplateRepository,
        SourceContextRepository, GraphCapabilityPort,
    };

    #[derive(Default)]
    struct FakeDraftRepo;

    #[async_trait]
    impl GraphCapabilityPort for FakeDraftRepo {
        async fn ensure_graph_contract(&self, _context_key: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

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
            _run_id: &str,
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
                verified_support: vec![SeoVerifiedFactSupportState {
                    fragment_text: "Passport required".to_string(),
                    support_ref: "rule:passport".to_string(),
                    role_type: "document_required".to_string(),
                    source_label: "Consulate".to_string(),
                    source_tier: "official".to_string(),
                    freshness_class: "watch".to_string(),
                    observed_at: String::new(),
                    valid_until: String::new(),
                }],
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

    struct DiagnosticDraftRepo;

    #[async_trait]
    impl GraphCapabilityPort for DiagnosticDraftRepo {
        async fn ensure_graph_contract(&self, _context_key: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[async_trait]
    impl DraftRepository for DiagnosticDraftRepo {
        async fn persist_draft_assemble_output(
            &self,
            _run_id: &str,
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
    impl SourceContextRepository for DiagnosticDraftRepo {
        async fn load_source_context_chunks(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SourceContextChunkState>, DomainError> {
            Ok(vec![SourceContextChunkState {
                chunk_key: "verified_rules_4:rule-passport".to_string(),
                source_url: "https://example.com/rule".to_string(),
                source_domain: "example.com".to_string(),
                heading_path: "passport".to_string(),
                section_type: "verified_rules_4".to_string(),
                content_md: "Passport copy is required for tourist visa applications".to_string(),
                retrieval_score: "0.95".to_string(),
                usage_policy: "verified_fact_support".to_string(),
            }])
        }
    }

    #[tokio::test]
    async fn drafting_qa_appends_semantic_diagnostics_trace_entries() {
        let repo = DiagnosticDraftRepo;
        let output = run_draft_qa(
            &repo,
            &DraftQaInputPayload {
                run_id: "run-1".to_string(),
                draft: Some(DraftState {
                    body_markdown:
                        "Passport copy is required for tourist visa applications. Passport copy is required for tourist visa applications.".to_string(),
                    sections: vec![
                        SeoDraftSectionState {
                            section_role: "overview".to_string(),
                            body_markdown:
                                "Passport copy is required for tourist visa applications."
                                    .to_string(),
                            required: true,
                            ..Default::default()
                        },
                        SeoDraftSectionState {
                            section_role: "documents".to_string(),
                            body_markdown:
                                "Passport copy is required for tourist visa applications."
                                    .to_string(),
                            required: true,
                            ..Default::default()
                        },
                    ],
                    traceability_entries: vec![SeoTraceabilityEntryState {
                        fragment_key: "frag-1".to_string(),
                        fragment_text: "Passport copy is required".to_string(),
                        fragment_kind: "factual".to_string(),
                        traceability_label: "unsupported_factual_fragment".to_string(),
                        support_refs: Vec::new(),
                        validation_verdict: "blocked".to_string(),
                    }],
                    ..Default::default()
                }),
                supported_fragments: Vec::new(),
                required_links: Vec::new(),
            },
        )
        .await
        .unwrap();
        let labels = output
            .traceability_entries
            .iter()
            .map(|entry| entry.traceability_label.as_str())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"qa_unsupported_claim_retrieval"));
        assert!(labels.contains(&"qa_semantic_duplication"));
        assert!(labels.contains(&"qa_boilerplate_or_plagiarism_risk"));
    }
}
