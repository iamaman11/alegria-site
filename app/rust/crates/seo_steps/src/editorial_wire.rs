//! Conversion at the Temporal wire boundary. No editorial policy lives here.

use contracts::generated::alegria::temporal::v1::{
    ClaimLedgerEntry, LinkRecommendationState, RenderedContentBlock, SectionTemplateBinding,
    SeoVerifiedFactSupportState,
};
use seo_domain::editorial::{
    ContentBlock, EditorialClaim, EditorialProposal, PlannedContentBlock, SectionTemplate,
    VerifiedSupport,
};

/// Adapt the one canonical rendered-block shape at the wire boundary.
/// Content Contract and final Draft QA consume this same domain view.
pub fn content_block_views(blocks: &[RenderedContentBlock]) -> Vec<ContentBlock<'_>> {
    blocks
        .iter()
        .map(|block| ContentBlock {
            role: &block.section_role,
            block_type: &block.block_type,
            heading: &block.heading,
            markdown: &block.markdown,
            required: block.required,
            support_refs: &block.support_refs,
        })
        .collect()
}

pub fn claim_view(claim: &ClaimLedgerEntry) -> EditorialClaim<'_> {
    EditorialClaim {
        source_section_key: &claim.source_section_key,
        claim_kind: &claim.claim_kind,
        validation_verdict: &claim.validation_verdict,
        traceability_label: &claim.traceability_label,
        fragment_text: &claim.fragment_text,
        support_refs: &claim.support_refs,
        proposal: claim
            .editorial_proposal
            .as_ref()
            .map(|proposal| EditorialProposal {
                text: &proposal.text,
                question: &proposal.question,
                source_url: &proposal.source_url,
                evidence_spans: &proposal.evidence_spans,
            }),
    }
}

pub fn support_views(support: &[SeoVerifiedFactSupportState]) -> Vec<VerifiedSupport<'_>> {
    support
        .iter()
        .map(|entry| VerifiedSupport {
            support_ref: &entry.support_ref,
            fragment_text: &entry.fragment_text,
            role_type: &entry.role_type,
            freshness_class: &entry.freshness_class,
            source_url: &entry.source_url,
        })
        .collect()
}

pub fn block_plan(
    templates: &[SectionTemplateBinding],
    support: &[SeoVerifiedFactSupportState],
    required_links: &[LinkRecommendationState],
) -> Vec<PlannedContentBlock> {
    let templates = templates
        .iter()
        .map(|entry| SectionTemplate {
            section_role: &entry.section_role,
            heading: &entry.heading,
            required: entry.required,
        })
        .collect::<Vec<_>>();
    seo_domain::editorial::planned_content_blocks(
        &templates,
        &support_views(support),
        required_links.iter().any(|link| link.required_flag),
    )
}

#[cfg(test)]
mod required_link_plan_tests {
    use super::*;

    #[test]
    fn optional_link_does_not_activate_an_unrequired_related_pages_block() {
        let template = SectionTemplateBinding {
            section_role: "related_pages".into(),
            required: false,
            ..Default::default()
        };
        let optional = LinkRecommendationState {
            target_page_key: "optional-target".into(),
            required_flag: false,
            ..Default::default()
        };
        assert!(block_plan(&[template.clone()], &[], &[optional]).is_empty());

        let required = LinkRecommendationState {
            target_page_key: "required-target".into(),
            required_flag: true,
            ..Default::default()
        };
        let plan = block_plan(&[template], &[], &[required]);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].section_role, "related_pages");
    }
}

#[cfg(test)]
mod localized_claim_tests {
    use super::*;
    fn validate_editorial_claim(
        claim: &ClaimLedgerEntry,
        support: &[SeoVerifiedFactSupportState],
    ) -> Result<(), seo_domain::editorial::EditorialClaimError> {
        seo_domain::editorial::validate_editorial_claim(claim_view(claim), &support_views(support))
    }

    fn render_editorial_claim(
        claim: &ClaimLedgerEntry,
        locale: &str,
    ) -> Result<String, seo_domain::editorial::EditorialClaimError> {
        seo_domain::editorial::render_editorial_claim(claim_view(claim), locale)
    }

    use seo_domain::editorial::EditorialClaimError;

    use contracts::generated::alegria::temporal::v1::{ClaimLedgerEntry, EditorialClaimProposal};

    fn sample() -> (ClaimLedgerEntry, SeoVerifiedFactSupportState) {
        let source = SeoVerifiedFactSupportState {
            support_ref: "rule-one".into(),
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
            support_refs: vec![source.support_ref.clone()],
            fragment_text: source.fragment_text.clone(),
            editorial_proposal: Some(EditorialClaimProposal {
                text: "Обычно решение принимается за 15 дней.".into(),
                source_url: source.source_url.clone(),
                question: String::new(),
                evidence_spans: Vec::new(),
            }),
            ..Default::default()
        };
        (claim, source)
    }

