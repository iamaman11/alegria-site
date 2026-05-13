use contracts::generated::alegria::temporal::v1::SeoScopePayload;
use primitives::{
    context_key,
    errors::DomainError,
    seo::{canonical_scope_tuple, scope_signature},
};

const ALLOWED_APPLICANT_PROFILES: &[&str] = &["standard", "minor", "student", "family"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedTruthIdentity {
    pub country_code: String,
    pub visa_family: String,
    pub visa_subtype: String,
    pub citizenship_code: String,
    pub context_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedSeoScope {
    pub market: String,
    pub locale: String,
    pub country_code: String,
    pub visa_type: String,
    pub applicant_profile: String,
    pub raw_scope_tuple: String,
    pub scope_signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedSeoBuildIdentity {
    pub truth: ValidatedTruthIdentity,
    pub scope: ValidatedSeoScope,
}

fn validation_failure(message: impl Into<String>) -> DomainError {
    DomainError::ValidationFailure {
        message: message.into(),
    }
}

fn require_non_empty(value: &str, label: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(validation_failure(format!("{label} is required")));
    }
    Ok(())
}

pub fn normalize_applicant_profile(value: &str) -> Result<String, DomainError> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(validation_failure(
            "SeoScopePayload.applicant_profile is required",
        ));
    }
    if !ALLOWED_APPLICANT_PROFILES.contains(&normalized.as_str()) {
        return Err(validation_failure(format!(
            "unsupported applicant_profile `{}`; allowed: {}",
            value.trim(),
            ALLOWED_APPLICANT_PROFILES.join(", ")
        )));
    }
    Ok(normalized)
}

pub fn normalize_optional_subtype(value: Option<&str>) -> Option<String> {
    value
        .map(|subtype| subtype.trim().to_ascii_lowercase())
        .filter(|subtype| !subtype.is_empty())
}

pub fn derive_truth_identity(
    country_code: &str,
    visa_family: &str,
    visa_subtype: Option<&str>,
    citizenship_code: &str,
) -> Result<ValidatedTruthIdentity, DomainError> {
    let country_code = country_code.trim().to_ascii_uppercase();
    let visa_family = visa_family.trim().to_ascii_lowercase();
    let visa_subtype = normalize_optional_subtype(visa_subtype).unwrap_or_default();
    let citizenship_code = citizenship_code.trim().to_ascii_uppercase();
    require_non_empty(&country_code, "country_code")?;
    require_non_empty(&visa_family, "visa_family")?;
    require_non_empty(&citizenship_code, "citizenship_code")?;
    let context_key = context_key::normalize_context_key(
        &country_code,
        &visa_family,
        if visa_subtype.is_empty() {
            None
        } else {
            Some(visa_subtype.as_str())
        },
        &citizenship_code,
    )
    .map_err(|err| validation_failure(format!("invalid seo context parts: {err}")))?;
    Ok(ValidatedTruthIdentity {
        country_code,
        visa_family,
        visa_subtype,
        citizenship_code,
        context_key,
    })
}

pub fn truth_identity_tuple(truth: &ValidatedTruthIdentity) -> String {
    format!(
        "country_code={}|visa_family={}|visa_subtype={}|citizenship_code={}",
        truth.country_code, truth.visa_family, truth.visa_subtype, truth.citizenship_code
    )
}

pub fn derive_scope(
    market: &str,
    locale: &str,
    country_code: &str,
    visa_type: &str,
    applicant_profile: &str,
) -> Result<ValidatedSeoScope, DomainError> {
    let market = market.trim().to_string();
    let locale = locale.trim().to_string();
    let country_code = country_code.trim().to_ascii_uppercase();
    let visa_type = visa_type.trim().to_ascii_lowercase();
    let applicant_profile = normalize_applicant_profile(applicant_profile)?;
    require_non_empty(&market, "SeoScopePayload.market")?;
    require_non_empty(&locale, "SeoScopePayload.locale")?;
    require_non_empty(&country_code, "SeoScopePayload.country_code")?;
    require_non_empty(&visa_type, "SeoScopePayload.visa_type")?;
    let raw_scope_tuple = canonical_scope_tuple(&[
        ("market", &market),
        ("locale", &locale),
        ("country_code", &country_code),
        ("visa_type", &visa_type),
        ("applicant_profile", &applicant_profile),
    ]);
    let scope_signature = scope_signature(&[
        ("market", &market),
        ("locale", &locale),
        ("country_code", &country_code),
        ("visa_type", &visa_type),
        ("applicant_profile", &applicant_profile),
    ]);
    Ok(ValidatedSeoScope {
        market,
        locale,
        country_code,
        visa_type,
        applicant_profile,
        raw_scope_tuple,
        scope_signature,
    })
}

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
        return Err(validation_failure(
            "SeoScopePayload.raw_scope_tuple does not match canonical normalized scope",
        ));
    }
    if !scope.scope_signature.trim().is_empty()
        && scope.scope_signature.trim() != validated.scope_signature
    {
        return Err(validation_failure(
            "SeoScopePayload.scope_signature does not match normalized scope",
        ));
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

pub fn assert_scope_matches_context(
    scope: &ValidatedSeoScope,
    truth: &ValidatedTruthIdentity,
) -> Result<(), DomainError> {
    if !scope.country_code.eq_ignore_ascii_case(&truth.country_code)
        || !scope.visa_type.eq_ignore_ascii_case(&truth.visa_family)
    {
        return Err(validation_failure(
            "SeoScopePayload must match truth identity country_code and visa_family",
        ));
    }
    Ok(())
}

pub fn build_scope_payload(scope: &ValidatedSeoScope) -> SeoScopePayload {
    SeoScopePayload {
        market: scope.market.clone(),
        locale: scope.locale.clone(),
        country_code: scope.country_code.clone(),
        visa_type: scope.visa_type.clone(),
        applicant_profile: scope.applicant_profile.clone(),
        raw_scope_tuple: scope.raw_scope_tuple.clone(),
        scope_signature: scope.scope_signature.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_scope_fields_are_normalized_once() {
        let scope = derive_scope(" alegria-site ", "ru-RU", "es", "Tourist", " Standard ").unwrap();
        assert_eq!(scope.country_code, "ES");
        assert_eq!(scope.visa_type, "tourist");
        assert_eq!(scope.applicant_profile, "standard");
    }

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
