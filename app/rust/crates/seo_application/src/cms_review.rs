use primitives::errors::DomainError;
use seo_ports::{CmsReviewDecisionPort, CmsReviewDecisionRequest, SeoWorkflowControlPort};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyHumanReviewDecisionInput {
    pub page_node_key: String,
    pub actor_role: String,
    pub decision: String,
    pub reason: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyHumanReviewDecisionResult {
    pub decision_key: String,
    pub revision_id: String,
    pub workflow_id: String,
}

fn validate_request(request: &ApplyHumanReviewDecisionInput) -> Result<(), DomainError> {
    if request.page_node_key.trim().is_empty() {
        return Err(DomainError::ValidationFailure {
            message: "page_node_key is required".to_string(),
        });
    }
    if request.actor_role.trim().is_empty() || request.actor_role == "seo_system" {
        return Err(DomainError::ValidationFailure {
            message: "human actor_role is required".to_string(),
        });
    }
    match request.decision.as_str() {
        "approved" | "blocked" | "reopened" => {}
        other => {
            return Err(DomainError::ValidationFailure {
                message: format!("unsupported decision: {other}"),
            });
        }
    }
    Ok(())
}

pub async fn apply_human_review_decision<R, W>(
    repo: &R,
    workflow: &W,
    request: &ApplyHumanReviewDecisionInput,
) -> Result<ApplyHumanReviewDecisionResult, DomainError>
where
    R: CmsReviewDecisionPort,
    W: SeoWorkflowControlPort,
{
    validate_request(request)?;
    let outcome = repo
        .apply_human_review_decision(&CmsReviewDecisionRequest {
            page_node_key: request.page_node_key.clone(),
            actor_role: request.actor_role.clone(),
            decision: request.decision.clone(),
            reason: request.reason.clone(),
            source: request.source.clone(),
        })
        .await?;
    workflow.signal_resume(&outcome.workflow_id).await?;
    Ok(ApplyHumanReviewDecisionResult {
        decision_key: outcome.decision_key,
        revision_id: outcome.revision_id,
        workflow_id: outcome.workflow_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct FakeRepo;
    struct FakeWorkflow;

    #[async_trait]
    impl CmsReviewDecisionPort for FakeRepo {
        async fn apply_human_review_decision(
            &self,
            request: &CmsReviewDecisionRequest,
        ) -> Result<seo_ports::CmsReviewDecisionOutcome, DomainError> {
            Ok(seo_ports::CmsReviewDecisionOutcome {
                decision_key: format!("decision:{}", request.decision),
                revision_id: "revision-1".to_string(),
                workflow_id: "workflow-1".to_string(),
            })
        }
    }

    #[async_trait]
    impl SeoWorkflowControlPort for FakeWorkflow {
        async fn signal_resume(&self, _workflow_id: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn cms_review_routes_through_ports() {
        let result = apply_human_review_decision(
            &FakeRepo,
            &FakeWorkflow,
            &ApplyHumanReviewDecisionInput {
                page_node_key: "page-1".to_string(),
                actor_role: "editor".to_string(),
                decision: "approved".to_string(),
                reason: "ok".to_string(),
                source: "test".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.workflow_id, "workflow-1");
    }

    #[tokio::test]
    async fn cms_review_rejects_system_actor() {
        let err = apply_human_review_decision(
            &FakeRepo,
            &FakeWorkflow,
            &ApplyHumanReviewDecisionInput {
                page_node_key: "page-1".to_string(),
                actor_role: "seo_system".to_string(),
                decision: "approved".to_string(),
                reason: String::new(),
                source: "test".to_string(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.class_str(), "validation_failure");
    }
}
