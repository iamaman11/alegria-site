//! The only CMS approval wire mapping for existing Temporal/scenario payloads.
//! No eligibility policy lives here; the typed domain owns that decision.

use contracts::generated::alegria::temporal::v1::CmsApprovalDecision;
use primitives::errors::DomainError;
use seo_domain::review_decision::{
    editorial_review_disposition, CmsApproval, EditorialReviewDisposition, HumanReviewDecision,
};

pub fn from_wire(input: &CmsApprovalDecision) -> Result<CmsApproval, DomainError> {
    Ok(CmsApproval {
        decision_key: input.decision_key.clone(),
        page_node_key: input.page_node_key.clone(),
        revision_id: input.revision_id.clone(),
        actor_role: input.actor_role.clone(),
        decision: HumanReviewDecision::parse(&input.decision)?,
        reason: input.reason.clone(),
        decided_at: input.decided_at.clone(),
    })
}

pub fn to_wire(input: CmsApproval) -> CmsApprovalDecision {
    CmsApprovalDecision {
        decision_key: input.decision_key,
        page_node_key: input.page_node_key,
        revision_id: input.revision_id,
        actor_role: input.actor_role,
        decision: input.decision.as_str().to_string(),
        reason: input.reason,
        decided_at: input.decided_at,
    }
}

/// Translate a previously recorded Temporal payload into a domain decision.
/// An unknown historical string can only wait, never authorize publication or
/// terminal rework. The strict `from_wire` remains mandatory for persistence.
pub fn workflow_disposition(
    input: &CmsApprovalDecision,
    terminal_rework_enabled: bool,
) -> EditorialReviewDisposition {
    editorial_review_disposition(
        HumanReviewDecision::parse(&input.decision).ok(),
        terminal_rework_enabled,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_temporal_decisions_are_fail_closed() {
        use EditorialReviewDisposition::{Approved, ReworkRequired, Wait};
        let mut wire = CmsApprovalDecision::default();
        wire.decision = "approved".to_string();
        assert_eq!(workflow_disposition(&wire, false), Approved);
        wire.decision = "blocked".to_string();
        assert_eq!(workflow_disposition(&wire, true), ReworkRequired);
        assert_eq!(workflow_disposition(&wire, false), Wait);
        for raw in ["reopened", "", "unexpected", "APPROVED"] {
            wire.decision = raw.to_string();
            assert_eq!(workflow_disposition(&wire, true), Wait);
        }
    }

    #[test]
    fn wire_mapping_preserves_immutable_revision_and_rejects_unknown_decision() {
        let wire = CmsApprovalDecision {
            decision_key: "decision-1".to_string(),
            page_node_key: "page-1".to_string(),
            revision_id: "revision-2".to_string(),
            actor_role: "human_editor".to_string(),
            decision: "approved".to_string(),
            reason: "checked".to_string(),
            decided_at: "2026-11-23T00:00:00Z".to_string(),
        };
        let approval = from_wire(&wire).unwrap();
        assert!(approval.approves_revision("page-1", "revision-2"));
        assert_eq!(to_wire(approval), wire);
        let invalid = CmsApprovalDecision {
            decision: "published".to_string(),
            ..wire
        };
        assert_eq!(
            from_wire(&invalid).unwrap_err().class_str(),
            "validation_failure"
        );
    }
}
