//! Editorial quality gates beyond the content-block contract.
//!
//! This is the one owner for evidence coverage, canonical typed-claim
//! rendering and draft completeness. Temporal only converts input/output.

use std::collections::BTreeSet;

use crate::editorial::{
    editorial_cta, factual_role, render_editorial_claim, render_verified_source_list,
    role_matches_section, validate_editorial_claim, verified_traceability_evidence, ContentBlock,
    EditorialClaim, VerifiedSupport,
};

#[derive(Clone, Copy)]
pub struct Traceability<'a> {
    pub fragment_kind: &'a str,
    pub fragment_text: &'a str,
    pub traceability_label: &'a str,
    pub validation_verdict: &'a str,
    pub support_refs: &'a [String],
}

#[derive(Clone, Copy)]
pub struct Section<'a> {
    pub role: &'a str,
    pub heading: &'a str,
    pub body_markdown: &'a str,
    pub support_refs: &'a [String],
    pub required: bool,
}

#[derive(Clone, Copy)]
pub struct DraftQuality<'a> {
    pub body_markdown: &'a str,
    pub locale: &'a str,
    pub truth_snapshot_ref: &'a str,
    pub schema_markup_json: &'a str,
    pub sections: &'a [Section<'a>],
    /// Approved blueprint roles, not self-reported section.required flags.
    pub expected_required_sections: &'a [String],
    pub blocks: &'a [ContentBlock<'a>],
    pub claims: &'a [EditorialClaim<'a>],
    pub traceability: &'a [Traceability<'a>],
    pub verified_support: &'a [VerifiedSupport<'a>],
    pub supported_fragments: &'a [String],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Violation {
    UnsupportedFactualFragment,
    UnsupportedClaimLedgerEntry,
    InvalidEditorialClaimRendering,
    FactualBlockWithoutExactEvidence,
    ForbiddenSerpAsFactUsage,
    MissingRequiredSections,
    MissingRequiredMetadata,
    MissingTraceabilityManifest,
    MissingClaimLedger,
    MissingRenderedContentBlocks,
    MissingSchemaMarkup,
}

impl Violation {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::UnsupportedFactualFragment => "unsupported_factual_fragment",
            Self::UnsupportedClaimLedgerEntry => "unsupported_claim_ledger_entry",
            Self::InvalidEditorialClaimRendering => "invalid_editorial_claim_rendering",
            Self::FactualBlockWithoutExactEvidence => "factual_block_without_exact_evidence",
            Self::ForbiddenSerpAsFactUsage => "forbidden_serp_as_fact_usage",
            Self::MissingRequiredSections => "missing_required_sections",
            Self::MissingRequiredMetadata => "missing_required_metadata",
            Self::MissingTraceabilityManifest => "missing_traceability_manifest",
            Self::MissingClaimLedger => "missing_claim_ledger",
            Self::MissingRenderedContentBlocks => "missing_rendered_content_blocks",
            Self::MissingSchemaMarkup => "missing_schema_markup",
        }
    }

    /// Preserve the historical distinction: the ledger's diagnostic is more
    /// specific than the remediation task's common unsupported-fragment code.
    pub const fn blocker_reason_code(self) -> &'static str {
        match self {
            Self::UnsupportedClaimLedgerEntry => "unsupported_factual_fragment",
            _ => self.reason_code(),
        }
    }

    pub const fn task_type(self) -> &'static str {
        match self {
            Self::UnsupportedFactualFragment
            | Self::UnsupportedClaimLedgerEntry
            | Self::ForbiddenSerpAsFactUsage => "unsupported_factual_fragment",
            _ => "publish_gate_blocker",
        }
    }

    pub const fn required_next_action(self) -> &'static str {
        match self {
            Self::UnsupportedFactualFragment =>
                "Bind every factual fragment to verified_fact or verified_summary support.",
            Self::UnsupportedClaimLedgerEntry =>
                "Regenerate the editorial draft or bind every claim-ledger entry to verified support.",
            Self::InvalidEditorialClaimRendering =>
                "Rebuild the entire localized draft from its typed, source-bound claim ledger.",
            Self::FactualBlockWithoutExactEvidence =>
                "Use the exact verified source and canonical claim rendering for each factual block or request source revalidation.",
            Self::ForbiddenSerpAsFactUsage =>
                "Remove SERP-derived evidence from factual support and bind to verified truth.",
            Self::MissingRequiredSections =>
                "Assemble all required expert page sections from the attached blueprint.",
            Self::MissingRequiredMetadata =>
                "Populate truth snapshot metadata before publish gate.",
            Self::MissingTraceabilityManifest =>
                "Persist fragment-level traceability metadata before publish gate.",
            Self::MissingClaimLedger =>
                "Persist the LLM/editorial claim ledger before publish gate.",
            Self::MissingRenderedContentBlocks =>
                "Build typed content blocks for headless CMS rendering.",
            Self::MissingSchemaMarkup =>
                "Generate Article/Breadcrumb/FAQ schema candidates before review.",
        }
    }
}

