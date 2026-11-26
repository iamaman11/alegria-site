//! Provider-owned typed contract for one Google Organic Standard paid SERP task.
//!
//! This crate deliberately contains no HTTP client, credential, database,
//! Temporal SDK, worker registration or effect dispatcher. A parameter check
//! does NOT authorize even one DataForSEO request: the independent owner must
//! first validate its concrete command, tenant, cost budget and durable task
//! claim against real Temporal History and operator admission.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    ArtifactSchema, ArtifactTypeId, EffectId, IdempotencyPolicy, Operation, OperationContract,
    OperationId, OperationOwnerId, OperationRef, PortId, RunScope,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const OPERATION_ID: &str = "dataforseo.google_organic.search";
pub const OPERATION_REVISION: u32 = 1;
pub const OWNER_ID: &str = "dataforseo";
pub const INPUT_PORT: &str = "query";
pub const OUTPUT_PORT: &str = "organic_serp";
pub const INPUT_ARTIFACT_TYPE: &str = "dataforseo.google_organic.query";
pub const OUTPUT_ARTIFACT_TYPE: &str = "dataforseo.google_organic.discovery";
pub const PAID_EFFECT_ID: &str = "dataforseo.google_organic.paid_task_post";
pub const SCHEMA_REVISION: u16 = 1;

/// A deliberately low per-task ceiling until a separate operator-authorized
/// project budget exists. This is only an INPUT LIMIT, never a price estimate
/// or evidence that the provider will charge less than this amount.
pub const MAX_REQUEST_COST_USD_MICROS: u64 = 10_000_000;
pub const MAX_DISCOVERY_OUTPUT_BYTES: usize = 512 * 1024;

/// Concrete, versioned provider parameters. No credentials, endpoint URL,
/// Task Queue or authority fields can be supplied from the recipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataForSeoGoogleOrganicSearchInputV1 {
    pub schema_revision: u16,
    pub scope: RunScope,
    pub keyword: String,
    pub language_code: String,
    pub location_code: i64,
    pub depth: u32,
    pub max_cost_usd_micros: u64,
}

/// A discovery result is NOT a verified claim about a website or consumer.
/// Actual page observations must originate from separately authenticated
/// fetches, not SERP titles and snippets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataForSeoSourceStatusV1 {
    SerpDiscoveryOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataForSeoOrganicHitV1 {
    pub rank: u32,
    pub title: String,
    pub source_url: String,
    pub domain_norm: String,
    pub snippet: String,
}

/// Result shape for a future independently admitted provider owner. A
/// provider response hash is not proof of source/owner authenticity by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataForSeoGoogleOrganicSearchOutputV1 {
    pub schema_revision: u16,
    pub source_status: DataForSeoSourceStatusV1,
    pub provider_task_id: String,
    pub provider_response_hash: String,
    pub input_hash: String,
    pub provider_reported_cost_usd_micros: Option<u64>,
    pub organic_results: Vec<DataForSeoOrganicHitV1>,
}

#[derive(Debug, Error)]
pub enum DataForSeoContractError {
    #[error("unsupported DataForSEO operation input revision")]
    Revision,
    #[error("DataForSEO input run or tenant does not match the expected run scope")]
    Scope,
    #[error("DataForSEO keyword is empty, malformed or too long")]
    Keyword,
    #[error("DataForSEO language code is invalid")]
    Language,
    #[error("DataForSEO location code must be positive")]
    Location,
    #[error("DataForSEO depth is outside the supported 1..=100 range")]
    Depth,
    #[error("DataForSEO maximum cost is zero or exceeds the per-task safety ceiling")]
    CostCap,
    #[error("DataForSEO typed parameter encoding failed")]
    Encoding(#[source] serde_json::Error),
    #[error("DataForSEO provider discovery result has an invalid shape or exceeds bounds")]
    InvalidDiscovery,
}

/// No accepted provider spend or owner ValidateOnly Child is implied.
/// A caller-supplied `expected_scope` is NOT an authenticated tenant token:
/// the future owner must independently derive that scope from exact History.
#[derive(Debug, Clone, Serialize)]
pub struct DataForSeoParameterPreflightV1 {
    pub status: &'static str,
    pub parameter_hash: String,
    pub declared_max_cost_usd_micros: u64,
    pub owner_admission: &'static str,
    pub source_authentication: &'static str,
    pub execution: &'static str,
}

pub struct DataForSeoGoogleOrganicSearchV1;

impl Operation for DataForSeoGoogleOrganicSearchV1 {
    type Input = DataForSeoGoogleOrganicSearchInputV1;
    type Output = DataForSeoGoogleOrganicSearchOutputV1;
    type Failure = DataForSeoContractError;

