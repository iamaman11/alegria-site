use contracts::generated::alegria::temporal::v1::{
    CmsApprovalDecision, CmsPublishInputPayload, CmsPublishOutputPayload,
    FinalizePublishInputPayload, FinalizePublishOutputPayload, PublishMaterializeInputPayload,
    PublishMaterializeOutputPayload, RenderPreviewValidateInputPayload,
    RenderPreviewValidateOutputPayload,
};
use primitives::errors::DomainError;
use seo_ports::{CmsReviewPort, PublishArtifactRepository};

pub async fn run_cms_publish<P: CmsReviewPort>(
    port: &P,
    input: &CmsPublishInputPayload,
) -> Result<CmsPublishOutputPayload, DomainError> {
    let output = seo_steps::cms_publish_step::execute(input);
    port.persist_cms_publish_output(input, &output).await
}

pub async fn load_cms_approval_decision<P: CmsReviewPort>(
    port: &P,
    page_node_key: &str,
    revision_id: &str,
) -> Result<CmsApprovalDecision, DomainError> {
    port.load_latest_approval_decision(page_node_key, revision_id)
        .await?
        .ok_or_else(|| DomainError::ValidationFailure {
            message: "cms approval decision not found".to_string(),
        })
}

pub async fn run_publish_materialize<P: PublishArtifactRepository>(
    repo: &P,
    input: &PublishMaterializeInputPayload,
) -> Result<PublishMaterializeOutputPayload, DomainError> {
    let output = seo_steps::publish_materialize_step::execute(input);
    repo.persist_publish_materialize_output(input, &output)
        .await
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
    let output = seo_steps::finalize_publish_step::execute(input);
    repo.persist_finalize_publish_output(input, &output).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use contracts::generated::alegria::temporal::v1::{
        DraftState, PageNodeState, PublishArtifact, RenderPreviewPageState,
    };
    use seo_ports::{CmsReviewPort, PublishArtifactRepository};

    struct FakeReviewPort;

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
        ) -> Result<Option<CmsApprovalDecision>, DomainError> {
            Ok(Some(CmsApprovalDecision {
                decision_key: "decision-1".to_string(),
                page_node_key: page_node_key.to_string(),
                revision_id: revision_id.to_string(),
                actor_role: "editor".to_string(),
                decision: "approved".to_string(),
                reason: "ok".to_string(),
                decided_at: String::new(),
            }))
        }
    }

    struct FakePublishRepo;

    #[async_trait]
    impl PublishArtifactRepository for FakePublishRepo {
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
        assert_eq!(decision.decision, "approved");
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
        assert_eq!(output.verdict, "published");
    }

    #[test]
    fn review_publish_render_validate_stays_pure() {
        let output = run_render_preview_validate(&RenderPreviewValidateInputPayload {
            run_id: "run-1".to_string(),
            preview_pages: vec![RenderPreviewPageState {
                page_node_key: "page-1".to_string(),
                revision_id: "rev-1".to_string(),
                canonical_url_path: "/visa/spain".to_string(),
                rendered_html: "<html><link rel=\"canonical\"/><nav class=\"breadcrumbs\"></nav><script type=\"application/ld+json\">{}</script></html>".to_string(),
                has_breadcrumbs: true,
                has_schema_markup: true,
                required_link_count: 0,
                rendered_link_count: 0,
            }],
        });
        assert_eq!(output.verdict, "render_ready");
    }
}
