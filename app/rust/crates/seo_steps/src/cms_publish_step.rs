use contracts::generated::alegria::temporal::v1::{
    CmsPublishInputPayload, CmsPublishOutputPayload,
};
use primitives::errors::DomainError;
use seo_domain::cms_revision::{
    cms_publication_blockers, CmsPublicationGate, CmsPublishIntent, CmsRevisionState,
};

use crate::seo_step_support::artifact_key;

pub fn execute(input: &CmsPublishInputPayload) -> Result<CmsPublishOutputPayload, DomainError> {
    let intent = CmsPublishIntent::parse(&input.publish_mode)?;
    let page_node = input.page_node.clone().unwrap_or_default();
    let draft = input.draft.clone().unwrap_or_default();
    let human_approved = input
        .approval_decision
        .as_ref()
            .and_then(|decision| crate::cms_review_wire::from_wire(decision).ok())
            .map(|decision| decision.is_human_approved())
            .unwrap_or(false);
    let blocking_reasons = cms_publication_blockers(
        intent,
        CmsPublicationGate {
            page_key: &page_node.page_node_key,
            draft_key: &draft.page_draft_key,
            qa_verdict: &draft.qa_verdict,
            body_markdown: &draft.body_markdown,
            canonical_path: &page_node.canonical_url_path,
            llm_provider: &draft.llm_provider_key,
            human_approved,
        },
    )
    .into_iter()
    .map(|reason| reason.reason_code().to_string())
    .collect::<Vec<_>>();

    let revision_id = if draft.page_draft_key.trim().is_empty() {
        String::new()
    } else {
        artifact_key(
            "cms_revision",
            &[
                &page_node.page_node_key,
                &draft.page_draft_key,
                &draft.draft_revision.to_string(),
                "seo_cms_publish@1",
            ],
        )
    };
    let cms_document_id = if page_node.page_node_key.trim().is_empty() {
        String::new()
    } else {
        artifact_key("cms_document", &[&page_node.page_node_key])
    };
    let verdict = CmsRevisionState::decide(intent, !blocking_reasons.is_empty()).planned_verdict();

    Ok(CmsPublishOutputPayload {
        page_node_key: page_node.page_node_key,
        page_draft_key: draft.page_draft_key,
        revision_id,
        cms_document_id,
        verdict: verdict.to_string(),
        blocking_reasons,
        outbox_emitted: 0,
        publish_artifact: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        CmsApprovalDecision, DraftState, PageNodeState,
    };

    #[test]
    fn deterministic_fallback_blocks_publish() {
        let output = execute(&CmsPublishInputPayload {
            run_id: "run".to_string(),
            page_node: Some(PageNodeState {
                page_node_key: "page".to_string(),
                canonical_url_path: "/ru/visa/spain/".to_string(),
                ..PageNodeState::default()
            }),
            draft: Some(DraftState {
                page_draft_key: "draft".to_string(),
                qa_verdict: "publish_ready".to_string(),
                body_markdown: "body".to_string(),
                llm_provider_key: "deterministic_fallback".to_string(),
                ..DraftState::default()
            }),
            actor_role: "seo_system".to_string(),
            publish_mode: "request_review".to_string(),
            approval_decision: None,
        })
        .unwrap();

        assert_eq!(output.verdict, "publish_blocked");
        assert!(output
            .blocking_reasons
            .contains(&"deterministic_fallback_publish_blocked".to_string()));
    }

    #[test]
    fn approved_publish_still_requires_human_approval() {
        let output = execute(&CmsPublishInputPayload {
            run_id: "run".to_string(),
            page_node: Some(PageNodeState {
                page_node_key: "page".to_string(),
                canonical_url_path: "/ru/visa/spain/".to_string(),
                ..PageNodeState::default()
            }),
            draft: Some(DraftState {
                page_draft_key: "draft".to_string(),
                qa_verdict: "publish_ready".to_string(),
                body_markdown: "body".to_string(),
                llm_provider_key: "source_backed_template@1".to_string(),
                ..DraftState::default()
            }),
            actor_role: "seo_system".to_string(),
            publish_mode: "approved_publish".to_string(),
            approval_decision: Some(CmsApprovalDecision {
                decision_key: "decision".to_string(),
                page_node_key: "page".to_string(),
                revision_id: "rev".to_string(),
                actor_role: "seo_system".to_string(),
                decision: "approved".to_string(),
                reason: "test".to_string(),
                decided_at: "2026-05-15T00:00:00Z".to_string(),
            }),
        })
        .unwrap();

        assert_eq!(output.verdict, "publish_blocked");
    }

    #[test]
    fn unknown_publish_mode_is_rejected_before_a_revision_can_be_planned() {
        let error = execute(&CmsPublishInputPayload {
            publish_mode: "publish_with_hitl".to_string(),
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(error.class_str(), "validation_failure");
    }
}
