use std::collections::BTreeSet;

use contracts::generated::alegria::temporal::v1::{
    ContentContractValidateInputPayload, ContentContractValidateOutputPayload, DraftQaInputPayload,
    RenderedContentBlock, SeoPublishBlockerState,
};
use runtime_models::seo_blocks::{block_type_for_role, default_section_roles, factual_role};

fn blocker(reason_code: &str, required_next_action: &str) -> SeoPublishBlockerState {
    SeoPublishBlockerState {
        reason_code: reason_code.to_string(),
        task_type: "publish_gate_blocker".to_string(),
        required_next_action: required_next_action.to_string(),
        recheck_trigger: "draft_qa_rerun".to_string(),
    }
}

fn find_block<'a>(
    blocks: &'a [RenderedContentBlock],
    section_role: &str,
) -> Option<&'a RenderedContentBlock> {
    blocks
        .iter()
        .find(|block| block.section_role == section_role)
}

pub fn validate(input: &DraftQaInputPayload) -> Vec<SeoPublishBlockerState> {
    let draft = input.draft.as_ref().cloned().unwrap_or_default();
    let mut blockers = Vec::new();
    let mut seen_roles = BTreeSet::new();

    for block in &draft.content_blocks {
        if !seen_roles.insert(block.section_role.clone()) {
            blockers.push(blocker(
                "duplicate_content_block_role",
                "Keep one canonical content block per section role.",
            ));
        }
        let expected_block_type = block_type_for_role(&block.section_role);
        if block.block_type != expected_block_type {
            blockers.push(blocker(
                "content_block_type_mismatch",
                "Normalize content blocks so each section role maps to the canonical block type.",
            ));
        }
        if block.required && block.markdown.trim().is_empty() {
            blockers.push(blocker(
                "empty_required_content_block",
                "Populate every required content block before publish gate.",
            ));
        }
        if factual_role(&block.section_role) && block.required && block.support_refs.is_empty() {
            blockers.push(blocker(
                "unsupported_factual_content_block",
                "Attach verified support refs to every required factual content block.",
            ));
        }
    }

    for role in default_section_roles() {
        let Some(block) = find_block(&draft.content_blocks, role) else {
            blockers.push(blocker(
                "missing_required_content_block",
                "Build the complete canonical content block plan before publish gate.",
            ));
            continue;
        };
        if *role == "related_pages"
            && !input.required_links.is_empty()
            && block.markdown.trim().is_empty()
        {
            blockers.push(blocker(
                "missing_required_internal_links",
                "Render required internal link obligations into the related pages block.",
            ));
        }
    }

    blockers.sort_by(|left, right| left.reason_code.cmp(&right.reason_code));
    blockers.dedup_by(|left, right| left.reason_code == right.reason_code);
    blockers
}

pub fn execute(
    input: &ContentContractValidateInputPayload,
) -> ContentContractValidateOutputPayload {
    let blockers = validate(&DraftQaInputPayload {
        run_id: input.run_id.clone(),
        draft: input.draft.clone(),
        supported_fragments: Vec::new(),
        required_links: input.required_links.clone(),
    });
    let mut blocking_reasons = blockers
        .iter()
        .map(|blocker| blocker.reason_code.clone())
        .collect::<Vec<_>>();
    blocking_reasons.sort();
    blocking_reasons.dedup();
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
        DraftQaInputPayload, DraftState, RenderedContentBlock,
    };

    fn block(section_role: &str, block_type: &str, with_support: bool) -> RenderedContentBlock {
        RenderedContentBlock {
            block_key: format!("block:{section_role}"),
            block_type: block_type.to_string(),
            section_role: section_role.to_string(),
            heading: section_role.to_string(),
            markdown: "filled".to_string(),
            support_refs: if with_support {
                vec!["rule:1".to_string()]
            } else {
                Vec::new()
            },
            traceability_label: "verified_fact".to_string(),
            required: true,
        }
    }

    #[test]
    fn detects_mismatched_block_types() {
        let input = DraftQaInputPayload {
            run_id: "run".to_string(),
            draft: Some(DraftState {
                content_blocks: vec![
                    block("overview", "prose", true),
                    block("who_fits", "prose", true),
                    block("documents", "prose", true),
                    block("process", "prose", true),
                    block("fees", "fees_table", true),
                    block("timing", "timeline", true),
                    block("where_to_apply", "prose", true),
                    block("faq", "faq", true),
                    block("related_pages", "related_links", true),
                    block("cta_disclaimer", "cta_disclaimer", true),
                ],
                ..DraftState::default()
            }),
            supported_fragments: Vec::new(),
            required_links: Vec::new(),
        };

        let blockers = validate(&input);
        assert!(blockers
            .iter()
            .any(|blocker| blocker.reason_code == "content_block_type_mismatch"));
    }

    #[test]
    fn detects_missing_factual_support() {
        let input = DraftQaInputPayload {
            run_id: "run".to_string(),
            draft: Some(DraftState {
                content_blocks: default_section_roles()
                    .iter()
                    .map(|role| {
                        block(
                            role,
                            block_type_for_role(role),
                            !matches!(*role, "documents"),
                        )
                    })
                    .collect(),
                ..DraftState::default()
            }),
            supported_fragments: Vec::new(),
            required_links: Vec::new(),
        };

        let blockers = validate(&input);
        assert!(blockers
            .iter()
            .any(|blocker| blocker.reason_code == "unsupported_factual_content_block"));
    }
}
