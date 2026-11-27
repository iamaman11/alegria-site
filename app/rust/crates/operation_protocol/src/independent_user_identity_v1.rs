//! Strict, provider-free OAuth user-to-one-step-request binding preflight.
//!
//! A ChatGPT/MCP JSON argument is NEVER authenticated user identity. The
//! `OAuthVerifierClaimsV1` value below must come from a real protected-resource
//! OAuth 2.1 verifier *after* checking the Bearer credential's signature or
//! introspection, trusted issuer, resource audience, validity and scopes.
//! This pure module has no credential parser, key loader, token issuer, OAuth
//! endpoint, database connection, or Temporal Start authority.
//!
//! The independent subject mapping and resource policy must be installed by
//! the server/DBA, never deserialized from an MCP request. A successful result
//! is only preflight; the separate SQL one-use reservation, source admission,
//! real Temporal History and provider budget enforcement are still mandatory.

use thiserror::Error;

use crate::{
    preflight_issued_one_step_request_for_reservation_v1,
    IndependentlyAuthenticatedOneStepPolicyV1, OneStepRequestGrantErrorV1,
    OneStepRequestReservationPreflightV1, OpaqueOneStepRequestLocatorV1, OperationDagRunV6,
    TrustedOneStepRequestGrantV1, TrustedRouteCatalog,
};

pub const SITE_ARCHITECT_ONE_STEP_OAUTH_SCOPE_V1: &str = "site_architect.run_one";
pub const MAX_OAUTH_ACCESS_TOKEN_LIFETIME_SECONDS_V1: i64 = 86_400;

/// Facts supplied only by an already-authenticated server OAuth ingress.
/// Intentionally not `Serialize`/`Deserialize`: neither MCP JSON nor a
/// database row can make this type evidence of a verified Bearer token.
///
/// Constructing this Rust value alone does NOT verify its provenance: the
/// caller must supply it from its independently configured verifier only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthVerifierClaimsV1 {
    pub issuer: String,
    pub audience: String,
    pub subject: String,
    pub tenant_id: String,
    /// Canonical, space-delimited OAuth scopes validated by the verifier.
    pub scope: String,
    pub issued_at_unix_seconds: i64,
    pub not_before_unix_seconds: i64,
    pub expires_at_unix_seconds: i64,
}

/// Trusted resource-server configuration, independent of user parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledOAuthResourcePolicyV1 {
    pub expected_issuer: String,
    pub expected_audience: String,
    pub required_scope: String,
    pub maximum_token_lifetime_seconds: i64,
}

/// Independent OAuth issuer+subject to tenant/principal/policy mapping.
/// This is the identity ACL configured by an administrator, not an identity
/// or role self-asserted by the caller or by an unsigned request token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndependentlyProvisionedOAuthSubjectV1 {
    pub issuer: String,
    pub subject: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub policy_id: String,
    pub context_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthOneStepIdentityPreflightV1 {
    request_id: String,
}

impl OAuthOneStepIdentityPreflightV1 {
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}

#[derive(Debug, Error)]
pub enum OAuthOneStepIdentityErrorV1 {
    #[error("independently installed OAuth resource policy is invalid")]
    InvalidResourcePolicy,
    #[error("verified ingress token claims are malformed")]
    MalformedClaims,
    #[error("OAuth issuer or audience differs from the installed resource")]
    UntrustedIssuerOrAudience,
    #[error("OAuth Bearer token is not valid at this instant")]
    NotActive,
    #[error("OAuth Bearer token does not have the required exact scope")]
    MissingScope,
    #[error("authenticated OAuth subject is not independently mapped to this request")]
    UnmappedUser,
    #[error("independently approved SQL principal and one-use grant do not match OAuth subject")]
    ScopeMismatch,
    #[error("one-use SQL grant or native owner preflight rejected the request")]
    Grant(#[from] OneStepRequestGrantErrorV1),
}

fn bounded_identity(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn canonical_space_delimited_scopes(value: &str) -> Option<Vec<&str>> {
    if value.len() > 4096 || value.is_empty() || !value.is_ascii() {
        return None;
    }
    let parts = value.split(' ').collect::<Vec<_>>();
    if parts.iter().any(|part| {
        part.is_empty()
            || part.len() > 128
            || !part.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'.' | b'_' | b'-' | b'/')
            })
    }) {
        return None;
    }
    let mut distinct = parts.clone();
    distinct.sort_unstable();
    distinct.dedup();
    (distinct.len() == parts.len()).then_some(parts)
}

