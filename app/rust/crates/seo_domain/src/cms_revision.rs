//! CMS editorial revision decisions. Preparing a review candidate is not a
//! verified public release. SQL owns atomic writes, not status selection.

use primitives::errors::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmsPublishIntent {
    RequestReview,
    ApproveForRelease,
}

impl CmsPublishIntent {
    /// Decode an external Temporal request at each entry boundary. Unknown
    /// modes must not silently become a valid editorial transition.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw {
            "request_review" => Ok(Self::RequestReview),
            "approved_publish" => Ok(Self::ApproveForRelease),
            other => Err(DomainError::ValidationFailure {
                message: format!("unsupported CMS publish mode: {other}"),
            }),
        }
    }

    pub fn requires_human_approval(self) -> bool {
        self == Self::ApproveForRelease
    }
}

/// Only business inputs, never the persisted revision or Temporal DTO.
/// The persistence adapter can add its separate DB-observed blockers.
#[derive(Debug, Clone, Copy)]
pub struct CmsPublicationGate<'a> {
    pub page_key: &'a str,
    pub draft_key: &'a str,
    pub qa_verdict: &'a str,
    pub body_markdown: &'a str,
    pub canonical_path: &'a str,
    pub llm_provider: &'a str,
    pub human_approved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmsPublishGateViolation {
    MissingPageNode,
    MissingDraft,
    DraftNotPublishReady,
    EmptyBody,
    MissingCanonicalUrl,
    DeterministicFallbackPublishBlocked,
    HumanApprovalRequired,
}

impl CmsPublishGateViolation {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::MissingPageNode => "missing_page_node",
            Self::MissingDraft => "missing_draft",
            Self::DraftNotPublishReady => "draft_not_publish_ready",
            Self::EmptyBody => "empty_body",
            Self::MissingCanonicalUrl => "missing_canonical_url",
            Self::DeterministicFallbackPublishBlocked => "deterministic_fallback_publish_blocked",
            Self::HumanApprovalRequired => "human_approval_required",
        }
    }
}

