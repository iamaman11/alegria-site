use contracts::generated::alegria::temporal::v1::{
    CmsPublishInputPayload, CmsPublishOutputPayload, FinalizePublishInputPayload,
    FinalizePublishOutputPayload, PublishMaterializeInputPayload, PublishMaterializeOutputPayload,
    PublishArtifact, RenderPreviewValidateInputPayload, RenderPreviewValidateOutputPayload,
};
use primitives::errors::DomainError;
use seo_ports::{CmsApproval, CmsReviewPort, PublishArtifactRepository};

pub async fn run_cms_publish<P: CmsReviewPort>(
    port: &P,
    input: &CmsPublishInputPayload,
) -> Result<CmsPublishOutputPayload, DomainError> {
    let output = seo_steps::cms_publish_step::execute(input)?;
    port.persist_cms_publish_output(input, &output).await
}

pub async fn load_cms_approval_decision<P: CmsReviewPort>(
    port: &P,
    page_node_key: &str,
    revision_id: &str,
) -> Result<CmsApproval, DomainError> {
    port.load_latest_approval_decision(page_node_key, revision_id)
        .await?
        .ok_or_else(|| DomainError::ValidationFailure {
            message: "cms approval decision not found".to_string(),
        })
        .and_then(|approval| {
            if approval.page_node_key != page_node_key || approval.revision_id != revision_id {
                Err(DomainError::ContractViolation {
                    message: "CMS review port returned another page or revision".to_string(),
                })
            } else {
                Ok(approval)
            }
        })
}

pub async fn run_publish_materialize<P: PublishArtifactRepository>(
    repo: &P,
    input: &PublishMaterializeInputPayload,
) -> Result<PublishMaterializeOutputPayload, DomainError> {
    let resolved = repo.resolve_publish_materialize_input(input);
    let output = plan_publish_materialization(&resolved);
    repo.persist_publish_materialize_output(&resolved, &output)
        .await
}

/// Prepare a release artifact without touching the filesystem or deployment.
/// The domain owns the required-field policy; the application owns the
/// generated Temporal payload and deterministic artifact identity.
pub fn plan_publish_materialization(
    input: &PublishMaterializeInputPayload,
) -> PublishMaterializeOutputPayload {
    let blocking_reasons = seo_domain::publishing::materialization::MaterializationFields {
        page_node_key: &input.page_node_key,
        revision_id: &input.revision_id,
        canonical_url_path: &input.canonical_url_path,
        output_dir: &input.output_dir,
        base_url: &input.base_url,
    }
    .missing_required()
    .into_iter()
    .map(|missing| missing.reason_code().to_string())
    .collect::<Vec<_>>();
    let artifact = input.publish_artifact.clone().unwrap_or(PublishArtifact {
        artifact_key: seo_domain::artifact_identity::seo_artifact_key(
            "publish_artifact",
            &[&input.page_node_key, &input.revision_id, "headless_snapshot"],
        ),
        page_node_key: input.page_node_key.clone(),
        revision_id: input.revision_id.clone(),
        artifact_type: "headless_snapshot".to_string(),
        artifact_uri: String::new(),
        manifest_json: "{}".to_string(),
        status: "pending_materialization".to_string(),
    });
    PublishMaterializeOutputPayload {
        publish_artifact: Some(artifact),
        preview_pages: Vec::new(),
        materialization_status: if blocking_reasons.is_empty() {
            "ready_to_build".to_string()
        } else {
            "blocked".to_string()
        },
        blocking_reasons,
    }
}

pub fn run_render_preview_validate(
    input: &RenderPreviewValidateInputPayload,
) -> RenderPreviewValidateOutputPayload {
    seo_steps::render_preview_validate_step::execute(input)
}

pub async fn run_finalize_publish<P: PublishArtifactRepository>(
    repo: &P,
    input: &FinalizePublishInputPayload,
) -> Result<FinalizePublishOutputPayload, DomainError> {
    let output = plan_finalize_publish(input);
    repo.persist_finalize_publish_output(input, &output).await
}

