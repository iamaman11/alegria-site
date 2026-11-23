use crate::seo_identity;
use contracts::generated::alegria::temporal::v1::SeoScopePayload;
use seo_domain::artifact_identity;

pub fn scope_signature(scope: Option<&SeoScopePayload>) -> String {
    seo_identity::scope_signature_from_payload(scope)
}

pub fn slug(input: &str) -> String {
    let slug = artifact_identity::canonical_slug(input);
    if slug.is_empty() {
        "page".to_string()
    } else {
        slug
    }
}

pub fn artifact_key(prefix: &str, parts: &[&str]) -> String {
    artifact_identity::seo_artifact_key(prefix, parts)
}

pub fn score(value: f64) -> f64 {
    artifact_identity::clamp_score(value)
}
