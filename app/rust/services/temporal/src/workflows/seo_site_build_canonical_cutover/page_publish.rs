enum PagePublishLoopOutcome {
    Completed(usize),
    Blocked {
        status: String,
        published_pages: usize,
    },
}

impl PagePublishLoopOutcome {
    fn blocked(status: String, published_pages: usize) -> Self {
        Self::Blocked {
            status,
            published_pages,
        }
    }
}

// A blocked editorial revision must not trap its business run at the same
// immutable approval gate. A new draft belongs to a new, auditable run; a
// reopened review is the only non-approval decision that keeps this gate open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorialReviewDisposition {
    Approved,
    ReworkRequired,
    Wait,
}

fn editorial_review_disposition(
    decision: &str,
    terminal_rework_patch: bool,
) -> EditorialReviewDisposition {
    match decision {
        "approved" => EditorialReviewDisposition::Approved,
        "blocked" if terminal_rework_patch => EditorialReviewDisposition::ReworkRequired,
        _ => EditorialReviewDisposition::Wait,
    }
}

async fn run_page_publish_phases(
    ctx: &mut WorkflowContext<SeoSiteBuildCanonicalCutoverWorkflow>,
    run_id: &str,
    plan: &SeoExecutionPlan,
    page_nodes: &[PageNodeState],
    page_blueprints: &[PageBlueprintState],
    link_recommendations: &[LinkRecommendationState],
    content_gaps: &[contracts::generated::alegria::temporal::v1::ContentGapState],
    support_bundle: &[SeoVerifiedFactSupportState],
    factual_fragments: &[String],
    context_key: &str,
    applicant_profile: &str,
    scope: &SeoScopePayload,
    raw_page_ids: &[i64],
) -> WorkflowResult<PagePublishLoopOutcome> {
    use runtime_models::seo_blocks::{page_admissibility, PageAdmissibility};
    let active_pages = page_nodes
        .iter()
        .filter(|node| node.lifecycle_state != "deprecated")
        .collect::<Vec<_>>();
    let page_total = active_pages.len();
    if page_total == 0 {
        return Ok(PagePublishLoopOutcome::Blocked {
            status: block_execution_plan_invariant(ctx, run_id, "no_publishable_pages"),
            published_pages: 0,
        });
    }
    let page_keys = page_phase_keys(plan);
    let requires_editorial = page_keys.contains(&SeoPhaseKey::EditorialDraftGenerate);
    let mut published_pages = 0usize;
    let mut published_page_keys = std::collections::BTreeSet::new();
    let mut first_blocked: Option<String> = None;
    let mut published_before_block = 0usize;

    for (page_index, page_node) in active_pages.into_iter().cloned().enumerate() {
        let page_blueprint = page_blueprints
            .iter()
            .find(|blueprint| blueprint.blueprint_key == page_node.blueprint_key)
            .cloned()
            .unwrap_or_default();
        let page_support =
            runtime_models::seo_blocks::page_verified_support(&page_blueprint, support_bundle);
        match page_admissibility(
            &page_node,
            &page_blueprint,
            &page_support,
            content_gaps,
            requires_editorial,
        ) {
            PageAdmissibility::Retired => continue,
            PageAdmissibility::Blocked(detail) => {
                let reason = format!("{detail}:page_node_key={}", page_node.page_node_key);
                ctx.state_mut(|state| state.phase = reason.clone());
                if first_blocked.is_none() {
                    published_before_block = published_pages;
                    first_blocked = Some(reason);
                }
                continue;
            }
            PageAdmissibility::Eligible => {}
        }
        // A target is linkable only after this run has actually finalized it.
        let page_fragments = factual_fragments
            .iter()
            .filter(|text| {
                page_support
                    .iter()
                    .any(|support| support.fragment_text.as_str() == text.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        let support_bundle = page_support.as_slice();
        let factual_fragments = page_fragments.as_slice();
        let required_links = runtime_models::seo_blocks::required_links_for_published_targets(
            link_recommendations,
            &page_node.page_node_key,
            &published_page_keys,
        );

        let mut draft_plan: Option<DraftAssembleOutputPayload> = None;
        let mut editorial_candidate: Option<EditorialDraftGenerateOutputPayload> = None;
        let mut draft: Option<DraftNormalizeOutputPayload> = None;
        let mut cms_requested: Option<CmsPublishOutputPayload> = None;
        let mut cms_approved: Option<CmsPublishOutputPayload> = None;
        let mut materialized: Option<PublishMaterializeOutputPayload> = None;
        let mut page_blocked = false;
        let mut editorial_rework_required = false;

        'page_phases: for key in page_keys.iter().copied() {
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
                    // page_brief is created and queued by draft_assemble, not by
                    // site IA. Materialize and check it before editorial work or
                    // approval/publishing; use a unique durable step per page.
                    if plan.policy.projection != RunProjectionPolicy::WarnOnly {
                        let sync_step = format!("neo4j_sync_post_brief_{}", page_index + 1);
                        ctx.state_mut(|s| s.phase = sync_step.clone());
                        let brief_sync = sync_target(ctx, run_id, &sync_step, "neo4j").await?;
                        ensure_sync_drained(&brief_sync, &sync_step)?;
                        ctx.wait_condition(|s| !s.paused).await?;
                        ctx.state_mut(|s| {
                            s.phase = format!("seo_materialization_gate:page:{}", page_index + 1)
                        });
                        ctx.start_activity(
                            AlegriaActivities::run_seo_preflight_step,
                            SeoPreflightInput {
                                run_id: run_id.to_string(),
                                context_key: context_key.to_string(),
                                scope: scope.clone(),
                                raw_page_ids: raw_page_ids.to_vec(),
                                projection_max_lag_ms: 300_000,
                                require_materialized_projections: true,
                                enforce_required_contract: true,
                                require_page_brief_projection: true,
                                gate_key: format!("page:{}", page_index + 1),
                                skip_provider_probes: true,
                            },
                            db_opts(30),
                        )
                        .await?;
                        ctx.wait_condition(|s| !s.paused).await?;
                    }
                }
                SeoPhaseKey::EditorialDraftGenerate => {
                    let Some(draft_plan_ref) = draft_plan.as_ref() else {
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(ctx, run_id, "draft_assemble_required"),
                            published_pages,
                        ));
                    };
                    let request = EditorialDraftGenerateInputPayload {
                        run_id: run_id.to_string(),
                        request: draft_plan_ref.llm_request.clone(),
                        provider_policy: "multi_provider:first_available".to_string(),
                    };
                    let mut generated: EditorialDraftGenerateOutputPayload = ctx
                        .start_activity(
                            AlegriaActivities::run_editorial_draft_generate,
                            request.clone(),
                            db_opts(60),
                        )
                        .await?;
                    if generated.status == "awaiting_manual" {
                        // The activity has committed the manual task in PostgreSQL.
                        // A signal only wakes us; a separate activity reads and
                        // revalidates that this precise request is completed.
                        ctx.state_mut(|s| {
                            s.phase = format!("llm_wait:page:{}/{}", page_index + 1, page_total);
                        });
                        loop {
                            // The durable LLM task is authoritative. A result
                            // saved before Reset/Replay must not require a new
                            // signal, and a signal is never evidence of success.
                            generated = ctx
                                .start_activity(
                                    AlegriaActivities::load_manual_editorial_result,
                                    request.clone(),
                                    db_opts(30),
                                )
                                .await?;
                            if generated.status == "done" {
                                break;
                            }
                            if generated.status != "awaiting_manual" {
                                return Ok(PagePublishLoopOutcome::blocked(
                                    block_execution_plan_invariant(
                                        ctx,
                                        run_id,
                                        "manual_llm_result_invalid",
                                    ),
                                    published_pages,
                                ));
                            }
                            ctx.wait_condition(|s| !s.paused && s.llm_result_ready)
                                .await?;
                            ctx.state_mut(|s| s.llm_result_ready = false);
                        }
                        ctx.state_mut(|s| {
                            s.phase = phase_with_ordinal(key, page_index, page_total)
                        });
                    }
                    if generated.candidate.is_none() {
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(
                                ctx,
                                run_id,
                                "editorial_llm_candidate_missing",
                            ),
                            published_pages,
                        ));
                    }
                    editorial_candidate = Some(generated);
                }
                SeoPhaseKey::DraftNormalize => {
                    let Some(draft_plan_ref) = draft_plan.as_ref() else {
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(ctx, run_id, "draft_assemble_required"),
                            published_pages,
                        ));
                    };
                    let Some(editorial_ref) = editorial_candidate.as_ref() else {
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(
                                ctx,
                                run_id,
                                "editorial_generation_required",
                            ),
                            published_pages,
                        ));
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
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(ctx, run_id, "draft_normalize_required"),
                            published_pages,
                        ));
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
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(ctx, run_id, "draft_normalize_required"),
                            published_pages,
                        ));
                    };
                    let qa = ctx
                        .start_activity(
                            AlegriaActivities::run_draft_qa_step,
                            DraftQaInputPayload {
                                run_id: run_id.to_string(),
                                draft: draft_ref.draft.clone(),
                                supported_fragments: factual_fragments.to_vec(),
                                required_links: required_links.clone(),
                                verified_support: support_bundle.to_vec(),
                            },
                            db_opts(30),
                        )
                        .await?;
                    if let Some(draft_state) = draft.as_mut().and_then(|state| state.draft.as_mut())
                    {
                        draft_state.qa_verdict = qa.verdict;
                    }
                }
                SeoPhaseKey::CmsRequestReview => {
                    let Some(draft_ref) = draft.as_ref() else {
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(ctx, run_id, "draft_normalize_required"),
                            published_pages,
                        ));
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
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(
                                ctx,
                                run_id,
                                "cms_request_review_output",
                            ),
                            published_pages,
                        ));
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
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(ctx, run_id, "cms_request_required"),
                            published_pages,
                        ));
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
                                ctx.wait_condition(|s| !s.paused && s.resume_requested)
                                    .await?;
                                let approval_decision: CmsApprovalDecision = ctx
                                    .start_activity(
                                        AlegriaActivities::run_human_approval_wait_step,
                                        approval_input.clone(),
                                        db_opts(30),
                                    )
                                    .await?;
                                // Temporal records a patch marker for new histories.
                                // Old histories replay the original wait behavior.
                                let terminal_rework_patch =
                                    ctx.patched("seo-editorial-terminal-rework-v1");
                                match editorial_review_disposition(
                                    &approval_decision.decision,
                                    terminal_rework_patch,
                                ) {
                                    EditorialReviewDisposition::Approved => break,
                                    EditorialReviewDisposition::ReworkRequired => {
                                        page_blocked = true;
                                        editorial_rework_required = true;
                                        ctx.state_mut(|s| {
                                            s.waiting_hitl = false;
                                            s.resume_requested = false;
                                            s.paused = false;
                                            s.phase = format!(
                                                "blocked:editorial_rework_required:{}/{}",
                                                page_index + 1,
                                                page_total
                                            );
                                        });
                                        break 'page_phases;
                                    }
                                    EditorialReviewDisposition::Wait => {}
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
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(ctx, run_id, "draft_normalize_required"),
                            published_pages,
                        ));
                    };
                    let Some(cms_requested_ref) = cms_requested.as_ref() else {
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(ctx, run_id, "cms_request_required"),
                            published_pages,
                        ));
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
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(ctx, run_id, "cms_approved_output"),
                            published_pages,
                        ));
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
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(ctx, run_id, "cms_request_required"),
                            published_pages,
                        ));
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
                    let Some(materialized_ref) = materialized.as_ref() else {
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(
                                ctx,
                                run_id,
                                "publish_materialize_output",
                            ),
                            published_pages,
                        ));
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
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(
                                ctx,
                                run_id,
                                "publish_materialize_required",
                            ),
                            published_pages,
                        ));
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
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(ctx, run_id, "cms_request_required"),
                            published_pages,
                        ));
                    };
                    let Some(materialized_ref) = materialized.as_ref() else {
                        return Ok(PagePublishLoopOutcome::blocked(
                            block_execution_plan_invariant(
                                ctx,
                                run_id,
                                "publish_materialize_required",
                            ),
                            published_pages,
                        ));
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
                        published_page_keys.insert(page_node.page_node_key.clone());
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
            ctx.wait_condition(|s| !s.paused).await?;
        }

        if page_blocked {
            let reason = if editorial_rework_required {
                format!(
                    "blocked:editorial_rework_required:page_node_key={}",
                    page_node.page_node_key
                )
            } else {
                format!(
                    "{}:page_node_key={}",
                    blocked_publish_gate_status(),
                    page_node.page_node_key,
                )
            };
            if first_blocked.is_none() {
                published_before_block = published_pages;
                first_blocked = Some(reason);
            }
            if editorial_rework_required {
                break;
            }
        }
    }

    if let Some(reason) = first_blocked {
        return Ok(PagePublishLoopOutcome::Blocked {
            status: format!(
                "{reason}:published_before_block={published_before_block}:published_pages={published_pages}",
            ),
            published_pages,
        });
    }
    Ok(PagePublishLoopOutcome::Completed(published_pages))
}

#[cfg(test)]
mod editorial_review_disposition_tests {
    use super::*;

    #[test]
    fn approved_revision_may_continue_to_publication() {
        assert_eq!(
            editorial_review_disposition("approved", true),
            EditorialReviewDisposition::Approved
        );
    }

    #[test]
    fn blocked_revision_ends_the_attempt_for_rework() {
        assert_eq!(
            editorial_review_disposition("blocked", true),
            EditorialReviewDisposition::ReworkRequired
        );
    }

    #[test]
    fn reopened_or_unknown_decision_must_not_publish_or_end_review() {
        for decision in ["reopened", "", "unexpected"] {
            assert_eq!(
                editorial_review_disposition(decision, true),
                EditorialReviewDisposition::Wait
            );
        }
    }

    #[test]
    fn old_history_without_patch_still_waits_on_blocked_decision() {
        assert_eq!(
            editorial_review_disposition("blocked", false),
            EditorialReviewDisposition::Wait
        );
    }
}
