use contracts::generated::alegria::temporal::v1::{
    QualityPolicyEvaluationInputPayload, QualityPolicyEvaluationOutputPayload,
};

pub fn execute(
    input: &QualityPolicyEvaluationInputPayload,
) -> QualityPolicyEvaluationOutputPayload {
    let policy =
        policies::quality_policy::resolve_quality_policy(&input.page_type_key, &input.locale);
    let score = policies::quality_policy::score_quality(
        &policy,
        input.completeness_score,
        input.supported_claims,
        input.missing_required_sections,
    );
    QualityPolicyEvaluationOutputPayload {
        page_type_key: policy.page_type_key,
        locale: policy.locale,
        policy_version: policy.policy_version,
        min_quality_score: policy.min_quality_score,
        min_supported_claims: policy.min_supported_claims,
        max_missing_required_sections: policy.max_missing_required_sections,
        score: score.score,
        blocked: score.blocked,
        blocking_reasons: score.reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_registry_thresholds() {
        let output = execute(&QualityPolicyEvaluationInputPayload {
            page_type_key: "requirement_page".to_string(),
            locale: "ru-RU".to_string(),
            completeness_score: 0.95,
            supported_claims: 5,
            missing_required_sections: 0,
        });

        assert_eq!(output.policy_version, "quality_policy@1");
        assert!(!output.blocked);
        assert!(output.score >= 0.95);
    }
}
