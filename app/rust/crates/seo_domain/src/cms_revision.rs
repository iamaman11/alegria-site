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
}
