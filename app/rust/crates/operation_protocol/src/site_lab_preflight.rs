//! Read-only Site Lab preflight for an immutable V2 Temporal DAG start.
//!
//! This checks that the operator's *previously captured* command and source
//! digests still match the exact bytes in a proposed run. It intentionally delegates
//! all plan, route, artifact and contract validation to the existing V2 DAG
//! machine; it does not issue a business run, authenticate source bytes,
//! verify any owner's concrete DTO, or authorize a Temporal start/effect.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::owner_dispatch::is_canonical_blake3_hash;
use crate::{
    operation_dag_parent_workflow_id_v2, owner_content_hash, ArtifactId, GenericDagError,
    OperationDagMachine, OperationDagRunV6, OperationInstanceId, PlanHash, TrustedRouteCatalog,
};

pub const SITE_LAB_START_PREFLIGHT_SCHEMA_REVISION_V1: u16 = 1;

/// Digests captured *before* composing a Temporal Start, by the trusted
/// operation owner's command codec. The user cannot forge owner validation
/// or execution permission merely by supplying these untrusted values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteLabExpectedStartV1 {
    pub schema_revision: u16,
    pub plan_hash: PlanHash,
    pub command_hashes: BTreeMap<OperationInstanceId, String>,
    /// Exact source *payload bytes* captured before Start; not source Truth
    /// authentication or a substitute for the PG/source admission issuer.
    pub source_payload_hashes: BTreeMap<ArtifactId, String>,
}

/// Machine-readable, non-authorizing result. Does not expose command bytes,
/// credentials, provider payloads, or the actual source content.
#[derive(Debug, Clone, Serialize)]
pub struct SiteLabUnadmittedPreflightV1 {
    pub status: &'static str,
    pub plan_hash: PlanHash,
    pub command_hashes: BTreeMap<OperationInstanceId, String>,
    pub source_payload_hashes: BTreeMap<ArtifactId, String>,
    pub steps_requiring_owner_validate_only: Vec<OperationInstanceId>,
    pub source_authentication: &'static str,
    pub owner_admission: &'static str,
    pub execution: &'static str,
}

#[derive(Debug, Error)]
pub enum SiteLabStartPreflightError {
    #[error("V2 Site Lab expected command set or plan hash is invalid")]
    InvalidExpectation,
    #[error("V2 Site Lab command digest differs from checked run for {0:?}")]
    CommandDigestMismatch(OperationInstanceId),
    #[error("V2 Site Lab source payload digest differs from checked run for {0:?}")]
    SourceDigestMismatch(ArtifactId),
    #[error("V2 Site Lab run was rejected by its existing native DAG validator")]
    InvalidRun(#[from] GenericDagError),
}

/// Fail closed if any command/source payload bytes, source declaration, owner route, operation
/// contract, plan pin or expected digest changed between typed authoring and
/// the proposed V2 Start. The existing machine still requires every owner's
/// independent ValidateOnly Child *after* the actual Start before effects.
pub fn preflight_unadmitted_site_lab_v2_start(
    run: &OperationDagRunV6,
    installed_routes: &TrustedRouteCatalog,
    expected: &SiteLabExpectedStartV1,
) -> Result<SiteLabUnadmittedPreflightV1, SiteLabStartPreflightError> {
    let node_ids = run
        .plan
        .nodes
        .iter()
        .map(|node| node.instance.clone())
        .collect::<BTreeSet<_>>();
    if expected.schema_revision != SITE_LAB_START_PREFLIGHT_SCHEMA_REVISION_V1
        || expected.plan_hash != run.plan.hash
        || node_ids.len() != run.plan.nodes.len()
        || expected.command_hashes.keys().collect::<BTreeSet<_>>()
            != node_ids.iter().collect::<BTreeSet<_>>()
        || expected.source_payload_hashes.keys().collect::<BTreeSet<_>>()
            != run.plan.external_artifacts.keys().collect::<BTreeSet<_>>()
        || expected
            .command_hashes
            .values()
            .any(|digest| !is_canonical_blake3_hash(digest))
        || expected
            .source_payload_hashes
            .values()
            .any(|digest| !is_canonical_blake3_hash(digest))
    {
        return Err(SiteLabStartPreflightError::InvalidExpectation);
    }

    // Reuse the one authoritative DAG V2 Start preflight for all native
    // contract, route, input and command checks. This prepares no effect and
    // must not be construed as owner ValidateOnly or operator admission.
    let workflow_id = operation_dag_parent_workflow_id_v2(&run.plan.scope.run_id);
    OperationDagMachine::new_v2_with_owner_validation(
        &workflow_id,
        run.clone(),
        installed_routes.clone(),
    )?;

    for (instance, expected_digest) in &expected.command_hashes {
        let actual = run
            .owner_commands
            .get(instance)
            .ok_or(SiteLabStartPreflightError::InvalidExpectation)?;
        if owner_content_hash(&actual.command_bytes) != *expected_digest {
            return Err(SiteLabStartPreflightError::CommandDigestMismatch(
                instance.clone(),
            ));
        }
    }

    for (artifact_id, expected_digest) in &expected.source_payload_hashes {
        let source = run
            .admitted
            .get(artifact_id)
            .ok_or(SiteLabStartPreflightError::InvalidExpectation)?;
        if owner_content_hash(&source.payload) != *expected_digest {
            return Err(SiteLabStartPreflightError::SourceDigestMismatch(
                artifact_id.clone(),
            ));
        }
    }

    Ok(SiteLabUnadmittedPreflightV1 {
        status: "preflight_only_not_admitted",
        plan_hash: run.plan.hash.clone(),
        command_hashes: expected.command_hashes.clone(),
        source_payload_hashes: expected.source_payload_hashes.clone(),
        steps_requiring_owner_validate_only: node_ids.into_iter().collect(),
        source_authentication: "not_checked",
        owner_admission: "not_issued",
        execution: "not_started",
    })
}
