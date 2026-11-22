//! Publishing decisions. An editorial approval makes a revision eligible for
//! deployment, but only an independently verified public release is published.
//! SQL transactions, the live pointer, and HTTP remain adapter responsibilities.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReadiness {
    ReadyToVerify,
    Blocked,
}

impl ReleaseReadiness {
    pub fn from_render_validation(verdict: &str, blocking_reasons: &[String]) -> Self {
        if verdict == "render_ready" && blocking_reasons.is_empty() {
            Self::ReadyToVerify
        } else {
            Self::Blocked
        }
    }

    pub fn verdict(self) -> &'static str {
        match self {
            Self::ReadyToVerify => "ready_for_release",
            Self::Blocked => "blocked",
        }
    }

    pub fn current_status(self) -> &'static str {
        match self {
            Self::ReadyToVerify => "approved",
            Self::Blocked => "blocked",
        }
    }

    pub fn can_activate(self, planned_verdict: &str) -> bool {
        self == Self::ReadyToVerify && planned_verdict == self.verdict()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicReleaseOutcome {
    Published,
    Blocked,
}

impl PublicReleaseOutcome {
    /// `verified` means that the exact public HTTPS bytes were read back after
    /// activating the immutable release, not that the renderer succeeded.
    pub fn after_readback(ready: ReleaseReadiness, verified: bool) -> Self {
        if ready == ReleaseReadiness::ReadyToVerify && verified {
            Self::Published
        } else {
            Self::Blocked
        }
    }

    pub fn is_published(self) -> bool {
        self == Self::Published
    }

    pub fn verdict(self) -> &'static str {
        match self {
            Self::Published => "published",
            Self::Blocked => "blocked",
        }
    }

    pub fn artifact_status(self) -> &'static str {
        match self {
            Self::Published => "ready",
            Self::Blocked => "blocked",
        }
    }

    pub fn event_type(self) -> &'static str {
        match self {
            Self::Published => "seo_page_published",
            Self::Blocked => "seo_page_publish_blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveReleasePointer {
    Candidate,
    Previous,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Database committed; restore the exact committed version to the CDN.
    ReassertCommitted,
    /// The first release has no safe predecessor. Only the original finalizer
    /// may commit after establishing exact approval and public readback.
    VerifyPendingFirst,
    /// No database commit; restore the previous live version.
    RestorePrevious,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryRefusal {
    CommittedPointerMismatch,
    UnknownLivePointer,
}

pub fn plan_recovery(
    db_committed: bool,
    live: LiveReleasePointer,
    has_previous: bool,
) -> Result<RecoveryAction, RecoveryRefusal> {
    if db_committed {
        return match live {
            LiveReleasePointer::Candidate => Ok(RecoveryAction::ReassertCommitted),
            _ => Err(RecoveryRefusal::CommittedPointerMismatch),
        };
    }
    if live == LiveReleasePointer::Unknown {
        return Err(RecoveryRefusal::UnknownLivePointer);
    }
    if has_previous {
        Ok(RecoveryAction::RestorePrevious)
    } else {
        Ok(RecoveryAction::VerifyPendingFirst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_ready_is_only_a_candidate_never_a_publication() {
        let ready = ReleaseReadiness::from_render_validation("render_ready", &[]);
        assert_eq!(ready.verdict(), "ready_for_release");
        assert_eq!(ready.current_status(), "approved");
        assert!(ready.can_activate("ready_for_release"));
        assert!(!ready.can_activate("published"));
        assert_eq!(
            PublicReleaseOutcome::after_readback(ready, false),
            PublicReleaseOutcome::Blocked,
        );
        assert_eq!(
            PublicReleaseOutcome::after_readback(ready, true),
            PublicReleaseOutcome::Published,
        );
    }

    #[test]
    fn blockers_or_failed_render_never_activate_publication() {
        for (verdict, blockers) in [
            ("blocked", vec![]),
            ("render_ready", vec!["missing_approval".to_string()]),
        ] {
            let ready = ReleaseReadiness::from_render_validation(verdict, &blockers);
            assert_eq!(ready, ReleaseReadiness::Blocked);
            assert!(!ready.can_activate("ready_for_release"));
            assert_eq!(
                PublicReleaseOutcome::after_readback(ready, true),
                PublicReleaseOutcome::Blocked,
            );
        }
        assert_eq!(PublicReleaseOutcome::Published.artifact_status(), "ready");
        assert_eq!(
            PublicReleaseOutcome::Blocked.event_type(),
            "seo_page_publish_blocked"
        );
    }

    #[test]
    fn recovery_distinguishes_committed_uncertain_and_first_release() {
        use LiveReleasePointer::{Candidate, Previous, Unknown};
        assert_eq!(
            plan_recovery(true, Candidate, true),
            Ok(RecoveryAction::ReassertCommitted)
        );
        assert_eq!(
            plan_recovery(true, Previous, true),
            Err(RecoveryRefusal::CommittedPointerMismatch)
        );
        assert_eq!(
            plan_recovery(false, Candidate, true),
            Ok(RecoveryAction::RestorePrevious)
        );
        assert_eq!(
            plan_recovery(false, Previous, true),
            Ok(RecoveryAction::RestorePrevious)
        );
        assert_eq!(
            plan_recovery(false, Candidate, false),
            Ok(RecoveryAction::VerifyPendingFirst)
        );
        assert_eq!(
            plan_recovery(false, Unknown, true),
            Err(RecoveryRefusal::UnknownLivePointer)
        );
    }
}
