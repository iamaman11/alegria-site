//! One domain owner for content-block and required-link publish obligations.
//!
//! Temporal payloads are adapted at the `seo_steps` boundary. The standalone
//! content-contract phase and the final draft QA therefore share the exact
//! same policy and reason/action definitions.

use std::collections::BTreeSet;

use crate::editorial::{
    block_type_for_role, factual_role, verified_traceability_evidence, ContentBlock,
};

#[derive(Clone, Copy)]
pub struct Evidence<'a> {
    pub validation_verdict: &'a str,
    pub traceability_label: &'a str,
    pub support_refs: &'a [String],
}

#[derive(Clone, Copy)]
pub struct LinkObligation {
    pub required: bool,
    /// Transport identity comparison, resolved at the adapter boundary.
    pub bound_exactly: bool,
}

#[derive(Clone, Copy)]
pub struct PublishLinkCoverage<'a> {
    pub related_pages_body: &'a str,
}

pub struct ContentContract<'a> {
    pub blocks: &'a [ContentBlock<'a>],
    pub claims: &'a [Evidence<'a>],
    pub traceability: &'a [Evidence<'a>],
    pub links: &'a [LinkObligation],
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
            verified_traceability_evidence(entry.validation_verdict, entry.traceability_label)
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
    let missing_links_block = input.links.iter().any(|link| link.required)
        && related_block.is_none_or(|block| block.markdown.trim().is_empty());
    let missing_links_at_publish = input.publish_links.is_some_and(|coverage| {
        input.links.iter().any(|link| link.required)
            && (coverage.related_pages_body.trim().is_empty()
                || input
                    .links
                    .iter()
                    .any(|link| link.required && !link.bound_exactly))
    });
    if missing_links_block || missing_links_at_publish {
        violations.push(Violation::MissingRequiredInternalLinks);
    }

    violations.sort_by_key(|violation| violation.reason_code());
    violations.dedup();
    violations
}

/// Independent authoring check of a concrete draft content contract. These
/// owned DTOs deliberately exclude mutable CMS state and database effects.
pub mod operations {
    use std::collections::{BTreeMap, BTreeSet};
    use std::convert::Infallible;

