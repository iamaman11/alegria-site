use contracts::generated::alegria::temporal::v1::SeoScopePayload;
use primitives::errors::DomainError;
use seo_domain::artifact_identity::scope_signature;

pub use seo_domain::identity::{
    assert_scope_matches_context, derive_scope, derive_truth_identity, normalize_applicant_profile,
    normalize_optional_subtype, truth_identity_tuple, ValidatedSeoBuildIdentity, ValidatedSeoScope,
    ValidatedTruthIdentity,
};

/// Decode the Temporal wire type at the boundary; pure SEO identity has no
/// knowledge of protobuf or the workflow transport.
pub fn derive_scope_from_payload(
    scope: &SeoScopePayload,
) -> Result<ValidatedSeoScope, DomainError> {
    let validated = derive_scope(
        &scope.market,
        &scope.locale,
        &scope.country_code,
        &scope.visa_type,
        &scope.applicant_profile,
    )?;
    if !scope.raw_scope_tuple.trim().is_empty()
        && scope.raw_scope_tuple.trim() != validated.raw_scope_tuple
    {
        return Err(DomainError::ValidationFailure {
            message: "SeoScopePayload.raw_scope_tuple does not match canonical normalized scope"
                .into(),
        });
    }
    if !scope.scope_signature.trim().is_empty()
        && scope.scope_signature.trim() != validated.scope_signature
    {
        return Err(DomainError::ValidationFailure {
            message: "SeoScopePayload.scope_signature does not match normalized scope".into(),
        });
    }
    Ok(validated)
}

pub fn scope_signature_from_payload(scope: Option<&SeoScopePayload>) -> String {
    match scope {
        Some(scope) => derive_scope_from_payload(scope)
            .map(|validated| validated.scope_signature)
            .unwrap_or_else(|_| {
                if !scope.scope_signature.trim().is_empty() {
                    scope.scope_signature.trim().to_string()
                } else {
                    scope_signature(&[("scope", "default")])
                }
            }),
        None => scope_signature(&[("scope", "default")]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_raw_scope_tuple_must_match_canonical_scope() {
        let err = derive_scope_from_payload(&SeoScopePayload {
            market: "alegria-site".to_string(),
            locale: "ru-RU".to_string(),
            country_code: "ES".to_string(),
            visa_type: "tourist".to_string(),
            applicant_profile: "standard".to_string(),
            raw_scope_tuple: "scope=bad".to_string(),
            scope_signature: String::new(),
        })
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("raw_scope_tuple does not match canonical normalized scope"));
    }
}
