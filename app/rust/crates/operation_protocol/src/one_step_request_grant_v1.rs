//! Pure, fail-closed check of ONE separately issued user Start request.
//!
//! There is intentionally no Temporal Start, authorization issuer, database
//! mutation or MCP Start tool here. The `TrustedOneStepRequestGrantV1` MUST be
//! loaded from a separately credentialed, immutable issuer record. The
//! `IndependentlyAuthenticatedOneStepPolicyV1` MUST come from authenticated
//! identity/tenant ACL and budget policy, not the user-supplied locator.
//! This check permits only an **atomic one-use reservation attempt** by a
//! future privileged bridge. It does NOT authenticate source provenance,
//! issue a SQL business root, verify owner receipts or meter provider usage.
//! It is not executable authorization by itself.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    operation_dag_parent_workflow_id_v2, preflight_unadmitted_single_operation_v2_start,
    OperationDagRunV6, OperationRef, SiteArchitectExpectedStartV1,
    SiteArchitectSingleOperationPreflightError, TrustedRouteCatalog, DAG_MAX_INPUT_BYTES,
};

pub const ONE_STEP_REQUEST_GRANT_SCHEMA_REVISION_V1: u16 = 1;
pub const ONE_STEP_REQUEST_GRANT_MAX_LIFETIME_SECONDS_V1: i64 = 600;

/// The **only** client-supplied input to a future preauthorized Start bridge.
/// No tenant, role, run, URL, path, command, source or Temporal route is valid
/// in this envelope. It is a lookup key, never a bearer permission by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpaqueOneStepRequestLocatorV1 {
    pub request_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OneStepRequestIssuanceStateV1 {
    /// Eligible for an external issuer's **atomic** claim; not a Start permit.
    Unclaimed,
    Reserved,
    StartSubmitted,
    StartOutcomeUnknown,
    StartConfirmed,
}

/// Finite upper bound for an independent user's approved experiment. This is
/// not a provider billing/usage meter: the owner must enforce each call and
/// charge against the actual budget in its own durable admission path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OneStepResourceLimitsV1 {
    pub max_admitted_source_bytes: u32,
    pub max_effect_dispatches: u16,
    pub max_provider_requests: u32,
    pub currency_code: String,
    pub max_charge_minor_units: u64,
}

/// Private issuer database row, never deserialized from an MCP argument.
/// `expected` comes from independently sealed owner-command/source bytes;
/// self-consistent hashes calculated from the caller's run are not authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedOneStepRequestGrantV1 {
    pub schema_revision: u16,
    pub request_id: String,
    pub authorization_event_id: String,
    pub tenant_id: String,
    pub authenticated_principal_id: String,
    pub authorization_policy_id: String,
    pub context_key: String,
    pub canonical_run_id: String,
    pub operation: OperationRef,
    pub parent_workflow_id: String,
    pub expected: SiteArchitectExpectedStartV1,
    pub limits: OneStepResourceLimitsV1,
    pub issued_at_unix_seconds: i64,
    pub expires_at_unix_seconds: i64,
    pub revoked_at_unix_seconds: Option<i64>,
    pub state: OneStepRequestIssuanceStateV1,
}

/// Independent identity and ACL policy checked by the privileged service;
/// do not populate this struct from an untrusted MCP request or grant row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndependentlyAuthenticatedOneStepPolicyV1 {
    pub tenant_id: String,
    pub principal_id: String,
    pub policy_id: String,
    pub context_key: String,
    pub maximum_limits: OneStepResourceLimitsV1,
}

/// Private proof that the record can be submitted to a transactionally
/// exclusive one-use claim. No public Serialize, no Start permission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneStepRequestReservationPreflightV1 {
    request_id: String,
}

impl OneStepRequestReservationPreflightV1 {
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}

#[derive(Debug, Error)]
pub enum OneStepRequestGrantErrorV1 {
    #[error("one-step request locator is invalid or not canonical")]
    InvalidLocator,
    #[error("one-step request issuer record is invalid or overbroad")]
    InvalidIssuerRecord,
    #[error("one-step request issuer record is not yet valid")]
    NotYetValid,
    #[error("one-step request issuer record expired")]
    Expired,
    #[error("one-step request was revoked")]
    Revoked,
    #[error("one-step request is reserved, used or has an ambiguous Start outcome")]
    AlreadyClaimedOrUnknown,
    #[error("independently authenticated user, tenant or policy does not match request")]
    UnauthorizedPrincipal,
    #[error("one-step request run or operation identity differs")]
    ScopeMismatch,
    #[error("one-step request exceeds independently authorized resource limits")]
    BudgetExceeded,
    #[error("one-step sealed bytes, plan or trusted route do not match")]
    SealedPlanMismatch(#[source] SiteArchitectSingleOperationPreflightError),
}

pub(super) fn canonical_non_nil_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value)
        .map(|id| !id.is_nil() && id.to_string() == value)
        .unwrap_or(false)
}