#[derive(Debug, Default)]
pub struct Result {
    pub supported_claims: u32,
    pub unsupported_claims: u32,
    pub violations: Vec<Violation>,
}

/// Translate the QA verdict into the single review lifecycle decision. A
/// successful QA must have no blockers; a failed QA must report at least one.
/// Inconsistent persisted output must be rejected rather than inferred.
pub fn review_lifecycle(verdict: &str, blocker_count: usize) -> Option<&'static str> {
    match (verdict, blocker_count) {
        ("publish_ready", 0) | ("qa_failed", 1..) => Some("review_required"),
        _ => None,
    }
}

fn valid_typed_block(draft: &DraftQuality<'_>, block: &ContentBlock<'_>) -> bool {
    let claims = draft
        .claims
        .iter()
        .filter(|claim| claim.source_section_key == block.role)
        .copied()
        .collect::<Vec<_>>();
    if claims.is_empty() || draft.locale.is_empty() {
        return false;
    }
    let mut rendered = Vec::new();
    let mut refs = BTreeSet::new();
    for claim in claims {
        if validate_editorial_claim(claim, draft.verified_support).is_err()
            || !refs.insert(claim.support_refs[0].as_str())
        {
            return false;
        }
        match render_editorial_claim(claim, draft.locale) {
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
    if !matches!(block.role, "overview" | "faq")
        && draft
            .verified_support
            .iter()
            .filter(|entry| role_matches_section(entry.role_type, block.role))
            .map(|entry| entry.support_ref)
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
            .filter(|section| section.role == block.role)
            .count()
            == 1
        && draft.sections.iter().any(|section| {
            section.role == block.role
                && section.body_markdown == expected
                && section.heading == block.heading
                && section.support_refs == block.support_refs
        })
}

pub fn evaluate(draft: &DraftQuality<'_>) -> Result {
    let mut result = Result::default();
    let verified_fragments = draft
        .supported_fragments
        .iter()
        .map(|fragment| fragment.trim())
        .filter(|fragment| !fragment.is_empty())
        .collect::<BTreeSet<_>>();
    let has_exact_evidence = |fragment: &str| verified_fragments.contains(fragment.trim());
    result.supported_claims = draft
        .traceability
        .iter()
        .filter(|entry| {
            verified_traceability_evidence(entry.validation_verdict, entry.traceability_label)
                && !entry.support_refs.is_empty()
                && has_exact_evidence(entry.fragment_text)
        })
        .count() as u32;
    let unsupported_traceability = draft
        .traceability
        .iter()
        .filter(|entry| {
            entry.traceability_label == "unsupported_factual_fragment"
                || (entry.fragment_kind == "factual"
                    && (entry.support_refs.is_empty()
                        || entry.validation_verdict != "supported"
                        || !has_exact_evidence(entry.fragment_text)))
        })
        .count() as u32;
    let unsupported_ledger = draft
        .claims
        .iter()
        .filter(|claim| {
            claim.validation_verdict == "blocked"
                || claim.traceability_label == "unsupported_factual_fragment"
                || (claim.claim_kind == "factual"
                    && (claim.support_refs.is_empty()
                        || claim.validation_verdict != "supported"
                        || !has_exact_evidence(claim.fragment_text)))
        })
        .count() as u32;
    if draft.body_markdown.contains("[unsupported") || unsupported_traceability > 0 {
        result
            .violations
            .push(Violation::UnsupportedFactualFragment);
    }
    if unsupported_ledger > 0 {
        result
            .violations
            .push(Violation::UnsupportedClaimLedgerEntry);
    }
    let typed_editorial =
        !draft.locale.is_empty() || draft.claims.iter().any(|claim| claim.proposal.is_some());
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
        let complete = !draft.locale.is_empty()
            && !draft.claims.is_empty()
            && draft.claims.iter().all(|claim| {
                claim.proposal.is_some()
                    && draft.blocks.iter().any(|block| {
                        factual_role(block.role) && block.role == claim.source_section_key
                    })
            });
        let sections_match = draft.sections.len() == draft.blocks.len()
            && draft.sections.iter().all(|section| {
                draft
                    .blocks
                    .iter()
                    .filter(|block| block.role == section.role)
                    .count()
                    == 1
                    && draft.blocks.iter().any(|block| {
                        block.role == section.role
                            && block.heading == section.heading
                            && block.markdown == section.body_markdown
                            && block.support_refs == section.support_refs
                    })
            });
        let non_factual_match = draft
            .blocks
            .iter()
            .filter(|block| !factual_role(block.role))
            .all(|block| match block.role {
                "cta_disclaimer" => block.markdown == editorial_cta(draft.locale),
                "related_pages" => {
                    block.markdown
                        == render_verified_source_list(draft.verified_support, draft.locale)
                }
                _ => false,
            });
        if !body_matches || !complete || !sections_match || !non_factual_match {
            result
                .violations
                .push(Violation::InvalidEditorialClaimRendering);
        }
    }
    let unsupported_blocks = draft
        .blocks
        .iter()
        .filter(|block| factual_role(block.role))
        .filter(|block| {
            if typed_editorial {
                return !valid_typed_block(draft, block);
            }
            !draft.claims.iter().any(|claim| {
                claim.claim_kind == "factual"
                    && claim.validation_verdict == "supported"
                    && has_exact_evidence(claim.fragment_text)
                    && block
                        .support_refs
                        .iter()
                        .any(|support_ref| claim.support_refs.contains(support_ref))
                    && block.markdown.contains(claim.fragment_text.trim())
            })
        })
        .count() as u32;
    if unsupported_blocks > 0 {
        result
            .violations
            .push(Violation::FactualBlockWithoutExactEvidence);
    }
    if draft
        .traceability
        .iter()
        .any(|entry| entry.traceability_label == "serp_pattern_reference")
    {
        result.violations.push(Violation::ForbiddenSerpAsFactUsage);
    }
    let present_required_roles = draft
        .sections
        .iter()
        .filter(|section| section.required && !section.body_markdown.trim().is_empty())
        .map(|section| section.role)
        .collect::<BTreeSet<_>>();
    if draft.expected_required_sections.is_empty()
        || draft
            .expected_required_sections
            .iter()
            .any(|role| !present_required_roles.contains(role.as_str()))
    {
        result.violations.push(Violation::MissingRequiredSections);
    }
    if draft.truth_snapshot_ref.trim().is_empty() {
        result.violations.push(Violation::MissingRequiredMetadata);
    }
    if draft.traceability.is_empty() {
        result
            .violations
            .push(Violation::MissingTraceabilityManifest);
    }
    if draft.claims.is_empty() {
        result.violations.push(Violation::MissingClaimLedger);
    }
    if draft.blocks.is_empty() {
        result
            .violations
            .push(Violation::MissingRenderedContentBlocks);
    }
    if draft.schema_markup_json.trim().is_empty() || draft.schema_markup_json.trim() == "{}" {
        result.violations.push(Violation::MissingSchemaMarkup);
    }
    result.unsupported_claims = unsupported_traceability + unsupported_ledger + unsupported_blocks;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editorial::EditorialProposal;

    #[test]
    fn empty_draft_fails_closed_with_stable_diagnostics() {
        let input = DraftQuality {
            body_markdown: "",
            locale: "",
            truth_snapshot_ref: "",
            schema_markup_json: "",
            sections: &[],
            expected_required_sections: &[],
            blocks: &[],
            claims: &[],
            traceability: &[],
            verified_support: &[],
            supported_fragments: &[],
        };
        let result = evaluate(&input);
        assert_eq!(result.supported_claims, 0);
        assert_eq!(result.unsupported_claims, 0);
        assert_eq!(result.violations.len(), 6);
        assert!(result.violations.contains(&Violation::MissingSchemaMarkup));
        assert!(result.violations.contains(&Violation::MissingClaimLedger));
    }

    #[test]
    fn unsupported_ledger_remains_distinct_from_external_blocker_code() {
        assert_eq!(
            Violation::UnsupportedClaimLedgerEntry.reason_code(),
            "unsupported_claim_ledger_entry"
        );
        assert_eq!(
            Violation::UnsupportedClaimLedgerEntry.blocker_reason_code(),
            "unsupported_factual_fragment"
        );
        assert_eq!(
            Violation::UnsupportedClaimLedgerEntry.task_type(),
            "unsupported_factual_fragment"
        );
    }

    #[test]
    fn qa_verdict_and_blockers_have_one_review_lifecycle_owner() {
        assert_eq!(
            review_lifecycle("publish_ready", 0),
            Some("review_required")
        );
        assert_eq!(review_lifecycle("qa_failed", 1), Some("review_required"));
        assert_eq!(review_lifecycle("qa_failed", 4), Some("review_required"));
        for (verdict, count) in [("publish_ready", 1), ("qa_failed", 0), ("unexpected", 0)] {
            assert!(review_lifecycle(verdict, count).is_none());
        }
    }

    #[test]
    fn exact_verified_editorial_claim_is_publish_quality_ready() {
        let refs = vec!["rule:one".to_string()];
        let spans = Vec::new();
        let claim = EditorialClaim {
            source_section_key: "timing",
            claim_kind: "factual",
            validation_verdict: "supported",
            traceability_label: "verified_fact",
            fragment_text: "15 days",
            support_refs: &refs,
            proposal: Some(EditorialProposal {
                text: "15 days to decide.",
                question: "",
                source_url: "https://example.org/official",
                evidence_spans: &spans,
            }),
        };
        let markdown = render_editorial_claim(claim, "en").unwrap();
        let sections = [Section {
            role: "timing",
            heading: "Timing",
            body_markdown: &markdown,
            support_refs: &refs,
            required: true,
        }];
        let blocks = [ContentBlock {
            role: "timing",
            block_type: "timeline",
            heading: "Timing",
            markdown: &markdown,
            required: true,
            support_refs: &refs,
        }];
        let traceability = [Traceability {
            fragment_kind: "factual",
            fragment_text: "15 days",
            traceability_label: "verified_fact",
            validation_verdict: "supported",
            support_refs: &refs,
        }];
        let support = [VerifiedSupport {
            support_ref: "rule:one",
            role_type: "timeline_item",
            freshness_class: "fresh",
            fragment_text: "15 days",
            source_url: "https://example.org/official",
        }];
        let fragments = vec!["15 days".to_string()];
        let body = format!("# Title\n\n## Timing\n\n{markdown}\n");
        let result = evaluate(&DraftQuality {
            body_markdown: &body,
            locale: "en",
            truth_snapshot_ref: "snapshot",
            schema_markup_json: "{\"@type\":\"Article\"}",
            sections: &sections,
            expected_required_sections: &["timing".to_string()],
            blocks: &blocks,
            claims: &[claim],
            traceability: &traceability,
            verified_support: &support,
            supported_fragments: &fragments,
        });
        assert!(result.violations.is_empty(), "{:?}", result.violations);
        assert_eq!(result.supported_claims, 1);
        assert_eq!(result.unsupported_claims, 0);
        let missing_approved_section = evaluate(&DraftQuality {
            body_markdown: &body,
            locale: "en",
            truth_snapshot_ref: "snapshot",
            schema_markup_json: "{\"@type\":\"Article\"}",
            sections: &sections,
            expected_required_sections: &["timing".to_string(), "documents".to_string()],
            blocks: &blocks,
            claims: &[claim],
            traceability: &traceability,
            verified_support: &support,
            supported_fragments: &fragments,
        });
        assert!(missing_approved_section
            .violations
            .contains(&Violation::MissingRequiredSections));
    }

    #[test]
    fn canonical_render_gate_rejects_extra_prose_missing_support_and_forged_reference() {
        let refs = vec!["rule1".to_string()];
        let wrong_refs = vec!["wrong-ref".to_string()];
        let spans = Vec::new();
        let claim = EditorialClaim {
            claim_kind: "factual",
            traceability_label: "verified_fact",
            validation_verdict: "supported",
            source_section_key: "timing",
            fragment_text: "The decision normally takes 15 days.",
            support_refs: &refs,
            proposal: Some(EditorialProposal {
                text: "Обычно решение принимается за 15 дней.",
                source_url: "https://example.org/official",
                question: "",
                evidence_spans: &spans,
            }),
        };
        let source = VerifiedSupport {
            support_ref: "rule1",
            role_type: "timeline_item",
            freshness_class: "fresh",
            fragment_text: "The decision normally takes 15 days.",
            source_url: "https://example.org/official",
        };
        let markdown = render_editorial_claim(claim, "ru").unwrap();
        let block = ContentBlock {
            role: "timing",
            block_type: "timeline",
            heading: "Сроки",
            markdown: &markdown,
            required: true,
            support_refs: &refs,
        };
        let sections = [Section {
            role: "timing",
            heading: block.heading,
            body_markdown: &markdown,
            support_refs: &refs,
            required: true,
        }];
        let claims = [claim];
        let support = [source];
        let fragments = vec!["The decision normally takes 15 days.".to_string()];
        let draft = DraftQuality {
            body_markdown: "",
            locale: "ru",
            truth_snapshot_ref: "snapshot",
            schema_markup_json: "{\"@type\":\"Article\"}",
            sections: &sections,
            expected_required_sections: &["timing".to_string()],
            blocks: &[],
            claims: &claims,
            traceability: &[],
            verified_support: &support,
            supported_fragments: &fragments,
        };
        // Same policy formerly exercised through Temporal-specific helpers.
        assert!(valid_typed_block(&draft, &block));
        let extra_markdown = format!("{markdown}\n\nГарантированное одобрение.");
        assert!(!valid_typed_block(
            &draft,
            &ContentBlock {
                markdown: &extra_markdown,
                ..block
            },
        ));
        assert!(!valid_typed_block(
            &DraftQuality {
                verified_support: &[],
                ..draft
            },
            &block,
        ));
        let forged_claims = [EditorialClaim {
            support_refs: &wrong_refs,
            ..claim
        }];
        assert!(!valid_typed_block(
            &DraftQuality {
                claims: &forged_claims,
                ..draft
            },
            &block,
        ));
    }
}
