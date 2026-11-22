//! Read-only Knowledge contract for SEO's verified-rule projection.
//! PostgreSQL Truth is authoritative; search and graph projections cannot
//! promote a candidate into this result.

use runtime_models::{RuleParams, RuleRoleType};

/// Non-authoritative Qdrant/Voyage suggestion. A vector hit is not a verified
/// rule, source assertion, or permission to write PostgreSQL Truth.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalVectorCandidate {
    pub canonical_key: Option<String>,
    pub score: f32,
}

/// Unavailable optional provider differs from a successful, empty search.
/// The latter requires HITL instead of retaining a symbolic candidate key.
#[derive(Debug, Clone, PartialEq)]
pub enum CanonicalVectorLookup {
    Unavailable,
    Candidates(Vec<CanonicalVectorCandidate>),
}

/// The only retrieval neighborhoods used by Truth diagnostics. These are
/// search projections, never verified source assertions or write authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticNeighborCollection {
    RawContextual,
    RawStandard,
    VerifiedRules,
    CanonicalConcepts,
}

impl SemanticNeighborCollection {
    pub fn name(self) -> &'static str {
        match self {
            Self::RawContextual => "raw_chunks_ctx",
            Self::RawStandard => "raw_chunks_4",
            Self::VerifiedRules => "verified_rules_4",
            Self::CanonicalConcepts => "kb_canonical_4",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticNeighborCandidate {
    pub entity_key: String,
    pub score: f32,
    /// Only for Voyage rerank; not permission to use the text as verified Truth.
    pub retrieval_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticNeighborQuery {
    pub text: String,
    pub primary: SemanticNeighborCollection,
    pub fallback: Option<SemanticNeighborCollection>,
    pub limit: usize,
    pub retrieval_required: bool,
    pub contextual_required: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticNeighborLookup {
    pub collection_used: Option<String>,
    pub neighbors: Vec<SemanticNeighborCandidate>,
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRulesQuery {
    pub run_id: String,
    pub context_key: String,
}

#[derive(Debug, Clone)]
pub struct VerifiedRule {
    pub rule_instance_id: String,
    pub rule_type_key: String,
    pub role: RuleRoleType,
    pub params: RuleParams,
    pub effective_from: String,
    pub effective_to: String,
    pub source_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VerifiedRulesView {
    pub context_key: String,
    pub rules: Vec<VerifiedRule>,
}