fn bounded_identifier(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn valid_currency(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn checked_limits(limits: &OneStepResourceLimitsV1) -> bool {
    limits.max_admitted_source_bytes > 0
        && limits.max_admitted_source_bytes as usize <= DAG_MAX_INPUT_BYTES
        && limits.max_effect_dispatches <= 1
        && limits.max_provider_requests <= 1000
        && valid_currency(&limits.currency_code)
}

/// Validate immutable record shape, independently authenticated ACL identity,
/// finite time window, one-use status, resource ceiling, and the **same**
/// native V2 owner command/route/plan/source preflight as operator Start.
///
/// Successful return **cannot** start Temporal. The caller must next perform
/// a durable, exclusive `Unclaimed -> Reserved` compare-and-swap, pin the
/// reservation owner, and persist `StartSubmitted/StartOutcomeUnknown` before
/// sending a Start RPC. Independently authenticated source records and a
/// tenant business-root issuer remain separate mandatory prerequisites.
/// An unknown outcome must never be retried blindly.
pub fn preflight_issued_one_step_request_for_reservation_v1(
    locator: &OpaqueOneStepRequestLocatorV1,
    record: &TrustedOneStepRequestGrantV1,
    run: &OperationDagRunV6,
    installed_routes: &TrustedRouteCatalog,
    independently_authenticated_policy: &IndependentlyAuthenticatedOneStepPolicyV1,
    now_unix_seconds: i64,
) -> Result<OneStepRequestReservationPreflightV1, OneStepRequestGrantErrorV1> {
    if record.schema_revision != ONE_STEP_REQUEST_GRANT_SCHEMA_REVISION_V1
        || !canonical_non_nil_uuid(&locator.request_id)
        || !canonical_non_nil_uuid(&record.request_id)
        || !canonical_non_nil_uuid(&record.authorization_event_id)
        || !canonical_non_nil_uuid(&record.canonical_run_id)
        || !bounded_identifier(&record.tenant_id, 128)
        || !bounded_identifier(&record.authenticated_principal_id, 128)
        || !bounded_identifier(&record.authorization_policy_id, 128)
        || !bounded_identifier(&record.context_key, 512)
        || !bounded_identifier(record.operation.id.as_str(), 128)
        || record.operation.revision == 0
        || !checked_limits(&record.limits)
        || record.issued_at_unix_seconds < 0
        || record.expires_at_unix_seconds <= record.issued_at_unix_seconds
        || record.expires_at_unix_seconds - record.issued_at_unix_seconds
            > ONE_STEP_REQUEST_GRANT_MAX_LIFETIME_SECONDS_V1
    {
        return Err(OneStepRequestGrantErrorV1::InvalidIssuerRecord);
    }
    if record.request_id != locator.request_id {
        return Err(OneStepRequestGrantErrorV1::InvalidLocator);
    }
    if record.revoked_at_unix_seconds.is_some() {
        return Err(OneStepRequestGrantErrorV1::Revoked);
    }
    if record.state != OneStepRequestIssuanceStateV1::Unclaimed {
        return Err(OneStepRequestGrantErrorV1::AlreadyClaimedOrUnknown);
    }
    if now_unix_seconds < record.issued_at_unix_seconds {
        return Err(OneStepRequestGrantErrorV1::NotYetValid);
    }
    if now_unix_seconds >= record.expires_at_unix_seconds {
        return Err(OneStepRequestGrantErrorV1::Expired);
    }
    if !bounded_identifier(&independently_authenticated_policy.tenant_id, 128)
        || !bounded_identifier(&independently_authenticated_policy.principal_id, 128)
        || !bounded_identifier(&independently_authenticated_policy.policy_id, 128)
        || !bounded_identifier(&independently_authenticated_policy.context_key, 512)
        || record.tenant_id != independently_authenticated_policy.tenant_id
        || record.authenticated_principal_id != independently_authenticated_policy.principal_id
        || record.authorization_policy_id != independently_authenticated_policy.policy_id
        || record.context_key != independently_authenticated_policy.context_key
    {
        return Err(OneStepRequestGrantErrorV1::UnauthorizedPrincipal);
    }
    if !checked_limits(&independently_authenticated_policy.maximum_limits)
        || record.limits.currency_code
            != independently_authenticated_policy
                .maximum_limits
                .currency_code
        || record.limits.max_admitted_source_bytes
            > independently_authenticated_policy
                .maximum_limits
                .max_admitted_source_bytes
        || record.limits.max_effect_dispatches
            > independently_authenticated_policy
                .maximum_limits
                .max_effect_dispatches
        || record.limits.max_provider_requests
            > independently_authenticated_policy
                .maximum_limits
                .max_provider_requests
        || record.limits.max_charge_minor_units
            > independently_authenticated_policy
                .maximum_limits
                .max_charge_minor_units
    {
        return Err(OneStepRequestGrantErrorV1::BudgetExceeded);
    }
    if run.plan.scope.tenant_id != record.tenant_id
        || run.plan.scope.run_id != record.canonical_run_id
        || run.plan.nodes.len() != 1
        || run.plan.nodes[0].operation != record.operation
        || record.parent_workflow_id
            != operation_dag_parent_workflow_id_v2(&record.canonical_run_id)
    {
        return Err(OneStepRequestGrantErrorV1::ScopeMismatch);
    }
    let admitted_bytes = run.admitted.values().try_fold(0_usize, |sum, artifact| {
        sum.checked_add(artifact.payload.len())
    });
    if admitted_bytes.is_none_or(|sum| sum > record.limits.max_admitted_source_bytes as usize)
        || run.plan.nodes[0].permitted_effects.len() > record.limits.max_effect_dispatches as usize
        || run.plan.allowed_effects.len() > record.limits.max_effect_dispatches as usize
    {
        return Err(OneStepRequestGrantErrorV1::BudgetExceeded);
    }
    preflight_unadmitted_single_operation_v2_start(run, installed_routes, &record.expected)
        .map_err(OneStepRequestGrantErrorV1::SealedPlanMismatch)?;
    Ok(OneStepRequestReservationPreflightV1 {
        request_id: record.request_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::decode_opaque_one_step_request_locator_v1;
    use crate::{
        owner_content_hash, AdmittedArtifactV1, ArtifactBinding, ArtifactId, ArtifactSchema,
        ArtifactTypeId, EffectId, ExecutionPlan, ExternalArtifact, IdempotencyPolicy,
        OperationContract, OperationId, OperationInstanceId, OperationNode, OperationOwnerId,
        OwnerCommandWireV1, PinnedOwnerRouteV1, PlanHash, PortId, RunScope,
        OPERATION_DAG_RUN_SCHEMA_REVISION_V6, OWNER_OPERATION_CHILD_TYPE_V1, PLAN_SCHEMA_REVISION,
        SITE_ARCHITECT_START_PREFLIGHT_SCHEMA_REVISION_V1,
    };

    const NOW: i64 = 1_900_000_100;
    const REQUEST_ID: &str = "e0813cac-2277-454e-980a-2e08110c1a08";
    const RUN_ID: &str = "befd8f0c-8f83-4b29-8c4b-9c5a7694a328";

    fn fixture() -> (
        OpaqueOneStepRequestLocatorV1,
        TrustedOneStepRequestGrantV1,
        OperationDagRunV6,
        TrustedRouteCatalog,
        IndependentlyAuthenticatedOneStepPolicyV1,
    ) {
        let scope = RunScope {
            tenant_id: "tenant-a".into(),
            run_id: RUN_ID.into(),
        };
        let operation = OperationRef {
            id: OperationId::from("acquisition.classify_source_page"),
            revision: 1,
        };
        let schema = ArtifactSchema {
            id: ArtifactTypeId::from("acquisition.source_page.v1"),
            revision: 1,
        };
        let effects = BTreeSet::from([EffectId::from("acquisition.sample.effect")]);
        let contract = OperationContract {
            reference: operation.clone(),
            owner: OperationOwnerId::from("acquisition"),
            inputs: BTreeMap::from([(PortId::from("input"), schema.clone())]),
            outputs: BTreeMap::from([(
                PortId::from("output"),
                ArtifactSchema {
                    id: ArtifactTypeId::from("acquisition.classified_page.v1"),
                    revision: 1,
                },
            )]),
            invalidates: BTreeSet::new(),
            effects: effects.clone(),
            idempotency: IdempotencyPolicy::AtMostOnce,
        };
        let contract_hash = contract.fingerprint().unwrap();
        let mut routes = TrustedRouteCatalog::default();
        routes
            .register(
                contract.clone(),
                PinnedOwnerRouteV1 {
                    operation: operation.clone(),
                    owner: contract.owner.clone(),
                    contract_hash: contract_hash.clone(),
                    workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
                    task_queue: "acquisition-native".into(),
                    build_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                },
            )
            .unwrap();
        let source = ArtifactId::from("source");
        let instance = OperationInstanceId::from("single-step");
        let plan = ExecutionPlan {
            schema_revision: PLAN_SCHEMA_REVISION,
            scope: scope.clone(),
            allowed_effects: effects.clone(),
            external_artifacts: BTreeMap::from([(
                source.clone(),
                ExternalArtifact {
                    schema: schema.clone(),
                    scope: scope.clone(),
                    provenance: "source-record-v1:0e2dddf2-0f45-46ed-89dd-3dce7e8b247d".into(),
                },
            )]),
            pinned_contracts: Vec::new(),
            nodes: vec![OperationNode {
                instance: instance.clone(),
                operation: operation.clone(),
                inputs: BTreeMap::from([(
                    PortId::from("input"),
                    ArtifactBinding::External {
                        artifact: source.clone(),
                    },
                )]),
                outputs: BTreeMap::from([(PortId::from("output"), ArtifactId::from("classified"))]),
                after: BTreeSet::new(),
                permitted_effects: effects,
            }],
            hash: PlanHash::from("unsealed"),
        }
        .seal(routes.catalog())
        .unwrap();
        let source_bytes = br#"{"title":"source","raw_text":"pinned"}"#.to_vec();
        let command_bytes = br#"{"mode":"strict"}"#.to_vec();
        let run = OperationDagRunV6 {
            schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
            plan,
            admitted: BTreeMap::from([(
                source.clone(),
                AdmittedArtifactV1 {
                    schema,
                    scope,
                    provenance: "source-record-v1:0e2dddf2-0f45-46ed-89dd-3dce7e8b247d".into(),
                    payload: source_bytes.clone(),
                },
            )]),
            owner_commands: BTreeMap::from([(
                instance.clone(),
                OwnerCommandWireV1 {
                    operation: operation.clone(),
                    owner: contract.owner.clone(),
                    contract_hash,
                    command_bytes: command_bytes.clone(),
                },
            )]),
            pinned_routes: routes.trusted_snapshot_for([&operation]).unwrap(),
        };
        let expected = SiteArchitectExpectedStartV1 {
            schema_revision: SITE_ARCHITECT_START_PREFLIGHT_SCHEMA_REVISION_V1,
            plan_hash: run.plan.hash.clone(),
            command_hashes: BTreeMap::from([(instance, owner_content_hash(&command_bytes))]),
            source_payload_hashes: BTreeMap::from([(source, owner_content_hash(&source_bytes))]),
        };
        let limits = OneStepResourceLimitsV1 {
            max_admitted_source_bytes: 1024,
            max_effect_dispatches: 1,
            max_provider_requests: 1,
            currency_code: "USD".into(),
            max_charge_minor_units: 200,
        };
        let grant = TrustedOneStepRequestGrantV1 {
            schema_revision: ONE_STEP_REQUEST_GRANT_SCHEMA_REVISION_V1,
            request_id: REQUEST_ID.into(),
            authorization_event_id: "0e2dddf2-0f45-46ed-89dd-3dce7e8b247d".into(),
            tenant_id: "tenant-a".into(),
            authenticated_principal_id: "principal-7".into(),
            authorization_policy_id: "operator-approved-v1".into(),
            context_key: "site-a/research".into(),
            canonical_run_id: RUN_ID.into(),
            operation,
            parent_workflow_id: operation_dag_parent_workflow_id_v2(RUN_ID),
            expected,
            limits: limits.clone(),
            issued_at_unix_seconds: NOW - 10,
            expires_at_unix_seconds: NOW + 90,
            revoked_at_unix_seconds: None,
            state: OneStepRequestIssuanceStateV1::Unclaimed,
        };
        let policy = IndependentlyAuthenticatedOneStepPolicyV1 {
            tenant_id: "tenant-a".into(),
            principal_id: "principal-7".into(),
            policy_id: "operator-approved-v1".into(),
            context_key: "site-a/research".into(),
            maximum_limits: limits,
        };
        (
            OpaqueOneStepRequestLocatorV1 {
                request_id: REQUEST_ID.into(),
            },
            grant,
            run,
            routes,
            policy,
        )
    }

    #[test]
    fn locator_has_only_one_canonical_field_and_never_carries_authority() {
        let bytes = format!(r#"{{"request_id":"{REQUEST_ID}"}}"#);
        assert_eq!(
            decode_opaque_one_step_request_locator_v1(bytes.as_bytes())
                .unwrap()
                .request_id,
            REQUEST_ID
        );
        for bad in [
            format!(r#"{{"request_id":"{}"}}"#, REQUEST_ID.to_uppercase()),
            format!(r#"{{"request_id":"{REQUEST_ID}","tenant_id":"tenant-a"}}"#),
            format!(r#"{{"request_id":"{REQUEST_ID}","request_id":"{REQUEST_ID}"}}"#),
            format!(" {bytes}"),
            format!("{bytes}\n"),
            r#"{"request_id":"00000000-0000-0000-0000-000000000000"}"#.into(),
            r#"{"request_id":"../tmp/intent.json"}"#.into(),
        ] {
            assert!(
                matches!(
                    decode_opaque_one_step_request_locator_v1(bad.as_bytes()),
                    Err(OneStepRequestGrantErrorV1::InvalidLocator)
                ),
                "unexpected accepted locator: {bad}"
            );
        }
    }

    #[test]
    fn issuer_grant_is_only_ready_for_atomic_one_use_reservation() {
        let (locator, grant, run, routes, policy) = fixture();
        let proof = preflight_issued_one_step_request_for_reservation_v1(
            &locator, &grant, &run, &routes, &policy, NOW,
        )
        .unwrap();
        assert_eq!(proof.request_id(), REQUEST_ID);
    }

    #[test]
    fn subject_tenant_context_acl_and_operation_must_all_match() {
        let (locator, grant, run, routes, policy) = fixture();
        for other_policy in [
            {
                let mut p = policy.clone();
                p.principal_id = "other".into();
                p
            },
            {
                let mut p = policy.clone();
                p.tenant_id = "tenant-b".into();
                p
            },
            {
                let mut p = policy.clone();
                p.policy_id = "unapproved".into();
                p
            },
            {
                let mut p = policy.clone();
                p.context_key = "other".into();
                p
            },
        ] {
            assert!(matches!(
                preflight_issued_one_step_request_for_reservation_v1(
                    &locator,
                    &grant,
                    &run,
                    &routes,
                    &other_policy,
                    NOW
                ),
                Err(OneStepRequestGrantErrorV1::UnauthorizedPrincipal)
            ));
        }
        let mut cross_run = grant.clone();
        cross_run.canonical_run_id = "2478e012-c4c9-472e-912a-6551112a0c55".into();
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator, &cross_run, &run, &routes, &policy, NOW
            ),
            Err(OneStepRequestGrantErrorV1::ScopeMismatch)
        ));
        let mut other_operation = grant.clone();
        other_operation.operation.id = OperationId::from("arbitrary.remote.execute");
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &other_operation,
                &run,
                &routes,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::ScopeMismatch)
        ));
        let mut empty_actor = grant.clone();
        empty_actor.authenticated_principal_id.clear();
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &empty_actor,
                &run,
                &routes,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::InvalidIssuerRecord)
        ));
        let mut control_bearing_context = grant.clone();
        control_bearing_context.context_key.push('\n');
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &control_bearing_context,
                &run,
                &routes,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::InvalidIssuerRecord)
        ));
    }

    #[test]
    fn expired_revoked_consumed_and_ambiguous_requests_fail_closed() {
        let (locator, mut grant, run, routes, policy) = fixture();
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &grant,
                &run,
                &routes,
                &policy,
                NOW - 100
            ),
            Err(OneStepRequestGrantErrorV1::NotYetValid)
        ));
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &grant,
                &run,
                &routes,
                &policy,
                NOW + 90
            ),
            Err(OneStepRequestGrantErrorV1::Expired)
        ));
        grant.revoked_at_unix_seconds = Some(NOW);
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator, &grant, &run, &routes, &policy, NOW
            ),
            Err(OneStepRequestGrantErrorV1::Revoked)
        ));
        grant.revoked_at_unix_seconds = None;
        for state in [
            OneStepRequestIssuanceStateV1::Reserved,
            OneStepRequestIssuanceStateV1::StartSubmitted,
            OneStepRequestIssuanceStateV1::StartOutcomeUnknown,
            OneStepRequestIssuanceStateV1::StartConfirmed,
        ] {
            grant.state = state;
            assert!(matches!(
                preflight_issued_one_step_request_for_reservation_v1(
                    &locator, &grant, &run, &routes, &policy, NOW
                ),
                Err(OneStepRequestGrantErrorV1::AlreadyClaimedOrUnknown)
            ));
        }
        grant.state = OneStepRequestIssuanceStateV1::Unclaimed;
        grant.expires_at_unix_seconds = NOW + 700;
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator, &grant, &run, &routes, &policy, NOW
            ),
            Err(OneStepRequestGrantErrorV1::InvalidIssuerRecord)
        ));
    }

    #[test]
    fn command_source_route_and_budget_tampering_refused() {
        let (locator, grant, run, routes, policy) = fixture();
        let mut changed_command = run.clone();
        changed_command
            .owner_commands
            .values_mut()
            .next()
            .unwrap()
            .command_bytes
            .push(b'x');
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &grant,
                &changed_command,
                &routes,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::SealedPlanMismatch(_))
        ));
        let mut changed_source = run.clone();
        changed_source
            .admitted
            .values_mut()
            .next()
            .unwrap()
            .payload
            .push(b'x');
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &grant,
                &changed_source,
                &routes,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::SealedPlanMismatch(_))
        ));
        let mut changed_route = run.clone();
        changed_route.pinned_routes[0].task_queue = "attacker".into();
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &grant,
                &changed_route,
                &routes,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::SealedPlanMismatch(_))
        ));
        let mut over_budget = grant.clone();
        over_budget.limits.max_charge_minor_units += 1;
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &over_budget,
                &run,
                &routes,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::BudgetExceeded)
        ));
        let mut insufficient_source_budget = grant.clone();
        insufficient_source_budget.limits.max_admitted_source_bytes = 1;
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &insufficient_source_budget,
                &run,
                &routes,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::BudgetExceeded)
        ));
        let mut forged_currency = grant.clone();
        forged_currency.limits.currency_code = "usd".into();
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &forged_currency,
                &run,
                &routes,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::InvalidIssuerRecord)
        ));

        let mut oversized_effect_envelope = run.clone();
        oversized_effect_envelope
            .plan
            .allowed_effects
            .insert(EffectId::from("acquisition.extra.unused.effect"));
        oversized_effect_envelope.plan = oversized_effect_envelope
            .plan
            .seal(routes.catalog())
            .expect("a valid plan can include a second allowed but unused effect");
        let mut independently_pinned_grant = grant.clone();
        independently_pinned_grant.expected.plan_hash = oversized_effect_envelope.plan.hash.clone();
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &independently_pinned_grant,
                &oversized_effect_envelope,
                &routes,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::BudgetExceeded)
        ));
    }

    #[test]
    fn request_replay_and_self_consistent_untrusted_hashes_cannot_steal_grant() {
        let (locator, grant, run, routes, policy) = fixture();
        let other_locator = OpaqueOneStepRequestLocatorV1 {
            request_id: "0e2dddf2-0f45-46ed-89dd-3dce7e8b247d".into(),
        };
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &other_locator,
                &grant,
                &run,
                &routes,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::InvalidLocator)
        ));

        let mut forged_grant = grant.clone();
        forged_grant
            .expected
            .command_hashes
            .values_mut()
            .next()
            .unwrap()
            .replace_range(.., &owner_content_hash(b"attacker-controlled-command"));
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &forged_grant,
                &run,
                &routes,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::SealedPlanMismatch(_))
        ));

        let mut two_steps = run.clone();
        two_steps.plan.nodes.push(two_steps.plan.nodes[0].clone());
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator, &grant, &two_steps, &routes, &policy, NOW
            ),
            Err(OneStepRequestGrantErrorV1::ScopeMismatch)
        ));
    }

    #[test]
    fn unregistered_routes_never_authorize() {
        let (locator, grant, run, _routes, policy) = fixture();
        let unknown_catalog = TrustedRouteCatalog::default();
        assert!(matches!(
            preflight_issued_one_step_request_for_reservation_v1(
                &locator,
                &grant,
                &run,
                &unknown_catalog,
                &policy,
                NOW
            ),
            Err(OneStepRequestGrantErrorV1::SealedPlanMismatch(_))
        ));
    }
}
