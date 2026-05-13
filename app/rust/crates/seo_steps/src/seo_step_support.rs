use contracts::generated::alegria::temporal::v1::SeoScopePayload;
use primitives::seo;
use seo_domain::identity;

pub fn scope_signature(scope: Option<&SeoScopePayload>) -> String {
    identity::scope_signature_from_payload(scope)
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
