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