/// Check an external OAuth verifier's claims against server-provisioned
/// policy/subject ACL, the DBA-bound SQL principal and the exact grant.
///
/// This function is NOT an OAuth verifier. It must never be called with
/// identity values derived from public MCP arguments. In particular, a
/// matching `subject` string from JSON is no credential or permission.
pub fn preflight_trusted_oauth_one_step_identity_v1(
    claims_from_verifier: &OAuthVerifierClaimsV1,
    installed_resource: &InstalledOAuthResourcePolicyV1,
    installed_subject: &IndependentlyProvisionedOAuthSubjectV1,
    approved_sql_policy: &IndependentlyAuthenticatedOneStepPolicyV1,
    locator: &OpaqueOneStepRequestLocatorV1,
    grant: &TrustedOneStepRequestGrantV1,
    now_unix_seconds: i64,
) -> Result<OAuthOneStepIdentityPreflightV1, OAuthOneStepIdentityErrorV1> {
    if !bounded_identity(&installed_resource.expected_issuer, 512)
        || !bounded_identity(&installed_resource.expected_audience, 512)
        || installed_resource.required_scope != SITE_ARCHITECT_ONE_STEP_OAUTH_SCOPE_V1
        || !(1..=MAX_OAUTH_ACCESS_TOKEN_LIFETIME_SECONDS_V1)
            .contains(&installed_resource.maximum_token_lifetime_seconds)
    {
        return Err(OAuthOneStepIdentityErrorV1::InvalidResourcePolicy);
    }
    if !bounded_identity(&claims_from_verifier.issuer, 512)
        || !bounded_identity(&claims_from_verifier.audience, 512)
        || !bounded_identity(&claims_from_verifier.subject, 512)
        || !bounded_identity(&claims_from_verifier.tenant_id, 128)
    {
        return Err(OAuthOneStepIdentityErrorV1::MalformedClaims);
    }
    if claims_from_verifier.issuer != installed_resource.expected_issuer
        || claims_from_verifier.audience != installed_resource.expected_audience
    {
        return Err(OAuthOneStepIdentityErrorV1::UntrustedIssuerOrAudience);
    }
    let token_lifetime = claims_from_verifier
        .expires_at_unix_seconds
        .checked_sub(claims_from_verifier.issued_at_unix_seconds);
    if claims_from_verifier.issued_at_unix_seconds < 0
        || claims_from_verifier.not_before_unix_seconds
            < claims_from_verifier.issued_at_unix_seconds
        || claims_from_verifier.expires_at_unix_seconds
            <= claims_from_verifier.not_before_unix_seconds
        || !matches!(
            token_lifetime,
            Some(seconds) if seconds > 0 && seconds <= installed_resource.maximum_token_lifetime_seconds
        )
        || now_unix_seconds < claims_from_verifier.not_before_unix_seconds
        || now_unix_seconds >= claims_from_verifier.expires_at_unix_seconds
    {
        return Err(OAuthOneStepIdentityErrorV1::NotActive);
    }
    let scopes = canonical_space_delimited_scopes(&claims_from_verifier.scope)
        .ok_or(OAuthOneStepIdentityErrorV1::MalformedClaims)?;
    if !scopes.contains(&installed_resource.required_scope.as_str()) {
        return Err(OAuthOneStepIdentityErrorV1::MissingScope);
    }
    if !bounded_identity(&installed_subject.issuer, 512)
        || !bounded_identity(&installed_subject.subject, 512)
        || !bounded_identity(&installed_subject.tenant_id, 128)
        || !bounded_identity(&installed_subject.principal_id, 128)
        || !bounded_identity(&installed_subject.policy_id, 128)
        || !bounded_identity(&installed_subject.context_key, 512)
        || installed_subject.issuer != claims_from_verifier.issuer
        || installed_subject.subject != claims_from_verifier.subject
        || installed_subject.tenant_id != claims_from_verifier.tenant_id
    {
        return Err(OAuthOneStepIdentityErrorV1::UnmappedUser);
    }
    if installed_subject.tenant_id != approved_sql_policy.tenant_id
        || installed_subject.principal_id != approved_sql_policy.principal_id
        || installed_subject.policy_id != approved_sql_policy.policy_id
        || installed_subject.context_key != approved_sql_policy.context_key
        || installed_subject.tenant_id != grant.tenant_id
        || installed_subject.principal_id != grant.authenticated_principal_id
        || installed_subject.policy_id != grant.authorization_policy_id
        || installed_subject.context_key != grant.context_key
        || locator.request_id != grant.request_id
    {
        return Err(OAuthOneStepIdentityErrorV1::ScopeMismatch);
    }
    Ok(OAuthOneStepIdentityPreflightV1 {
        request_id: grant.request_id.clone(),
    })
}

