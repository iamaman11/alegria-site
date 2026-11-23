//! Editorial review policy. The domain owns admissible human decisions and
//! their HITL transitions; Temporal and PostgreSQL only transport or persist
//! the result.

use primitives::errors::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanReviewDecision {
    Approved,
    Blocked,
    Reopened,
}

impl HumanReviewDecision {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "approved" => Ok(Self::Approved),
            "blocked" => Ok(Self::Blocked),
            "reopened" => Ok(Self::Reopened),
            other => Err(DomainError::ValidationFailure {
                message: format!("unsupported decision: {other}"),
            }),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Blocked => "blocked",
            Self::Reopened => "reopened",
        }
    }

    pub const fn hitl_queue_state(self) -> &'static str {
        match self {
            Self::Approved => "resolved",
            Self::Blocked | Self::Reopened => "reopened",
        }
    }
}

/// Shared precondition for the application use case and the direct SQL port.
/// A missing revision is a deliberate legacy lookup, which the persistence
/// adapter must refuse if it cannot resolve an exact pending review revision.
pub fn validate_human_reviewer(
    page_node_key: &str,
    revision_id: Option<&str>,
    actor_role: &str,
) -> Result<(), DomainError> {
    if page_node_key.trim().is_empty() {
        return Err(DomainError::ValidationFailure {
            message: "page_node_key is required".to_string(),
        });
    }
    if revision_id.is_some_and(|revision| revision.trim().is_empty()) {
        return Err(DomainError::ValidationFailure {
            message: "revision_id must be an exact non-empty review candidate".to_string(),
        });
    }
    if actor_role.trim().is_empty() || actor_role.trim() == "seo_system" {
        return Err(DomainError::ValidationFailure {
            message: "human actor_role is required".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_review_transitions_are_a_typed_domain_decision() {
        for (raw, decision, queue) in [
            ("approved", HumanReviewDecision::Approved, "resolved"),
            ("blocked", HumanReviewDecision::Blocked, "reopened"),
            ("reopened", HumanReviewDecision::Reopened, "reopened"),
        ] {
            let parsed = HumanReviewDecision::parse(raw).unwrap();
            assert_eq!(parsed, decision);
            assert_eq!(parsed.as_str(), raw);
            assert_eq!(parsed.hitl_queue_state(), queue);
        }
        for invalid in ["", "approve", "APPROVED", "published"] {
            assert_eq!(
                HumanReviewDecision::parse(invalid).unwrap_err().class_str(),
                "validation_failure"
            );
        }
    }

    #[test]
    fn only_human_reviewers_with_identified_pages_are_admitted() {
        assert!(validate_human_reviewer("page", Some("revision"), "editor").is_ok());
        // Absence is permitted only so the adapter can refuse ambiguous or
        // already-published current-revision lookups using the live database.
        assert!(validate_human_reviewer("page", None, "editor").is_ok());
        for (page, revision, actor) in [
            (" ", Some("revision"), "editor"),
            ("page", Some(" "), "editor"),
            ("page", Some("revision"), " "),
            ("page", Some("revision"), "seo_system"),
            ("page", Some("revision"), " seo_system "),
        ] {
            assert_eq!(
                validate_human_reviewer(page, revision, actor)
                    .unwrap_err()
                    .class_str(),
                "validation_failure"
            );
        }
    }
}
