use contracts::generated::alegria::temporal::v1::{
    CmsApprovalDecision, CmsPublishInputPayload, CmsPublishOutputPayload,
    ContentContractValidateInputPayload, CrawlSourcesInputPayload, CrawlSourcesOutputPayload,
    DraftAssembleInputPayload, DraftAssembleOutputPayload, DraftNormalizeInputPayload,
    DraftNormalizeOutputPayload, DraftQaInputPayload, DraftQaOutputPayload,
    EditorialDraftGenerateInputPayload, EditorialDraftGenerateOutputPayload,
    FinalizePublishInputPayload, GlobalSiteReconcileInputPayload,
    GlobalSiteReconcileOutputPayload, IaBuildInputPayload, IaBuildOutputPayload,
    LinkRecommendInputPayload, LinkRecommendOutputPayload, OpportunityBuildInputPayload,
    OpportunityBuildOutputPayload, PublishMaterializeInputPayload,
    PublishMaterializeOutputPayload, RawKnowledgeIngestionInputPayload,
    RawKnowledgeIngestionOutputPayload, RebuildDetectInputPayload,
    RenderPreviewValidateInputPayload, SeoSiteBuildInputPayload, SerpIngestInputPayload,
    SerpIngestOutputPayload, SerpNormalizeInputPayload, SerpNormalizeOutputPayload,
};
use infrastructure::adapters::temporalio_sdk_adapter::{
    workflow, workflow_methods, SyncWorkflowContext, WorkerOptions, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};
use seo_application::execution::{
    advance_cursor, blocked_publish_gate_status, build_execution_plan, final_phase_keys,
    initial_phase_keys, next_phase, page_phase_keys, phase_label, planning_phase_keys,
    SeoExecutionCursor, SeoExecutionSegment, SeoPhaseDecision, SeoPhaseKey, SeoRunPolicy,
};
use seo_application::scenario::{SeoExecutionMode, SeoScenarioKind, SeoScenarioRequest};

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

