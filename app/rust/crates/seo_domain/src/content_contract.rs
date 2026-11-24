//! One domain owner for content-block and required-link publish obligations.
//!
//! Temporal payloads are adapted at the `seo_steps` boundary. The standalone
//! content-contract phase and the final draft QA therefore share the exact
//! same policy and reason/action definitions.

use std::collections::BTreeSet;

use crate::editorial::{block_type_for_role, factual_role};

#[derive(Clone, Copy)]
pub struct ContentBlock<'a> {
    pub role: &'a str,
    pub block_type: &'a str,
    pub markdown: &'a str,
    pub required: bool,
    pub support_refs: &'a [String],
}

#[derive(Clone, Copy)]
pub struct Evidence<'a> {
    pub validation_verdict: &'a str,
    pub traceability_label: &'a str,
    pub support_refs: &'a [String],
}

#[derive(Clone, Copy)]
pub struct LinkObligation<'a> {
    pub target_page_key: &'a str,
    pub role: &'a str,
    pub required: bool,
    /// Transport identity comparison, resolved at the adapter boundary.
    pub bound_exactly: bool,
}

#[derive(Clone, Copy)]
pub struct PublishLinkCoverage<'a> {
    pub typed_editorial: bool,
    pub related_pages_body: &'a str,
}

pub struct ContentContract<'a> {
    pub blocks: &'a [ContentBlock<'a>],
    pub claims: &'a [Evidence<'a>],
    pub traceability: &'a [Evidence<'a>],
    pub links: &'a [LinkObligation<'a>],
    /// Only the final QA verifies link binding in assembled sections. The
    /// earlier contract phase verifies the existence of the content block.
    pub publish_links: Option<PublishLinkCoverage<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Violation {
    DuplicateContentBlockRole,
    ContentBlockTypeMismatch,
    EmptyRequiredContentBlock,
    UnsupportedFactualContentBlock,
    UnmatchedFactualSupportRef,
    PlaceholderFactualContent,
    MissingRequiredContentBlock,
    MissingRequiredInternalLinks,
}

impl Violation {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::DuplicateContentBlockRole => "duplicate_content_block_role",
            Self::ContentBlockTypeMismatch => "content_block_type_mismatch",
            Self::EmptyRequiredContentBlock => "empty_required_content_block",
            Self::UnsupportedFactualContentBlock => "unsupported_factual_content_block",
            Self::UnmatchedFactualSupportRef => "unmatched_factual_support_ref",
            Self::PlaceholderFactualContent => "placeholder_factual_content",
            Self::MissingRequiredContentBlock => "missing_required_content_block",
            Self::MissingRequiredInternalLinks => "missing_required_internal_links",
        }
    }

    pub const fn required_next_action(self) -> &'static str {
        match self {
            Self::DuplicateContentBlockRole => "Keep one canonical content block per section role.",
            Self::ContentBlockTypeMismatch => {
                "Normalize content blocks so each section role maps to the canonical block type."
            }
            Self::EmptyRequiredContentBlock => {
                "Populate every required content block before publish gate."
            }
            Self::UnsupportedFactualContentBlock => {
                "Attach verified support refs to every required factual content block."
            }
            Self::UnmatchedFactualSupportRef => {
                "Resolve every factual block reference against supported claims and traceability."
            }
            Self::PlaceholderFactualContent => {
                "Replace placeholder prose with actual verified evidence before publishing."
            }
            Self::MissingRequiredContentBlock => {
                "Build the complete canonical content block plan before publish gate."
            }
            Self::MissingRequiredInternalLinks => {
                "Render required internal link obligations into the related pages block."
            }
        }
    }
}

fn supported_refs<'a>(entries: &[Evidence<'a>]) -> BTreeSet<&'a str> {
    entries
        .iter()
        .filter(|entry| {
            entry.validation_verdict == "supported"
                && matches!(
                    entry.traceability_label,
                    "verified_fact" | "verified_summary"
                )
        })
        .flat_map(|entry| entry.support_refs.iter().map(String::as_str))
        .collect()
}

