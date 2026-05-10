use contracts::generated::alegria::temporal::v1::{
    CmsPublishInputPayload, ContentContractValidateInputPayload, CrawlSourcesInputPayload,
    DraftAssembleInputPayload, DraftNormalizeInputPayload, DraftQaInputPayload,
    EditorialDraftGenerateInputPayload, FinalizePublishInputPayload,
    GlobalSiteReconcileInputPayload, IaBuildInputPayload, LinkRecommendInputPayload,
    OpportunityBuildInputPayload, PublishMaterializeInputPayload,
    RawKnowledgeIngestionInputPayload, RebuildDetectInputPayload,
    RenderPreviewValidateInputPayload, SeoSiteBuildInputPayload, SerpIngestInputPayload,
    SerpNormalizeInputPayload,
};
use infrastructure::adapters::temporalio_sdk_adapter::{
    workflow, workflow_methods, SyncWorkflowContext, WorkerOptions, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};

use crate::activities::AlegriaActivities;
use crate::metrics;

use super::runtime::{db_opts, BasicWorkflowStatus};

#[workflow]
#[derive(Default)]
struct SeoSiteBuildWorkflow {
    paused: bool,
    waiting_hitl: bool,
    resume_requested: bool,
    phase: String,
}

pub(crate) fn register(opts: &mut WorkerOptions) {
    opts.register_workflow::<SeoSiteBuildWorkflow>();
}

