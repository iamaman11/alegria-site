use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalMappingInput {
    pub section_id: String,
    pub mentions: Vec<MentionForMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MentionForMapping {
    pub raw_text: String,
    pub entity_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingResult {
    pub raw_text: String,
    pub canonical_key: Option<String>,
    pub mapping_type: String,
    pub match_method: String,
    pub matching_stage: MatchingStage,
    pub qdrant_score: Option<f32>,
    pub confidence: f32,
    pub needs_hitl: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalMappingOutput {
    pub mappings: Vec<MappingResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchingStage {
    ExactAlias,
    NormalizedAlias,
    RegexSymbolic,
    VectorQdrant,
}

fn normalized(s: &str) -> String {
    s.trim().to_lowercase().replace('ё', "е")
}

fn alias_lookup(s: &str) -> Option<&'static str> {
    match s {
        "паспорт" | "загранпаспорт" => Some("passport"),
        "страховка" | "медицинская страховка" => {
            Some("medical_insurance")
        }
        "анкета" => Some("application_form"),
        "справка о несудимости" => Some("criminal_record_certificate"),
        "vfs" | "визовый центр" => Some("vfs_global"),
        _ => None,
    }
}

static PASSPORT_RE: OnceLock<Regex> = OnceLock::new();
static INSURANCE_RE: OnceLock<Regex> = OnceLock::new();
fn passport_re() -> &'static Regex {
    PASSPORT_RE.get_or_init(|| Regex::new(r"(?i)\b(passport|паспорт)\b").unwrap())
}
fn insurance_re() -> &'static Regex {
    INSURANCE_RE.get_or_init(|| Regex::new(r"(?i)\b(insurance|страхов)\w*\b").unwrap())
}

fn regex_symbolic_lookup(s: &str) -> Option<&'static str> {
    if passport_re().is_match(s) {
        return Some("passport");
    }
    if insurance_re().is_match(s) {
        return Some("medical_insurance");
    }
    None
}

fn pseudo_qdrant_score(s: &str) -> f32 {
    let len = s.chars().count() as f32;
    (0.45 + (len.min(24.0) / 100.0)).min(0.90)
}

pub fn execute(input: &CanonicalMappingInput) -> CanonicalMappingOutput {
    let mut out = Vec::with_capacity(input.mentions.len());
    for m in &input.mentions {
        let exact = alias_lookup(m.raw_text.as_str());
        if let Some(k) = exact {
            out.push(MappingResult {
                raw_text: m.raw_text.clone(),
                canonical_key: Some(k.to_string()),
                mapping_type: "alias".to_string(),
                match_method: "exact_alias".to_string(),
                matching_stage: MatchingStage::ExactAlias,
                qdrant_score: None,
                confidence: 1.0,
                needs_hitl: false,
            });
            continue;
        }

        let norm = normalized(&m.raw_text);
        let norm_match = alias_lookup(norm.as_str());
        if let Some(k) = norm_match {
            out.push(MappingResult {
                raw_text: m.raw_text.clone(),
                canonical_key: Some(k.to_string()),
                mapping_type: "alias".to_string(),
                match_method: "normalized_alias".to_string(),
                matching_stage: MatchingStage::NormalizedAlias,
                qdrant_score: None,
                confidence: 0.96,
                needs_hitl: false,
            });
            continue;
        }

        if let Some(k) = regex_symbolic_lookup(&norm) {
            out.push(MappingResult {
                raw_text: m.raw_text.clone(),
                canonical_key: Some(k.to_string()),
                mapping_type: "symbolic".to_string(),
                match_method: "regex_symbolic".to_string(),
                matching_stage: MatchingStage::RegexSymbolic,
                qdrant_score: None,
                confidence: 0.9,
                needs_hitl: false,
            });
            continue;
        }

        // Symbolic path is exhausted; vector stage is allowed only here.
        let score = pseudo_qdrant_score(&norm);
        let (mapping_type, needs_hitl, key, confidence) = if score >= 0.88 {
            (
                "auto_map",
                false,
                Some(format!("candidate:{}", norm.replace(' ', "_"))),
                score,
            )
        } else if score >= 0.75 {
            (
                "review",
                true,
                Some(format!("candidate:{}", norm.replace(' ', "_"))),
                score,
            )
        } else {
            ("new_candidate", true, None, score)
        };

        out.push(MappingResult {
            raw_text: m.raw_text.clone(),
            canonical_key: key,
            mapping_type: mapping_type.to_string(),
            match_method: "qdrant".to_string(),
            matching_stage: MatchingStage::VectorQdrant,
            qdrant_score: Some(score),
            confidence,
            needs_hitl,
        });
    }
    CanonicalMappingOutput { mappings: out }
}