/// Render readiness only plans a release candidate; public HTTPS readback
/// remains mandatory before a repository may report a published revision.
pub fn plan_finalize_publish(input: &FinalizePublishInputPayload) -> FinalizePublishOutputPayload {
    let validation = input.render_validation.clone().unwrap_or_default();
    let gate = seo_domain::publishing::ReleaseReadiness::from_render_validation(
        &validation.verdict,
        &validation.blocking_reasons,
    );
    FinalizePublishOutputPayload {
        verdict: gate.verdict().to_string(),
        current_status: gate.current_status().to_string(),
        blocking_reasons: validation.blocking_reasons,
        publish_artifact: input.publish_artifact.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use contracts::generated::alegria::temporal::v1::{
        DraftState, PageNodeState, PublishArtifact, RenderPreviewPageState,
    };
    use seo_ports::{CmsReviewPort, PublishArtifactRepository};

    #[test]
    fn missing_materialization_fields_keep_stable_temporal_reason_order() {
        let result = plan_publish_materialization(&PublishMaterializeInputPayload {
            page_node_key: " ".to_string(),
            revision_id: String::new(),
            canonical_url_path: String::new(),
            output_dir: String::new(),
            base_url: String::new(),
            ..Default::default()
        });
        assert_eq!(result.materialization_status, "blocked");
        assert_eq!(
            result.blocking_reasons,
            vec![
                "missing_page_node_key".to_string(),
                "missing_revision_id".to_string(),
                "missing_canonical_url_path".to_string(),
                "missing_output_dir".to_string(),
                "missing_base_url".to_string(),
            ]
        );
        let artifact = result.publish_artifact.unwrap();
        assert_eq!(
            artifact.artifact_key,
            seo_domain::artifact_identity::seo_artifact_key(
                "publish_artifact",
                &[" ", "", "headless_snapshot"],
            )
        );
        assert_eq!(artifact.artifact_type, "headless_snapshot");
        assert_eq!(artifact.status, "pending_materialization");
        assert_eq!(artifact.manifest_json, "{}");
    }

    #[test]
    fn preexisting_materialization_artifact_is_preserved() {
        let artifact = PublishArtifact {
            artifact_key: "unchanged-artifact".to_string(),
            page_node_key: "page-1".to_string(),
            revision_id: "rev-1".to_string(),
            ..Default::default()
        };
        let output = plan_publish_materialization(&PublishMaterializeInputPayload {
            page_node_key: "page-1".to_string(),
            revision_id: "rev-1".to_string(),
            canonical_url_path: "/visa/spain".to_string(),
            output_dir: "/out".to_string(),
            base_url: "https://example.test".to_string(),
            publish_artifact: Some(artifact.clone()),
            ..Default::default()
        });
        assert_eq!(output.materialization_status, "ready_to_build");
        assert_eq!(output.publish_artifact, Some(artifact));
    }

    #[test]
    fn finalize_never_reports_publication_before_public_readback() {
        let candidate = plan_finalize_publish(&FinalizePublishInputPayload {
            render_validation: Some(RenderPreviewValidateOutputPayload {
                verdict: "render_ready".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(candidate.verdict, "ready_for_release");
        assert_eq!(candidate.current_status, "approved");

        let blocked = plan_finalize_publish(&FinalizePublishInputPayload {
            render_validation: Some(RenderPreviewValidateOutputPayload {
                verdict: "render_ready".to_string(),
                blocking_reasons: vec!["missing_approval".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(blocked.verdict, "blocked");
        assert_eq!(blocked.current_status, "blocked");
        assert_eq!(blocked.blocking_reasons, vec!["missing_approval".to_string()]);
    }

    struct FakeReviewPort;

    struct WrongRevisionPort;

    #[async_trait]
    impl CmsReviewPort for FakeReviewPort {
        async fn persist_cms_publish_output(
            &self,
            _input: &CmsPublishInputPayload,
            output: &CmsPublishOutputPayload,
        ) -> Result<CmsPublishOutputPayload, DomainError> {
            Ok(output.clone())
        }

        async fn load_latest_approval_decision(
            &self,
            page_node_key: &str,
            revision_id: &str,
        ) -> Result<Option<CmsApproval>, DomainError> {
            Ok(Some(CmsApproval {
                decision_key: "decision-1".to_string(),
                page_node_key: page_node_key.to_string(),
                revision_id: revision_id.to_string(),
                actor_role: "editor".to_string(),
                decision: seo_ports::HumanReviewDecision::Approved,
                reason: "ok".to_string(),
                decided_at: String::new(),
            }))
        }
    }

    #[async_trait]
    impl CmsReviewPort for WrongRevisionPort {
        async fn persist_cms_publish_output(
            &self,
            _input: &CmsPublishInputPayload,
            output: &CmsPublishOutputPayload,
        ) -> Result<CmsPublishOutputPayload, DomainError> {
            Ok(output.clone())
        }

        async fn load_latest_approval_decision(
            &self,
            page_node_key: &str,
            _revision_id: &str,
        ) -> Result<Option<CmsApproval>, DomainError> {
            Ok(Some(CmsApproval {
                decision_key: "wrong-revision".to_string(),
                page_node_key: page_node_key.to_string(),
                revision_id: "another-revision".to_string(),
                actor_role: "editor".to_string(),
                decision: seo_ports::HumanReviewDecision::Approved,
                reason: String::new(),
                decided_at: String::new(),
            }))
        }
    }

    struct FakePublishRepo;

    #[async_trait]
    impl PublishArtifactRepository for FakePublishRepo {
        fn resolve_publish_materialize_input(
            &self,
            input: &PublishMaterializeInputPayload,
        ) -> PublishMaterializeInputPayload {
            let mut resolved = input.clone();
            if resolved.output_dir.trim().is_empty() {
                resolved.output_dir = "/configured/static-site".to_string();
            }
            if resolved.base_url.trim().is_empty() {
                resolved.base_url = "https://configured.example".to_string();
            }
            resolved
        }

        async fn persist_publish_materialize_output(
            &self,
            _input: &PublishMaterializeInputPayload,
            output: &PublishMaterializeOutputPayload,
        ) -> Result<PublishMaterializeOutputPayload, DomainError> {
            Ok(output.clone())
        }

        async fn persist_finalize_publish_output(
            &self,
            _input: &FinalizePublishInputPayload,
            output: &FinalizePublishOutputPayload,
        ) -> Result<FinalizePublishOutputPayload, DomainError> {
            Ok(output.clone())
        }
    }

    #[tokio::test]
    async fn review_publish_uses_review_port() {
        let port = FakeReviewPort;
        let output = run_cms_publish(
            &port,
            &CmsPublishInputPayload {
                run_id: "run-1".to_string(),
                publish_mode: "request_review".to_string(),
                actor_role: "seo_system".to_string(),
                page_node: Some(PageNodeState {
                    page_node_key: "page-1".to_string(),
                    canonical_url_path: "/visa/spain".to_string(),
                    ..Default::default()
                }),
                draft: Some(DraftState {
                    page_draft_key: "draft-1".to_string(),
                    qa_verdict: "publish_ready".to_string(),
                    body_markdown: "body".to_string(),
                    ..Default::default()
                }),
                approval_decision: None,
            },
        )
        .await
        .unwrap();
        assert!(!output.verdict.is_empty());
    }

    #[tokio::test]
    async fn review_publish_loads_approval_without_sql_adapter() {
        let port = FakeReviewPort;
        let decision = load_cms_approval_decision(&port, "page-1", "revision-1")
            .await
            .unwrap();
        assert!(decision.approves_revision("page-1", "revision-1"));
    }

    #[tokio::test]
    async fn review_publish_rejects_wrong_revision_from_replaceable_port() {
        let err = load_cms_approval_decision(&WrongRevisionPort, "page-1", "revision-1")
            .await
            .unwrap_err();
        assert_eq!(err.class_str(), "contract_violation");
    }

    #[tokio::test]
    async fn review_publish_resolves_runtime_before_materialize_gate() {
        let repo = FakePublishRepo;
        let output = run_publish_materialize(
            &repo,
            &PublishMaterializeInputPayload {
                run_id: "run-1".to_string(),
                page_node_key: "page-1".to_string(),
                revision_id: "rev-1".to_string(),
                canonical_url_path: "/visa/spain".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(output.materialization_status, "ready_to_build");
        assert!(output.blocking_reasons.is_empty());
    }

    #[tokio::test]
    async fn review_publish_finalize_uses_publish_repo() {
        let repo = FakePublishRepo;
        let output = run_finalize_publish(
            &repo,
            &FinalizePublishInputPayload {
                run_id: "run-1".to_string(),
                page_node_key: "page-1".to_string(),
                revision_id: "rev-1".to_string(),
                render_validation: Some(RenderPreviewValidateOutputPayload {
                    verdict: "render_ready".to_string(),
                    blocking_reasons: Vec::new(),
                    blockers: Vec::new(),
                }),
                publish_artifact: Some(PublishArtifact {
                    artifact_key: "artifact-1".to_string(),
                    page_node_key: "page-1".to_string(),
                    revision_id: "rev-1".to_string(),
                    artifact_type: "headless_snapshot".to_string(),
                    artifact_uri: "/tmp".to_string(),
                    manifest_json: "{}".to_string(),
                    status: "built".to_string(),
                }),
            },
        )
        .await
        .unwrap();
        // The fake port cannot prove public HTTPS readback; it receives only
        // the release candidate, never a false publication success.
        assert_eq!(output.verdict, "ready_for_release");
        assert_eq!(output.current_status, "approved");
    }

    #[test]
    fn review_publish_render_validate_stays_pure() {
        let output = run_render_preview_validate(&RenderPreviewValidateInputPayload {
            run_id: "run-1".to_string(),
            preview_pages: vec![RenderPreviewPageState {
                page_node_key: "page-1".to_string(),
                revision_id: "rev-1".to_string(),
                canonical_url_path: "/visa/spain".to_string(),
                rendered_html: "<!doctype html><html lang=\"en\"><head><title>Spain visa</title><meta name=\"description\" content=\"Spain visa guide\"><link rel=\"canonical\" href=\"https://example.com/visa/spain\"><script type=\"application/ld+json\">{}</script></head><body><nav class=\"breadcrumbs\"></nav><h1>Spain visa</h1></body></html>".to_string(),
                has_breadcrumbs: true,
                has_schema_markup: true,
                required_link_count: 0,
                rendered_link_count: 0,
            }],
        });
        assert_eq!(output.verdict, "render_ready");
    }
}
