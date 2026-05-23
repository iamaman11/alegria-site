use contracts::generated::alegria::temporal::v1::{
    CrawlSourcesInputPayload, RawKnowledgeIngestionInputPayload, SeoSiteBuildInputPayload,
    SeoVerifiedFactSupportState, SerpIngestInputPayload,
};
use infrastructure::adapters::temporalio_sdk_adapter::{
    workflow, workflow_methods, SyncWorkflowContext, WorkerOptions, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};
use seo_ports::VerifiedSupportBundleRequest;

use crate::activities::operations::SemanticSectionSampleInput;
use crate::activities::AlegriaActivities;
use crate::metrics;

use super::runtime::{db_opts, BasicWorkflowStatus};

#[workflow]
#[derive(Default)]
struct ExpertDecomposedExtractionWorkflow {
    paused: bool,
    phase: String,
}

pub(crate) fn register(opts: &mut WorkerOptions) {
    opts.register_workflow::<ExpertDecomposedExtractionWorkflow>();
}

fn support_request(run_id: &str, site_input: &SeoSiteBuildInputPayload) -> WorkflowResult<String> {
    Ok(serde_json::to_string(&VerifiedSupportBundleRequest {
        run_id: run_id.to_string(),
        context_key: site_input.context_key.clone(),
        scope_signature: site_input
            .scope
            .as_ref()
            .map(|value| value.scope_signature.clone())
            .unwrap_or_default(),
        applicant_profile: site_input
            .scope
            .as_ref()
            .map(|value| value.applicant_profile.clone())
            .unwrap_or_default(),
    })
    .map_err(anyhow::Error::from)?)
}

