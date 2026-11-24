use contracts::generated::alegria::temporal::v1::SeoVerifiedFactSupportState;
use primitives::errors::DomainError;
use seo_domain::page_readiness::{truth_support_defect, TruthSupportDefect};

pub fn truth_admissibility_gate(
    bundle: &[SeoVerifiedFactSupportState],
    context_key: &str,
    applicant_profile: &str,
) -> Result<(), DomainError> {
    if bundle.is_empty() {
        return Err(DomainError::ValidationFailure {
            message: format!(
                "truth_admissibility_gate failed: admissible verified support is empty for context_key `{context_key}` and applicant_profile `{applicant_profile}`"
            ),
        });
    }
    for support in bundle {
        let defect = truth_support_defect(
            &support.support_ref,
            &support.fragment_text,
            &support.role_type,
            &support.source_label,
            &support.freshness_class,
        );
        let message = match defect {
            Some(TruthSupportDefect::MissingMandatoryFields) => format!(
                "truth_admissibility_gate failed: support row `{}` is missing mandatory admissibility fields",
                support.support_ref
            ),
            Some(TruthSupportDefect::NonFresh) => format!(
                "truth_admissibility_gate failed: support row `{}` has non-fresh freshness_class `{}`",
                support.support_ref, support.freshness_class
            ),
            None => continue,
        };
        return Err(DomainError::ValidationFailure { message });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn support_with_freshness(freshness_class: &str) -> SeoVerifiedFactSupportState {
        SeoVerifiedFactSupportState {
            fragment_text: "Passport required".to_string(),
            support_ref: "rule:passport".to_string(),
            role_type: "document_required".to_string(),
            source_label: "Consulate".to_string(),
            source_tier: "official".to_string(),
            freshness_class: freshness_class.to_string(),
            observed_at: String::new(),
            valid_until: String::new(),
            source_url: String::new(),
        }
    }

    #[test]
    fn rejects_empty_truth_bundle_at_admissibility_gate() {
        let err = truth_admissibility_gate(&[], "ES|tourist||BY", "standard").unwrap_err();
        assert_eq!(err.class_str(), "validation_failure");
        assert!(err.to_string().contains("truth_admissibility_gate failed"));
    }

    #[test]
    fn rejects_non_fresh_truth_bundle_at_admissibility_gate() {
        for freshness in ["watch", "stale", "unknown", ""] {
            let err = truth_admissibility_gate(
                &[support_with_freshness(freshness)],
                "ES|tourist||BY",
                "standard",
            )
            .unwrap_err();
            assert!(err.to_string().contains("non-fresh freshness_class"));
        }
        truth_admissibility_gate(
            &[support_with_freshness("fresh")],
            "ES|tourist||BY",
            "standard",
        )
        .unwrap();
    }
}
