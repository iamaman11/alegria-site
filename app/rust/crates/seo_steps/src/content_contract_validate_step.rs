//! Temporal transport adapter for the shared content-contract domain policy.

use contracts::generated::alegria::temporal::v1::{
    ContentContractValidateInputPayload, ContentContractValidateOutputPayload, DraftQaInputPayload,
    DraftState, SeoPublishBlockerState,
};
use seo_domain::content_contract::{
    self, ContentBlock, ContentContract, Evidence, LinkObligation, PublishLinkCoverage,
};

fn blocker(violation: content_contract::Violation) -> SeoPublishBlockerState {
    SeoPublishBlockerState {
        reason_code: violation.reason_code().to_string(),
        task_type: "publish_gate_blocker".to_string(),
        required_next_action: violation.required_next_action().to_string(),
        recheck_trigger: "draft_qa_rerun".to_string(),
    }
}

fn validate_with_coverage(
    input: &DraftQaInputPayload,
    verify_publish_links: bool,
) -> Vec<SeoPublishBlockerState> {
    let empty = DraftState::default();
    let draft = input.draft.as_ref().unwrap_or(&empty);
    let blocks = draft
        .content_blocks
        .iter()
        .map(|block| ContentBlock {
            role: &block.section_role,
            block_type: &block.block_type,
            markdown: &block.markdown,
            required: block.required,
            support_refs: &block.support_refs,
        })
        .collect::<Vec<_>>();
    let claims = draft
        .claim_ledger
        .iter()
        .map(|claim| Evidence {
            validation_verdict: &claim.validation_verdict,
            traceability_label: &claim.traceability_label,
            support_refs: &claim.support_refs,
        })
        .collect::<Vec<_>>();
    let traceability = draft
        .traceability_entries
        .iter()
        .map(|entry| Evidence {
            validation_verdict: &entry.validation_verdict,
            traceability_label: &entry.traceability_label,
            support_refs: &entry.support_refs,
        })
        .collect::<Vec<_>>();
    let links = input
        .required_links
        .iter()
        .map(|link| LinkObligation {
            target_page_key: &link.target_page_key,
            role: &link.link_role,
            required: link.required_flag,
            bound_exactly: draft.required_links.contains(link),
        })
        .collect::<Vec<_>>();
    let related_pages_body = if verify_publish_links {
        draft
            .sections
            .iter()
            .filter(|section| section.section_role == "related_pages")
            .map(|section| section.body_markdown.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        String::new()
    };
    let typed_editorial = !draft.editorial_locale.is_empty()
        || draft
            .claim_ledger
            .iter()
            .any(|claim| claim.editorial_proposal.is_some());
    content_contract::validate(&ContentContract {
        blocks: &blocks,
        claims: &claims,
        traceability: &traceability,
        links: &links,
        publish_links: verify_publish_links.then_some(PublishLinkCoverage {
            typed_editorial,
            related_pages_body: &related_pages_body,
        }),
    })
    .into_iter()
    .map(blocker)
    .collect()
}

/// Standalone pre-QA contract check, retained as a real Temporal activity.
pub fn validate(input: &DraftQaInputPayload) -> Vec<SeoPublishBlockerState> {
    validate_with_coverage(input, false)
}

/// Final QA uses the same domain owner and extends it with link-binding checks.
pub fn validate_for_publish(input: &DraftQaInputPayload) -> Vec<SeoPublishBlockerState> {
    validate_with_coverage(input, true)
}

pub fn execute(
    input: &ContentContractValidateInputPayload,
) -> ContentContractValidateOutputPayload {
    let blockers = validate(&DraftQaInputPayload {
        run_id: input.run_id.clone(),
        draft: input.draft.clone(),
        required_links: input.required_links.clone(),
        ..DraftQaInputPayload::default()
    });
    let blocking_reasons = blockers
        .iter()
        .map(|blocker| blocker.reason_code.clone())
        .collect::<Vec<_>>();
    ContentContractValidateOutputPayload {
        verdict: if blockers.is_empty() {
            "ready".to_string()
        } else {
            "blocked".to_string()
        },
        blocking_reasons,
        blockers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        ClaimLedgerEntry, LinkRecommendationState, RenderedContentBlock, SeoDraftSectionState,
        SeoTraceabilityEntryState,
    };

    fn block(role: &str, block_type: &str, refs: Vec<String>) -> RenderedContentBlock {
        RenderedContentBlock {
            section_role: role.to_string(),
            block_type: block_type.to_string(),
            markdown: "verified text".to_string(),
            support_refs: refs,
            required: true,
            ..RenderedContentBlock::default()
        }
    }

    #[test]
    fn mismatched_content_block_type_is_reported() {
        let input = DraftQaInputPayload {
            draft: Some(DraftState {
                content_blocks: vec![block("fees", "prose", vec![])],
                ..DraftState::default()
            }),
            ..DraftQaInputPayload::default()
        };
        assert!(validate(&input)
            .iter()
            .any(|state| state.reason_code == "content_block_type_mismatch"));
    }

    #[test]
    fn matching_evidence_must_be_in_both_manifests() {
        let refs = vec!["rule:1".to_string()];
        let mut draft = DraftState {
            content_blocks: vec![block("fees", "fees_table", refs.clone())],
            claim_ledger: vec![ClaimLedgerEntry {
                support_refs: refs.clone(),
                validation_verdict: "supported".into(),
                traceability_label: "verified_fact".into(),
                ..ClaimLedgerEntry::default()
            }],
            ..DraftState::default()
        };
        assert!(validate(&DraftQaInputPayload {
            draft: Some(draft.clone()),
            ..DraftQaInputPayload::default()
        })
        .iter()
        .any(|state| state.reason_code == "unmatched_factual_support_ref"));
        draft.traceability_entries.push(SeoTraceabilityEntryState {
            support_refs: refs,
            validation_verdict: "supported".into(),
            traceability_label: "verified_fact".into(),
            ..SeoTraceabilityEntryState::default()
        });
        assert!(!validate(&DraftQaInputPayload {
            draft: Some(draft),
            ..DraftQaInputPayload::default()
        })
        .iter()
        .any(|state| state.reason_code == "unmatched_factual_support_ref"));
    }

    #[test]
    fn publish_links_have_one_blocker_when_missing_from_block_and_draft() {
        let input = DraftQaInputPayload {
            draft: Some(DraftState {
                sections: vec![SeoDraftSectionState {
                    section_role: "related_pages".into(),
                    body_markdown: "".into(),
                    ..SeoDraftSectionState::default()
                }],
                ..DraftState::default()
            }),
            required_links: vec![LinkRecommendationState {
                required_flag: true,
                target_page_key: "target".into(),
                ..LinkRecommendationState::default()
            }],
            ..DraftQaInputPayload::default()
        };
        let qa_blockers = validate_for_publish(&input);
        assert_eq!(
            qa_blockers
                .iter()
                .filter(|state| state.reason_code == "missing_required_internal_links")
                .count(),
            1
        );
        let standalone = execute(&ContentContractValidateInputPayload {
            draft: input.draft,
            required_links: input.required_links,
            ..ContentContractValidateInputPayload::default()
        });
        assert_eq!(
            standalone
                .blockers
                .iter()
                .filter(|state| state.reason_code == "missing_required_internal_links")
                .count(),
            1
        );
    }
}
