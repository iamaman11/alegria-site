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
        let entity_type = normalized(&m.entity_type);
        let entity_type_map = match entity_type.as_str() {
            "fee" => Some(("consular_fee", "typed_entity", 0.97f32)),
            "timeline" => Some(("processing_time", "typed_entity", 0.96f32)),
            "organization" if normalized(&m.raw_text).contains("vfs") => {
                Some(("vfs_global", "typed_entity", 0.94f32))
            }
            _ => None,
        };
        if let Some((canonical_key, method, confidence)) = entity_type_map {
            out.push(MappingResult {
                raw_text: m.raw_text.clone(),
                canonical_key: Some(canonical_key.to_string()),
                mapping_type: "typed_entity".to_string(),
                match_method: method.to_string(),
                matching_stage: MatchingStage::RegexSymbolic,
                qdrant_score: None,
                confidence,
                needs_hitl: false,
            });
            continue;
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn map_one(raw_text: &str) -> MappingResult {
        execute(&CanonicalMappingInput {
            section_id: "section-1".to_string(),
            mentions: vec![MentionForMapping {
                raw_text: raw_text.to_string(),
                entity_type: "concept".to_string(),
            }],
        })
        .mappings
        .into_iter()
        .next()
        .unwrap()
    }

    #[test]
    fn alias_mapping_is_stable_across_whitespace_and_case() {
        let variants = ["паспорт", " Паспорт ", "ПАСПОРТ", "passport"];
        let baseline = map_one(variants[0]);
        for variant in variants.iter().skip(1) {
            let mapped = map_one(variant);
            assert_eq!(mapped.canonical_key, baseline.canonical_key);
            assert!(!mapped.needs_hitl);
        }
    }

    #[test]
    fn unknown_concept_does_not_silently_verify() {
        let mapped = map_one("mysterious sponsor credential");
        assert!(mapped.needs_hitl || mapped.canonical_key.is_none());
        assert_ne!(mapped.mapping_type, "alias");
    }
}
