use std::collections::BTreeSet;

use contracts::generated::alegria::temporal::v1::{
    DraftQaInputPayload, DraftQaOutputPayload, SeoPublishBlockerState,
};
use runtime_models::seo_blocks::factual_role;
fn blocker(reason_code: &str, required_next_action: &str) -> SeoPublishBlockerState {
    SeoPublishBlockerState {
        reason_code: reason_code.to_string(),
        task_type: match reason_code {
            "unsupported_factual_fragment" | "forbidden_serp_as_fact_usage" => {
                "unsupported_factual_fragment"
            }
            "missing_required_internal_links" => "publish_gate_blocker",
            "missing_required_sections"
            | "missing_required_metadata"
            | "missing_traceability_manifest" => "publish_gate_blocker",
            _ => "publish_gate_blocker",
        }
        .to_string(),
        required_next_action: required_next_action.to_string(),
        recheck_trigger: "draft_qa_rerun".to_string(),
    }
}

pub fn execute(input: &DraftQaInputPayload) -> DraftQaOutputPayload {
    let draft = input.draft.clone().unwrap_or_default();
    let mut blocking_reasons = Vec::new();
    let mut blockers = Vec::new();
    let contract_blockers = crate::content_contract_validate_step::validate(input);
    for blocker_state in contract_blockers {
        blocking_reasons.push(blocker_state.reason_code.clone());
        blockers.push(blocker_state);
    }
    if let Some(draft_ref) = input.draft.as_ref() {
        for blocker_state in crate::licensing_gate_step::evaluate(draft_ref) {
            blocking_reasons.push(blocker_state.reason_code.clone());
            blockers.push(blocker_state);
        }
    }
    // The source-backed fragments come from the DB-resolved VERIFIED bundle,
    // not self-declared support refs in the LLM response. An invented claim
    // carrying a real ref cannot pass merely because the ref is well formed.
    let verified_fragments = input
        .supported_fragments
        .iter()
        .map(|fragment| fragment.trim())
        .filter(|fragment| !fragment.is_empty())
        .collect::<BTreeSet<_>>();
    let has_exact_evidence = |fragment: &str| verified_fragments.contains(fragment.trim());
    let supported_claims = draft
        .traceability_entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.traceability_label.as_str(),
                "verified_fact" | "verified_summary"
            ) && !entry.support_refs.is_empty()
                && entry.validation_verdict == "supported"
                && has_exact_evidence(&entry.fragment_text)
        })
        .count() as u32;
    let unsupported_claims = draft
        .traceability_entries
        .iter()
        .filter(|entry| {
            entry.traceability_label == "unsupported_factual_fragment"
                || (entry.fragment_kind == "factual"
                    && (entry.support_refs.is_empty()
                        || entry.validation_verdict != "supported"
                        || !has_exact_evidence(&entry.fragment_text)))
        })
        .count() as u32;
    let unsupported_ledger_claims = draft
        .claim_ledger
        .iter()
        .filter(|claim| {
            claim.validation_verdict == "blocked"
                || claim.traceability_label == "unsupported_factual_fragment"
                || (claim.claim_kind == "factual"
                    && (claim.support_refs.is_empty()
                        || claim.validation_verdict != "supported"
                        || !has_exact_evidence(&claim.fragment_text)))
        })
        .count() as u32;
    if draft.body_markdown.contains("[unsupported") || unsupported_claims > 0 {
        blocking_reasons.push("unsupported_factual_fragment".to_string());
        blockers.push(blocker(
            "unsupported_factual_fragment",
            "Bind every factual fragment to verified_fact or verified_summary support.",
        ));
    }
    if unsupported_ledger_claims > 0 {
        blocking_reasons.push("unsupported_claim_ledger_entry".to_string());
        blockers.push(blocker(
            "unsupported_factual_fragment",
            "Regenerate the editorial draft or bind every claim-ledger entry to verified support.",
        ));
    }
    let unsupported_factual_blocks = draft
        .content_blocks
        .iter()
        .filter(|block| factual_role(&block.section_role))
        .filter(|block| {
            !draft.claim_ledger.iter().any(|claim| {
                claim.claim_kind == "factual"
                    && claim.validation_verdict == "supported"
                    && has_exact_evidence(&claim.fragment_text)
                    && block
                        .support_refs
                        .iter()
                        .any(|support_ref| claim.support_refs.contains(support_ref))
                    && block.markdown.contains(claim.fragment_text.trim())
            })
        })
        .count();
    if unsupported_factual_blocks > 0 {
        blocking_reasons.push("factual_block_without_exact_evidence".to_string());
        blockers.push(blocker(
            "factual_block_without_exact_evidence",
            "Use an exact VERIFIED source quotation for each factual block or request source revalidation.",
        ));
    }
    if draft
        .traceability_entries
        .iter()
        .any(|entry| entry.traceability_label == "serp_pattern_reference")
    {
        blocking_reasons.push("forbidden_serp_as_fact_usage".to_string());
        blockers.push(blocker(
            "forbidden_serp_as_fact_usage",
            "Remove SERP-derived evidence from factual support and bind to verified truth.",
        ));
    }
    let required_link_obligations = input
        .required_links
        .iter()
        .filter(|link| link.required_flag)
        .collect::<Vec<_>>();
    if !required_link_obligations.is_empty() {
        let related_pages_body = draft
            .sections
            .iter()
            .filter(|section| section.section_role == "related_pages")
            .map(|section| section.body_markdown.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let missing_required_links = required_link_obligations.iter().any(|link| {
            !related_pages_body.contains(&link.target_page_key)
                && !related_pages_body.contains(&link.link_role)
        });
        if missing_required_links {
            blocking_reasons.push("missing_required_internal_links".to_string());
            blockers.push(blocker(
                "missing_required_internal_links",
                "Generate and attach at least one required internal link obligation.",
            ));
        }
    }
    let missing_required_section = !draft.sections.iter().any(|section| section.required);
    if missing_required_section {
        blocking_reasons.push("missing_required_sections".to_string());
        blockers.push(blocker(
            "missing_required_sections",
            "Assemble all required expert page sections from the attached blueprint.",
        ));
    }
    if draft.truth_snapshot_ref.trim().is_empty() {
        blocking_reasons.push("missing_required_metadata".to_string());
        blockers.push(blocker(
            "missing_required_metadata",
            "Populate truth snapshot metadata before publish gate.",
        ));
    }
    if draft.traceability_entries.is_empty() {
        blocking_reasons.push("missing_traceability_manifest".to_string());
        blockers.push(blocker(
            "missing_traceability_manifest",
            "Persist fragment-level traceability metadata before publish gate.",
        ));
    }
    if draft.claim_ledger.is_empty() {
        blocking_reasons.push("missing_claim_ledger".to_string());
        blockers.push(blocker(
            "missing_claim_ledger",
            "Persist the LLM/editorial claim ledger before publish gate.",
        ));
    }
    if draft.content_blocks.is_empty() {
        blocking_reasons.push("missing_rendered_content_blocks".to_string());
        blockers.push(blocker(
            "missing_rendered_content_blocks",
            "Build typed content blocks for headless CMS rendering.",
        ));
    }
    if draft.schema_markup_json.trim().is_empty() || draft.schema_markup_json.trim() == "{}" {
        blocking_reasons.push("missing_schema_markup".to_string());
        blockers.push(blocker(
            "missing_schema_markup",
            "Generate Article/Breadcrumb/FAQ schema candidates before review.",
        ));
    }
    blocking_reasons.sort();
    blocking_reasons.dedup();
    let verdict = if blocking_reasons.is_empty() {
        "publish_ready"
    } else {
        "qa_failed"
    };
    DraftQaOutputPayload {
        verdict: verdict.to_string(),
        blocking_reasons,
        supported_claims,
        unsupported_claims: unsupported_claims + unsupported_ledger_claims
            + unsupported_factual_blocks as u32,
        traceability_entries: draft.traceability_entries,
        blockers,
        required_next_action: if verdict == "publish_ready" {
            "request_human_review".to_string()
        } else {
            "resolve_blockers_and_rerun_draft_qa".to_string()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        ClaimLedgerEntry, DraftState, RenderedContentBlock, SeoTraceabilityEntryState,
    };

    #[test]
    fn fabricated_factual_statement_with_valid_ref_is_blocked() {
        let forged = "An unsupported processing time is exactly 3 days.";
        let draft = DraftState {
            claim_ledger: vec![ClaimLedgerEntry {
                claim_kind: "factual".to_string(),
                validation_verdict: "supported".to_string(),
                traceability_label: "verified_fact".to_string(),
                fragment_text: forged.to_string(),
                support_refs: vec!["rule:1".to_string()],
                ..Default::default()
            }],
            traceability_entries: vec![SeoTraceabilityEntryState {
                fragment_kind: "factual".to_string(),
                traceability_label: "verified_fact".to_string(),
                validation_verdict: "supported".to_string(),
                fragment_text: forged.to_string(),
                support_refs: vec!["rule:1".to_string()],
                ..Default::default()
            }],
            content_blocks: vec![RenderedContentBlock {
                section_role: "timing".to_string(),
                markdown: forged.to_string(),
                support_refs: vec!["rule:1".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let qa = execute(&DraftQaInputPayload {
            draft: Some(draft),
            supported_fragments: vec!["A different, actual primary-source quotation.".to_string()],
            ..Default::default()
        });
        assert_eq!(qa.verdict, "qa_failed");
        assert!(qa.blocking_reasons.contains(&"unsupported_claim_ledger_entry".to_string()));
        assert!(qa.blocking_reasons.contains(&"factual_block_without_exact_evidence".to_string()));
    }

    #[test]
    fn exact_verified_quote_is_not_rejected_by_new_evidence_gate() {
        let quote = "The competent authority determines processing time.";
        let draft = DraftState {
            claim_ledger: vec![ClaimLedgerEntry {
                claim_kind: "factual".to_string(),
                validation_verdict: "supported".to_string(),
                traceability_label: "verified_fact".to_string(),
                fragment_text: quote.to_string(),
                support_refs: vec!["rule:1".to_string()],
                ..Default::default()
            }],
            traceability_entries: vec![SeoTraceabilityEntryState {
                fragment_kind: "factual".to_string(),
                traceability_label: "verified_fact".to_string(),
                validation_verdict: "supported".to_string(),
                fragment_text: quote.to_string(),
                support_refs: vec!["rule:1".to_string()],
                ..Default::default()
            }],
            content_blocks: vec![RenderedContentBlock {
                section_role: "timing".to_string(),
                markdown: quote.to_string(),
                support_refs: vec!["rule:1".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let qa = execute(&DraftQaInputPayload {
            draft: Some(draft),
            supported_fragments: vec![quote.to_string()],
            ..Default::default()
        });
        assert!(!qa.blocking_reasons.contains(&"unsupported_claim_ledger_entry".to_string()));
        assert!(!qa.blocking_reasons.contains(&"factual_block_without_exact_evidence".to_string()));
    }
}
