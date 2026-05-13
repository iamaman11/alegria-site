use contracts::generated::alegria::temporal::v1::{
    CmsPublishInputPayload, CmsPublishOutputPayload,
};

use crate::seo_step_support::artifact_key;

pub fn execute(input: &CmsPublishInputPayload) -> CmsPublishOutputPayload {
    let page_node = input.page_node.clone().unwrap_or_default();
    let draft = input.draft.clone().unwrap_or_default();
    let mut blocking_reasons = Vec::new();

    if page_node.page_node_key.trim().is_empty() {
        blocking_reasons.push("missing_page_node".to_string());
    }
    if draft.page_draft_key.trim().is_empty() {
        blocking_reasons.push("missing_draft".to_string());
    }
    if draft.qa_verdict != "publish_ready" {
        blocking_reasons.push("draft_not_publish_ready".to_string());
    }
    if draft.body_markdown.trim().is_empty() {
        blocking_reasons.push("empty_body".to_string());
    }
    if page_node.canonical_url_path.trim().is_empty() {
        blocking_reasons.push("missing_canonical_url".to_string());
    }
    if input.publish_mode == "approved_publish" {
        let approval = input.approval_decision.as_ref();
        let human_approved = approval
            .map(|decision| {
                decision.decision == "approved"
                    && !decision.actor_role.trim().is_empty()
                    && decision.actor_role != "seo_system"
            })
            .unwrap_or(false);
        if !human_approved {
            blocking_reasons.push("human_approval_required".to_string());
        }
    }

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
    let verdict = if blocking_reasons.is_empty() {
        if input.publish_mode == "approved_publish" {
            "approved"
        } else {
            "review_requested"
        }
    } else {
        "publish_blocked"
    };

    CmsPublishOutputPayload {
        page_node_key: page_node.page_node_key,
        page_draft_key: draft.page_draft_key,
        revision_id,
        cms_document_id,
        verdict: verdict.to_string(),
        blocking_reasons,
        outbox_emitted: 0,
        publish_artifact: None,
    }
}
