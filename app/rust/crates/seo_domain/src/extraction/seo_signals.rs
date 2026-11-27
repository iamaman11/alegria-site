//! Pure SEO signal extraction from an already admitted section.
//!
//! The Temporal caller owns section loading, source provenance and gates.
//! This operation cannot access PostgreSQL, authorize a crawler or perform
//! provider I/O. It mirrors the existing SEO signal policy of EX-07.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeoSignalInput {
    pub section_id: String,
    pub raw_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeoSignal {
    pub signal_type: String,
    pub value: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeoSignalOutput {
    pub signals: Vec<SeoSignal>,
}

/// Deterministic, side-effect-free SEO language classifier.
/// No source is made authoritative by these lexical signals.
pub fn execute(input: &SeoSignalInput) -> SeoSignalOutput {
    let lowered = input.raw_text.to_lowercase();
    let mut signals = Vec::new();
    if lowered.contains("seo") || lowered.contains("serp") {
        signals.push(SeoSignal {
            signal_type: "ranking_signal".to_owned(),
            value: "seo_or_serp_mentioned".to_owned(),
            confidence: 0.82,
        });
    }
    if lowered.contains("keyword") || lowered.contains("ключев") {
        signals.push(SeoSignal {
            signal_type: "keyword_signal".to_owned(),
            value: "keyword_language_present".to_owned(),
            confidence: 0.79,
        });
    }
    SeoSignalOutput { signals }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seo_and_keyword_signals_remain_distinct() {
        let result = execute(&SeoSignalInput {
            section_id: "s1".to_owned(),
            raw_text: "SERP ranking и ключевые запросы".to_owned(),
        });
        assert_eq!(result.signals.len(), 2);
        assert_eq!(result.signals[0].signal_type, "ranking_signal");
        assert_eq!(result.signals[0].confidence, 0.82);
        assert_eq!(result.signals[1].signal_type, "keyword_signal");
        assert_eq!(result.signals[1].confidence, 0.79);
    }

    #[test]
    fn no_signals_for_unrelated_or_empty_text() {
        for raw_text in ["", "порядок оплаты услуг", "очередь клиентов"]
        {
            let result = execute(&SeoSignalInput {
                section_id: "s2".to_owned(),
                raw_text: raw_text.to_owned(),
            });
            assert!(result.signals.is_empty());
        }
    }
}