pub fn cms_publication_blockers(
    intent: CmsPublishIntent,
    candidate: CmsPublicationGate<'_>,
) -> Vec<CmsPublishGateViolation> {
    let mut reasons = Vec::new();
    if candidate.page_key.trim().is_empty() {
        reasons.push(CmsPublishGateViolation::MissingPageNode);
    }
    if candidate.draft_key.trim().is_empty() {
        reasons.push(CmsPublishGateViolation::MissingDraft);
    }
    if candidate.qa_verdict != "publish_ready" {
        reasons.push(CmsPublishGateViolation::DraftNotPublishReady);
    }
    if candidate.body_markdown.trim().is_empty() {
        reasons.push(CmsPublishGateViolation::EmptyBody);
    }
    if candidate.canonical_path.trim().is_empty() {
        reasons.push(CmsPublishGateViolation::MissingCanonicalUrl);
    }
    if candidate.llm_provider == "deterministic_fallback" {
        reasons.push(CmsPublishGateViolation::DeterministicFallbackPublishBlocked);
    }
    if intent.requires_human_approval() && !candidate.human_approved {
        reasons.push(CmsPublishGateViolation::HumanApprovalRequired);
    }
    reasons
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmsRevisionState {
    ReviewRequired,
    Approved,
    Blocked,
}

impl CmsRevisionState {
    /// Both the provisional step and the persisted SQL operation use this
    /// decision. Persistence may add database-derived blockers before calling
    /// it again, without implementing a competing transition policy.
    pub fn decide(intent: CmsPublishIntent, has_blockers: bool) -> Self {
        if has_blockers {
            Self::Blocked
        } else {
            match intent {
                CmsPublishIntent::RequestReview => Self::ReviewRequired,
                CmsPublishIntent::ApproveForRelease => Self::Approved,
            }
        }
    }

    pub fn status(self) -> &'static str {
        match self {
            Self::ReviewRequired => "review_required",
            Self::Approved => "approved",
            Self::Blocked => "blocked",
        }
    }

    pub fn planned_verdict(self) -> &'static str {
        match self {
            Self::ReviewRequired => "review_requested",
            Self::Approved => "approved",
            Self::Blocked => "publish_blocked",
        }
    }

    pub fn persisted_verdict(self) -> &'static str {
        match self {
            Self::ReviewRequired => "review_requested",
            Self::Approved => "approved",
            Self::Blocked => "blocked",
        }
    }

    pub fn event_type(self) -> &'static str {
        match self {
            Self::ReviewRequired => "seo_page_review_requested",
            Self::Approved => "seo_page_approved",
            Self::Blocked => "seo_page_publish_blocked",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_modes_are_admitted() {
        assert_eq!(
            CmsPublishIntent::parse("request_review").unwrap(),
            CmsPublishIntent::RequestReview
        );
        assert_eq!(
            CmsPublishIntent::parse("approved_publish").unwrap(),
            CmsPublishIntent::ApproveForRelease
        );
        for invalid in [
            "",
            "publish_with_hitl",
            "approve",
            "published",
            "APPROVED_PUBLISH",
        ] {
            assert_eq!(
                CmsPublishIntent::parse(invalid).unwrap_err().class_str(),
                "validation_failure"
            );
        }
    }

    #[test]
    fn one_state_machine_owns_revision_status_events_and_verdicts() {
        let review = CmsRevisionState::decide(CmsPublishIntent::RequestReview, false);
        assert_eq!(review.status(), "review_required");
        assert_eq!(review.planned_verdict(), "review_requested");
        assert_eq!(review.persisted_verdict(), "review_requested");
        assert_eq!(review.event_type(), "seo_page_review_requested");

        let approved = CmsRevisionState::decide(CmsPublishIntent::ApproveForRelease, false);
        assert!(CmsPublishIntent::ApproveForRelease.requires_human_approval());
        assert!(!CmsPublishIntent::RequestReview.requires_human_approval());
        assert_eq!(approved.status(), "approved");
        assert_eq!(approved.planned_verdict(), "approved");
        assert_eq!(approved.event_type(), "seo_page_approved");

        for mode in [
            CmsPublishIntent::RequestReview,
            CmsPublishIntent::ApproveForRelease,
        ] {
            let blocked = CmsRevisionState::decide(mode, true);
            assert_eq!(blocked, CmsRevisionState::Blocked);
            assert_eq!(blocked.status(), "blocked");
            assert_eq!(blocked.planned_verdict(), "publish_blocked");
            assert_eq!(blocked.persisted_verdict(), "blocked");
            assert_eq!(blocked.event_type(), "seo_page_publish_blocked");
        }
    }

    #[test]
    fn publication_gates_preserve_all_blockers_and_mode_specific_approval() {
        let empty = CmsPublicationGate {
            page_key: "",
            draft_key: "",
            qa_verdict: "qa_failed",
            body_markdown: "",
            canonical_path: "",
            llm_provider: "deterministic_fallback",
            human_approved: false,
        };
        let reasons = cms_publication_blockers(CmsPublishIntent::ApproveForRelease, empty);
        assert_eq!(reasons.len(), 7);
        assert_eq!(reasons[0].reason_code(), "missing_page_node");
        assert_eq!(reasons[6].reason_code(), "human_approval_required");

        let complete = CmsPublicationGate {
            page_key: "page",
            draft_key: "draft",
            qa_verdict: "publish_ready",
            body_markdown: "verified body",
            canonical_path: "/ru/guide/",
            llm_provider: "openai",
            human_approved: false,
        };
        assert!(cms_publication_blockers(CmsPublishIntent::RequestReview, complete).is_empty());
        assert_eq!(
            cms_publication_blockers(CmsPublishIntent::ApproveForRelease, complete),
            vec![CmsPublishGateViolation::HumanApprovalRequired]
        );
        assert!(cms_publication_blockers(
            CmsPublishIntent::ApproveForRelease,
            CmsPublicationGate { human_approved: true, ..complete }
        ).is_empty());
    }
}