pub fn validate(input: &ContentContract<'_>) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut seen_roles = BTreeSet::new();
    // A reference is evidence only when the same *supported* identity exists
    // in both the claim ledger and traceability manifest.
    let ledger_refs = supported_refs(input.claims);
    let trace_refs = supported_refs(input.traceability);

    for block in input.blocks {
        if !seen_roles.insert(block.role) {
            violations.push(Violation::DuplicateContentBlockRole);
        }
        if block.block_type != block_type_for_role(block.role) {
            violations.push(Violation::ContentBlockTypeMismatch);
        }
        if block.required && block.markdown.trim().is_empty() {
            violations.push(Violation::EmptyRequiredContentBlock);
        }
        if block.required && factual_role(block.role) {
            if block.support_refs.is_empty() {
                violations.push(Violation::UnsupportedFactualContentBlock);
            } else if block.support_refs.iter().any(|support_ref| {
                support_ref.trim().is_empty()
                    || !ledger_refs.contains(support_ref.as_str())
                    || !trace_refs.contains(support_ref.as_str())
            }) {
                violations.push(Violation::UnmatchedFactualSupportRef);
            }
        }
        if block
            .markdown
            .contains("Verified source-backed structured content")
        {
            violations.push(Violation::PlaceholderFactualContent);
        }
    }
    if !input.blocks.iter().any(|block| block.required) {
        violations.push(Violation::MissingRequiredContentBlock);
    }

    let related_block = input
        .blocks
        .iter()
        .find(|block| block.role == "related_pages");
    let missing_links_block = !input.links.is_empty()
        && related_block.is_none_or(|block| block.markdown.trim().is_empty());
    let missing_links_at_publish = input.publish_links.is_some_and(|coverage| {
        let required = input
            .links
            .iter()
            .filter(|link| link.required)
            .collect::<Vec<_>>();
        !required.is_empty()
            && (if coverage.typed_editorial {
                coverage.related_pages_body.trim().is_empty()
                    || required.iter().any(|link| !link.bound_exactly)
            } else {
                required.iter().any(|link| {
                    !coverage.related_pages_body.contains(link.target_page_key)
                        && !coverage.related_pages_body.contains(link.role)
                })
            })
    });
    if missing_links_block || missing_links_at_publish {
        violations.push(Violation::MissingRequiredInternalLinks);
    }

    violations.sort_by_key(|violation| violation.reason_code());
    violations.dedup();
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_link_failure_has_one_domain_reason_in_publish_qa() {
        let links = [LinkObligation {
            target_page_key: "dest",
            role: "hub_child",
            required: true,
            bound_exactly: false,
        }];
        let input = ContentContract {
            blocks: &[],
            claims: &[],
            traceability: &[],
            links: &links,
            publish_links: Some(PublishLinkCoverage {
                typed_editorial: true,
                related_pages_body: "",
            }),
        };
        let result = validate(&input);
        assert_eq!(
            result
                .iter()
                .filter(|reason| **reason == Violation::MissingRequiredInternalLinks)
                .count(),
            1
        );
    }

    #[test]
    fn exact_supported_ref_in_both_manifests_is_required() {
        let refs = ["rule:one".to_string()];
        let blocks = [ContentBlock {
            role: "fees",
            block_type: "fees_table",
            markdown: "Fee from verified rule",
            required: true,
            support_refs: &refs,
        }];
        let verified = [Evidence {
            validation_verdict: "supported",
            traceability_label: "verified_fact",
            support_refs: &refs,
        }];
        let input = ContentContract {
            blocks: &blocks,
            claims: &verified,
            traceability: &[],
            links: &[],
            publish_links: None,
        };
        assert!(validate(&input).contains(&Violation::UnmatchedFactualSupportRef));
        let complete = ContentContract {
            traceability: &verified,
            ..input
        };
        assert!(!validate(&complete).contains(&Violation::UnmatchedFactualSupportRef));
    }
}
