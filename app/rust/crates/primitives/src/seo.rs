use crate::hash::blake3_hex;

fn normalize_component(input: &str) -> String {
    input
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub fn canonical_scope_tuple(parts: &[(&str, &str)]) -> String {
    let mut normalized = parts
        .iter()
        .map(|(key, value)| (normalize_component(key), normalize_component(value)))
        .filter(|(key, value)| !key.is_empty() && !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort_by(|a, b| a.0.cmp(&b.0));
    normalized
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn scope_signature(parts: &[(&str, &str)]) -> String {
    blake3_hex(canonical_scope_tuple(parts).as_bytes())
}

pub fn seo_artifact_key(prefix: &str, parts: &[&str]) -> String {
    let mut material = Vec::with_capacity(parts.len() + 1);
    material.push(prefix);
    material.extend_from_slice(parts);
    blake3_hex(material.join("|").as_bytes())
}

/// A gap identifies one detector finding for one scope or page and topic.
/// Its reason code is already present in the protobuf contract; translate it
/// to a closed type here, not in individual SQL writers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentGapDetector {
    SerpReliability,
    GraphCoverage,
    ReconcileCoverage,
    MissingVerifiedSupport,
    DuplicateVerifiedEvidence,
}

impl ContentGapDetector {
    pub fn from_reason(reason: &str) -> Option<Self> {
        match reason {
            "low_reliability_serp" => Some(Self::SerpReliability),
            "graph_missing_topic_coverage" => Some(Self::GraphCoverage),
            "global_reconcile_missing_topic" => Some(Self::ReconcileCoverage),
            "page_missing_verified_support" => Some(Self::MissingVerifiedSupport),
            "page_duplicate_verified_evidence" => Some(Self::DuplicateVerifiedEvidence),
            _ => None,
        }
    }

    pub fn reason(self) -> &'static str {
        match self {
            Self::SerpReliability => "low_reliability_serp",
            Self::GraphCoverage => "graph_missing_topic_coverage",
            Self::ReconcileCoverage => "global_reconcile_missing_topic",
            Self::MissingVerifiedSupport => "page_missing_verified_support",
            Self::DuplicateVerifiedEvidence => "page_duplicate_verified_evidence",
        }
    }

    pub fn version(self) -> &'static str {
        match self {
            Self::SerpReliability => "serp_reliability@1",
            Self::GraphCoverage => "graph_coverage@1",
            Self::ReconcileCoverage => "reconcile_coverage@1",
            Self::MissingVerifiedSupport => "page_missing_support@1",
            Self::DuplicateVerifiedEvidence => "page_duplicate_evidence@1",
        }
    }

    pub fn valid_owner(self, page: &str) -> bool {
        match self {
            Self::SerpReliability => page.is_empty(),
            _ => !page.is_empty(),
        }
    }

    pub fn key(self, scope: &str, page: &str, topic: &str) -> String {
        // Length framing prevents arbitrary topic text containing separators
        // from becoming a different logical identity with the same digest.
        let material = [scope, page, topic, self.version()]
            .iter()
            .map(|part| format!("{}:{part}", part.len()))
            .collect::<String>();
        seo_artifact_key("content_gap", &[&material])
    }
}

pub fn canonical_slug(input: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in input.trim().chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

pub fn clamp_score(score: f64) -> f64 {
    if score.is_nan() {
        0.0
    } else {
        score.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_scope_tuple, canonical_slug, scope_signature};

    #[test]
    fn scope_tuple_is_order_and_case_stable() {
        let a = canonical_scope_tuple(&[("Market", "BY"), ("Locale", "ru-RU")]);
        let b = canonical_scope_tuple(&[("locale", " RU-RU "), ("market", "by")]);
        assert_eq!(a, b);
        assert_eq!(
            scope_signature(&[("Market", "BY")]),
            scope_signature(&[("market", " by ")])
        );
    }

    #[test]
    fn slug_preserves_human_unicode_keywords() {
        assert_eq!(
            canonical_slug("Виза в Испанию: документы и сроки 2026"),
            "виза-в-испанию-документы-и-сроки-2026"
        );
        assert_eq!(
            canonical_slug("España: visado turístico"),
            "españa-visado-turístico"
        );
    }

    #[test]
    fn slug_is_ascii_and_stable() {
        assert_eq!(
            canonical_slug(" Tourist Visa / Belarus "),
            "tourist-visa-belarus"
        );
    }
}

#[cfg(test)]
mod content_gap_identity_tests {
    use super::ContentGapDetector as Detector;

    #[test]
    fn gap_identity_distinguishes_owner_and_detector() {
        let d = Detector::MissingVerifiedSupport;
        let a = d.key("scope", "page-a", "fees");
        assert_eq!(a, d.key("scope", "page-a", "fees"));
        assert_ne!(a, d.key("scope", "page-b", "fees"));
        assert_ne!(a, Detector::SerpReliability.key("scope", "", "fees"));
        assert_ne!(a, Detector::GraphCoverage.key("scope", "page-a", "fees"));
        assert!(!d.valid_owner(""));
        assert!(!Detector::SerpReliability.valid_owner("page-a"));
        assert_ne!(d.key("a|b", "c", "d"), d.key("a", "b|c", "d"));
        assert_eq!(Detector::from_reason(d.reason()), Some(d));
        assert_eq!(Detector::from_reason("unknown"), None);
    }
}
