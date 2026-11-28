//! Pure commercial signal extraction, independent from SEO signal policy.
//!
//! Source loading, consent and section gates belong to a separate owner.
//! This deterministic algorithm reproduces the commercial half of EX-07.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommercialSignalInput {
    pub section_id: String,
    pub raw_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommercialSignal {
    pub signal_type: String,
    pub value: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommercialSignalOutput {
    pub signals: Vec<CommercialSignal>,
}

/// Lexical commercial detection only; no sale, pricing decision or effect.
pub fn execute(input: &CommercialSignalInput) -> CommercialSignalOutput {
    let lowered = input.raw_text.to_lowercase();
    let mut signals = Vec::new();
    if lowered.contains("консультац")
        || lowered.contains("под ключ")
        || lowered.contains("заказать")
        || lowered.contains("service")
    {
        signals.push(CommercialSignal {
            signal_type: "service_offer".to_owned(),
            value: "service_offer_detected".to_owned(),
            confidence: 0.84,
        });
    }
    if lowered.contains("стоимость услуги")
        || lowered.contains("our fee")
        || lowered.contains("price")
    {
        signals.push(CommercialSignal {
            signal_type: "commercial_price".to_owned(),
            value: "commercial_price_detected".to_owned(),
            confidence: 0.81,
        });
    }
    CommercialSignalOutput { signals }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_and_price_are_separate_signals() {
        let result = execute(&CommercialSignalInput {
            section_id: "s1".to_owned(),
            raw_text: "Услуги под ключ, стоимость услуги и консультация".to_owned(),
        });
        assert_eq!(result.signals.len(), 2);
        assert_eq!(result.signals[0].signal_type, "service_offer");
        assert_eq!(result.signals[0].confidence, 0.84);
        assert_eq!(result.signals[1].signal_type, "commercial_price");
        assert_eq!(result.signals[1].confidence, 0.81);
    }

    #[test]
    fn no_commercial_signal_in_keyword_context() {
        let result = execute(&CommercialSignalInput {
            section_id: "s2".to_owned(),
            raw_text: "SEO ranking, SERP, keyword and internal links".to_owned(),
        });
        assert!(result.signals.is_empty());
    }
}