#[workflow_methods]
impl ExpertDecomposedExtractionWorkflow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        metrics::global()
            .workflow_starts_total
            .with_label_values(&["ExpertDecomposedExtractionWorkflow"])
            .inc();

        let run_id = ctx.workflow_initial_info().workflow_id.clone();
        ctx.state_mut(|s| s.phase = "load_seo_site_build_input".to_string());
        let site_input: SeoSiteBuildInputPayload = ctx
            .start_activity(
                AlegriaActivities::load_seo_site_build_input,
                run_id.clone(),
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        let support_bundle_request = support_request(&run_id, &site_input)?;
        ctx.state_mut(|s| s.phase = "load_verified_support_bundle.initial".to_string());
        let mut support_bundle: Vec<SeoVerifiedFactSupportState> = ctx
            .start_activity(
                AlegriaActivities::load_verified_support_bundle,
                support_bundle_request.clone(),
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "serp_ingest".to_string());
        let ingest = ctx
            .start_activity(
                AlegriaActivities::run_serp_ingest_step,
                SerpIngestInputPayload {
                    run_id: run_id.clone(),
                    query_batch_key: site_input.query_batch_key.clone(),
                    scope: site_input.scope.clone(),
                    queries: site_input.queries.clone(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "crawl_sources".to_string());
        let crawled = ctx
            .start_activity(
                AlegriaActivities::run_crawl_sources_step,
                CrawlSourcesInputPayload {
                    run_id: run_id.clone(),
                    query_batch_key: ingest.query_batch_key.clone(),
                    limit: 25,
                    emit_qdrant: true,
                },
                db_opts(120),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "load_semantic_section_sample".to_string());
        let section = ctx
            .start_activity(
                AlegriaActivities::load_semantic_section_sample_step,
                SemanticSectionSampleInput {
                    run_id: run_id.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "page_utility_classifier".to_string());
        let page_utility = ctx
            .start_activity(
                AlegriaActivities::run_page_utility_classifier_step,
                seo_steps::page_utility_classifier_step::PageUtilityClassifierInput {
                    url: section.source_url.clone(),
                    title: section.heading_path.clone(),
                    raw_text: section.raw_text.clone(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "dom_block_relevance_filter".to_string());
        let dom = ctx
            .start_activity(
                AlegriaActivities::run_dom_block_relevance_step,
                vec![seo_steps::dom_block_relevance_step::DomBlockInput {
                    dom_block_id: format!("section:{}", section.section_id),
                    block_role: seo_steps::dom_block_relevance_step::BlockRole::ContentMain,
                    text: section.raw_text.clone(),
                }],
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        let mut semantic_skipped = false;
        let mut semantic_summary = "semantic_skipped".to_string();
        if page_utility.allow_procedural_extraction
            && dom.blocks.iter().any(|block| block.allow_extraction)
        {
            ctx.state_mut(|s| s.phase = "layer_router".to_string());
            let layer = ctx
                .start_activity(
                    AlegriaActivities::run_layer_router_step,
                    seo_steps::layer_router_step::LayerRouterInput {
                        section_id: section.section_id.clone(),
                        heading_text: section.heading_path.clone(),
                        raw_text: section.raw_text.clone(),
                        source_tier: section.source_domain.clone(),
                        block_type: section.section_type.clone(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| s.phase = "entity_span_detection".to_string());
            let spans = ctx
                .start_activity(
                    AlegriaActivities::run_entity_span_detection_step,
                    seo_steps::entity_span_detection_step::EntitySpanInput {
                        section_id: section.section_id.clone(),
                        raw_text: section.raw_text.clone(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| s.phase = "canonical_mapping".to_string());
            let mapping = ctx
                .start_activity(
                    AlegriaActivities::run_canonical_mapping_step,
                    seo_steps::canonical_mapping_step::CanonicalMappingInput {
                        section_id: section.section_id.clone(),
                        mentions: spans
                            .mentions
                            .iter()
                            .map(
                                |mention| seo_steps::canonical_mapping_step::MentionForMapping {
                                    raw_text: mention.raw_text.clone(),
                                    entity_type: mention.entity_type.clone(),
                                },
                            )
                            .collect(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| s.phase = "procedural_extraction".to_string());
            let procedural = ctx
                .start_activity(
                    AlegriaActivities::run_procedural_extraction_step,
                    seo_steps::procedural_extraction_step::ProceduralExtractionInput {
                        section_id: section.section_id.clone(),
                        raw_text: section.raw_text.clone(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| s.phase = "operational_extraction".to_string());
            let operational = ctx
                .start_activity(
                    AlegriaActivities::run_operational_extraction_step,
                    seo_steps::operational_extraction_step::OperationalExtractionInput {
                        section_id: section.section_id.clone(),
                        raw_text: section.raw_text.clone(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| s.phase = "editorial_extraction".to_string());
            let editorial = ctx
                .start_activity(
                    AlegriaActivities::run_editorial_extraction_step,
                    seo_steps::editorial_extraction_step::EditorialExtractionInput {
                        section_id: section.section_id.clone(),
                        raw_text: section.raw_text.clone(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| s.phase = "completeness_judge".to_string());
            let completeness = ctx
                .start_activity(
                    AlegriaActivities::run_completeness_judge_step,
                    seo_steps::completeness_judge_step::CompletenessJudgeInput {
                        section_id: section.section_id.clone(),
                        raw_text: section.raw_text.clone(),
                        source_numeric_tokens: spans
                            .mentions
                            .iter()
                            .filter(|mention| mention.has_numeric)
                            .map(|mention| mention.raw_text.clone())
                            .collect(),
                        extracted_numeric_tokens: procedural
                            .rules
                            .iter()
                            .flat_map(|rule| rule.numeric_tokens.clone())
                            .collect(),
                        extracted_rule_keys: procedural
                            .rules
                            .iter()
                            .map(|rule| rule.rule_key.clone())
                            .collect(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| s.phase = "triple_builder".to_string());
            let triples = ctx
                .start_activity(
                    AlegriaActivities::run_triple_builder_step,
                    seo_steps::triple_builder_step::TripleBuilderInput {
                        section_id: section.section_id.clone(),
                        procedural_rules: procedural
                            .rules
                            .iter()
                            .map(
                                |rule| seo_steps::triple_builder_step::ProceduralRuleForTriple {
                                    rule_key: rule.rule_key.clone(),
                                    role_type: rule.role_type.clone(),
                                },
                            )
                            .collect(),
                        operational_entities: operational
                            .entities
                            .iter()
                            .map(|entity| {
                                seo_steps::triple_builder_step::OperationalEntityForTriple {
                                    entity_kind: entity.entity_kind.clone(),
                                    value: entity.value.clone(),
                                }
                            })
                            .collect(),
                        editorial_topics: editorial
                            .topics
                            .iter()
                            .map(
                                |topic| seo_steps::triple_builder_step::EditorialTopicForTriple {
                                    topic_type: topic.topic_type.clone(),
                                    topic_key_candidate: topic.topic_key_candidate.clone(),
                                },
                            )
                            .collect(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| s.phase = "contradiction_gate".to_string());
            let contradiction = ctx
                .start_activity(
                    AlegriaActivities::run_contradiction_gate_step,
                    seo_steps::contradiction_gate_step::ContradictionGateInput {
                        run_id: run_id.clone(),
                        facts: procedural
                            .rules
                            .iter()
                            .map(|rule| seo_steps::contradiction_gate_step::FactAssertion {
                                subject_key: rule.rule_key.clone(),
                                predicate_key: format!("{:?}", rule.role_type),
                                value_normalized: if rule.numeric_tokens.is_empty() {
                                    rule.rule_key.clone()
                                } else {
                                    rule.numeric_tokens.join("|")
                                },
                                source_key: Some(section.source_url.clone()),
                                confidence: rule.confidence,
                            })
                            .collect(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| s.phase = "hitl_decision".to_string());
            let hitl = ctx
                .start_activity(
                    AlegriaActivities::run_hitl_decision_step,
                    seo_steps::hitl_decision_step::HitlDecisionInput {
                        run_id: run_id.clone(),
                        step_name: "expert_decomposed_extraction".to_string(),
                        layer_confidence: layer.confidence,
                        completeness_score: completeness.completeness_score,
                        unresolved_mappings: mapping
                            .mappings
                            .iter()
                            .filter(|result| result.needs_hitl)
                            .count(),
                        contradiction_blocked: contradiction.is_blocked,
                        conflict_count: contradiction.conflict_count,
                        loss_count: completeness.missing_elements.len(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            semantic_summary = format!(
                "semantic_ok section={} layer={} mentions={} mapped={} rules={} triples={} completeness={:.2} hitl={}",
                section.section_id,
                layer.primary_layer,
                spans.mentions.len(),
                mapping.mappings.len(),
                procedural.rules.len(),
                triples.triples.len(),
                completeness.completeness_score,
                hitl.requires_hitl
            );
        } else {
            semantic_skipped = true;
        }

        ctx.state_mut(|s| s.phase = "raw_knowledge_ingestion".to_string());
        let raw_knowledge = ctx
            .start_activity(
                AlegriaActivities::run_raw_knowledge_ingestion_step,
                RawKnowledgeIngestionInputPayload {
                    run_id: run_id.clone(),
                    context_key: site_input.context_key.clone(),
                    query_batch_key: ingest.query_batch_key.clone(),
                    raw_page_ids: crawled.raw_page_ids.clone(),
                    source_policy: "candidate_only_truth_extraction@1".to_string(),
                },
                db_opts(60),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        if !raw_knowledge.changed_truth_keys.is_empty() {
            ctx.state_mut(|s| s.phase = "load_verified_support_bundle.refresh".to_string());
            support_bundle = ctx
                .start_activity(
                    AlegriaActivities::load_verified_support_bundle,
                    support_bundle_request,
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;
        }

        ctx.state_mut(|s| s.phase = "done:expert_decomposed_extraction".to_string());
        metrics::global()
            .workflow_completions_total
            .with_label_values(&["ExpertDecomposedExtractionWorkflow"])
            .inc();

        Ok(format!(
            "expert_decomposed_extraction_ok run_id={} semantic_skipped={} semantic_summary=\"{}\" raw_pages={} raw_sections={} verified_rules={} support_bundle={} changed_truth_keys={}",
            run_id,
            semantic_skipped,
            semantic_summary,
            raw_knowledge.raw_page_count,
            raw_knowledge.raw_section_count,
            raw_knowledge.verified_rule_count,
            support_bundle.len(),
            raw_knowledge.changed_truth_keys.len()
        ))
    }

    #[signal(name = "pause")]
    fn pause(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.paused = true;
        self.phase = "paused".to_string();
    }

    #[signal(name = "resume")]
    fn resume(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.paused = false;
        self.phase = "resumed".to_string();
    }

    #[query(name = "status")]
    fn status(&self, _ctx: &WorkflowContextView) -> BasicWorkflowStatus {
        BasicWorkflowStatus {
            phase: self.phase.clone(),
            paused: self.paused,
        }
    }
}