    use operation_protocol::{
        ArtifactSchema, IdempotencyPolicy, Operation, OperationContract, OperationId,
        OperationOwnerId, OperationRef, PortId,
    };
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use super::{validate, ContentContract, Evidence, LinkObligation, PublishLinkCoverage};
    use crate::editorial::ContentBlock;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ContentBlockInput {
        pub role: String,
        pub block_type: String,
        pub heading: String,
        pub markdown: String,
        pub required: bool,
        pub support_refs: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct EvidenceInput {
        pub validation_verdict: String,
        pub traceability_label: String,
        pub support_refs: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct LinkObligationInput {
        pub required: bool,
        pub bound_exactly: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct LinkCoverageInput {
        pub related_pages_body: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ContentContractInput {
        pub blocks: Vec<ContentBlockInput>,
        pub claims: Vec<EvidenceInput>,
        pub traceability: Vec<EvidenceInput>,
        pub links: Vec<LinkObligationInput>,
        pub publish_links: Option<LinkCoverageInput>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ContentContractViolation {
        pub code: String,
        pub next_action: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ContentContractOutput {
        pub valid: bool,
        pub violations: Vec<ContentContractViolation>,
    }

    pub struct ValidateContentContract;

    impl Operation for ValidateContentContract {
        type Input = ContentContractInput;
        type Output = ContentContractOutput;
        type Failure = Infallible;

        fn contract() -> OperationContract {
            OperationContract {
                reference: OperationRef {
                    id: OperationId::from("editorial.validate_content_contract"),
                    revision: 1,
                },
                owner: OperationOwnerId::from("editorial_policy"),
                inputs: BTreeMap::from([(
                    PortId::from("input"),
                    ArtifactSchema {
                        id: "editorial.content_contract_input".into(),
                        revision: 1,
                    },
                )]),
                outputs: BTreeMap::from([(
                    PortId::from("output"),
                    ArtifactSchema {
                        id: "editorial.content_contract_output".into(),
                        revision: 1,
                    },
                )]),
                invalidates: BTreeSet::new(),
                effects: BTreeSet::new(),
                idempotency: IdempotencyPolicy::Pure,
            }
        }
    }

    impl ValidateContentContract {
        pub fn evaluate(input: &ContentContractInput) -> ContentContractOutput {
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
                .map(|entry| Evidence {
                    validation_verdict: &entry.validation_verdict,
                    traceability_label: &entry.traceability_label,
                    support_refs: &entry.support_refs,
                })
                .collect::<Vec<_>>();
            let traceability = input
                .traceability
                .iter()
                .map(|entry| Evidence {
                    validation_verdict: &entry.validation_verdict,
                    traceability_label: &entry.traceability_label,
                    support_refs: &entry.support_refs,
                })
                .collect::<Vec<_>>();
            let links = input
                .links
                .iter()
                .map(|link| LinkObligation {
                    required: link.required,
                    bound_exactly: link.bound_exactly,
                })
                .collect::<Vec<_>>();
            let publish_links = input
                .publish_links
                .as_ref()
                .map(|coverage| PublishLinkCoverage {
                    related_pages_body: &coverage.related_pages_body,
                });
            let violations = validate(&ContentContract {
                blocks: &blocks,
                claims: &claims,
                traceability: &traceability,
                links: &links,
                publish_links,
            })
            .into_iter()
            .map(|violation| ContentContractViolation {
                code: violation.reason_code().to_owned(),
                next_action: violation.required_next_action().to_owned(),
            })
            .collect::<Vec<_>>();
            ContentContractOutput {
                valid: violations.is_empty(),
                violations,
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn owner_content_contract_detects_unbound_required_links_without_effects() {
            let contract = ValidateContentContract::contract();
            assert!(contract.effects.is_empty());
            assert_eq!(contract.idempotency, IdempotencyPolicy::Pure);
            let result = ValidateContentContract::evaluate(&ContentContractInput {
                blocks: Vec::new(),
                claims: Vec::new(),
                traceability: Vec::new(),
                links: vec![LinkObligationInput {
                    required: true,
                    bound_exactly: false,
                }],
                publish_links: None,
            });
            assert!(!result.valid);
            assert!(result
                .violations
                .iter()
                .any(|violation| violation.code == "missing_required_internal_links"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_link_failure_has_one_domain_reason_in_publish_qa() {
        let links = [LinkObligation {
            required: true,
            bound_exactly: false,
        }];
        let input = ContentContract {
            blocks: &[],
            claims: &[],
            traceability: &[],
            links: &links,
            publish_links: Some(PublishLinkCoverage {
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
    fn optional_recommendations_never_create_a_required_publish_block() {
        let optional = [LinkObligation {
            required: false,
            bound_exactly: false,
        }];
        let contract = ContentContract {
            blocks: &[],
            claims: &[],
            traceability: &[],
            links: &optional,
            publish_links: Some(PublishLinkCoverage {
                related_pages_body: "",
            }),
        };
        assert!(!validate(&contract).contains(&Violation::MissingRequiredInternalLinks));
    }

    #[test]
    fn mere_prose_containing_a_target_does_not_satisfy_a_required_typed_link() {
        let required = [LinkObligation {
            required: true,
            bound_exactly: false,
        }];
        let body = "See target-page for details.";
        let blocks = [ContentBlock {
            role: "related_pages",
            block_type: "related_links",
            heading: "Related",
            markdown: body,
            required: true,
            support_refs: &[],
        }];
        let contract = ContentContract {
            blocks: &blocks,
            claims: &[],
            traceability: &[],
            links: &required,
            publish_links: Some(PublishLinkCoverage {
                related_pages_body: body,
            }),
        };
        assert!(validate(&contract).contains(&Violation::MissingRequiredInternalLinks));
    }

    #[test]
    fn exact_supported_ref_in_both_manifests_is_required() {
        let refs = ["rule:one".to_string()];
        let blocks = [ContentBlock {
            role: "fees",
            block_type: "fees_table",
            heading: "Fees",
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
