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

/// What an editorial workflow may do after reading a human decision.
/// The Temporal version patch is an input, not a domain dependency: histories
/// written before the terminal-rework change must keep waiting on `blocked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorialReviewDisposition {
    Approved,
    ReworkRequired,
    Wait,
}

pub fn editorial_review_disposition(
    decision: Option<HumanReviewDecision>,
    terminal_rework_enabled: bool,
) -> EditorialReviewDisposition {
    match decision {
        Some(HumanReviewDecision::Approved) => EditorialReviewDisposition::Approved,
        Some(HumanReviewDecision::Blocked) if terminal_rework_enabled => {
            EditorialReviewDisposition::ReworkRequired
        }
        Some(HumanReviewDecision::Blocked | HumanReviewDecision::Reopened) | None => {
            EditorialReviewDisposition::Wait
        }
    }
}

/// A persisted editorial decision, independent of Temporal payloads and SQL
/// rows. The identity and human actor are part of the approval, not incidental
/// adapter metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmsApproval {
    pub decision_key: String,
    pub page_node_key: String,
    pub revision_id: String,
    pub actor_role: String,
    pub decision: HumanReviewDecision,
    pub reason: String,
    pub decided_at: String,
}

impl CmsApproval {
    pub fn is_human_approved(&self) -> bool {
        self.decision == HumanReviewDecision::Approved
            && !self.decision_key.trim().is_empty()
            && human_actor(&self.actor_role)
    }

    pub fn approves_revision(&self, page_node_key: &str, revision_id: &str) -> bool {
        self.is_human_approved()
            && !page_node_key.trim().is_empty()
            && !revision_id.trim().is_empty()
            && self.page_node_key == page_node_key
            && self.revision_id == revision_id
    }
}

fn human_actor(actor_role: &str) -> bool {
    !actor_role.trim().is_empty() && actor_role.trim() != "seo_system"
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
    if !human_actor(actor_role) {
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
    fn editorial_workflow_disposition_is_owned_by_the_domain() {
        use EditorialReviewDisposition::{Approved, ReworkRequired, Wait};
        assert_eq!(
            editorial_review_disposition(Some(HumanReviewDecision::Approved), true),
            Approved
        );
        assert_eq!(
            editorial_review_disposition(Some(HumanReviewDecision::Blocked), true),
            ReworkRequired
        );
        for decision in [
            Some(HumanReviewDecision::Blocked),
            Some(HumanReviewDecision::Reopened),
            None,
        ] {
            assert_eq!(editorial_review_disposition(decision, false), Wait);
        }
        assert_eq!(
            editorial_review_disposition(Some(HumanReviewDecision::Reopened), true),
            Wait
        );
    }

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

    #[test]
    fn approval_is_bound_to_human_actor_page_and_exact_revision() {
        let valid = CmsApproval {
            decision_key: "decision-1".to_string(),
            page_node_key: "page-1".to_string(),
            revision_id: "revision-2".to_string(),
            actor_role: "editor".to_string(),
            decision: HumanReviewDecision::Approved,
            reason: String::new(),
            decided_at: String::new(),
        };
        assert!(valid.approves_revision("page-1", "revision-2"));
        assert!(!valid.approves_revision("page-1", "revision-1"));
        assert!(!valid.approves_revision("other-page", "revision-2"));
        assert!(!valid.approves_revision("", "revision-2"));
        assert!(!valid.approves_revision("page-1", ""));
        for invalid in [
            CmsApproval {
                actor_role: "seo_system".to_string(),
                ..valid.clone()
            },
            CmsApproval {
                actor_role: " ".to_string(),
                ..valid.clone()
            },
            CmsApproval {
                decision: HumanReviewDecision::Blocked,
                ..valid.clone()
            },
            CmsApproval {
                decision: HumanReviewDecision::Reopened,
                ..valid.clone()
            },
            CmsApproval {
                decision_key: String::new(),
                ..valid.clone()
            },
        ] {
            assert!(!invalid.is_human_approved());
            assert!(!invalid.approves_revision("page-1", "revision-2"));
        }
    }
}