    fn contract() -> OperationContract {
        OperationContract {
            reference: OperationRef {
                id: OperationId(OPERATION_ID.to_owned()),
                revision: OPERATION_REVISION,
            },
            owner: OperationOwnerId(OWNER_ID.to_owned()),
            inputs: BTreeMap::from([(
                PortId(INPUT_PORT.to_owned()),
                ArtifactSchema {
                    id: ArtifactTypeId(INPUT_ARTIFACT_TYPE.to_owned()),
                    revision: u32::from(SCHEMA_REVISION),
                },
            )]),
            outputs: BTreeMap::from([(
                PortId(OUTPUT_PORT.to_owned()),
                ArtifactSchema {
                    id: ArtifactTypeId(OUTPUT_ARTIFACT_TYPE.to_owned()),
                    revision: u32::from(SCHEMA_REVISION),
                },
            )]),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::from([EffectId(PAID_EFFECT_ID.to_owned())]),
            // Until a V2 tenant- and History-bound durable paid-task fence
            // exists, an ambiguous response must NEVER trigger an auto-retry.
            idempotency: IdempotencyPolicy::AtMostOnce,
        }
    }
}

/// Reusable pure DTO preflight for the future owner, not owner admission.
/// Returns only a digest and explicit pending state, never query text.
pub fn validate_parameters_only(
    input: &DataForSeoGoogleOrganicSearchInputV1,
    expected_scope: &RunScope,
) -> Result<DataForSeoParameterPreflightV1, DataForSeoContractError> {
    if input.schema_revision != SCHEMA_REVISION {
        return Err(DataForSeoContractError::Revision);
    }
    let run =
        uuid::Uuid::parse_str(&input.scope.run_id).map_err(|_| DataForSeoContractError::Scope)?;
    if &input.scope != expected_scope
        || input.scope.run_id != run.to_string()
        || input.scope.tenant_id.is_empty()
        || input.scope.tenant_id.len() > 256
        || input.scope.tenant_id.trim() != input.scope.tenant_id
        || input.scope.tenant_id.chars().any(char::is_control)
    {
        return Err(DataForSeoContractError::Scope);
    }
    if input.keyword.is_empty()
        || input.keyword.len() > 2_048
        || input.keyword.trim() != input.keyword
        || input.keyword.chars().any(char::is_control)
    {
        return Err(DataForSeoContractError::Keyword);
    }
    if !(2..=8).contains(&input.language_code.len())
        || input.language_code.starts_with('-')
        || input.language_code.ends_with('-')
        || input
            .language_code
            .bytes()
            .filter(|byte| *byte == b'-')
            .count()
            > 1
        || !input
            .language_code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
    {
        return Err(DataForSeoContractError::Language);
    }
    if input.location_code <= 0 {
        return Err(DataForSeoContractError::Location);
    }
    if !(1..=100).contains(&input.depth) {
        return Err(DataForSeoContractError::Depth);
    }
    if !(1..=MAX_REQUEST_COST_USD_MICROS).contains(&input.max_cost_usd_micros) {
        return Err(DataForSeoContractError::CostCap);
    }
    let parameter_bytes = serde_json::to_vec(input).map_err(DataForSeoContractError::Encoding)?;
    Ok(DataForSeoParameterPreflightV1 {
        status: "pending_owner_admission",
        parameter_hash: blake3::hash(&parameter_bytes).to_hex().to_string(),
        declared_max_cost_usd_micros: input.max_cost_usd_micros,
        owner_admission: "not_issued",
        source_authentication: "not_checked",
        execution: "not_started",
    })
}

/// Check the bounded shape of a future owner result, without pretending to
/// authenticate the provider task, webpage contents or real charged amount.
/// Source URLs discovered here are NEVER authority for a crawler to fetch.
pub fn validate_discovery_shape_only(
    result: &DataForSeoGoogleOrganicSearchOutputV1,
    request: &DataForSeoGoogleOrganicSearchInputV1,
) -> Result<(), DataForSeoContractError> {
    let expected_hash =
        blake3::hash(&serde_json::to_vec(request).map_err(DataForSeoContractError::Encoding)?)
            .to_hex()
            .to_string();
    let canonical_hash = |hash: &str| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    let task_id = &result.provider_task_id;
    if result.schema_revision != SCHEMA_REVISION
        || result.input_hash != expected_hash
        || !canonical_hash(&result.provider_response_hash)
        || task_id.is_empty()
        || task_id.len() > 128
        || !task_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || result.organic_results.len() > usize::try_from(request.depth).unwrap_or(0)
        || result
            .provider_reported_cost_usd_micros
            .is_some_and(|amount| amount > request.max_cost_usd_micros)
        || result.organic_results.iter().any(|hit| {
            hit.rank == 0
                || hit.rank > request.depth
                || hit.title.len() > 4_096
                || hit.snippet.len() > 8_192
                || hit.source_url.len() > 2_048
                || hit.domain_norm.len() > 256
                || !hit.source_url.starts_with("https://")
                || hit.domain_norm.is_empty()
        })
        || serde_json::to_vec(result)
            .map_err(DataForSeoContractError::Encoding)?
            .len()
            > MAX_DISCOVERY_OUTPUT_BYTES
    {
        return Err(DataForSeoContractError::InvalidDiscovery);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_input() -> DataForSeoGoogleOrganicSearchInputV1 {
        DataForSeoGoogleOrganicSearchInputV1 {
            schema_revision: SCHEMA_REVISION,
            scope: RunScope {
                tenant_id: "tenant-a".into(),
                run_id: "123e4567-e89b-42d3-a456-426614174000".into(),
            },
            keyword: "kitchen installation customer problems".into(),
            language_code: "en".into(),
            location_code: 2840,
            depth: 10,
            max_cost_usd_micros: 100_000,
        }
    }

    #[test]
    fn paid_operation_contract_is_versioned_and_not_retryable_by_default() {
        let contract = DataForSeoGoogleOrganicSearchV1::contract();
        assert_eq!(contract.reference.id.as_str(), OPERATION_ID);
        assert_eq!(contract.reference.revision, 1);
        assert_eq!(contract.owner.as_str(), OWNER_ID);
        assert_eq!(contract.idempotency, IdempotencyPolicy::AtMostOnce);
        assert_eq!(
            contract.effects,
            BTreeSet::from([EffectId(PAID_EFFECT_ID.into())])
        );
        assert_eq!(contract.inputs.len(), 1);
        assert_eq!(contract.outputs.len(), 1);
        assert_eq!(contract.inputs[&PortId(INPUT_PORT.into())].revision, 1);
        assert_eq!(contract.outputs[&PortId(OUTPUT_PORT.into())].revision, 1);
        assert_eq!(contract.fingerprint().unwrap().as_str().len(), 64);
    }

    #[test]
    fn typed_parameters_have_pending_status_and_no_source_or_execution_claim() {
        let input = valid_input();
        let checked = validate_parameters_only(&input, &input.scope).unwrap();
        assert_eq!(checked.status, "pending_owner_admission");
        assert_eq!(checked.owner_admission, "not_issued");
        assert_eq!(checked.source_authentication, "not_checked");
        assert_eq!(checked.execution, "not_started");
        assert_eq!(checked.parameter_hash.len(), 64);
        let same = validate_parameters_only(&input, &input.scope).unwrap();
        assert_eq!(checked.parameter_hash, same.parameter_hash);
        let mut changed = input.clone();
        changed.max_cost_usd_micros += 1;
        assert_ne!(
            checked.parameter_hash,
            validate_parameters_only(&changed, &input.scope)
                .unwrap()
                .parameter_hash
        );
        // The preview is intentionally safe to show; the query is not.
        let rendered = serde_json::to_string(&checked).unwrap();
        assert!(!rendered.contains("kitchen installation"));
    }

    #[test]
    fn rejects_wrong_tenant_run_revision_keyword_locale_and_paid_budget() {
        let baseline = valid_input();
        let scope = baseline.scope.clone();
        let check = |change: fn(&mut DataForSeoGoogleOrganicSearchInputV1)| {
            let mut input = baseline.clone();
            change(&mut input);
            assert!(validate_parameters_only(&input, &scope).is_err());
        };
        check(|input| input.schema_revision = 2);
        check(|input| input.scope.tenant_id = "other-tenant".into());
        check(|input| input.scope.tenant_id = " tenant-a".into());
        check(|input| input.scope.run_id = "arbitrary-client-string".into());
        check(|input| input.keyword.clear());
        check(|input| input.keyword = "malformed\nkeyword".into());
        check(|input| input.keyword = " leading-space".into());
        check(|input| input.keyword = "x".repeat(2_049));
        check(|input| input.language_code = "EN".into());
        check(|input| input.language_code = "en_uk".into());
        check(|input| input.location_code = -1);
        check(|input| input.depth = 0);
        check(|input| input.depth = 101);
        check(|input| input.max_cost_usd_micros = 0);
        check(|input| input.max_cost_usd_micros = MAX_REQUEST_COST_USD_MICROS + 1);
    }

    #[test]
    fn discovery_result_is_bounded_and_never_promoted_to_verified_page_fact() {
        let input = valid_input();
        let input_hash = validate_parameters_only(&input, &input.scope)
            .unwrap()
            .parameter_hash;
        let valid = DataForSeoGoogleOrganicSearchOutputV1 {
            schema_revision: SCHEMA_REVISION,
            source_status: DataForSeoSourceStatusV1::SerpDiscoveryOnly,
            provider_task_id: "provider-task-1".into(),
            provider_response_hash: "a".repeat(64),
            input_hash,
            provider_reported_cost_usd_micros: Some(50_000),
            organic_results: vec![DataForSeoOrganicHitV1 {
                rank: 1,
                title: "Sample customer review".into(),
                source_url: "https://example.com/reviews/1".into(),
                domain_norm: "example.com".into(),
                snippet: "A search snippet, not a verified consumer observation".into(),
            }],
        };
        validate_discovery_shape_only(&valid, &input).unwrap();
        let mut forged = valid.clone();
        forged.provider_response_hash = "unverified".into();
        assert!(validate_discovery_shape_only(&forged, &input).is_err());
        forged = valid.clone();
        forged.organic_results[0].rank = 0;
        assert!(validate_discovery_shape_only(&forged, &input).is_err());
        forged = valid.clone();
        forged.organic_results[0].source_url = "http://127.0.0.1/".into();
        assert!(validate_discovery_shape_only(&forged, &input).is_err());
        forged = valid.clone();
        forged.provider_reported_cost_usd_micros = Some(input.max_cost_usd_micros + 1);
        assert!(validate_discovery_shape_only(&forged, &input).is_err());
        forged = valid;
        forged.organic_results[0].snippet = "x".repeat(MAX_DISCOVERY_OUTPUT_BYTES);
        assert!(validate_discovery_shape_only(&forged, &input).is_err());
    }
}
