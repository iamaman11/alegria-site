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

/// Pure, full-fidelity editorial draft QA. This operation reports policy
/// diagnostics only; CMS approval and publication remain effectful owners.
pub mod operations {
    use std::collections::{BTreeMap, BTreeSet};
    use std::convert::Infallible;

    use operation_protocol::{
        ArtifactSchema, IdempotencyPolicy, Operation, OperationContract, OperationId,
        OperationOwnerId, OperationRef, PortId,
    };
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use super::{evaluate, DraftQuality, Section, Traceability};
    use crate::content_contract::operations::ContentBlockInput;
    use crate::editorial::{ContentBlock, EditorialClaim, EditorialProposal, VerifiedSupport};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct DraftSectionInput {
        pub role: String,
        pub heading: String,
        pub body_markdown: String,
        pub support_refs: Vec<String>,
        pub required: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TraceabilityInput {
        pub fragment_kind: String,
        pub fragment_text: String,
        pub traceability_label: String,
        pub validation_verdict: String,
        pub support_refs: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct VerifiedSupportInput {
        pub support_ref: String,
        pub fragment_text: String,
        pub role_type: String,
        pub freshness_class: String,
        pub source_url: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct EditorialProposalInput {
        pub text: String,
        pub question: String,
        pub source_url: String,
        pub evidence_spans: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct EditorialClaimInput {
        pub source_section_key: String,
        pub claim_kind: String,
        pub validation_verdict: String,
        pub traceability_label: String,
        pub fragment_text: String,
        pub support_refs: Vec<String>,
        pub proposal: Option<EditorialProposalInput>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct DraftQualityInput {
        pub body_markdown: String,
        pub locale: String,
        pub truth_snapshot_ref: String,
        pub schema_markup_json: String,
        pub sections: Vec<DraftSectionInput>,
        pub expected_required_sections: Vec<String>,
        pub blocks: Vec<ContentBlockInput>,
        pub claims: Vec<EditorialClaimInput>,
        pub traceability: Vec<TraceabilityInput>,
        pub verified_support: Vec<VerifiedSupportInput>,
        pub supported_fragments: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct DraftQualityDiagnostic {
        pub code: String,
        pub task_type: String,
        pub next_action: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct DraftQualityOutput {
        pub policy_passed: bool,
        pub supported_claims: u32,
        pub unsupported_claims: u32,
        pub diagnostics: Vec<DraftQualityDiagnostic>,
    }

    pub struct ValidateDraftQuality;

    impl Operation for ValidateDraftQuality {
        type Input = DraftQualityInput;
        type Output = DraftQualityOutput;
        type Failure = Infallible;

        fn contract() -> OperationContract {
            OperationContract {
                reference: OperationRef {
                    id: OperationId::from("editorial.validate_draft_quality"),
                    revision: 1,
                },
                owner: OperationOwnerId::from("editorial_policy"),
                inputs: BTreeMap::from([(
                    PortId::from("input"),
                    ArtifactSchema {
                        id: "editorial.draft_quality_input".into(),
                        revision: 1,
                    },
                )]),
                outputs: BTreeMap::from([(
                    PortId::from("output"),
                    ArtifactSchema {
                        id: "editorial.draft_quality_output".into(),
                        revision: 1,
                    },
                )]),
                invalidates: BTreeSet::new(),
                effects: BTreeSet::new(),
                idempotency: IdempotencyPolicy::Pure,
            }
        }
    }

    impl ValidateDraftQuality {
        pub fn evaluate(input: &DraftQualityInput) -> DraftQualityOutput {
            let sections = input
                .sections
                .iter()
                .map(|section| Section {
                    role: &section.role,
                    heading: &section.heading,
                    body_markdown: &section.body_markdown,
                    support_refs: &section.support_refs,
                    required: section.required,
                })
                .collect::<Vec<_>>();
            let blocks = input
                .blocks
                .iter()
                .map(|block| ContentBlock {
                    role: &block.role,
                    block_type: &block.block_type,
                    heading: &block.heading,
                    markdown: &block.markdown,
                    required: block.required,
                    support_refs: &block.support_refs,
                })
                .collect::<Vec<_>>();
            let claims = input
                .claims
                .iter()
                .map(|claim| EditorialClaim {
                    source_section_key: &claim.source_section_key,
                    claim_kind: &claim.claim_kind,
                    validation_verdict: &claim.validation_verdict,
                    traceability_label: &claim.traceability_label,
                    fragment_text: &claim.fragment_text,
                    support_refs: &claim.support_refs,
                    proposal: claim.proposal.as_ref().map(|proposal| EditorialProposal {
                        text: &proposal.text,
                        question: &proposal.question,
                        source_url: &proposal.source_url,
                        evidence_spans: &proposal.evidence_spans,
                    }),
                })
                .collect::<Vec<_>>();
            let traceability = input
                .traceability
                .iter()
                .map(|entry| Traceability {
                    fragment_kind: &entry.fragment_kind,
                    fragment_text: &entry.fragment_text,
                    traceability_label: &entry.traceability_label,
                    validation_verdict: &entry.validation_verdict,
                    support_refs: &entry.support_refs,
                })
                .collect::<Vec<_>>();
            let verified_support = input
                .verified_support
                .iter()
                .map(|entry| VerifiedSupport {
                    support_ref: &entry.support_ref,
                    fragment_text: &entry.fragment_text,
                    role_type: &entry.role_type,
                    freshness_class: &entry.freshness_class,
                    source_url: &entry.source_url,
                })
                .collect::<Vec<_>>();
            let result = evaluate(&DraftQuality {
                body_markdown: &input.body_markdown,
                locale: &input.locale,
                truth_snapshot_ref: &input.truth_snapshot_ref,
                schema_markup_json: &input.schema_markup_json,
                sections: &sections,
                expected_required_sections: &input.expected_required_sections,
                blocks: &blocks,
                claims: &claims,
                traceability: &traceability,
                verified_support: &verified_support,
                supported_fragments: &input.supported_fragments,
            });
            let diagnostics = result
                .violations
                .into_iter()
                .map(|violation| DraftQualityDiagnostic {
                    code: violation.reason_code().to_owned(),
                    task_type: violation.task_type().to_owned(),
                    next_action: violation.required_next_action().to_owned(),
                })
                .collect::<Vec<_>>();
            DraftQualityOutput {
                policy_passed: diagnostics.is_empty(),
                supported_claims: result.supported_claims,
                unsupported_claims: result.unsupported_claims,
                diagnostics,
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn one_step_draft_quality_retains_full_owner_diagnostics() {
            let contract = ValidateDraftQuality::contract();
            assert!(contract.effects.is_empty());
            assert_eq!(contract.idempotency, IdempotencyPolicy::Pure);
            let result = ValidateDraftQuality::evaluate(&DraftQualityInput {
                body_markdown: String::new(),
                locale: String::new(),
                truth_snapshot_ref: String::new(),
                schema_markup_json: String::new(),
                sections: Vec::new(),
                expected_required_sections: Vec::new(),
                blocks: Vec::new(),
                claims: Vec::new(),
                traceability: Vec::new(),
                verified_support: Vec::new(),
                supported_fragments: Vec::new(),
            });
            assert!(!result.policy_passed);
            assert_eq!(result.diagnostics.len(), 6);
            assert!(result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "missing_schema_markup"));
        }
    }
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
