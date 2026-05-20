use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletenessJudgeInput {
    pub section_id: String,
    pub raw_text: String,
    pub extracted_numeric_tokens: Vec<String>,
    pub extracted_rule_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingElement {
    pub loss_type: String,
    pub raw_fragment: String,
    pub reason: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletenessJudgeOutput {
    pub completeness_score: f32,
    pub missing_elements: Vec<MissingElement>,
    pub needs_hitl: bool,
    pub hitl_reason: Option<String>,
}

static NUMBER_RE: OnceLock<Regex> = OnceLock::new();
fn number_re() -> &'static Regex {
    // Rust regex does not support look-around; word boundaries are enough for
    // deterministic numeric-token recovery in completeness judging.
    NUMBER_RE.get_or_init(|| Regex::new(r"(?m)\b\d+(?:[.,]\d+)?\b").unwrap())
}

pub fn execute(input: &CompletenessJudgeInput) -> CompletenessJudgeOutput {
    let mut missing = Vec::new();
    let text = input.raw_text.to_lowercase();

    let source_numbers: BTreeSet<String> = number_re()
        .find_iter(input.raw_text.as_str())
        .map(|m| m.as_str().replace(',', "."))
        .collect();
    let extracted: BTreeSet<String> = input
        .extracted_numeric_tokens
        .iter()
        .map(|s| s.replace(',', "."))
        .collect();
    for n in source_numbers.difference(&extracted) {
        missing.push(MissingElement {
            loss_type: "number".to_string(),
            raw_fragment: n.clone(),
            reason: "numeric token not reflected in extracted output".to_string(),
            action: "add_numeric_fact".to_string(),
        });
    }

    if text.contains("кроме") || text.contains("за исключением") {
        if !input
            .extracted_rule_keys
            .iter()
            .any(|k| k.contains("exception"))
        {
            missing.push(MissingElement {
                loss_type: "exception".to_string(),
                raw_fragment: "кроме / за исключением".to_string(),
                reason: "exception clause is not represented".to_string(),
                action: "add_exception_rule".to_string(),
            });
        }
    }
    if text.contains("либо") || text.contains("или предоставить") || text.contains("вместо")
    {
        missing.push(MissingElement {
            loss_type: "alternative".to_string(),
            raw_fragment: "либо/вместо".to_string(),
            reason: "alternative path requires explicit modeling".to_string(),
            action: "add_alternative_rule".to_string(),
        });
    }

    let penalties = (missing.len() as f32) * 0.12;
    let score = (1.0 - penalties).clamp(0.0, 1.0);
    let needs_hitl = !missing.is_empty();
    let hitl_reason = if needs_hitl {
        Some("completeness_gaps_detected".to_string())
    } else {
        None
    };

    CompletenessJudgeOutput {
        completeness_score: score,
        missing_elements: missing,
        needs_hitl,
        hitl_reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_missing_numbers_without_panicking() {
        let output = execute(&CompletenessJudgeInput {
            section_id: "section-1".to_string(),
            raw_text: "Fee is 80 EUR and processing time is 15 days.".to_string(),
            extracted_numeric_tokens: vec!["80".to_string()],
            extracted_rule_keys: vec!["consular_fee".to_string()],
        });

        assert!(output.needs_hitl);
        assert!(output
            .missing_elements
            .iter()
            .any(|missing| missing.raw_fragment == "15"));
    }
}