fn phase_with_ordinal(key: SeoPhaseKey, page_index: usize, page_total: usize) -> String {
    format!("{}:{}/{}", phase_label(key), page_index + 1, page_total)
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

        let request = SeoScenarioRequest {
            scenario: SeoScenarioKind::Full,
            mode: SeoExecutionMode::TemporalDurable,
            policy: SeoRunPolicy::for_temporal_durable(),
            output_dir: String::new(),
            base_url: String::new(),
            site_input: site_input.clone(),
        };
        let plan = build_execution_plan(&request);

        let scope = site_input.scope.clone();
        let verified_support_request = serde_json::to_string(
            &seo_ports::VerifiedSupportBundleRequest {
                run_id: run_id.clone(),
                context_key: site_input.context_key.clone(),
                scope_signature: scope
                    .as_ref()
                    .map(|value| value.scope_signature.clone())
                    .unwrap_or_default(),
                applicant_profile: scope
                    .as_ref()
                    .map(|value| value.applicant_profile.clone())
                    .unwrap_or_default(),
            },
        )
        .map_err(anyhow::Error::from)?;

        let mut verified_support = Vec::new();
        let mut ingest: Option<SerpIngestOutputPayload> = None;
        let mut crawl_sources: Option<CrawlSourcesOutputPayload> = None;
        let mut raw_knowledge: Option<RawKnowledgeIngestionOutputPayload> = None;
        let mut serp: Option<SerpNormalizeOutputPayload> = None;
        let mut opportunities: Option<OpportunityBuildOutputPayload> = None;
        let mut ia: Option<IaBuildOutputPayload> = None;
        let mut links: Option<LinkRecommendOutputPayload> = None;
        let mut reconciled: Option<GlobalSiteReconcileOutputPayload> = None;

        let mut cursor = SeoExecutionCursor {
            segment: Some(SeoExecutionSegment::Initial),
            ..Default::default()
        };
        loop {
            match next_phase(initial_phase_keys(), &cursor) {
                SeoPhaseDecision::Complete(_) => break,
                SeoPhaseDecision::Run(input) => {
                    ctx.state_mut(|s| s.phase = phase_label(input.key).to_string());
                    match input.key {
                        SeoPhaseKey::LoadVerifiedSupportBundle => {
                            verified_support = ctx
                                .start_activity(
                                    AlegriaActivities::load_verified_support_bundle,
                                    verified_support_request.clone(),
                                    db_opts(30),
                                )
                                .await?;
                        }
                        SeoPhaseKey::SerpIngest => {
                            ingest = Some(
                                ctx.start_activity(
                                    AlegriaActivities::run_serp_ingest_step,
                                    SerpIngestInputPayload {
                                        run_id: run_id.clone(),
                                        query_batch_key: site_input.query_batch_key.clone(),
                                        scope: scope.clone(),
                                        queries: site_input.queries.clone(),
                                    },
                                    db_opts(30),
                                )
                                .await?,
                            );
                        }
                        SeoPhaseKey::CrawlSources => {
                            let ingest_ref = ingest.as_ref().expect("serp_ingest required");
                            crawl_sources = Some(
                                ctx.start_activity(
                                    AlegriaActivities::run_crawl_sources_step,
                                    CrawlSourcesInputPayload {
                                        run_id: run_id.clone(),
                                        query_batch_key: ingest_ref.query_batch_key.clone(),
                                        limit: 25,
                                        emit_qdrant: true,
                                    },
                                    db_opts(120),
                                )
                                .await?,
                            );
                        }
                        SeoPhaseKey::RawKnowledgeIngestion => {
                            let ingest_ref = ingest.as_ref().expect("serp_ingest required");
                            let crawl_ref = crawl_sources.as_ref().expect("crawl_sources required");
                            raw_knowledge = Some(
                                ctx.start_activity(
                                    AlegriaActivities::run_raw_knowledge_ingestion_step,
                                    RawKnowledgeIngestionInputPayload {
                                        run_id: run_id.clone(),
                                        context_key: site_input.context_key.clone(),
                                        query_batch_key: ingest_ref.query_batch_key.clone(),
                                        raw_page_ids: crawl_ref.raw_page_ids.clone(),
                                        source_policy: "auto_verify_high_confidence@1".to_string(),
                                    },
                                    db_opts(60),
                                )
                                .await?,
                            );
                        }
                        _ => unreachable!("invalid initial phase"),
                    }
                    ctx.wait_condition(|s| !s.paused).await;
                }
            }
            advance_cursor(&mut cursor);
        }

        let raw_knowledge = raw_knowledge.expect("raw knowledge required");
        let planning_keys =
            planning_phase_keys(&plan, !raw_knowledge.changed_truth_keys.is_empty());
        cursor = SeoExecutionCursor {
            segment: Some(SeoExecutionSegment::Planning),
            ..Default::default()
        };
        loop {
            match next_phase(&planning_keys, &cursor) {
                SeoPhaseDecision::Complete(_) => break,
                SeoPhaseDecision::Run(input) => {
                    ctx.state_mut(|s| s.phase = phase_label(input.key).to_string());
                    match input.key {
                        SeoPhaseKey::RefreshVerifiedSupportBundle => {
                            verified_support = ctx
                                .start_activity(
                                    AlegriaActivities::load_verified_support_bundle,
                                    verified_support_request.clone(),
                                    db_opts(30),
                                )
                                .await?;
                        }
                        SeoPhaseKey::SerpNormalize => {
                            let ingest_ref = ingest.as_ref().expect("serp_ingest required");
                            serp = Some(
                                ctx.start_activity(
                                    AlegriaActivities::run_serp_normalize_step,
                                    SerpNormalizeInputPayload {
                                        run_id: run_id.clone(),
                                        query_batch_key: ingest_ref.query_batch_key.clone(),
                                        scope: scope.clone(),
                                        queries: site_input.queries.clone(),
                                    },
                                    db_opts(30),
                                )
                                .await?,
                            );
                        }
                        SeoPhaseKey::OpportunityBuild => {
                            let serp_ref = serp.as_ref().expect("serp_normalize required");
                            opportunities = Some(
                                ctx.start_activity(
                                    AlegriaActivities::run_opportunity_build_step,
                                    OpportunityBuildInputPayload {
                                        run_id: run_id.clone(),
                                        scope: scope.clone(),
                                        serp_patterns: serp_ref.serp_patterns.clone(),
                                    },
                                    db_opts(30),
                                )
                                .await?,
                            );
                        }
                        SeoPhaseKey::IaBuild => {
                            let opportunities_ref =
                                opportunities.as_ref().expect("opportunity_build required");
                            ia = Some(
                                ctx.start_activity(
                                    AlegriaActivities::run_ia_build_step,
                                    IaBuildInputPayload {
                                        run_id: run_id.clone(),
                                        scope: scope.clone(),
                                        keyword_clusters: opportunities_ref.keyword_clusters.clone(),
                                    },
                                    db_opts(30),
                                )
                                .await?,
                            );
                        }
                        SeoPhaseKey::LinkRecommend => {
                            let ia_ref = ia.as_ref().expect("ia_build required");
                            links = Some(
                                ctx.start_activity(
                                    AlegriaActivities::run_link_recommend_step,
                                    LinkRecommendInputPayload {
                                        run_id: run_id.clone(),
                                        page_nodes: ia_ref.page_nodes.clone(),
                                        max_links_per_page: 3,
                                    },
                                    db_opts(30),
                                )
                                .await?,
                            );
                        }
                        SeoPhaseKey::GlobalSiteReconcile => {
                            let ia_ref = ia.as_ref().expect("ia_build required");
                            let links_ref = links.as_ref().expect("link_recommend required");
                            reconciled = Some(
                                ctx.start_activity(
                                    AlegriaActivities::run_global_site_reconcile_step,
                                    GlobalSiteReconcileInputPayload {
                                        run_id: run_id.clone(),
                                        scope: scope.clone(),
                                        page_nodes: ia_ref.page_nodes.clone(),
                                        link_recommendations: links_ref.link_recommendations.clone(),
                                        reconcile_reason: "seo_site_build_workflow@1".to_string(),
                                    },
                                    db_opts(30),
                                )
                                .await?,
                            );
                        }
                        _ => unreachable!("invalid planning phase"),
                    }
                    ctx.wait_condition(|s| !s.paused).await;
                }
            }
            advance_cursor(&mut cursor);
        }

        let ia = ia.expect("ia_build required");
        let links = links.expect("link_recommend required");
        let reconciled = reconciled.expect("global_site_reconcile required");
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
        let page_keys = page_phase_keys(&plan);

        for (page_index, page_node) in page_nodes.iter().cloned().enumerate() {
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
            let mut draft_plan: Option<DraftAssembleOutputPayload> = None;
            let mut editorial_candidate: Option<EditorialDraftGenerateOutputPayload> = None;
            let mut draft: Option<DraftNormalizeOutputPayload> = None;
            let mut cms_requested: Option<CmsPublishOutputPayload> = None;
            let mut cms_approved: Option<CmsPublishOutputPayload> = None;
            let mut materialized: Option<PublishMaterializeOutputPayload> = None;
            let mut page_blocked = false;
            cursor = SeoExecutionCursor {
                segment: Some(SeoExecutionSegment::PerPage),
                page_index,
                page_total,
                ..Default::default()
            };
            loop {
                match next_phase(&page_keys, &cursor) {
                    SeoPhaseDecision::Complete(_) => break,
                    SeoPhaseDecision::Run(input) => {
                        ctx.state_mut(|s| {
                            s.phase = phase_with_ordinal(input.key, page_index, page_total)
                        });
                        match input.key {
                            SeoPhaseKey::DraftAssemble => {
                                draft_plan = Some(
                                    ctx.start_activity(
                                        AlegriaActivities::run_draft_assemble_step,
                                        DraftAssembleInputPayload {
                                            run_id: run_id.clone(),
                                            page_node: Some(page_node.clone()),
                                            page_blueprint: Some(page_blueprint.clone()),
                                            factual_fragments: factual_fragments.clone(),
                                            verified_support: verified_support.clone(),
                                            required_links: required_links.clone(),
                                            section_templates: Vec::new(),
                                            llm_candidate: None,
                                            source_context_chunks: Vec::new(),
                                        },
                                        db_opts(30),
                                    )
                                    .await?,
                                );
                            }
                            SeoPhaseKey::EditorialDraftGenerate => {
                                let draft_plan_ref =
                                    draft_plan.as_ref().expect("draft_assemble required");
                                editorial_candidate = Some(
                                    ctx.start_activity(
                                        AlegriaActivities::run_editorial_draft_generate,
                                        EditorialDraftGenerateInputPayload {
                                            run_id: run_id.clone(),
                                            request: draft_plan_ref.llm_request.clone(),
                                            provider_policy: "multi_provider:first_available"
                                                .to_string(),
                                        },
                                        db_opts(60),
                                    )
                                    .await?,
                                );
                            }
                            SeoPhaseKey::DraftNormalize => {
                                let draft_plan_ref =
                                    draft_plan.as_ref().expect("draft_assemble required");
                                let editorial_ref = editorial_candidate
                                    .as_ref()
                                    .expect("editorial generation required");
                                draft = Some(
                                    ctx.start_activity(
                                        AlegriaActivities::run_draft_normalize_step,
                                        DraftNormalizeInputPayload {
                                            run_id: run_id.clone(),
                                            page_node: Some(page_node.clone()),
                                            page_blueprint: Some(page_blueprint.clone()),
                                            factual_fragments: factual_fragments.clone(),
                                            verified_support: verified_support.clone(),
                                            required_links: required_links.clone(),
                                            section_templates: draft_plan_ref
                                                .editorial_brief
                                                .as_ref()
                                                .map(|brief| brief.section_templates.clone())
                                                .unwrap_or_default(),
                                            content_block_plan: draft_plan_ref
                                                .content_block_plan
                                                .clone(),
                                            llm_candidate: editorial_ref.candidate.clone(),
                                        },
                                        db_opts(30),
                                    )
                                    .await?,
                                );
                            }
                            SeoPhaseKey::ContentContractValidate => {
                                let draft_ref = draft.as_ref().expect("draft_normalize required");
                                let _ = ctx
                                    .start_activity(
                                        AlegriaActivities::run_content_contract_validate_step,
                                        ContentContractValidateInputPayload {
                                            run_id: run_id.clone(),
                                            draft: draft_ref.draft.clone(),
                                            required_links: required_links.clone(),
                                        },
                                        db_opts(30),
                                    )
                                    .await?;
                            }
                            SeoPhaseKey::DraftQa => {
                                let draft_ref = draft.as_ref().expect("draft_normalize required");
                                let _qa: DraftQaOutputPayload = ctx
                                    .start_activity(
                                        AlegriaActivities::run_draft_qa_step,
                                        DraftQaInputPayload {
                                            run_id: run_id.clone(),
                                            draft: draft_ref.draft.clone(),
                                            supported_fragments: factual_fragments.clone(),
                                            required_links: required_links.clone(),
                                        },
                                        db_opts(30),
                                    )
                                    .await?;
                            }
                            SeoPhaseKey::CmsRequestReview => {
                                let draft_ref = draft.as_ref().expect("draft_normalize required");
                                cms_requested = Some(
                                    ctx.start_activity(
                                        AlegriaActivities::run_cms_publish_step,
                                        CmsPublishInputPayload {
                                            run_id: run_id.clone(),
                                            page_node: Some(page_node.clone()),
                                            draft: draft_ref.draft.clone(),
                                            actor_role: "seo_system".to_string(),
                                            publish_mode: "request_review".to_string(),
                                            approval_decision: None,
                                        },
                                        db_opts(30),
                                    )
                                    .await?,
                                );
                                if cms_requested
                                    .as_ref()
                                    .expect("cms_request_review output")
                                    .verdict
                                    != "review_requested"
                                {
                                    page_blocked = true;
                                    ctx.state_mut(|s| {
                                        s.phase = format!(
                                            "{}:{}",
                                            blocked_publish_gate_status(),
                                            phase_with_ordinal(input.key, page_index, page_total)
                                        )
                                    });
                                    break;
                                }
                            }
                            SeoPhaseKey::HumanApprovalWait => {
                                let cms_requested_ref =
                                    cms_requested.as_ref().expect("cms request required");
                                ctx.state_mut(|s| {
                                    s.waiting_hitl = true;
                                    s.resume_requested = false;
                                    s.paused = true;
                                    s.phase = phase_with_ordinal(input.key, page_index, page_total);
                                });
                                loop {
                                    ctx.wait_condition(|s| !s.paused && s.resume_requested).await;
                                    let approval_lookup_key = format!(
                                        "{}|{}|{}",
                                        run_id, page_node.page_node_key, cms_requested_ref.revision_id
                                    );
                                    let approval_decision: CmsApprovalDecision = ctx
                                        .start_activity(
                                            AlegriaActivities::load_cms_approval_decision,
                                            approval_lookup_key.clone(),
                                            db_opts(30),
                                        )
                                        .await?;
                                    if approval_decision.decision == "approved" {
                                        break;
                                    }
                                    ctx.state_mut(|s| {
                                        s.waiting_hitl = true;
                                        s.resume_requested = false;
                                        s.paused = true;
                                        s.phase = format!(
                                            "{}:{}:{}",
                                            phase_label(input.key),
                                            approval_decision.decision,
                                            page_index + 1
                                        );
                                    });
                                }
                                ctx.state_mut(|s| {
                                    s.waiting_hitl = false;
                                    s.resume_requested = false;
                                });
                            }
                            SeoPhaseKey::CmsPublishApproved => {
                                let draft_ref = draft.as_ref().expect("draft_normalize required");
                                let cms_requested_ref =
                                    cms_requested.as_ref().expect("cms request required");
                                let approval_lookup_key = format!(
                                    "{}|{}|{}",
                                    run_id, page_node.page_node_key, cms_requested_ref.revision_id
                                );
                                let approval_decision: CmsApprovalDecision = ctx
                                    .start_activity(
                                        AlegriaActivities::load_cms_approval_decision,
                                        approval_lookup_key,
                                        db_opts(30),
                                    )
                                    .await?;
                                cms_approved = Some(
                                    ctx.start_activity(
                                        AlegriaActivities::run_cms_publish_step,
                                        CmsPublishInputPayload {
                                            run_id: run_id.clone(),
                                            page_node: Some(page_node.clone()),
                                            draft: draft_ref.draft.clone(),
                                            actor_role: "seo_system".to_string(),
                                            publish_mode: "approved_publish".to_string(),
                                            approval_decision: Some(approval_decision),
                                        },
                                        db_opts(30),
                                    )
                                    .await?,
                                );
                                if cms_approved
                                    .as_ref()
                                    .expect("cms approved output")
                                    .verdict
                                    != "approved"
                                {
                                    page_blocked = true;
                                    ctx.state_mut(|s| {
                                        s.phase = format!(
                                            "{}:{}",
                                            blocked_publish_gate_status(),
                                            phase_with_ordinal(input.key, page_index, page_total)
                                        )
                                    });
                                    break;
                                }
                            }
                            SeoPhaseKey::PublishMaterialize => {
                                let cms_requested_ref =
                                    cms_requested.as_ref().expect("cms request required");
                                materialized = Some(
                                    ctx.start_activity(
                                        AlegriaActivities::run_publish_materialize_step,
                                        PublishMaterializeInputPayload {
                                            run_id: run_id.clone(),
                                            page_node_key: page_node.page_node_key.clone(),
                                            revision_id: cms_requested_ref.revision_id.clone(),
                                            cms_document_id: cms_requested_ref.cms_document_id.clone(),
                                            canonical_url_path: page_node.canonical_url_path.clone(),
                                            publish_artifact: cms_requested_ref.publish_artifact.clone().or(
                                                cms_approved
                                                    .as_ref()
                                                    .and_then(|approved| approved.publish_artifact.clone()),
                                            ),
                                            output_dir: String::new(),
                                            base_url: String::new(),
                                        },
                                        db_opts(60),
                                    )
                                    .await?,
                                );
                                if materialized
                                    .as_ref()
                                    .expect("publish materialize output")
                                    .materialization_status
                                    .contains("failed")
                                {
                                    page_blocked = true;
                                    ctx.state_mut(|s| {
                                        s.phase = format!(
                                            "{}:{}",
                                            blocked_publish_gate_status(),
                                            phase_with_ordinal(input.key, page_index, page_total)
                                        )
                                    });
                                    break;
                                }
                            }
                            SeoPhaseKey::RenderPreviewValidate => {
                                let materialized_ref =
                                    materialized.as_ref().expect("publish materialize required");
                                let render_validation = ctx
                                    .start_activity(
                                        AlegriaActivities::run_render_preview_validate_step,
                                        RenderPreviewValidateInputPayload {
                                            run_id: run_id.clone(),
                                            preview_pages: materialized_ref.preview_pages.clone(),
                                        },
                                        db_opts(30),
                                    )
                                    .await?;
                                if render_validation.verdict != "render_ready" {
                                    page_blocked = true;
                                    ctx.state_mut(|s| {
                                        s.phase = format!(
                                            "{}:{}",
                                            blocked_publish_gate_status(),
                                            phase_with_ordinal(input.key, page_index, page_total)
                                        )
                                    });
                                    break;
                                }
                            }
                            SeoPhaseKey::FinalizePublish => {
                                let cms_requested_ref =
                                    cms_requested.as_ref().expect("cms request required");
                                let materialized_ref =
                                    materialized.as_ref().expect("publish materialize required");
                                let render_validation = ctx
                                    .start_activity(
                                        AlegriaActivities::run_render_preview_validate_step,
                                        RenderPreviewValidateInputPayload {
                                            run_id: run_id.clone(),
                                            preview_pages: materialized_ref.preview_pages.clone(),
                                        },
                                        db_opts(30),
                                    )
                                    .await?;
                                let finalized = ctx
                                    .start_activity(
                                        AlegriaActivities::run_finalize_publish_step,
                                        FinalizePublishInputPayload {
                                            run_id: run_id.clone(),
                                            page_node_key: page_node.page_node_key.clone(),
                                            revision_id: cms_requested_ref.revision_id.clone(),
                                            publish_artifact: materialized_ref.publish_artifact.clone(),
                                            render_validation: Some(render_validation),
                                        },
                                        db_opts(30),
                                    )
                                    .await?;
                                if finalized.verdict == "published" {
                                    published_pages += 1;
                                } else {
                                    page_blocked = true;
                                    ctx.state_mut(|s| {
                                        s.phase = format!(
                                            "{}:{}",
                                            blocked_publish_gate_status(),
                                            phase_with_ordinal(input.key, page_index, page_total)
                                        )
                                    });
                                    break;
                                }
                            }
                            _ => unreachable!("invalid page phase"),
                        }
                        ctx.wait_condition(|s| !s.paused).await;
                    }
                }
                advance_cursor(&mut cursor);
            }
            if page_blocked {
                continue;
            }
        }

        cursor = SeoExecutionCursor {
            segment: Some(SeoExecutionSegment::Finalize),
            ..Default::default()
        };
        loop {
            match next_phase(final_phase_keys(&plan), &cursor) {
                SeoPhaseDecision::Complete(_) => break,
                SeoPhaseDecision::Run(input) => {
                    ctx.state_mut(|s| s.phase = phase_label(input.key).to_string());
                    match input.key {
                        SeoPhaseKey::RebuildDetect => {
                            let _ = ctx
                                .start_activity(
                                    AlegriaActivities::run_rebuild_detect_step,
                                    RebuildDetectInputPayload {
                                        run_id: run_id.clone(),
                                        changed_truth_keys: raw_knowledge
                                            .changed_truth_keys
                                            .clone(),
                                        page_nodes: page_nodes.clone(),
                                    },
                                    db_opts(30),
                                )
                                .await?;
                        }
                        _ => unreachable!("invalid final phase"),
                    }
                    ctx.wait_condition(|s| !s.paused).await;
                }
            }
            advance_cursor(&mut cursor);
        }

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
