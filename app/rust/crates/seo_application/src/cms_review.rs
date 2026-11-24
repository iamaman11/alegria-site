use primitives::errors::DomainError;
use seo_domain::review_decision::{validate_human_reviewer, HumanReviewDecision};
use seo_ports::{
    CmsReviewDecisionOutcome, CmsReviewDecisionPort, CmsReviewDecisionRequest,
    SeoWorkflowControlPort,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyHumanReviewDecisionInput {
    pub page_node_key: String,
    pub revision_id: String,
    pub actor_role: String,
    pub decision: HumanReviewDecision,
    pub reason: String,
    pub source: String,
}

pub async fn apply_human_review_decision<R, W>(
    repo: &R,
    workflow: &W,
    request: &ApplyHumanReviewDecisionInput,
) -> Result<CmsReviewDecisionOutcome, DomainError>
where
    R: CmsReviewDecisionPort,
    W: SeoWorkflowControlPort,
{
    validate_human_reviewer(
        &request.page_node_key,
        &request.revision_id,
        &request.actor_role,
    )?;
    let outcome = repo
        .apply_human_review_decision(&CmsReviewDecisionRequest {
            page_node_key: request.page_node_key.clone(),
            revision_id: request.revision_id.clone(),
            actor_role: request.actor_role.clone(),
            decision: request.decision,
            reason: request.reason.clone(),
            source: request.source.clone(),
        })
        .await?;
    workflow.signal_resume(&outcome.workflow_id).await?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct FakeRepo {
        calls: AtomicUsize,
    }
    #[derive(Default)]
    struct FakeWorkflow {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl CmsReviewDecisionPort for FakeRepo {
        async fn apply_human_review_decision(
            &self,
            request: &CmsReviewDecisionRequest,
        ) -> Result<CmsReviewDecisionOutcome, DomainError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(CmsReviewDecisionOutcome {
                decision_key: format!("decision:{}", request.decision.as_str()),
                revision_id: "revision-1".to_string(),
                workflow_id: "workflow-1".to_string(),
            })
        }
    }

    #[async_trait]
    impl SeoWorkflowControlPort for FakeWorkflow {
        async fn signal_resume(&self, _workflow_id: &str) -> Result<(), DomainError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn cms_review_routes_through_ports() {
        let repo = FakeRepo::default();
        let workflow = FakeWorkflow::default();
        let result = apply_human_review_decision(
            &repo,
            &workflow,
            &ApplyHumanReviewDecisionInput {
                page_node_key: "page-1".to_string(),
                revision_id: "revision-1".to_string(),
                actor_role: "editor".to_string(),
                decision: HumanReviewDecision::Approved,
                reason: "ok".to_string(),
                source: "test".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.workflow_id, "workflow-1");
        assert_eq!(result.decision_key, "decision:approved");
        assert_eq!(repo.calls.load(Ordering::SeqCst), 1);
        assert_eq!(workflow.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cms_review_rejects_system_actor() {
        let repo = FakeRepo::default();
        let workflow = FakeWorkflow::default();
        let err = apply_human_review_decision(
            &repo,
            &workflow,
            &ApplyHumanReviewDecisionInput {
                page_node_key: "page-1".to_string(),
                revision_id: "revision-1".to_string(),
                actor_role: "seo_system".to_string(),
                decision: HumanReviewDecision::Approved,
                reason: String::new(),
                source: "test".to_string(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.class_str(), "validation_failure");
        assert_eq!(repo.calls.load(Ordering::SeqCst), 0);
        assert_eq!(workflow.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cms_review_rejects_blank_page_before_effects() {
        let repo = FakeRepo::default();
        let workflow = FakeWorkflow::default();
        let err = apply_human_review_decision(
            &repo,
            &workflow,
            &ApplyHumanReviewDecisionInput {
                page_node_key: " ".to_string(),
                revision_id: "revision-1".to_string(),
                actor_role: "editor".to_string(),
                decision: HumanReviewDecision::Approved,
                reason: String::new(),
                source: "test".to_string(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.class_str(), "validation_failure");
        assert_eq!(repo.calls.load(Ordering::SeqCst), 0);
        assert_eq!(workflow.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cms_review_requires_exact_revision_before_effects() {
        let repo = FakeRepo::default();
        let workflow = FakeWorkflow::default();
        let err = apply_human_review_decision(
            &repo,
            &workflow,
            &ApplyHumanReviewDecisionInput {
                page_node_key: "page-1".to_string(),
                revision_id: "  ".to_string(),
                actor_role: "editor".to_string(),
                decision: HumanReviewDecision::Approved,
                reason: String::new(),
                source: "test".to_string(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.class_str(), "validation_failure");
        assert!(err.to_string().contains("revision_id"));
        assert_eq!(repo.calls.load(Ordering::SeqCst), 0);
        assert_eq!(workflow.calls.load(Ordering::SeqCst), 0);
    }
}
