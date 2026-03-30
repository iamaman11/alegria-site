use regex::Regex;
use runtime_models::RuleRoleType;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProceduralExtractionInput {
    pub section_id: String,
    pub raw_text: String,
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

static MONEY_RE: OnceLock<Regex> = OnceLock::new();
static DAY_RE: OnceLock<Regex> = OnceLock::new();
fn money_re() -> &'static Regex {
    MONEY_RE.get_or_init(|| Regex::new(r"(?i)\b\d+(?:[.,]\d+)?\s*(?:eur|€|евро)\b").unwrap())
}
fn day_re() -> &'static Regex {
    DAY_RE.get_or_init(|| Regex::new(r"(?i)\b\d+\s*(?:дн|дней|day|days)\b").unwrap())
}

pub fn execute(input: &ProceduralExtractionInput) -> ProceduralExtractionOutput {
    let mut rules = Vec::new();
    let t = input.raw_text.to_lowercase();

    let fee_tokens: Vec<String> = money_re()
        .find_iter(&input.raw_text)
        .map(|m| m.as_str().to_string())
        .collect();
    if !fee_tokens.is_empty() {
        rules.push(ProceduralRule {
            rule_key: "consular_fee".to_string(),
            role_type: RuleRoleType::MustPay,
            numeric_tokens: fee_tokens,
            confidence: 0.9,
        });
    }

    let day_tokens: Vec<String> = day_re()
        .find_iter(&input.raw_text)
        .map(|m| m.as_str().to_string())
        .collect();
    if !day_tokens.is_empty() {
        rules.push(ProceduralRule {
            rule_key: "processing_time".to_string(),
            role_type: RuleRoleType::Timeline,
            numeric_tokens: day_tokens,
            confidence: 0.86,
        });
    }

    if t.contains("паспорт") || t.contains("passport") {
        rules.push(ProceduralRule {
            rule_key: "passport_required".to_string(),
            role_type: RuleRoleType::MustProvide,
            numeric_tokens: Vec::new(),
            confidence: 0.84,
        });
    }
    if t.contains("страхов") || t.contains("insurance") {
        rules.push(ProceduralRule {
            rule_key: "insurance_required".to_string(),
            role_type: RuleRoleType::MustProvide,
            numeric_tokens: Vec::new(),
            confidence: 0.82,
        });
    }

    ProceduralExtractionOutput { rules }
}
