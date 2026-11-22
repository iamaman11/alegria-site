//! Read-only Knowledge contract for SEO's verified-rule projection.
//! PostgreSQL Truth is authoritative; search and graph projections cannot
//! promote a candidate into this result.

use runtime_models::{RuleParams, RuleRoleType};

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
