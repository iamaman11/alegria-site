//! Read-only Knowledge contract for SEO's verified-rule projection.
//! PostgreSQL Truth is authoritative; search and graph projections cannot
//! promote a candidate into this result.

use runtime_models::{RuleParams, RuleRoleType};

/// Search projections for editorial context, ordered by the Knowledge use case.
/// None of these collections is an authoritative PostgreSQL Truth read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceContextCollection {
    VerifiedRules,
    RawContextual,
    EditorialTopics,
    RawStandard,
}

impl SourceContextCollection {
    pub fn name(self) -> &'static str {
        match self {
            Self::VerifiedRules => "verified_rules_4",
            Self::RawContextual => "raw_chunks_ctx",
            Self::EditorialTopics => "editorial_topics_4",
            Self::RawStandard => "raw_chunks_4",
        }
    }

    pub fn usage_policy(self) -> &'static str {
        match self {
            Self::VerifiedRules => "verified_fact_support",
            Self::RawContextual => "contextual_source_neighborhood_not_fact_support",
            Self::EditorialTopics => "editorial_topic_support_not_fact_support",
            Self::RawStandard => "standard_source_neighborhood_not_fact_support",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceContextChunk {
    pub collection: SourceContextCollection,
    pub chunk_key: String,
    pub source_url: String,
    pub source_domain: String,
    pub heading_path: String,
    pub section_type: String,
    pub content_md: String,
    pub retrieval_score: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceContextQuery {
    pub run_id: String,
    pub text: String,
    pub limit: usize,
}

/// A global, already-verified Truth rule selected by Truth's scope gate.
/// Embedding this read-only projection does not confer Truth authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRuleProjectionCandidate {
    pub rule_instance_id: String,
    pub role_type: String,
    pub concept_key: String,
    pub source_key: String,
    pub evidence_quote: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRuleProjectionChange {
    pub run_id: String,
    pub context_key: String,
    pub delete_rule_ids: Vec<String>,
    pub upserts: Vec<VerifiedRuleProjectionCandidate>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedRuleEmbeddingBatch {
    pub model: String,
    pub vectors: Vec<Vec<f32>>,
}

/// Unavailable optional provider differs from a successful, empty search.
/// The latter requires HITL instead of retaining a symbolic candidate key.
#[derive(Debug, Clone, PartialEq)]
pub enum CanonicalVectorLookup {
    Unavailable,
    Candidates(Vec<seo_domain::canonical_mapping::CanonicalVectorCandidate>),
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
