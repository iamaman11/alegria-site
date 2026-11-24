use crate::seo_identity;
use contracts::generated::alegria::temporal::v1::{SeoPublishBlockerState, SeoScopePayload};
use primitives::errors::DomainError;
use seo_domain::artifact_identity;

pub fn scope_signature(scope: Option<&SeoScopePayload>) -> Result<String, DomainError> {
    seo_identity::scope_signature_from_payload(scope)
}

pub fn artifact_key(prefix: &str, parts: &[&str]) -> String {
    artifact_identity::seo_artifact_key(prefix, parts)
}

pub fn score(value: f64) -> f64 {
    artifact_identity::clamp_score(value)
}

/// One Temporal representation of a domain-owned publication blocker.
pub fn publish_blocker(reason: &str, task: &str, action: &str) -> SeoPublishBlockerState {
    SeoPublishBlockerState {
        reason_code: reason.to_string(),
        task_type: task.to_string(),
        required_next_action: action.to_string(),
        recheck_trigger: "draft_qa_rerun".to_string(),
    }
}
