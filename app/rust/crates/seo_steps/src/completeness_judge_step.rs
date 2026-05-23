use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletenessJudgeInput {
    pub section_id: String,
    pub raw_text: String,
    pub source_numeric_tokens: Vec<String>,
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

fn normalize_numeric_tokens(tokens: &[String]) -> BTreeSet<String> {
    tokens.iter().map(|s| s.replace(',', ".")).collect()
}

pub fn execute(input: &CompletenessJudgeInput) -> CompletenessJudgeOutput {
    let mut missing = Vec::new();
    let text = input.raw_text.to_lowercase();

    let source_numbers = normalize_numeric_tokens(&input.source_numeric_tokens);
    let extracted = normalize_numeric_tokens(&input.extracted_numeric_tokens);
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
            source_numeric_tokens: vec!["80".to_string(), "15".to_string()],
            extracted_numeric_tokens: vec!["80".to_string()],
            extracted_rule_keys: vec!["consular_fee".to_string()],
        });

        assert!(output.needs_hitl);
        assert!(output
            .missing_elements
            .iter()
            .any(|missing| missing.raw_fragment == "15"));
    }

    #[test]
    fn completeness_is_monotonic_when_more_numeric_tokens_are_extracted() {
        let raw_text = "Fee is 80 EUR and processing time is 15 days.";
        let sparse = execute(&CompletenessJudgeInput {
            section_id: "section-1".to_string(),
            raw_text: raw_text.to_string(),
            source_numeric_tokens: vec!["80".to_string(), "15".to_string()],
            extracted_numeric_tokens: vec!["80".to_string()],
            extracted_rule_keys: vec!["consular_fee".to_string()],
        });
        let complete = execute(&CompletenessJudgeInput {
            section_id: "section-1".to_string(),
            raw_text: raw_text.to_string(),
            source_numeric_tokens: vec!["80".to_string(), "15".to_string()],
            extracted_numeric_tokens: vec!["80".to_string(), "15".to_string()],
            extracted_rule_keys: vec![
                "consular_fee".to_string(),
                "processing_time".to_string(),
            ],
        });
        assert!(complete.completeness_score >= sparse.completeness_score);
        assert!(complete.missing_elements.len() <= sparse.missing_elements.len());
    }

    #[test]
    fn irrelevant_footer_text_without_numeric_loss_does_not_force_hitl() {
        let output = execute(&CompletenessJudgeInput {
            section_id: "section-1".to_string(),
            raw_text: "Passport required. Footer: contact us for updates.".to_string(),
            source_numeric_tokens: Vec::new(),
            extracted_numeric_tokens: Vec::new(),
            extracted_rule_keys: vec!["passport_required".to_string()],
        });
        assert!(!output.needs_hitl);
    }
}
