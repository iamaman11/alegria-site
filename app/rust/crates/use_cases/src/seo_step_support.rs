use contracts::generated::alegria::temporal::v1::SeoScopePayload;
use primitives::seo;

pub fn scope_signature(scope: Option<&SeoScopePayload>) -> String {
    if let Some(scope) = scope {
        if !scope.scope_signature.trim().is_empty() {
            return scope.scope_signature.trim().to_string();
        }
        if !scope.raw_scope_tuple.trim().is_empty() {
            return primitives::hash::blake3_hex(scope.raw_scope_tuple.trim().as_bytes());
        }
        return seo::scope_signature(&[
            ("market", &scope.market),
            ("locale", &scope.locale),
            ("country_code", &scope.country_code),
            ("visa_type", &scope.visa_type),
            ("applicant_profile", &scope.applicant_profile),
        ]);
    }
    seo::scope_signature(&[("scope", "default")])
}

pub fn slug(input: &str) -> String {
    let slug = seo::canonical_slug(input);
    if slug.is_empty() {
        "page".to_string()
    } else {
        slug
    }
}

pub fn artifact_key(prefix: &str, parts: &[&str]) -> String {
    seo::seo_artifact_key(prefix, parts)
}

pub fn score(value: f64) -> f64 {
    seo::clamp_score(value)
}
