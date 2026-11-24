//! Whole-page semantic decisions shared by the live Temporal pass and the
//! in-process expert extraction adapter.
//!
//! External page rows and advisory search hits are mapped at their adapters.
//! Nothing here reads the database, invokes a vector SDK, or owns workflow state.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

mod advisory;
mod context_profile;
mod scoring;

pub use advisory::fuse_with_advisory_retrieval;
pub use scoring::{build_page_sketch, deterministic_whole_page_snapshot, expert_section_semantics};

/// Exactly the fields consumed by page decisions, independent of SQL rows.
#[derive(Debug, Clone)]
pub struct WholePageSection {
    pub id: i64,
    pub page_id: i64,
    pub source_url: String,
    pub heading_path: String,
    pub section_type: String,
    pub content_md: String,
}

/// Advisory evidence selected by the I/O adapter; the domain only merges it.
#[derive(Debug, Clone)]
pub struct WholePageAdvisoryHit {
    pub prototype_family: String,
    pub score: f32,
    pub page_mode: String,
    pub dominant_layers: Vec<String>,
    pub country_hints: Vec<String>,
    pub visa_type_hints: Vec<String>,
    pub authority_hints: Vec<String>,
    pub mixed_section_pressure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WholePageContextProfile {
    pub country_hints: Vec<String>,
    pub visa_type_hints: Vec<String>,
    pub authority_hints: Vec<String>,
}

#[derive(Debug, Clone)]
struct WholePageAdvisorySignal {
    page_mode_candidates: Vec<String>,
    layer_candidates: Vec<String>,
    context_profile_candidates: WholePageContextProfile,
    mixed_section_pressure: bool,
    retrieval_confidence: f32,
}

/// Kept byte-for-byte compatible with the existing Temporal JSON payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WholePageSemanticPageState {
    pub page_id: i64,
    pub section_count: usize,
    pub page_mode_hint: String,
    pub page_mode_confidence: f32,
    pub dominant_layers: Vec<String>,
    pub layer_scores: BTreeMap<String, f32>,
    pub page_summary: String,
    pub page_context_profile: WholePageContextProfile,
    pub mixed_section_ids: Vec<i64>,
    pub global_entities: Vec<String>,
    pub advisory_model_used: bool,
    pub advisory_consensus: String,
    pub advisory_prototype_families: Vec<String>,
    pub uncertainty_flags: Vec<String>,
    pub reason_codes: Vec<String>,
}

/// Opaque deterministic result. Runtime code may pass it to a later pure
/// fusion step, but cannot modify classification scores or decision reasons.
#[derive(Debug, Clone)]
pub struct WholePageDeterministicSnapshot {
    page_mode_hint: String,
    page_mode_confidence: f32,
    dominant_layers: Vec<String>,
    layer_scores: BTreeMap<String, f32>,
    page_summary: String,
    page_context_profile: WholePageContextProfile,
    mixed_section_ids: Vec<i64>,
    global_entities: Vec<String>,
    uncertainty_flags: Vec<String>,
    reason_codes: Vec<String>,
}

impl WholePageDeterministicSnapshot {
    /// Read-only evidence for runtime contract tests; decisions remain domain-owned.
    pub fn page_mode_hint(&self) -> &str {
        &self.page_mode_hint
    }

    pub fn page_mode_confidence(&self) -> f32 {
        self.page_mode_confidence
    }

    pub fn dominant_layers(&self) -> &[String] {
        &self.dominant_layers
    }

    pub fn mixed_section_ids(&self) -> &[i64] {
        &self.mixed_section_ids
    }

    pub fn uncertainty_flags(&self) -> &[String] {
        &self.uncertainty_flags
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(id: i64, heading: &str, kind: &str, text: &str) -> WholePageSection {
        WholePageSection {
            id,
            page_id: 42,
            source_url: "https://example.test/fixture-page".to_owned(),
            heading_path: heading.to_owned(),
            section_type: kind.to_owned(),
            content_md: text.to_owned(),
        }
    }

    #[test]
    fn expert_and_temporal_semantics_have_explicit_different_marker_profiles() {
        let section = section(
            7,
            "Routing",
            "content",
            "Ранжирование и консультация. ranking. service package.",
        );
        let expert = expert_section_semantics(&section);
        let temporal =
            deterministic_whole_page_snapshot(std::slice::from_ref(&section), &section.content_md);
        assert_eq!(expert.page_mode_confidence, 0.70);
        assert_eq!(
            expert.reason_codes,
            vec!["page_mode:content_page".to_string()]
        );
        assert_eq!(expert.layer_scores["seo"], 0.25);
        assert_eq!(expert.layer_scores["commercial"], 0.25);
        assert_eq!(temporal.layer_scores["seo"], 0.20);
        assert_eq!(temporal.layer_scores["commercial"], 1.0 / 6.0);
        assert_eq!(expert.advisory_consensus, "not_used");
        assert!(!expert.advisory_model_used);
    }

    #[test]
    fn empty_advisory_evidence_does_not_promote_unsupported_context() {
        let section = section(
            55,
            "Student visa",
            "content",
            "Student visa steps and appointment notices.",
        );
        let snapshot =
            deterministic_whole_page_snapshot(std::slice::from_ref(&section), &section.content_md);
        let page = fuse_with_advisory_retrieval(std::slice::from_ref(&section), snapshot, &[]);
        assert!(!page.advisory_model_used);
        assert!(page.page_context_profile.country_hints.is_empty());
        assert_eq!(page.advisory_consensus, "not_used");
        assert_eq!(page.page_context_profile.visa_type_hints, ["student"]);
    }
}
