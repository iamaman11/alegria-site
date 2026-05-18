use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QualityPolicy {
    pub page_type_key: String,
    pub locale: String,
    pub min_quality_score: f32,
    pub min_supported_claims: u32,
    pub max_missing_required_sections: u32,
    pub policy_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QualityPolicyScore {
    pub score: f32,
    pub blocked: bool,
    pub reasons: Vec<String>,
}

pub fn resolve_quality_policy(page_type_key: &str, locale: &str) -> QualityPolicy {
    let page_type_key = page_type_key.trim().to_ascii_lowercase();
    let locale = locale.trim().to_string();
    match (page_type_key.as_str(), locale.as_str()) {
        ("fee_page", "ru-RU") => QualityPolicy {
            page_type_key,
            locale,
            min_quality_score: 0.88,
            min_supported_claims: 4,
            max_missing_required_sections: 0,
            policy_version: "quality_policy@1".to_string(),
        },
        ("timeline_page", "ru-RU") => QualityPolicy {
            page_type_key,
            locale,
            min_quality_score: 0.90,
            min_supported_claims: 4,
            max_missing_required_sections: 0,
            policy_version: "quality_policy@1".to_string(),
        },
        ("requirement_page", "ru-RU") => QualityPolicy {
            page_type_key,
            locale,
            min_quality_score: 0.92,
            min_supported_claims: 5,
            max_missing_required_sections: 0,
            policy_version: "quality_policy@1".to_string(),
        },
        ("faq_page", "ru-RU") => QualityPolicy {
            page_type_key,
            locale,
            min_quality_score: 0.82,
            min_supported_claims: 3,
            max_missing_required_sections: 0,
            policy_version: "quality_policy@1".to_string(),
        },
        (_, _) => QualityPolicy {
            page_type_key,
            locale,
            min_quality_score: 0.85,
            min_supported_claims: 3,
            max_missing_required_sections: 0,
            policy_version: "quality_policy@1".to_string(),
        },
    }
}

pub fn score_quality(
    policy: &QualityPolicy,
    completeness_score: f32,
    supported_claims: u32,
    missing_required_sections: u32,
) -> QualityPolicyScore {
    let score = (completeness_score + ((supported_claims as f32) * 0.03)
        - ((missing_required_sections as f32) * 0.12))
        .clamp(0.0, 1.0);
    let mut reasons = Vec::new();
    if score < policy.min_quality_score {
        reasons.push("below_quality_threshold".to_string());
    }
    if supported_claims < policy.min_supported_claims {
        reasons.push("insufficient_supported_claims".to_string());
    }
    if missing_required_sections > policy.max_missing_required_sections {
        reasons.push("missing_required_sections".to_string());
    }
    QualityPolicyScore {
        score,
        blocked: !reasons.is_empty(),
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_locale_specific_policy() {
        let policy = resolve_quality_policy("requirement_page", "ru-RU");
        assert_eq!(policy.min_quality_score, 0.92);
        assert_eq!(policy.min_supported_claims, 5);
    }

    #[test]
    fn scores_and_blocks_below_threshold() {
        let policy = resolve_quality_policy("fee_page", "ru-RU");
        let score = score_quality(&policy, 0.70, 1, 1);
        assert!(score.blocked);
        assert!(score
            .reasons
            .contains(&"below_quality_threshold".to_string()));
    }
}
