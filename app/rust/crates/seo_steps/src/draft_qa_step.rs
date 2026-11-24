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

fn typed_claim_block_valid(
    draft: &contracts::generated::alegria::temporal::v1::DraftState,
    block: &contracts::generated::alegria::temporal::v1::RenderedContentBlock,
    support: &[contracts::generated::alegria::temporal::v1::SeoVerifiedFactSupportState],
) -> bool {
    use runtime_models::seo_blocks::{
        render_editorial_claim, supports_for_role, validate_editorial_claim,
    };
    let claims = draft
        .claim_ledger
        .iter()
        .filter(|claim| claim.source_section_key == block.section_role)
        .collect::<Vec<_>>();
    if claims.is_empty() || draft.editorial_locale.is_empty() {
        return false;
    }
    let mut rendered = Vec::new();
    let mut refs = BTreeSet::new();
    for claim in claims {
        if validate_editorial_claim(claim, support).is_err()
            || !refs.insert(claim.support_refs[0].as_str())
        {
            return false;
        }
        match render_editorial_claim(claim, &draft.editorial_locale) {
            Ok(text) => rendered.push(text),
            Err(_) => return false,
        }
    }
    let block_refs = block
        .support_refs
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if block_refs.len() != block.support_refs.len() || block_refs != refs {
        return false;
    }
    if !matches!(block.section_role.as_str(), "overview" | "faq")
        && supports_for_role(support, &block.section_role)
            .iter()
            .map(|entry| entry.support_ref.as_str())
            .collect::<BTreeSet<_>>()
            != refs
    {
        return false;
    }
    let expected = rendered.join("\n\n");
    block.markdown == expected
        && draft
            .sections
            .iter()
            .filter(|section| section.section_role == block.section_role)
            .count()
            == 1
        && draft.sections.iter().any(|section| {
            section.section_role == block.section_role
                && section.body_markdown == expected
                && section.heading == block.heading
                && section.support_refs == block.support_refs
        })
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
    // This is a domain publish policy, not a separate Temporal activity.
    // One blocker is emitted even when several restricted fragments occur.
    if seo_domain::publishing::license::has_restricted_source(
        draft.traceability_entries.iter().map(|entry| {
            (
                entry.traceability_label.as_str(),
                entry.validation_verdict.as_str(),
            )
        }),
    ) {
        let reason = seo_domain::publishing::license::RESTRICTED_SOURCE_REASON;
        blocking_reasons.push(reason.to_string());
        blockers.push(blocker(
            reason,
            seo_domain::publishing::license::RESTRICTED_SOURCE_ACTION,
        ));
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
    let typed_editorial = !draft.editorial_locale.is_empty()
        || draft
            .claim_ledger
            .iter()
            .any(|claim| claim.editorial_proposal.is_some());
    if typed_editorial {
        let expected_body = draft
            .sections
            .iter()
            .map(|section| format!("## {}\n\n{}\n", section.heading, section.body_markdown))
            .collect::<Vec<_>>()
            .join("\n");
        let body_matches = draft
            .body_markdown
            .split_once("\n\n")
            .is_some_and(|(title, body)| {
                title.starts_with("# ") && !title.contains('\n') && body == expected_body
            });
        let complete = !draft.editorial_locale.is_empty()
            && !draft.claim_ledger.is_empty()
            && draft.claim_ledger.iter().all(|claim| {
                claim.editorial_proposal.is_some()
                    && draft.content_blocks.iter().any(|block| {
                        factual_role(&block.section_role)
                            && block.section_role == claim.source_section_key
                    })
            });
        let sections_match = draft.sections.len() == draft.content_blocks.len()
            && draft.sections.iter().all(|section| {
                draft
                    .content_blocks
                    .iter()
                    .filter(|block| block.section_role == section.section_role)
                    .count()
                    == 1
                    && draft.content_blocks.iter().any(|block| {
                        block.section_role == section.section_role
                            && block.heading == section.heading
                            && block.markdown == section.body_markdown
                            && block.support_refs == section.support_refs
                    })
            });
        let non_factual_match = draft
            .content_blocks
            .iter()
            .filter(|block| !factual_role(&block.section_role))
            .all(|block| match block.section_role.as_str() {
                "cta_disclaimer" => {
                    block.markdown
                        == runtime_models::seo_blocks::editorial_cta(&draft.editorial_locale)
                }
                "related_pages" => {
                    block.markdown
                        == runtime_models::seo_blocks::render_verified_source_list(
                            &input.verified_support,
                            &draft.editorial_locale,
                        )
                }
                _ => false,
            });
        if !body_matches || !complete || !sections_match || !non_factual_match {
            blocking_reasons.push("invalid_editorial_claim_rendering".to_string());
            blockers.push(blocker(
                "invalid_editorial_claim_rendering",
                "Rebuild the entire localized draft from its typed, source-bound claim ledger.",
            ));
        }
    }
    let unsupported_factual_blocks = draft
        .content_blocks
        .iter()
        .filter(|block| factual_role(&block.section_role))
        .filter(|block| {
            if typed_editorial {
                return !typed_claim_block_valid(&draft, block, &input.verified_support);
            }
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
            "Use the exact verified source and canonical claim rendering for each factual block or request source revalidation.",
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
        let missing_required_links = if typed_editorial {
            related_pages_body.trim().is_empty()
                || required_link_obligations
                    .iter()
                    .any(|link| !draft.required_links.iter().any(|bound| bound == *link))
        } else {
            required_link_obligations.iter().any(|link| {
                !related_pages_body.contains(&link.target_page_key)
                    && !related_pages_body.contains(&link.link_role)
            })
        };
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
        unsupported_claims: unsupported_claims
            + unsupported_ledger_claims
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
    fn multiple_restricted_fragments_emit_exactly_one_compatible_publish_blocker() {
        let draft = DraftState {
            traceability_entries: vec![
                SeoTraceabilityEntryState {
                    traceability_label: "restricted_redistribution_source".to_string(),
                    validation_verdict: "blocked".to_string(),
                    ..Default::default()
                },
                SeoTraceabilityEntryState {
                    traceability_label: "non_redistributable_source".to_string(),
                    validation_verdict: "supported".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let qa = execute(&DraftQaInputPayload {
            draft: Some(draft),
            ..Default::default()
        });
        let licensing_blockers = qa
            .blockers
            .iter()
            .filter(|blocker| blocker.reason_code == "restricted_redistribution_source")
            .collect::<Vec<_>>();
        assert_eq!(licensing_blockers.len(), 1);
        assert_eq!(licensing_blockers[0].task_type, "publish_gate_blocker");
        assert_eq!(licensing_blockers[0].recheck_trigger, "draft_qa_rerun");
        assert_eq!(
            licensing_blockers[0].required_next_action,
            seo_domain::publishing::license::RESTRICTED_SOURCE_ACTION
        );
        assert_eq!(
            qa.blocking_reasons
                .iter()
                .filter(|reason| reason.as_str() == "restricted_redistribution_source")
                .count(),
            1
        );
    }

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
        assert!(qa
            .blocking_reasons
            .contains(&"unsupported_claim_ledger_entry".to_string()));
        assert!(qa
            .blocking_reasons
            .contains(&"factual_block_without_exact_evidence".to_string()));
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
        assert!(!qa
            .blocking_reasons
            .contains(&"unsupported_claim_ledger_entry".to_string()));
        assert!(!qa
            .blocking_reasons
            .contains(&"factual_block_without_exact_evidence".to_string()));
    }
}

#[cfg(test)]
mod typed_claim_qa_tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::{
        ClaimLedgerEntry, DraftState, EditorialClaimProposal, RenderedContentBlock,
        SeoDraftSectionState, SeoVerifiedFactSupportState,
    };

    #[test]
    fn canonical_render_gate_rejects_unbound_extra_prose_and_forged_ref() {
        let source = SeoVerifiedFactSupportState {
            support_ref: "rule1".into(),
            role_type: "timeline_item".into(),
            freshness_class: "fresh".into(),
            fragment_text: "The decision normally takes 15 days.".into(),
            source_url: "https://example.org/official".into(),
            ..Default::default()
        };
        let claim = ClaimLedgerEntry {
            claim_kind: "factual".into(),
            traceability_label: "verified_fact".into(),
            validation_verdict: "supported".into(),
            source_section_key: "timing".into(),
            fragment_text: source.fragment_text.clone(),
            support_refs: vec![source.support_ref.clone()],
            editorial_proposal: Some(EditorialClaimProposal {
                text: "Обычно решение принимается за 15 дней.".into(),
                source_url: source.source_url.clone(),
                question: String::new(),
                evidence_spans: Vec::new(),
            }),
            ..Default::default()
        };
        let markdown = runtime_models::seo_blocks::render_editorial_claim(&claim, "ru").unwrap();
        let mut block = RenderedContentBlock {
            section_role: "timing".into(),
            heading: "Сроки".into(),
            markdown: markdown.clone(),
            support_refs: vec![source.support_ref.clone()],
            ..Default::default()
        };
        let mut draft = DraftState {
            editorial_locale: "ru".into(),
            claim_ledger: vec![claim],
            sections: vec![SeoDraftSectionState {
                section_role: "timing".into(),
                heading: block.heading.clone(),
                body_markdown: markdown.clone(),
                support_refs: block.support_refs.clone(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(typed_claim_block_valid(&draft, &block, &[source.clone()]));
        block.markdown.push_str("\n\nГарантированное одобрение.");
        assert!(!typed_claim_block_valid(&draft, &block, &[source.clone()]));
        block.markdown = markdown;
        assert!(!typed_claim_block_valid(&draft, &block, &[]));
        draft.claim_ledger[0].support_refs[0] = "wrong-ref".into();
        assert!(!typed_claim_block_valid(&draft, &block, &[source]));
    }
}