    #[test]
    fn verified_original_and_proposed_localized_text_are_distinct() {
        let (claim, source) = sample();
        assert_eq!(validate_editorial_claim(&claim, &[source.clone()]), Ok(()));
        let rendered = render_editorial_claim(&claim, "ru").unwrap();
        assert!(rendered.contains("Обычно решение принимается за 15 дней."));
        assert!(rendered.contains("[Источник](https://example.org/official)"));
        assert!(!rendered.contains(&source.fragment_text));
        assert_eq!(claim.fragment_text, source.fragment_text);
    }

    #[test]
    fn composite_fee_uses_exact_applicable_spans_without_unrelated_rate() {
        let (mut claim, mut source) = sample();
        source.role_type = "fee_item".into();
        source.fragment_text = concat!(
            "EUR 35 for applicants from Armenia, Azerbaijan, and Belarus\n",
            "EUR 67,50 for applicants from Cabo Verde.\n",
            "An additional fee may apply at visa service centres.\n",
            "The visa fee can be waived for specific categories."
        )
        .into();
        claim.source_section_key = "fees".into();
        claim.fragment_text = source.fragment_text.clone();
        let proposal = claim.editorial_proposal.as_mut().unwrap();
        proposal.text = "Для заявителей из Беларуси сбор EUR 35; сервисный сбор возможен отдельно, для некоторых категорий есть освобождение.".into();
        proposal.evidence_spans = vec![
            "EUR 35 for applicants from Armenia, Azerbaijan, and Belarus".into(),
            "An additional fee may apply at visa service centres.".into(),
            "The visa fee can be waived for specific categories.".into(),
        ];
        assert_eq!(validate_editorial_claim(&claim, &[source.clone()]), Ok(()));

        claim.editorial_proposal.as_mut().unwrap().evidence_spans[0] = "EUR 35".into();
        assert_eq!(
            validate_editorial_claim(&claim, &[source.clone()]),
            Err(EditorialClaimError::InvalidEvidenceSpan)
        );
        claim.editorial_proposal.as_mut().unwrap().evidence_spans[0] =
            "EUR 35 for applicants from Armenia, Azerbaijan, and Belarus".into();
        claim
            .editorial_proposal
            .as_mut()
            .unwrap()
            .evidence_spans
            .push("EUR 67,50 for applicants from Cabo Verde.".into());
        assert_eq!(
            validate_editorial_claim(&claim, &[source]),
            Err(EditorialClaimError::NumericEvidenceMismatch)
        );
    }

    #[test]
    fn reject_wrong_ref_quote_role_or_untrusted_link() {
        let (original, source) = sample();
        for claim in [
            ClaimLedgerEntry {
                support_refs: vec!["wrong".into()],
                ..original.clone()
            },
            ClaimLedgerEntry {
                fragment_text: "The decision takes 3 days.".into(),
                ..original.clone()
            },
            ClaimLedgerEntry {
                source_section_key: "fees".into(),
                ..original.clone()
            },
            ClaimLedgerEntry {
                editorial_proposal: Some(EditorialClaimProposal {
                    source_url: "https://unverified.test".into(),
                    ..original.editorial_proposal.clone().unwrap()
                }),
                ..original.clone()
            },
        ] {
            assert!(validate_editorial_claim(&claim, &[source.clone()]).is_err());
        }
    }

    #[test]
    fn reject_stale_ambiguous_evidence_and_markup_injection() {
        let (mut claim, mut source) = sample();
        assert!(validate_editorial_claim(&claim, &[source.clone(), source.clone()]).is_err());
        source.freshness_class = "stale".into();
        assert!(validate_editorial_claim(&claim, &[source.clone()]).is_err());
        claim.editorial_proposal.as_mut().unwrap().text =
            "[injected](https://elsewhere.test)".into();
        assert_eq!(
            render_editorial_claim(&claim, "ru"),
            Err(EditorialClaimError::InvalidText)
        );
    }

    #[test]
    fn faq_produces_reviewable_question_and_answer() {
        let (mut claim, source) = sample();
        claim.source_section_key = "faq".into();
        claim.traceability_label = "verified_summary".into();
        claim.editorial_proposal.as_mut().unwrap().question = "Сколько ждать решения?".into();
        assert!(validate_editorial_claim(&claim, &[source]).is_ok());
        assert!(render_editorial_claim(&claim, "ru")
            .unwrap()
            .starts_with("### Сколько ждать решения?"));
    }

    #[test]
    fn reject_new_or_dropped_numeric_values() {
        let (mut claim, source) = sample();
        for text in [
            "Решение принимается за 3 дня.",
            "Решение принимается быстро.",
        ] {
            claim.editorial_proposal.as_mut().unwrap().text = text.into();
            assert_eq!(
                validate_editorial_claim(&claim, &[source.clone()]),
                Err(EditorialClaimError::NumericEvidenceMismatch)
            );
        }
    }

    #[test]
    fn evidence_binding_is_not_a_semantic_verification() {
        let (mut claim, source) = sample();
        claim.editorial_proposal.as_mut().unwrap().text =
            "Решение никогда не принимается за 15 дней.".into();
        // Matching provenance does not prove the meaning of a paraphrase.
        // This proposal must still be rejected by the separate human review.
        assert!(validate_editorial_claim(&claim, &[source]).is_ok());
    }
}
