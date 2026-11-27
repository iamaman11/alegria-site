use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CanonicalMappingInput {
    pub section_id: String,
    pub mentions: Vec<MentionForMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MentionForMapping {
    pub raw_text: String,
    pub entity_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CanonicalMappingOutput {
    pub mappings: Vec<MappingResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchingStage {
    TypedEntity,
    ExactAlias,
    NormalizedAlias,
    LexiconToken,
    VectorQdrant,
}

/// A non-authoritative search suggestion. Vector scores never authorize Truth
/// writes; ambiguous or malformed hits must remain subject to human review.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalVectorCandidate {
    pub canonical_key: Option<String>,
    pub score: f32,
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

fn token_fragments(s: &str) -> Vec<String> {
    s.split(|ch: char| !ch.is_alphanumeric())
        .filter(|fragment| !fragment.trim().is_empty())
        .map(normalized)
        .collect()
}

fn has_token(tokens: &[String], expected: &str) -> bool {
    tokens.iter().any(|token| token == expected)
}

fn has_token_prefix(tokens: &[String], prefix: &str) -> bool {
    tokens.iter().any(|token| token.starts_with(prefix))
}

fn lexicon_token_lookup(s: &str) -> Option<&'static str> {
    let tokens = token_fragments(s);
    if has_token(&tokens, "passport")
        || has_token(&tokens, "паспорт")
        || has_token(&tokens, "загранпаспорт")
    {
        return Some("passport");
    }
    if has_token(&tokens, "insurance") || has_token_prefix(&tokens, "страхов") {
        return Some("medical_insurance");
    }
    None
}

pub fn execute(input: &CanonicalMappingInput) -> CanonicalMappingOutput {
    map_mentions(&input.mentions)
}

/// Pure canonical lookup over typed mentions. The source section identity is
/// carried by the admitted artifact envelope when this function is composed
/// after entity extraction, rather than being invented by the adapter.
pub fn map_mentions(mentions: &[MentionForMapping]) -> CanonicalMappingOutput {
    let mut out = Vec::with_capacity(mentions.len());
    for m in mentions {
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
                matching_stage: MatchingStage::TypedEntity,
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

        if let Some(k) = lexicon_token_lookup(&norm) {
            out.push(MappingResult {
                raw_text: m.raw_text.clone(),
                canonical_key: Some(k.to_string()),
                mapping_type: "lexicon_token".to_string(),
                match_method: "lexicon_token".to_string(),
                matching_stage: MatchingStage::LexiconToken,
                qdrant_score: None,
                confidence: 0.9,
                needs_hitl: false,
            });
            continue;
        }

        // Symbolic path is exhausted; runtime must resolve through real vector retrieval.
        let normalized_candidate = norm.replace(' ', "_");

        out.push(MappingResult {
            raw_text: m.raw_text.clone(),
            canonical_key: Some(format!("candidate:{normalized_candidate}")),
            mapping_type: "vector_required".to_string(),
            match_method: "vector_qdrant_required".to_string(),
            matching_stage: MatchingStage::VectorQdrant,
            qdrant_score: None,
            confidence: 0.0,
            needs_hitl: true,
        });
    }
    CanonicalMappingOutput { mappings: out }
}

/// Make the final canonical decision after the Knowledge search port has
/// returned its suggestions. This is a pure domain policy: Qdrant does not
/// decide which identities are safe to accept automatically.
pub fn resolve_vector_candidates(
    mapping: &MappingResult,
    candidates: &[CanonicalVectorCandidate],
) -> MappingResult {
    if mapping.matching_stage != MatchingStage::VectorQdrant {
        return mapping.clone();
    }
    let Some(top) = candidates.first() else {
        return unresolved_vector_mapping(mapping, "qdrant_no_candidates");
    };
    // Provider scores and candidate identities are untrusted. Invalid hits
    // cannot produce an automatic match even when their score is high.
    let Some(top_key) = top
        .canonical_key
        .as_deref()
        .filter(|key| !key.trim().is_empty())
    else {
        return unresolved_vector_mapping(mapping, "qdrant_invalid_candidate");
    };
    if !top.score.is_finite()
        || !(0.0..=1.0).contains(&top.score)
        || candidates
            .iter()
            .skip(1)
            .any(|candidate| !candidate.score.is_finite())
    {
        return unresolved_vector_mapping(mapping, "qdrant_invalid_score");
    }
    let margin = candidates
        .get(1)
        .map_or(top.score, |second| top.score - second.score);
    let (mapping_type, needs_hitl, canonical_key) = if top.score >= 0.88 && margin >= 0.03 {
        ("auto_map", false, Some(top_key.to_string()))
    } else if top.score >= 0.75 {
        ("review", true, Some(top_key.to_string()))
    } else {
        ("new_candidate", true, None)
    };
    MappingResult {
        raw_text: mapping.raw_text.clone(),
        canonical_key,
        mapping_type: mapping_type.to_string(),
        match_method: "qdrant_retrieval".to_string(),
        matching_stage: MatchingStage::VectorQdrant,
        qdrant_score: Some(top.score),
        confidence: top.score,
        needs_hitl,
    }
}

fn unresolved_vector_mapping(mapping: &MappingResult, method: &str) -> MappingResult {
    MappingResult {
        raw_text: mapping.raw_text.clone(),
        canonical_key: None,
        mapping_type: "new_candidate".to_string(),
        match_method: method.to_string(),
        matching_stage: MatchingStage::VectorQdrant,
        qdrant_score: None,
        confidence: 0.0,
        needs_hitl: true,
    }
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

    #[test]
    fn insurance_phrase_maps_via_lexicon_tokens_without_regex() {
        let mapped = map_one("требуется страховка путешественника");
        assert_eq!(mapped.canonical_key.as_deref(), Some("medical_insurance"));
        assert_eq!(mapped.match_method, "lexicon_token");
        assert_eq!(mapped.matching_stage, MatchingStage::LexiconToken);
        assert!(!mapped.needs_hitl);
    }

    #[test]
    fn vector_scores_have_one_fail_closed_domain_owner() {
        let unresolved = map_one("unknown entity without a symbolic alias");
        assert_eq!(unresolved.matching_stage, MatchingStage::VectorQdrant);
        for invalid in [
            vec![],
            vec![CanonicalVectorCandidate {
                canonical_key: None,
                score: 0.99,
            }],
            vec![CanonicalVectorCandidate {
                canonical_key: Some("valid".into()),
                score: f32::NAN,
            }],
            vec![CanonicalVectorCandidate {
                canonical_key: Some("valid".into()),
                score: 1.1,
            }],
        ] {
            let rejected = resolve_vector_candidates(&unresolved, &invalid);
            assert!(rejected.needs_hitl);
            assert!(rejected.canonical_key.is_none());
            assert_eq!(rejected.confidence, 0.0);
        }
        let hit = |key: &str, score| CanonicalVectorCandidate {
            canonical_key: Some(key.into()),
            score,
        };
        let accepted =
            resolve_vector_candidates(&unresolved, &[hit("passport", 0.94), hit("other", 0.80)]);
        assert!(!accepted.needs_hitl);
        assert_eq!(accepted.mapping_type, "auto_map");
        let tied =
            resolve_vector_candidates(&unresolved, &[hit("passport", 0.94), hit("other", 0.93)]);
        assert!(tied.needs_hitl);
        assert_eq!(tied.mapping_type, "review");

        // The domain itself must not let a high vector score override an
        // already verified symbolic alias even if a caller skips its gate.
        let fixed = map_one("паспорт");
        let cannot_override = resolve_vector_candidates(&fixed, &[hit("wrong", 1.0)]);
        assert_eq!(cannot_override.matching_stage, MatchingStage::ExactAlias);
        assert_eq!(cannot_override.canonical_key, fixed.canonical_key);
    }
}