#[workflow_methods]
impl SeoSiteBuildWorkflow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        metrics::global()
            .workflow_starts_total
            .with_label_values(&["SeoSiteBuildWorkflow"])
            .inc();

        let run_id: String = ctx.workflow_initial_info().workflow_id.clone();
        ctx.state_mut(|s| s.phase = "load_seo_site_build_input".to_string());
        let site_input: SeoSiteBuildInputPayload = ctx
            .start_activity(
                AlegriaActivities::load_seo_site_build_input,
                run_id.clone(),
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        let scope = site_input.scope.clone();
        let scope_signature = scope
            .as_ref()
            .map(|value| value.scope_signature.clone())
            .unwrap_or_default();

        ctx.state_mut(|s| s.phase = "load_verified_support_bundle".to_string());
        let mut verified_support = ctx
            .start_activity(
                AlegriaActivities::load_verified_support_bundle,
                format!("{}|{}|{}", run_id, site_input.context_key, scope_signature),
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
                    scope: scope.clone(),
                    queries: site_input.queries.clone(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "crawl_sources".to_string());
        let crawl_sources = ctx
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

        ctx.state_mut(|s| s.phase = "raw_knowledge_ingestion".to_string());
        let raw_knowledge = ctx
            .start_activity(
                AlegriaActivities::run_raw_knowledge_ingestion_step,
                RawKnowledgeIngestionInputPayload {
                    run_id: run_id.clone(),
                    context_key: site_input.context_key.clone(),
                    query_batch_key: ingest.query_batch_key.clone(),
                    raw_page_ids: crawl_sources.raw_page_ids.clone(),
                    source_policy: "auto_verify_high_confidence@1".to_string(),
                },
                db_opts(60),
            )
            .await?;
        if !raw_knowledge.changed_truth_keys.is_empty() {
            verified_support = ctx
                .start_activity(
                    AlegriaActivities::load_verified_support_bundle,
                    format!("{}|{}|{}", run_id, site_input.context_key, scope_signature),
                    db_opts(30),
                )
                .await?;
        }
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "serp_normalize".to_string());
        let serp = ctx
            .start_activity(
                AlegriaActivities::run_serp_normalize_step,
                SerpNormalizeInputPayload {
                    run_id: run_id.clone(),
                    query_batch_key: ingest.query_batch_key.clone(),
                    scope: scope.clone(),
                    queries: site_input.queries.clone(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "opportunity_build".to_string());
        let opportunities = ctx
            .start_activity(
                AlegriaActivities::run_opportunity_build_step,
                OpportunityBuildInputPayload {
                    run_id: run_id.clone(),
                    scope: scope.clone(),
                    serp_patterns: serp.serp_patterns.clone(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "ia_build".to_string());
        let ia = ctx
            .start_activity(
                AlegriaActivities::run_ia_build_step,
                IaBuildInputPayload {
                    run_id: run_id.clone(),
                    scope: scope.clone(),
                    keyword_clusters: opportunities.keyword_clusters.clone(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "link_recommend".to_string());
        let links = ctx
            .start_activity(
                AlegriaActivities::run_link_recommend_step,
                LinkRecommendInputPayload {
                    run_id: run_id.clone(),
                    page_nodes: ia.page_nodes.clone(),
                    max_links_per_page: 3,
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "global_site_reconcile".to_string());
        let reconciled = ctx
            .start_activity(
                AlegriaActivities::run_global_site_reconcile_step,
                GlobalSiteReconcileInputPayload {
                    run_id: run_id.clone(),
                    scope: scope.clone(),
                    page_nodes: ia.page_nodes.clone(),
                    link_recommendations: links.link_recommendations.clone(),
                    reconcile_reason: "seo_site_build_workflow@1".to_string(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        let page_nodes = if reconciled.page_nodes.is_empty() {
            ia.page_nodes.clone()
        } else {
            reconciled.page_nodes.clone()
        };
        let link_recommendations = if reconciled.link_recommendations.is_empty() {
            links.link_recommendations.clone()
        } else {
            reconciled.link_recommendations.clone()
        };
        if page_nodes.is_empty() {
            ctx.state_mut(|s| s.phase = "done:no_pages".to_string());
            return Ok(run_id);
        }
        let factual_fragments = verified_support
            .iter()
            .map(|support| support.fragment_text.clone())
            .collect::<Vec<_>>();
        let page_total = page_nodes.len();
        let mut published_pages = 0usize;

        for (page_index, page_node) in page_nodes.iter().cloned().enumerate() {
            let page_ordinal = page_index + 1;
            let page_blueprint = ia
                .page_blueprints
                .iter()
                .find(|blueprint| blueprint.blueprint_key == page_node.blueprint_key)
                .cloned()
                .unwrap_or_default();
            let required_links: Vec<_> = link_recommendations
                .iter()
                .filter(|link| {
                    link.required_flag && link.source_page_key == page_node.page_node_key
                })
                .cloned()
                .collect();

            ctx.state_mut(|s| s.phase = format!("draft_assemble_plan:{page_ordinal}/{page_total}"));
            let draft_input = DraftAssembleInputPayload {
                run_id: run_id.clone(),
                page_node: Some(page_node.clone()),
                page_blueprint: Some(page_blueprint.clone()),
                factual_fragments: factual_fragments.clone(),
                verified_support: verified_support.clone(),
                required_links: required_links.clone(),
                section_templates: Vec::new(),
                llm_candidate: None,
                source_context_chunks: Vec::new(),
            };
            let draft_plan = ctx
                .start_activity(
                    AlegriaActivities::run_draft_assemble_step,
                    draft_input.clone(),
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| {
                s.phase = format!("editorial_draft_generate:{page_ordinal}/{page_total}")
            });
            let editorial_candidate = ctx
                .start_activity(
                    AlegriaActivities::run_editorial_draft_generate,
                    EditorialDraftGenerateInputPayload {
                        run_id: run_id.clone(),
                        request: draft_plan.llm_request.clone(),
                        provider_policy: "multi_provider:first_available".to_string(),
                    },
                    db_opts(60),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| s.phase = format!("draft_normalize:{page_ordinal}/{page_total}"));
            let draft = ctx
                .start_activity(
                    AlegriaActivities::run_draft_normalize_step,
                    DraftNormalizeInputPayload {
                        run_id: run_id.clone(),
                        page_node: Some(page_node.clone()),
                        page_blueprint: Some(page_blueprint.clone()),
                        factual_fragments: factual_fragments.clone(),
                        verified_support: verified_support.clone(),
                        required_links: draft_input.required_links.clone(),
                        section_templates: draft_plan
                            .editorial_brief
                            .as_ref()
                            .map(|brief| brief.section_templates.clone())
                            .unwrap_or_default(),
                        content_block_plan: draft_plan.content_block_plan.clone(),
                        llm_candidate: editorial_candidate.candidate,
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| {
                s.phase = format!("content_contract_validate:{page_ordinal}/{page_total}")
            });
            let _content_contract = ctx
                .start_activity(
                    AlegriaActivities::run_content_contract_validate_step,
                    ContentContractValidateInputPayload {
                        run_id: run_id.clone(),
                        draft: draft.draft.clone(),
                        required_links: required_links.clone(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| s.phase = format!("draft_qa:{page_ordinal}/{page_total}"));
            let qa = ctx
                .start_activity(
                    AlegriaActivities::run_draft_qa_step,
                    DraftQaInputPayload {
                        run_id: run_id.clone(),
                        draft: draft.draft.clone(),
                        supported_fragments: factual_fragments.clone(),
                        required_links: required_links.clone(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            let mut cms_draft = draft.draft.clone();
            if let Some(draft_state) = cms_draft.as_mut() {
                draft_state.qa_verdict = qa.verdict.clone();
            }
            ctx.state_mut(|s| s.phase = format!("cms_request_review:{page_ordinal}/{page_total}"));
            let cms = ctx
                .start_activity(
                    AlegriaActivities::run_cms_publish_step,
                    CmsPublishInputPayload {
                        run_id: run_id.clone(),
                        page_node: Some(page_node.clone()),
                        draft: cms_draft,
                        actor_role: "seo_system".to_string(),
                        publish_mode: "request_review".to_string(),
                        approval_decision: None,
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            if cms.verdict != "review_requested" {
                ctx.state_mut(|s| s.phase = format!("review_blocked:{page_ordinal}/{page_total}"));
                continue;
            }

            let revision_id = cms.revision_id.clone();
            let cms_document_id = cms.cms_document_id.clone();
            let publish_artifact = cms.publish_artifact.clone();
            ctx.state_mut(|s| {
                s.waiting_hitl = true;
                s.resume_requested = false;
                s.paused = true;
                s.phase = format!("human_approval_wait:{page_ordinal}/{page_total}");
            });
            ctx.wait_condition(|s| !s.paused && s.resume_requested)
                .await;
            let approval_lookup_key =
                format!("{}|{}|{}", run_id, page_node.page_node_key, revision_id);
            let approval_decision = ctx
                .start_activity(
                    AlegriaActivities::load_cms_approval_decision,
                    approval_lookup_key.clone(),
                    db_opts(30),
                )
                .await?;
            if approval_decision.decision != "approved" {
                ctx.state_mut(|s| {
                    s.waiting_hitl = true;
                    s.resume_requested = false;
                    s.paused = true;
                    s.phase = format!(
                        "human_approval_wait:{}:{page_ordinal}/{page_total}",
                        approval_decision.decision
                    );
                });
                ctx.wait_condition(|s| !s.paused && s.resume_requested)
                    .await;
            }
            let approval_decision = ctx
                .start_activity(
                    AlegriaActivities::load_cms_approval_decision,
                    approval_lookup_key,
                    db_opts(30),
                )
                .await?;

            ctx.state_mut(|s| {
                s.waiting_hitl = false;
                s.resume_requested = false;
                s.phase = format!("cms_publish_approved:{page_ordinal}/{page_total}");
            });
            if approval_decision.decision != "approved" {
                continue;
            }
            let cms_approved = ctx
                .start_activity(
                    AlegriaActivities::run_cms_publish_step,
                    CmsPublishInputPayload {
                        run_id: run_id.clone(),
                        page_node: Some(page_node.clone()),
                        draft: draft.draft.clone(),
                        actor_role: "seo_system".to_string(),
                        publish_mode: "approved_publish".to_string(),
                        approval_decision: Some(approval_decision),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            if cms_approved.verdict != "approved" {
                ctx.state_mut(|s| {
                    s.phase = format!("approval_blocked:{page_ordinal}/{page_total}")
                });
                continue;
            }

            ctx.state_mut(|s| s.phase = format!("publish_materialize:{page_ordinal}/{page_total}"));
            let materialized = ctx
                .start_activity(
                    AlegriaActivities::run_publish_materialize_step,
                    PublishMaterializeInputPayload {
                        run_id: run_id.clone(),
                        page_node_key: page_node.page_node_key.clone(),
                        revision_id: revision_id.clone(),
                        cms_document_id,
                        canonical_url_path: page_node.canonical_url_path.clone(),
                        publish_artifact: publish_artifact
                            .or(cms_approved.publish_artifact.clone()),
                        output_dir: String::new(),
                        base_url: String::new(),
                    },
                    db_opts(60),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| {
                s.phase = format!("render_preview_validate:{page_ordinal}/{page_total}")
            });
            let render_validation = ctx
                .start_activity(
                    AlegriaActivities::run_render_preview_validate_step,
                    RenderPreviewValidateInputPayload {
                        run_id: run_id.clone(),
                        preview_pages: materialized.preview_pages.clone(),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.wait_condition(|s| !s.paused).await;

            ctx.state_mut(|s| s.phase = format!("finalize_publish:{page_ordinal}/{page_total}"));
            let finalized = ctx
                .start_activity(
                    AlegriaActivities::run_finalize_publish_step,
                    FinalizePublishInputPayload {
                        run_id: run_id.clone(),
                        page_node_key: page_node.page_node_key.clone(),
                        revision_id,
                        publish_artifact: materialized.publish_artifact,
                        render_validation: Some(render_validation),
                    },
                    db_opts(30),
                )
                .await?;
            if finalized.verdict == "published" {
                published_pages += 1;
            }
            ctx.wait_condition(|s| !s.paused).await;
        }

        ctx.state_mut(|s| s.phase = "rebuild_detect".to_string());
        let _rebuild = ctx
            .start_activity(
                AlegriaActivities::run_rebuild_detect_step,
                RebuildDetectInputPayload {
                    run_id: run_id.clone(),
                    changed_truth_keys: raw_knowledge.changed_truth_keys.clone(),
                    page_nodes: page_nodes.clone(),
                },
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = format!("done:published_pages={published_pages}/{page_total}"));
        metrics::global()
            .workflow_completions_total
            .with_label_values(&["SeoSiteBuildWorkflow"])
            .inc();

        Ok(run_id)
    }

    #[signal(name = "pause")]
    fn pause(&mut self, _ctx: &mut SyncWorkflowContext<Self>, reason: Option<String>) {
        self.paused = true;
        self.phase = format!("paused:{}", reason.unwrap_or_else(|| "manual".to_string()));
    }

    #[signal(name = "resume")]
    fn resume(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        if self.waiting_hitl {
            self.resume_requested = true;
            self.paused = false;
            self.phase = "resumed_hitl_review".to_string();
        } else {
            self.paused = false;
            if self.phase.starts_with("paused") {
                self.phase = "resumed".to_string();
            }
        }
    }

    #[query(name = "status")]
    fn status(&self, _ctx: &WorkflowContextView) -> BasicWorkflowStatus {
        BasicWorkflowStatus {
            phase: self.phase.clone(),
            paused: self.paused,
        }
    }

    #[update(name = "set_pause")]
    fn set_pause(&mut self, _ctx: &mut SyncWorkflowContext<Self>, paused: bool) -> bool {
        self.paused = paused;
        self.paused
    }
}
