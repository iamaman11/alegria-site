use contracts::generated::alegria::temporal::v1::SeoScopePayload;
use primitives::errors::DomainError;
use seo_domain::artifact_identity::scope_signature;
use seo_domain::identity::{derive_scope, ValidatedSeoScope};

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

/// Missing scope is the explicit planning-default scope. A supplied, invalid
/// scope is never replaced by a caller-controlled signature or a default.
pub fn scope_signature_from_payload(
    scope: Option<&SeoScopePayload>,
) -> Result<String, DomainError> {
    match scope {
        Some(scope) => Ok(derive_scope_from_payload(scope)?.scope_signature),
        None => Ok(scope_signature(&[("scope", "default")])),
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

    #[test]
    fn invalid_explicit_scope_does_not_trust_a_supplied_signature() {
        let scope = SeoScopePayload {
            market: "alegria-site".into(),
            locale: "ru-RU".into(),
            country_code: "ES".into(),
            visa_type: "tourist".into(),
            applicant_profile: "standard".into(),
            raw_scope_tuple: "invalid tuple".into(),
            scope_signature: "attacker-controlled".into(),
        };
        assert!(scope_signature_from_payload(Some(&scope)).is_err());
        assert_eq!(
            scope_signature_from_payload(None).unwrap(),
            scope_signature(&[("scope", "default")])
        );
    }
}
