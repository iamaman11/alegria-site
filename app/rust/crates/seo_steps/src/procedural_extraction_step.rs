use runtime_models::RuleRoleType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProceduralExtractionInput {
    pub section_id: String,
    pub raw_text: String,
    pub mentions: Vec<crate::entity_span_detection_step::EntityMention>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProceduralRule {
    pub rule_key: String,
    pub role_type: RuleRoleType,
    pub numeric_tokens: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProceduralExtractionOutput {
    pub rules: Vec<ProceduralRule>,
}

fn normalized(s: &str) -> String {
    s.trim().to_lowercase().replace('ё', "е")
}

pub fn execute(input: &ProceduralExtractionInput) -> ProceduralExtractionOutput {
    let mut rules = Vec::new();
    let fee_tokens: Vec<String> = input
        .mentions
        .iter()
        .filter(|mention| mention.entity_type == "fee" && mention.has_numeric)
        .map(|mention| mention.raw_text.clone())
        .collect();
    if !fee_tokens.is_empty() {
        rules.push(ProceduralRule {
            rule_key: "consular_fee".to_string(),
            role_type: RuleRoleType::MustPay,
            numeric_tokens: fee_tokens,
            confidence: 0.9,
        });
    }

    let day_tokens: Vec<String> = input
        .mentions
        .iter()
        .filter(|mention| mention.entity_type == "timeline" && mention.has_numeric)
        .map(|mention| mention.raw_text.clone())
        .collect();
    if !day_tokens.is_empty() {
        rules.push(ProceduralRule {
            rule_key: "processing_time".to_string(),
            role_type: RuleRoleType::Timeline,
            numeric_tokens: day_tokens,
            confidence: 0.86,
        });
    }

    if input.mentions.iter().any(|mention| {
        mention.entity_type == "concept"
            && matches!(
                normalized(&mention.raw_text).as_str(),
                "паспорт" | "passport"
            )
    }) {
        rules.push(ProceduralRule {
            rule_key: "passport_required".to_string(),
            role_type: RuleRoleType::MustProvide,
            numeric_tokens: Vec::new(),
            confidence: 0.84,
        });
    }
    if input.mentions.iter().any(|mention| {
        let raw = normalized(&mention.raw_text);
        mention.entity_type == "concept" && (raw.starts_with("страхов") || raw == "insurance")
    }) {
        rules.push(ProceduralRule {
            rule_key: "insurance_required".to_string(),
            role_type: RuleRoleType::MustProvide,
            numeric_tokens: Vec::new(),
            confidence: 0.82,
        });
    }

    ProceduralExtractionOutput { rules }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_span_detection_step::EntityMention;

    #[test]
    fn builds_fee_timeline_and_document_rules_from_mentions() {
        let output = execute(&ProceduralExtractionInput {
            section_id: "section-1".to_string(),
            raw_text: "ignored as authority".to_string(),
            mentions: vec![
                EntityMention {
                    raw_text: "80 EUR".to_string(),
                    entity_type: "fee".to_string(),
                    has_numeric: true,
                    is_central: true,
                    confidence: 0.95,
                },
                EntityMention {
                    raw_text: "15 days".to_string(),
                    entity_type: "timeline".to_string(),
                    has_numeric: true,
                    is_central: true,
                    confidence: 0.93,
                },
                EntityMention {
                    raw_text: "паспорт".to_string(),
                    entity_type: "concept".to_string(),
                    has_numeric: false,
                    is_central: true,
                    confidence: 0.82,
                },
            ],
        });
        let keys = output
            .rules
            .iter()
            .map(|rule| rule.rule_key.as_str())
            .collect::<Vec<_>>();
        assert!(keys.contains(&"consular_fee"));
        assert!(keys.contains(&"processing_time"));
        assert!(keys.contains(&"passport_required"));
    }

    #[test]
    fn raw_text_alone_does_not_create_rules_without_mentions() {
        let output = execute(&ProceduralExtractionInput {
            section_id: "section-1".to_string(),
            raw_text: "Passport required. Fee 80 EUR. 15 days.".to_string(),
            mentions: Vec::new(),
        });
        assert!(output.rules.is_empty());
    }
}
