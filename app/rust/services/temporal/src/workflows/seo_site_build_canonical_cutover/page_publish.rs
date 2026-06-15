enum PagePublishLoopOutcome {
    Completed(usize),
    Blocked(String),
}

async fn run_page_publish_phases(
    ctx: &mut WorkflowContext<SeoSiteBuildCanonicalCutoverWorkflow>,
    run_id: &str,
    plan: &SeoExecutionPlan,
    page_nodes: &[PageNodeState],
    page_blueprints: &[PageBlueprintState],
    link_recommendations: &[LinkRecommendationState],
    support_bundle: &[SeoVerifiedFactSupportState],
    factual_fragments: &[String],
    context_key: &str,
    applicant_profile: &str,
) -> WorkflowResult<PagePublishLoopOutcome> {
    let page_total = page_nodes.len();
    let page_keys = page_phase_keys(plan);
    let mut published_pages = 0usize;

    for (page_index, page_node) in page_nodes.iter().cloned().enumerate() {
        let page_blueprint = page_blueprints
            .iter()
            .find(|blueprint| blueprint.blueprint_key == page_node.blueprint_key)
            .cloned()
            .unwrap_or_default();
        let required_links: Vec<_> = link_recommendations
            .iter()
            .filter(|link| link.required_flag && link.source_page_key == page_node.page_node_key)
            .cloned()
            .collect();

        let mut draft_plan: Option<DraftAssembleOutputPayload> = None;
        let mut editorial_candidate: Option<EditorialDraftGenerateOutputPayload> = None;
        let mut draft: Option<DraftNormalizeOutputPayload> = None;
        let mut cms_requested: Option<CmsPublishOutputPayload> = None;
        let mut cms_approved: Option<CmsPublishOutputPayload> = None;
        let mut materialized: Option<PublishMaterializeOutputPayload> = None;
        let mut page_blocked = false;

        for key in page_keys.iter().copied() {
            ctx.state_mut(|s| s.phase = phase_with_ordinal(key, page_index, page_total));
            match key {
                SeoPhaseKey::DraftAssemble => {
                    let _ = ctx
                        .start_activity(
                            AlegriaActivities::run_truth_admissibility_gate_step,
                            TruthAdmissibilityGateInput {
                                run_id: run_id.to_string(),
                                context_key: context_key.to_string(),
                                applicant_profile: applicant_profile.to_string(),
                                verified_support: support_bundle.to_vec(),
                            },
                            db_opts(30),
                        )
                        .await?;
                    draft_plan = Some(
                        ctx.start_activity(
                            AlegriaActivities::run_draft_assemble_step,
                            DraftAssembleInputPayload {
                                run_id: run_id.to_string(),
                                page_node: Some(page_node.clone()),
                                page_blueprint: Some(page_blueprint.clone()),
                                factual_fragments: factual_fragments.to_vec(),
                                verified_support: support_bundle.to_vec(),
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
                    let Some(draft_plan_ref) = draft_plan.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "draft_assemble_required",
                        )));
                    };
                    editorial_candidate = Some(
                        ctx.start_activity(
                            AlegriaActivities::run_editorial_draft_generate,
                            EditorialDraftGenerateInputPayload {
                                run_id: run_id.to_string(),
                                request: draft_plan_ref.llm_request.clone(),
                                provider_policy: "multi_provider:first_available".to_string(),
                            },
                            db_opts(60),
                        )
                        .await?,
                    );
                }
                SeoPhaseKey::DraftNormalize => {
                    let Some(draft_plan_ref) = draft_plan.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "draft_assemble_required",
                        )));
                    };
                    let Some(editorial_ref) = editorial_candidate.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "editorial_generation_required",
                        )));
                    };
                    draft = Some(
                        ctx.start_activity(
                            AlegriaActivities::run_draft_normalize_step,
                            DraftNormalizeInputPayload {
                                run_id: run_id.to_string(),
                                page_node: Some(page_node.clone()),
                                page_blueprint: Some(page_blueprint.clone()),
                                factual_fragments: factual_fragments.to_vec(),
                                verified_support: support_bundle.to_vec(),
                                required_links: required_links.clone(),
                                section_templates: draft_plan_ref
                                    .editorial_brief
                                    .as_ref()
                                    .map(|brief| brief.section_templates.clone())
                                    .unwrap_or_default(),
                                content_block_plan: draft_plan_ref.content_block_plan.clone(),
                                llm_candidate: editorial_ref.candidate.clone(),
                            },
                            db_opts(30),
                        )
                        .await?,
                    );
                }
                SeoPhaseKey::ContentContractValidate => {
                    let Some(draft_ref) = draft.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "draft_normalize_required",
                        )));
                    };
                    let _ = ctx
                        .start_activity(
                            AlegriaActivities::run_content_contract_validate_step,
                            ContentContractValidateInputPayload {
                                run_id: run_id.to_string(),
                                draft: draft_ref.draft.clone(),
                                required_links: required_links.clone(),
                            },
                            db_opts(30),
                        )
                        .await?;
                }
                SeoPhaseKey::DraftQa => {
                    let Some(draft_ref) = draft.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "draft_normalize_required",
                        )));
                    };
                    let qa = ctx
                        .start_activity(
                            AlegriaActivities::run_draft_qa_step,
                            DraftQaInputPayload {
                                run_id: run_id.to_string(),
                                draft: draft_ref.draft.clone(),
                                supported_fragments: factual_fragments.to_vec(),
                                required_links: required_links.clone(),
                            },
                            db_opts(30),
                        )
                        .await?;
                    if let Some(draft_state) =
                        draft.as_mut().and_then(|state| state.draft.as_mut())
                    {
                        draft_state.qa_verdict = qa.verdict;
                    }
                }
                SeoPhaseKey::CmsRequestReview => {
                    let Some(draft_ref) = draft.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "draft_normalize_required",
                        )));
                    };
                    cms_requested = Some(
                        ctx.start_activity(
                            AlegriaActivities::run_cms_request_review_step,
                            CmsPublishInputPayload {
                                run_id: run_id.to_string(),
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
                    let Some(cms_requested_ref) = cms_requested.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "cms_request_review_output",
                        )));
                    };
                    if cms_requested_ref.verdict != "review_requested" {
                        page_blocked = true;
                        ctx.state_mut(|s| {
                            s.phase = format!(
                                "{}:{}",
                                blocked_publish_gate_status(),
                                phase_with_ordinal(key, page_index, page_total)
                            )
                        });
                        break;
                    }
                }
                SeoPhaseKey::HumanApprovalWait => {
                    let Some(cms_requested_ref) = cms_requested.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "cms_request_required",
                        )));
                    };
                    let approval_input = HumanApprovalWaitInput {
                        run_id: run_id.to_string(),
                        page_node_key: page_node.page_node_key.clone(),
                        revision_id: cms_requested_ref.revision_id.clone(),
                    };
                    match plan.policy.interaction {
                        RunInteractionPolicy::AllowHitlPause => {
                            ctx.state_mut(|s| {
                                s.waiting_hitl = true;
                                s.resume_requested = false;
                                s.paused = true;
                                s.phase = phase_with_ordinal(key, page_index, page_total);
                            });
                            loop {
                                ctx.wait_condition(|s| !s.paused && s.resume_requested).await;
                                let approval_decision: CmsApprovalDecision = ctx
                                    .start_activity(
                                        AlegriaActivities::run_human_approval_wait_step,
                                        approval_input.clone(),
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
                                        phase_label(key),
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
                        RunInteractionPolicy::FailIfHitlRequired => {
                            page_blocked = true;
                            ctx.state_mut(|s| {
                                s.waiting_hitl = false;
                                s.resume_requested = false;
                                s.phase = format!(
                                    "{}:{}",
                                    blocked_interaction_status(
                                        RunInteractionPolicy::FailIfHitlRequired
                                    ),
                                    phase_with_ordinal(key, page_index, page_total)
                                );
                            });
                            break;
                        }
                        RunInteractionPolicy::RequirePreApprovedDecision => {
                            let approval_decision: CmsApprovalDecision = ctx
                                .start_activity(
                                    AlegriaActivities::run_human_approval_wait_step,
                                    approval_input,
                                    db_opts(30),
                                )
                                .await?;
                            if approval_decision.decision != "approved" {
                                page_blocked = true;
                                ctx.state_mut(|s| {
                                    s.waiting_hitl = false;
                                    s.resume_requested = false;
                                    s.phase = format!(
                                        "{}:{}",
                                        blocked_interaction_status(
                                            RunInteractionPolicy::RequirePreApprovedDecision
                                        ),
                                        phase_with_ordinal(key, page_index, page_total)
                                    );
                                });
                                break;
                            }
                        }
                    }
                }
                SeoPhaseKey::CmsPublishApproved => {
                    let Some(draft_ref) = draft.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "draft_normalize_required",
                        )));
                    };
                    let Some(cms_requested_ref) = cms_requested.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "cms_request_required",
                        )));
                    };
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
                            AlegriaActivities::run_cms_publish_approved_step,
                            CmsPublishInputPayload {
                                run_id: run_id.to_string(),
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
                    let Some(cms_approved_ref) = cms_approved.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "cms_approved_output",
                        )));
                    };
                    if cms_approved_ref.verdict != "approved" {
                        page_blocked = true;
                        ctx.state_mut(|s| {
                            s.phase = format!(
                                "{}:{}",
                                blocked_publish_gate_status(),
                                phase_with_ordinal(key, page_index, page_total)
                            )
                        });
                        break;
                    }
                }
                SeoPhaseKey::PublishMaterialize => {
                    let Some(cms_requested_ref) = cms_requested.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "cms_request_required",
                        )));
                    };
                    materialized = Some(
                        ctx.start_activity(
                            AlegriaActivities::run_publish_materialize_step,
                            PublishMaterializeInputPayload {
                                run_id: run_id.to_string(),
                                page_node_key: page_node.page_node_key.clone(),
                                revision_id: cms_requested_ref.revision_id.clone(),
                                cms_document_id: cms_requested_ref.cms_document_id.clone(),
                                canonical_url_path: page_node.canonical_url_path.clone(),
                                publish_artifact: cms_requested_ref
                                    .publish_artifact
                                    .clone()
                                    .or(cms_approved.as_ref().and_then(|approved| {
                                        approved.publish_artifact.clone()
                                    })),
                                output_dir: String::new(),
                                base_url: String::new(),
                            },
                            db_opts(60),
                        )
                        .await?,
                    );
                    let Some(materialized_ref) = materialized.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "publish_materialize_output",
                        )));
                    };
                    if materialized_ref.materialization_status.contains("failed") {
                        page_blocked = true;
                        ctx.state_mut(|s| {
                            s.phase = format!(
                                "{}:{}",
                                blocked_publish_gate_status(),
                                phase_with_ordinal(key, page_index, page_total)
                            )
                        });
                        break;
                    }
                }
                SeoPhaseKey::RenderPreviewValidate => {
                    let Some(materialized_ref) = materialized.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "publish_materialize_required",
                        )));
                    };
                    let render_validation = ctx
                        .start_activity(
                            AlegriaActivities::run_render_preview_validate_step,
                            RenderPreviewValidateInputPayload {
                                run_id: run_id.to_string(),
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
                                phase_with_ordinal(key, page_index, page_total)
                            )
                        });
                        break;
                    }
                }
                SeoPhaseKey::FinalizePublish => {
                    let Some(cms_requested_ref) = cms_requested.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "cms_request_required",
                        )));
                    };
                    let Some(materialized_ref) = materialized.as_ref() else {
                        return Ok(PagePublishLoopOutcome::Blocked(block_execution_plan_invariant(
                            ctx,
                            run_id,
                            "publish_materialize_required",
                        )));
                    };
                    let render_validation = ctx
                        .start_activity(
                            AlegriaActivities::run_render_preview_validate_step,
                            RenderPreviewValidateInputPayload {
                                run_id: run_id.to_string(),
                                preview_pages: materialized_ref.preview_pages.clone(),
                            },
                            db_opts(30),
                        )
                        .await?;
                    let finalized = ctx
                        .start_activity(
                            AlegriaActivities::run_finalize_publish_step,
                            FinalizePublishInputPayload {
                                run_id: run_id.to_string(),
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
                                phase_with_ordinal(key, page_index, page_total)
                            )
                        });
                        break;
                    }
                }
                _ => {}
            }
            ctx.wait_condition(|s| !s.paused).await;
        }

        if page_blocked {
            continue;
        }
    }

    Ok(PagePublishLoopOutcome::Completed(published_pages))
}
