//! Deterministic facts/rules extraction from section text (R2).
//!
//! Typed core only. JSON wrappers live in `facts_extractor_json.rs`.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

static EUR_RE: OnceLock<Regex> = OnceLock::new();
static DAY_RE: OnceLock<Regex> = OnceLock::new();

fn eur_re() -> &'static Regex {
    EUR_RE.get_or_init(|| Regex::new(r"(?i)(?P<amount>\d+(?:[.,]\d+)?)\s*(?:eur|€|евро)").unwrap())
}

fn day_re() -> &'static Regex {
    DAY_RE.get_or_init(|| Regex::new(r"(?i)(?P<days>\d+)\s*(?:дн|дней|day|days)").unwrap())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtractionContext {
    #[serde(default)]
    pub source_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuleParams {
    None,
    Fee {
        amount: f64,
        currency: String,
        severity: String,
        conditions_key: String,
    },
    Document {
        severity: String,
        subtype: Option<String>,
        notarization_required: bool,
        translation_required: bool,
        accepts_alternatives: bool,
        conditions_key: String,
    },
    Timeline {
        days: i64,
        subtype: Option<String>,
        severity: String,
        conditions_key: String,
    },
}

impl Default for RuleParams {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtractedRule {
    pub rule_type_key: String,
    pub concept_key: String,
    pub role_type: String,
    #[serde(default)]
    pub params: RuleParams,
    #[serde(default)]
    pub source_key: Option<String>,
    #[serde(default)]
    pub extraction_confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FactValue {
    Null,
    Integer(i64),
    Decimal(f64),
    Text(String),
    Boolean(bool),
}

impl Default for FactValue {
    fn default() -> Self {
        Self::Null
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtractedFact {
    pub fact_key: String,
    #[serde(default)]
    pub fact_value: FactValue,
    #[serde(default)]
    pub extraction_confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtractedFactsEnvelope {
    #[serde(default)]
    pub rule_instances: Vec<ExtractedRule>,
    #[serde(default)]
    pub facts: Vec<ExtractedFact>,
}

pub fn extract_facts_typed(markdown: &str, context: &ExtractionContext) -> ExtractedFactsEnvelope {
    let text = markdown.trim();
    let mut rules: Vec<ExtractedRule> = Vec::new();
    let mut facts: Vec<ExtractedFact> = Vec::new();

    if text.is_empty() {
        return ExtractedFactsEnvelope {
            rule_instances: rules,
            facts,
        };
    }

    let lowered = text.to_lowercase();

    if let Some(cap) = eur_re().captures(text) {
        let amount_raw = cap["amount"].replace(',', ".");
        if let Ok(amount) = amount_raw.parse::<f64>() {
            rules.push(ExtractedRule {
                rule_type_key: "consular_fee".to_string(),
                concept_key: "consular_fee".to_string(),
                role_type: "must_pay".to_string(),
                params: RuleParams::Fee {
                    amount,
                    currency: "EUR".to_string(),
                    severity: "mandatory".to_string(),
                    conditions_key: String::new(),
                },
                source_key: context.source_key.clone(),
                extraction_confidence: 0.85,
            });
        }
    }

    if lowered.contains("паспорт") || lowered.contains("passport") {
        rules.push(ExtractedRule {
            rule_type_key: "passport_required".to_string(),
            concept_key: "passport".to_string(),
            role_type: "document_required".to_string(),
            params: RuleParams::Document {
                severity: "mandatory".to_string(),
                subtype: Some("identity_document".to_string()),
                notarization_required: false,
                translation_required: false,
                accepts_alternatives: false,
                conditions_key: String::new(),
            },
            source_key: context.source_key.clone(),
            extraction_confidence: 0.8,
        });
    }

    if lowered.contains("страхов") || lowered.contains("insurance") {
        rules.push(ExtractedRule {
            rule_type_key: "insurance_required".to_string(),
            concept_key: "medical_insurance".to_string(),
            role_type: "document_required".to_string(),
            params: RuleParams::Document {
                severity: "mandatory".to_string(),
                subtype: Some("insurance_policy".to_string()),
                notarization_required: false,
                translation_required: false,
                accepts_alternatives: false,
                conditions_key: String::new(),
            },
            source_key: context.source_key.clone(),
            extraction_confidence: 0.75,
        });
    }

    if let Some(cap) = day_re().captures(text) {
        if let Ok(days) = cap["days"].parse::<i64>() {
            rules.push(ExtractedRule {
                rule_type_key: "processing_timeline".to_string(),
                concept_key: "processing_timeline".to_string(),
                role_type: "timeline_item".to_string(),
                params: RuleParams::Timeline {
                    days,
                    subtype: Some("processing_window".to_string()),
                    severity: "informational".to_string(),
                    conditions_key: String::new(),
                },
                source_key: context.source_key.clone(),
                extraction_confidence: 0.7,
            });
            facts.push(ExtractedFact {
                fact_key: "processing_days".to_string(),
                fact_value: FactValue::Integer(days),
                extraction_confidence: 0.8,
            });
        }
    }

    ExtractedFactsEnvelope {
        rule_instances: rules,
        facts,
    }
}