/// Composition of the authenticated-user boundary with the **existing**
/// exact one-use grant/plan/owner/budget admission. The result still cannot
/// perform a Start; a separately credentialed SQL transaction must reserve.
pub fn preflight_trusted_oauth_one_step_reservation_v1(
    claims_from_verifier: &OAuthVerifierClaimsV1,
    installed_resource: &InstalledOAuthResourcePolicyV1,
    installed_subject: &IndependentlyProvisionedOAuthSubjectV1,
    approved_sql_policy: &IndependentlyAuthenticatedOneStepPolicyV1,
    locator: &OpaqueOneStepRequestLocatorV1,
    grant: &TrustedOneStepRequestGrantV1,
    run: &OperationDagRunV6,
    installed_routes: &TrustedRouteCatalog,
    now_unix_seconds: i64,
) -> Result<OneStepRequestReservationPreflightV1, OAuthOneStepIdentityErrorV1> {
    preflight_trusted_oauth_one_step_identity_v1(
        claims_from_verifier,
        installed_resource,
        installed_subject,
        approved_sql_policy,
        locator,
        grant,
        now_unix_seconds,
    )?;
    Ok(preflight_issued_one_step_request_for_reservation_v1(
        locator,
        grant,
        run,
        installed_routes,
        approved_sql_policy,
        now_unix_seconds,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        OneStepRequestIssuanceStateV1, OneStepResourceLimitsV1, OperationId, OperationRef,
        PlanHash, SiteLabExpectedStartV1,
    };
    use std::collections::BTreeMap;

    fn fixture() -> (
        OAuthVerifierClaimsV1,
        InstalledOAuthResourcePolicyV1,
        IndependentlyProvisionedOAuthSubjectV1,
        IndependentlyAuthenticatedOneStepPolicyV1,
        OpaqueOneStepRequestLocatorV1,
        TrustedOneStepRequestGrantV1,
    ) {
        let claims = OAuthVerifierClaimsV1 {
            issuer: "https://auth.example.test/oauth".into(),
            audience: "https://site-architect.example.test/mcp".into(),
            subject: "subject-id-123".into(),
            tenant_id: "tenant-a".into(),
            scope: format!("openid {SITE_ARCHITECT_ONE_STEP_OAUTH_SCOPE_V1}"),
            issued_at_unix_seconds: 1_900_000_000,
            not_before_unix_seconds: 1_900_000_000,
            expires_at_unix_seconds: 1_900_000_600,
        };
        let resource = InstalledOAuthResourcePolicyV1 {
            expected_issuer: claims.issuer.clone(),
            expected_audience: claims.audience.clone(),
            required_scope: SITE_ARCHITECT_ONE_STEP_OAUTH_SCOPE_V1.into(),
            maximum_token_lifetime_seconds: 900,
        };
        let mapping = IndependentlyProvisionedOAuthSubjectV1 {
            issuer: claims.issuer.clone(),
            subject: claims.subject.clone(),
            tenant_id: "tenant-a".into(),
            principal_id: "principal-from-dba".into(),
            policy_id: "bounded-policy-v1".into(),
            context_key: "context-a".into(),
        };
        let limits = OneStepResourceLimitsV1 {
            max_admitted_source_bytes: 1_024,
            max_effect_dispatches: 0,
            max_provider_requests: 0,
            currency_code: "USD".into(),
            max_charge_minor_units: 0,
        };
        let policy = IndependentlyAuthenticatedOneStepPolicyV1 {
            tenant_id: mapping.tenant_id.clone(),
            principal_id: mapping.principal_id.clone(),
            policy_id: mapping.policy_id.clone(),
            context_key: mapping.context_key.clone(),
            maximum_limits: limits.clone(),
        };
        let request_id = "109345ce-ae42-4f83-b981-28d78e7f0f02";
        let locator = OpaqueOneStepRequestLocatorV1 {
            request_id: request_id.into(),
        };
        let grant = TrustedOneStepRequestGrantV1 {
            schema_revision: 1,
            request_id: request_id.into(),
            authorization_event_id: "ed11a463-bbda-419b-90d0-778d0b445ccf".into(),
            tenant_id: policy.tenant_id.clone(),
            authenticated_principal_id: policy.principal_id.clone(),
            authorization_policy_id: policy.policy_id.clone(),
            context_key: policy.context_key.clone(),
            canonical_run_id: "b4b2a9d3-588a-44ee-9537-8ff59ef251ff".into(),
            operation: OperationRef {
                id: OperationId("acquisition.classify_source_page".into()),
                revision: 1,
            },
            parent_workflow_id: "independent-parent".into(),
            expected: SiteLabExpectedStartV1 {
                schema_revision: 1,
                plan_hash: PlanHash("a".repeat(64)),
                command_hashes: BTreeMap::new(),
                source_payload_hashes: BTreeMap::new(),
            },
            limits,
            issued_at_unix_seconds: 1_900_000_000,
            expires_at_unix_seconds: 1_900_000_300,
            revoked_at_unix_seconds: None,
            state: OneStepRequestIssuanceStateV1::Unclaimed,
        };
        (claims, resource, mapping, policy, locator, grant)
    }

    #[test]
    fn verifier_claims_require_exact_resource_subject_acl_and_grant() {
        let (claims, resource, mapping, policy, locator, grant) = fixture();
        let checked = preflight_trusted_oauth_one_step_identity_v1(
            &claims,
            &resource,
            &mapping,
            &policy,
            &locator,
            &grant,
            1_900_000_100,
        )
        .unwrap();
        assert_eq!(checked.request_id(), locator.request_id.as_str());
        let mut altered = claims.clone();
        altered.subject = "another-oauth-account".into();
        assert!(matches!(
            preflight_trusted_oauth_one_step_identity_v1(
                &altered,
                &resource,
                &mapping,
                &policy,
                &locator,
                &grant,
                1_900_000_100,
            ),
            Err(OAuthOneStepIdentityErrorV1::UnmappedUser)
        ));
        let mut altered = claims.clone();
        altered.tenant_id = "tenant-b".into();
        assert!(preflight_trusted_oauth_one_step_identity_v1(
            &altered,
            &resource,
            &mapping,
            &policy,
            &locator,
            &grant,
            1_900_000_100,
        )
        .is_err());
        let mut wrong = grant.clone();
        wrong.authenticated_principal_id = "other-principal".into();
        assert!(matches!(
            preflight_trusted_oauth_one_step_identity_v1(
                &claims,
                &resource,
                &mapping,
                &policy,
                &locator,
                &wrong,
                1_900_000_100,
            ),
            Err(OAuthOneStepIdentityErrorV1::ScopeMismatch)
        ));
        let mut wrong = policy.clone();
        wrong.context_key = "another-context".into();
        assert!(preflight_trusted_oauth_one_step_identity_v1(
            &claims,
            &resource,
            &mapping,
            &wrong,
            &locator,
            &grant,
            1_900_000_100,
        )
        .is_err());
        let mut other = locator.clone();
        other.request_id = "d8d1760b-9732-4480-b082-237bad3dd845".into();
        assert!(preflight_trusted_oauth_one_step_identity_v1(
            &claims,
            &resource,
            &mapping,
            &policy,
            &other,
            &grant,
            1_900_000_100,
        )
        .is_err());
    }

    #[test]
    fn arbitrary_oauth_scope_substrings_and_invalid_time_are_refused() {
        let (claims, resource, mapping, policy, locator, grant) = fixture();
        for bad_scope in [
            "site_architect.run_one.bypass",
            "all_site_architect.run_one",
            "site_architect.run_one site_architect.run_one",
            "site_architect.run_one  openid",
            "site_architect.run_one\nadmin",
            " ",
        ] {
            let mut altered = claims.clone();
            altered.scope = bad_scope.into();
            assert!(
                preflight_trusted_oauth_one_step_identity_v1(
                    &altered,
                    &resource,
                    &mapping,
                    &policy,
                    &locator,
                    &grant,
                    1_900_000_100,
                )
                .is_err(),
                "{bad_scope:?}"
            );
        }
        let mut altered = claims.clone();
        altered.not_before_unix_seconds = 1_900_000_200;
        assert!(matches!(
            preflight_trusted_oauth_one_step_identity_v1(
                &altered,
                &resource,
                &mapping,
                &policy,
                &locator,
                &grant,
                1_900_000_100,
            ),
            Err(OAuthOneStepIdentityErrorV1::NotActive)
        ));
        assert!(preflight_trusted_oauth_one_step_identity_v1(
            &claims,
            &resource,
            &mapping,
            &policy,
            &locator,
            &grant,
            1_900_000_600,
        )
        .is_err());
        let mut altered = claims.clone();
        altered.expires_at_unix_seconds = 1_900_086_400;
        assert!(preflight_trusted_oauth_one_step_identity_v1(
            &altered,
            &resource,
            &mapping,
            &policy,
            &locator,
            &grant,
            1_900_000_100,
        )
        .is_err());
        let mut altered = claims.clone();
        altered.issued_at_unix_seconds = i64::MIN;
        altered.expires_at_unix_seconds = i64::MAX;
        assert!(matches!(
            preflight_trusted_oauth_one_step_identity_v1(
                &altered,
                &resource,
                &mapping,
                &policy,
                &locator,
                &grant,
                1_900_000_100,
            ),
            Err(OAuthOneStepIdentityErrorV1::NotActive)
        ));
        let mut altered = resource.clone();
        altered.required_scope = "openid".into();
        assert!(matches!(
            preflight_trusted_oauth_one_step_identity_v1(
                &claims,
                &altered,
                &mapping,
                &policy,
                &locator,
                &grant,
                1_900_000_100,
            ),
            Err(OAuthOneStepIdentityErrorV1::InvalidResourcePolicy)
        ));
    }

    #[test]
    fn wrong_resource_audience_or_unmapped_issuer_are_not_interchangeable() {
        let (claims, resource, mapping, policy, locator, grant) = fixture();
        let mut wrong = claims.clone();
        wrong.audience = "https://unrelated-service.example.test".into();
        assert!(matches!(
            preflight_trusted_oauth_one_step_identity_v1(
                &wrong,
                &resource,
                &mapping,
                &policy,
                &locator,
                &grant,
                1_900_000_100,
            ),
            Err(OAuthOneStepIdentityErrorV1::UntrustedIssuerOrAudience)
        ));
        let mut wrong = mapping.clone();
        wrong.issuer = "https://other-issuer.example.test".into();
        assert!(matches!(
            preflight_trusted_oauth_one_step_identity_v1(
                &claims,
                &resource,
                &wrong,
                &policy,
                &locator,
                &grant,
                1_900_000_100,
            ),
            Err(OAuthOneStepIdentityErrorV1::UnmappedUser)
        ));
        let mut wrong = resource.clone();
        wrong.expected_audience = "\r\nauth injection".into();
        assert!(matches!(
            preflight_trusted_oauth_one_step_identity_v1(
                &claims,
                &wrong,
                &mapping,
                &policy,
                &locator,
                &grant,
                1_900_000_100,
            ),
            Err(OAuthOneStepIdentityErrorV1::InvalidResourcePolicy)
        ));
    }
}
