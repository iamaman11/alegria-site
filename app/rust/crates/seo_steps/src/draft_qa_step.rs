use contracts::generated::alegria::temporal::v1::{
    DraftQaInputPayload, DraftQaOutputPayload, DraftState, SeoPublishBlockerState,
};
use seo_domain::draft_quality::{self, ContentBlock, DraftQuality, Section, Traceability};

fn license_blocker(reason: &str, action: &str) -> SeoPublishBlockerState {
    SeoPublishBlockerState {
        reason_code: reason.to_string(),
        task_type: "publish_gate_blocker".to_string(),
        required_next_action: action.to_string(),
        recheck_trigger: "draft_qa_rerun".to_string(),
    }
}

pub fn execute(input: &DraftQaInputPayload) -> DraftQaOutputPayload {
    let empty = DraftState::default();
    let draft = input.draft.as_ref().unwrap_or(&empty);
    let mut blocking_reasons = Vec::new();
    let mut blockers = Vec::new();
    for blocker in crate::content_contract_validate_step::validate_for_publish(input) {
        blocking_reasons.push(blocker.reason_code.clone());
        blockers.push(blocker);
    }
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
        blockers.push(license_blocker(
            reason,
            seo_domain::publishing::license::RESTRICTED_SOURCE_ACTION,
        ));
    }

    let sections = draft
        .sections
        .iter()
        .map(|section| Section {
            role: &section.section_role,
            heading: &section.heading,
            body_markdown: &section.body_markdown,
            support_refs: &section.support_refs,
            required: section.required,
        })
        .collect::<Vec<_>>();
    let blocks = draft
        .content_blocks
        .iter()
        .map(|block| ContentBlock {
            role: &block.section_role,
            heading: &block.heading,
            markdown: &block.markdown,
            support_refs: &block.support_refs,
        })
        .collect::<Vec<_>>();
    let traceability = draft
        .traceability_entries
        .iter()
        .map(|entry| Traceability {
            fragment_kind: &entry.fragment_kind,
            fragment_text: &entry.fragment_text,
            traceability_label: &entry.traceability_label,
            validation_verdict: &entry.validation_verdict,
            support_refs: &entry.support_refs,
        })
        .collect::<Vec<_>>();
    let claims = draft
        .claim_ledger
        .iter()
        .map(crate::editorial_wire::claim_view)
        .collect::<Vec<_>>();
    let verified_support = crate::editorial_wire::support_views(&input.verified_support);
    let qa = draft_quality::evaluate(&DraftQuality {
        body_markdown: &draft.body_markdown,
        locale: &draft.editorial_locale,
        truth_snapshot_ref: &draft.truth_snapshot_ref,
        schema_markup_json: &draft.schema_markup_json,
        sections: &sections,
        blocks: &blocks,
        claims: &claims,
        traceability: &traceability,
        verified_support: &verified_support,
        supported_fragments: &input.supported_fragments,
    });
    for violation in qa.violations {
        blocking_reasons.push(violation.reason_code().to_string());
        blockers.push(SeoPublishBlockerState {
            reason_code: violation.blocker_reason_code().to_string(),
            task_type: violation.task_type().to_string(),
            required_next_action: violation.required_next_action().to_string(),
            recheck_trigger: "draft_qa_rerun".to_string(),
        });
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
        supported_claims: qa.supported_claims,
        unsupported_claims: qa.unsupported_claims,
        traceability_entries: draft.traceability_entries.clone(),
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
        let markdown = seo_domain::editorial::render_editorial_claim(
            crate::editorial_wire::claim_view(&claim),
            "ru",
        )
        .unwrap();
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
