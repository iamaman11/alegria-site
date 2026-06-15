async fn run_page_phase<
    R: DraftRepository
        + SectionTemplateRepository
        + SourceContextRepository
        + EditorialGenerationPort
        + CmsReviewPort
        + PublishArtifactRepository
        + GraphCapabilityPort,
>(
    repo: &R,
    request: &SeoScenarioRequest,
    page_node: &contracts::generated::alegria::temporal::v1::PageNodeState,
    page_blueprint: &contracts::generated::alegria::temporal::v1::PageBlueprintState,
    required_links: &[contracts::generated::alegria::temporal::v1::LinkRecommendationState],
    factual_fragments: &[String],
    verified_support: &[contracts::generated::alegria::temporal::v1::SeoVerifiedFactSupportState],
    page_state: &mut PageState,
    key: SeoPhaseKey,
    phase_reports: &mut Vec<SeoPhaseReport>,
) -> Result<Option<String>, DomainError> {
    let run_id = request.site_input.run_id.clone();
    match key {
        SeoPhaseKey::DraftAssemble => {
            let output = drafting::run_draft_assemble(
                repo,
                &DraftAssembleInputPayload {
                    run_id,
                    page_node: Some(page_node.clone()),
                    page_blueprint: Some(page_blueprint.clone()),
                    factual_fragments: factual_fragments.to_vec(),
                    verified_support: verified_support.to_vec(),
                    required_links: required_links.to_vec(),
                    section_templates: Vec::new(),
                    llm_candidate: None,
                    source_context_chunks: Vec::new(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                &page_node.page_node_key,
                "assembled",
            ));
            page_state.draft_plan = Some(output);
            Ok(None)
        }
        SeoPhaseKey::EditorialDraftGenerate => {
            let draft_plan = require_state_ref(&page_state.draft_plan, "draft_assemble_required")?;
            let output = drafting::run_editorial_draft_generate(
                repo,
                &EditorialDraftGenerateInputPayload {
                    run_id,
                    request: draft_plan.llm_request.clone(),
                    provider_policy: "multi_provider:first_available".to_string(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.status,
                &page_node.page_node_key,
                output.provider_key.clone(),
            ));
            page_state.editorial_candidate = Some(output);
            Ok(None)
        }
        SeoPhaseKey::DraftNormalize => {
            let draft_plan = require_state_ref(&page_state.draft_plan, "draft_assemble_required")?;
            let editorial_candidate = require_state_ref(
                &page_state.editorial_candidate,
                "editorial_draft_generate_required",
            )?;
            let output = drafting::run_draft_normalize(
                repo,
                &DraftNormalizeInputPayload {
                    run_id,
                    page_node: Some(page_node.clone()),
                    page_blueprint: Some(page_blueprint.clone()),
                    factual_fragments: factual_fragments.to_vec(),
                    verified_support: verified_support.to_vec(),
                    required_links: required_links.to_vec(),
                    section_templates: draft_plan
                        .editorial_brief
                        .as_ref()
                        .map(|brief| brief.section_templates.clone())
                        .unwrap_or_default(),
                    content_block_plan: draft_plan.content_block_plan.clone(),
                    llm_candidate: editorial_candidate.candidate.clone(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                &page_node.page_node_key,
                "normalized",
            ));
            page_state.draft = Some(output);
            Ok(None)
        }
        SeoPhaseKey::ContentContractValidate => {
            let draft = require_state_ref(&page_state.draft, "draft_normalize_required")?;
            let _output = drafting::run_content_contract_validate(
                repo,
                &ContentContractValidateInputPayload {
                    run_id,
                    draft: draft.draft.clone(),
                    required_links: required_links.to_vec(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                "done",
                &page_node.page_node_key,
                "validated",
            ));
            Ok(None)
        }
        SeoPhaseKey::DraftQa => {
            let draft = require_state_ref(&page_state.draft, "draft_normalize_required")?;
            let output = drafting::run_draft_qa(
                repo,
                &DraftQaInputPayload {
                    run_id,
                    draft: draft.draft.clone(),
                    supported_fragments: factual_fragments.to_vec(),
                    required_links: required_links.to_vec(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.verdict,
                &page_node.page_node_key,
                "qa_complete",
            ));
            if let Some(draft_state) = page_state
                .draft
                .as_mut()
                .and_then(|state| state.draft.as_mut())
            {
                draft_state.qa_verdict = output.verdict.clone();
            }
            page_state.qa = Some(output);
            Ok(None)
        }
        SeoPhaseKey::CmsRequestReview => {
            let draft = require_state_ref(&page_state.draft, "draft_normalize_required")?;
            let output = review_publish::run_cms_publish(
                repo,
                &CmsPublishInputPayload {
                    run_id,
                    page_node: Some(page_node.clone()),
                    draft: draft.draft.clone(),
                    actor_role: "seo_system".to_string(),
                    publish_mode: "request_review".to_string(),
                    approval_decision: None,
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.verdict,
                &page_node.page_node_key,
                output.blocking_reasons.join(","),
            ));
            if output.verdict != "review_requested" {
                return Ok(Some(blocked_publish_gate_status().to_string()));
            }
            page_state.cms_requested = Some(output);
            Ok(None)
        }
        SeoPhaseKey::HumanApprovalWait => {
            let cms_requested =
                require_state_ref(&page_state.cms_requested, "cms_request_review_required")?;
            match request.policy.interaction {
                RunInteractionPolicy::AllowHitlPause | RunInteractionPolicy::FailIfHitlRequired => {
                    let status = blocked_interaction_status(request.policy.interaction).to_string();
                    phase_reports.push(phase_report(
                        phase_label(key),
                        &status,
                        &page_node.page_node_key,
                        cms_requested.revision_id.clone(),
                    ));
                    Ok(Some(status))
                }
                RunInteractionPolicy::RequirePreApprovedDecision => {
                    match review_publish::load_cms_approval_decision(
                        repo,
                        &page_node.page_node_key,
                        &cms_requested.revision_id,
                    )
                    .await
                    {
                        Ok(decision) if decision.decision == "approved" => {
                            phase_reports.push(phase_report(
                                phase_label(key),
                                "approved",
                                &page_node.page_node_key,
                                decision.revision_id.clone(),
                            ));
                            Ok(None)
                        }
                        Ok(decision) => {
                            let status =
                                blocked_interaction_status(request.policy.interaction).to_string();
                            phase_reports.push(phase_report(
                                phase_label(key),
                                &status,
                                &page_node.page_node_key,
                                decision.decision,
                            ));
                            Ok(Some(status))
                        }
                        Err(_) => {
                            let status =
                                blocked_interaction_status(request.policy.interaction).to_string();
                            phase_reports.push(phase_report(
                                phase_label(key),
                                &status,
                                &page_node.page_node_key,
                                "missing_approval_decision",
                            ));
                            Ok(Some(status))
                        }
                    }
                }
            }
        }
        SeoPhaseKey::CmsPublishApproved => {
            let draft = require_state_ref(&page_state.draft, "draft_normalize_required")?;
            let cms_requested =
                require_state_ref(&page_state.cms_requested, "cms_request_review_required")?;
            let approval = review_publish::load_cms_approval_decision(
                repo,
                &page_node.page_node_key,
                &cms_requested.revision_id,
            )
            .await
            .ok();
            let output = review_publish::run_cms_publish(
                repo,
                &CmsPublishInputPayload {
                    run_id,
                    page_node: Some(page_node.clone()),
                    draft: draft.draft.clone(),
                    actor_role: "seo_system".to_string(),
                    publish_mode: "approved_publish".to_string(),
                    approval_decision: approval,
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.verdict,
                &page_node.page_node_key,
                output.blocking_reasons.join(","),
            ));
            if output.verdict != "approved" {
                return Ok(Some(blocked_publish_gate_status().to_string()));
            }
            page_state.cms_approved = Some(output);
            Ok(None)
        }
        SeoPhaseKey::PublishMaterialize => {
            let cms_requested =
                require_state_ref(&page_state.cms_requested, "cms_request_review_required")?;
            let output = review_publish::run_publish_materialize(
                repo,
                &PublishMaterializeInputPayload {
                    run_id,
                    page_node_key: page_node.page_node_key.clone(),
                    revision_id: cms_requested.revision_id.clone(),
                    cms_document_id: cms_requested.cms_document_id.clone(),
                    canonical_url_path: page_node.canonical_url_path.clone(),
                    publish_artifact: cms_requested.publish_artifact.clone().or(page_state
                        .cms_approved
                        .as_ref()
                        .and_then(|approved| approved.publish_artifact.clone())),
                    output_dir: request.output_dir.clone(),
                    base_url: request.base_url.clone(),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.materialization_status,
                &page_node.page_node_key,
                output.blocking_reasons.join(","),
            ));
            if output.materialization_status.contains("failed") {
                return Ok(Some(blocked_publish_gate_status().to_string()));
            }
            page_state.materialized = Some(output);
            Ok(None)
        }
        SeoPhaseKey::RenderPreviewValidate => {
            let materialized =
                require_state_ref(&page_state.materialized, "publish_materialize_required")?;
            let output =
                review_publish::run_render_preview_validate(&RenderPreviewValidateInputPayload {
                    run_id,
                    preview_pages: materialized.preview_pages.clone(),
                });
            phase_reports.push(phase_report(
                phase_label(key),
                &output.verdict,
                &page_node.page_node_key,
                output.blocking_reasons.join(","),
            ));
            if output.verdict != "render_ready" {
                return Ok(Some(blocked_publish_gate_status().to_string()));
            }
            Ok(None)
        }
        SeoPhaseKey::FinalizePublish => {
            let cms_requested =
                require_state_ref(&page_state.cms_requested, "cms_request_review_required")?;
            let materialized =
                require_state_ref(&page_state.materialized, "publish_materialize_required")?;
            let render_validation =
                review_publish::run_render_preview_validate(&RenderPreviewValidateInputPayload {
                    run_id: run_id.clone(),
                    preview_pages: materialized.preview_pages.clone(),
                });
            let output = review_publish::run_finalize_publish(
                repo,
                &FinalizePublishInputPayload {
                    run_id,
                    page_node_key: page_node.page_node_key.clone(),
                    revision_id: cms_requested.revision_id.clone(),
                    publish_artifact: materialized.publish_artifact.clone(),
                    render_validation: Some(render_validation),
                },
            )
            .await?;
            phase_reports.push(phase_report(
                phase_label(key),
                &output.verdict,
                &page_node.page_node_key,
                output
                    .publish_artifact
                    .as_ref()
                    .map(|artifact| artifact.artifact_uri.clone())
                    .unwrap_or_default(),
            ));
            if output.verdict != "published" {
                return Ok(Some(blocked_publish_gate_status().to_string()));
            }
            Ok(None)
        }
        _ => unreachable!("invalid page phase"),
    }
}

