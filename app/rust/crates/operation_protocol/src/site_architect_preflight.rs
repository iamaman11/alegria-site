//! Read-only siteArchitect preflight for an immutable V2 Temporal DAG start.
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
    operation_dag_parent_workflow_id_v2, owner_content_hash, ArtifactBinding, ArtifactId,
    ArtifactSchema, ContractHash, EffectId, GenericDagError, IdempotencyPolicy,
    OperationDagMachine, OperationDagRunV6, OperationInstanceId, OperationOwnerId, OperationRef,
    PinnedOwnerRouteV1, PlanHash, PortId, TrustedRouteCatalog,
};

pub const SITE_ARCHITECT_START_PREFLIGHT_SCHEMA_REVISION_V1: u16 = 1;
pub const SITE_ARCHITECT_START_PREVIEW_SCHEMA_REVISION_V1: u16 = 1;

/// Digests captured *before* composing a Temporal Start, by the trusted
/// operation owner's command codec. The user cannot forge owner validation
/// or execution permission merely by supplying these untrusted values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteArchitectExpectedStartV1 {
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
pub struct SiteArchitectUnadmittedPreflightV1 {
    pub status: &'static str,
    pub plan_hash: PlanHash,
    pub command_hashes: BTreeMap<OperationInstanceId, String>,
    pub source_payload_hashes: BTreeMap<ArtifactId, String>,
    pub steps_requiring_owner_validate_only: Vec<OperationInstanceId>,
    pub source_authentication: &'static str,
    pub owner_admission: &'static str,
    pub execution: &'static str,
}

/// A display-only view of the *same* immutable run accepted by native V2
/// preflight. It never carries owner command bytes, source payload bytes,
/// credentials, source provenance text, or an executable permission token.
#[derive(Debug, Clone, Serialize)]
pub struct SiteArchitectUnadmittedStartPreviewV1 {
    pub schema_revision: u16,
    pub preflight: SiteArchitectUnadmittedPreflightV1,
    pub execution_order: Vec<OperationInstanceId>,
    pub steps: Vec<SiteArchitectPreviewStepV1>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SiteArchitectPreviewStepV1 {
    pub instance: OperationInstanceId,
    pub operation: OperationRef,
    pub owner: OperationOwnerId,
    pub contract_hash: ContractHash,
    /// Exact trusted owner deployment route pinned in the Temporal Start
    /// snapshot, not a user-selected arbitrary workflow or task queue.
    pub owner_route: PinnedOwnerRouteV1,
    pub command_payload_hash: String,
    pub inputs: BTreeMap<PortId, SiteArchitectPreviewInputV1>,
    pub outputs: BTreeMap<PortId, SiteArchitectPreviewOutputV1>,
    pub after: BTreeSet<OperationInstanceId>,
    pub permitted_effects: BTreeSet<EffectId>,
    pub idempotency: IdempotencyPolicy,
    pub owner_validate_only: &'static str,
    pub effect_dispatch: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum SiteArchitectPreviewInputV1 {
    External {
        artifact: ArtifactId,
        schema: ArtifactSchema,
        /// A digest check does not authenticate the source or its tenant.
        payload_hash: String,
    },
    Produced {
        producer: OperationInstanceId,
        producer_port: PortId,
        artifact: ArtifactId,
        schema: ArtifactSchema,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct SiteArchitectPreviewOutputV1 {
    pub artifact: ArtifactId,
    pub schema: ArtifactSchema,
}

#[derive(Debug, Error)]
pub enum SiteArchitectStartPreflightError {
    #[error("V2 siteArchitect expected command set or plan hash is invalid")]
    InvalidExpectation,
    #[error("V2 siteArchitect command digest differs from checked run for {0:?}")]
    CommandDigestMismatch(OperationInstanceId),
    #[error("V2 siteArchitect source payload digest differs from checked run for {0:?}")]
    SourceDigestMismatch(ArtifactId),
    #[error("V2 siteArchitect run was rejected by its existing native DAG validator")]
    InvalidRun(#[from] GenericDagError),
}

/// An individual tool probe is the **same** immutable Temporal V2 run with
/// exactly one operation. It must never fork an alternative direct-to-provider
/// executor, nor manufacture business/source authority from user input.
#[derive(Debug, Error)]
pub enum SiteArchitectSingleOperationPreflightError {
    #[error("single-operation inspection requires exactly one V2 DAG node, found {count}")]
    StepCount { count: usize },
    #[error(transparent)]
    NativePreflight(#[from] SiteArchitectStartPreflightError),
}

fn check_single_operation_shape(
    run: &OperationDagRunV6,
) -> Result<(), SiteArchitectSingleOperationPreflightError> {
    if run.plan.nodes.len() != 1 {
        return Err(SiteArchitectSingleOperationPreflightError::StepCount {
            count: run.plan.nodes.len(),
        });
    }
    Ok(())
}

/// Test an installed operation in isolation by checking a *one-node* DAG V2.
/// The owner's actual command bytes, source envelopes, route, contract and
/// expected hashes must already exist in a typed `OperationDagRunV6`.
///
/// The expected hashes MUST be supplied separately by the trusted authoring
/// boundary, not minted here from the same possibly forged request. Passing
/// this check only proves byte/route consistency; it does not start Temporal,
/// authenticate sources, or grant the owner's independent effect admission.
pub fn preflight_unadmitted_single_operation_v2_start(
    run: &OperationDagRunV6,
    installed_routes: &TrustedRouteCatalog,
    expected: &SiteArchitectExpectedStartV1,
) -> Result<SiteArchitectUnadmittedPreflightV1, SiteArchitectSingleOperationPreflightError> {
    check_single_operation_shape(run)?;
    preflight_unadmitted_site_architect_v2_start(run, installed_routes, expected).map_err(Into::into)
}

/// Show one tool's exact typed inputs, outputs, trusted route, effects and
/// pending ValidateOnly without starting its Temporal child. A subsequent
/// authorized Start must reuse this *same* V2 DAG and native validation.
pub fn preview_unadmitted_single_operation_v2_start(
    run: &OperationDagRunV6,
    installed_routes: &TrustedRouteCatalog,
    expected: &SiteArchitectExpectedStartV1,
) -> Result<SiteArchitectUnadmittedStartPreviewV1, SiteArchitectSingleOperationPreflightError> {
    check_single_operation_shape(run)?;
    preview_unadmitted_site_architect_v2_start(run, installed_routes, expected).map_err(Into::into)
}

/// Fail closed if any command/source payload bytes, source declaration, owner route, operation
/// contract, plan pin or expected digest changed between typed authoring and
/// the proposed V2 Start. The existing machine still requires every owner's
/// independent ValidateOnly Child *after* the actual Start before effects.
pub fn preflight_unadmitted_site_architect_v2_start(
    run: &OperationDagRunV6,
    installed_routes: &TrustedRouteCatalog,
    expected: &SiteArchitectExpectedStartV1,
) -> Result<SiteArchitectUnadmittedPreflightV1, SiteArchitectStartPreflightError> {
    let node_ids = run
        .plan
        .nodes
        .iter()
        .map(|node| node.instance.clone())
        .collect::<BTreeSet<_>>();
    if expected.schema_revision != SITE_ARCHITECT_START_PREFLIGHT_SCHEMA_REVISION_V1
        || expected.plan_hash != run.plan.hash
        || node_ids.len() != run.plan.nodes.len()
        || expected.command_hashes.keys().collect::<BTreeSet<_>>()
            != node_ids.iter().collect::<BTreeSet<_>>()
        || expected
            .source_payload_hashes
            .keys()
            .collect::<BTreeSet<_>>()
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
        return Err(SiteArchitectStartPreflightError::InvalidExpectation);
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
            .ok_or(SiteArchitectStartPreflightError::InvalidExpectation)?;
        if owner_content_hash(&actual.command_bytes) != *expected_digest {
            return Err(SiteArchitectStartPreflightError::CommandDigestMismatch(
                instance.clone(),
            ));
        }
    }

    for (artifact_id, expected_digest) in &expected.source_payload_hashes {
        let source = run
            .admitted
            .get(artifact_id)
            .ok_or(SiteArchitectStartPreflightError::InvalidExpectation)?;
        if owner_content_hash(&source.payload) != *expected_digest {
            return Err(SiteArchitectStartPreflightError::SourceDigestMismatch(
                artifact_id.clone(),
            ));
        }
    }

    Ok(SiteArchitectUnadmittedPreflightV1 {
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

/// Show the operator the exact typed steps, bindings, owner deployment routes,
/// and effect declarations of a verified proposed V2 Start. This report is
/// explicitly NOT an owner ValidateOnly result, source attestation, business
/// approval, Temporal Start request, or effect-admission credential.
///
/// The only validator is the existing V2 machine above. The extra plan
/// validation obtains its native deterministic execution order; it does not
/// attempt to reinterpret contracts, bindings, or provider-specific bytes.
pub fn preview_unadmitted_site_architect_v2_start(
    run: &OperationDagRunV6,
    installed_routes: &TrustedRouteCatalog,
    expected: &SiteArchitectExpectedStartV1,
) -> Result<SiteArchitectUnadmittedStartPreviewV1, SiteArchitectStartPreflightError> {
    let preflight = preflight_unadmitted_site_architect_v2_start(run, installed_routes, expected)?;
    let validated = run
        .plan
        .validate(installed_routes.catalog())
        .map_err(GenericDagError::from)?;
    let execution_order = validated.execution_order().to_vec();
    let mut steps = Vec::with_capacity(execution_order.len());

    // All lookups below are total because the native V2 machine validated
    // the immutable exact run, trusted route catalog, and complete pins.
    for instance in &execution_order {
        let node = run
            .plan
            .nodes
            .iter()
            .find(|node| &node.instance == instance)
            .expect("native V2 preflight checked every plan node");
        let contract = installed_routes
            .catalog()
            .contract(&node.operation)
            .expect("native V2 preflight checked trusted contracts");
        let route = run
            .pinned_routes
            .iter()
            .find(|route| route.operation == node.operation)
            .expect("native V2 preflight checked exact owner routes");
        let inputs = node
            .inputs
            .iter()
            .map(|(port, binding)| {
                let schema = contract
                    .inputs
                    .get(port)
                    .expect("native V2 preflight checked input port schemas")
                    .clone();
                let view = match binding {
                    ArtifactBinding::External { artifact } => SiteArchitectPreviewInputV1::External {
                        artifact: artifact.clone(),
                        schema,
                        payload_hash: expected
                            .source_payload_hashes
                            .get(artifact)
                            .expect("exact checked External source set")
                            .clone(),
                    },
                    ArtifactBinding::Produced {
                        producer,
                        port: producer_port,
                        artifact,
                    } => SiteArchitectPreviewInputV1::Produced {
                        producer: producer.clone(),
                        producer_port: producer_port.clone(),
                        artifact: artifact.clone(),
                        schema,
                    },
                };
                (port.clone(), view)
            })
            .collect();
        let outputs = node
            .outputs
            .iter()
            .map(|(port, artifact)| {
                (
                    port.clone(),
                    SiteArchitectPreviewOutputV1 {
                        artifact: artifact.clone(),
                        schema: contract
                            .outputs
                            .get(port)
                            .expect("native V2 preflight checked output port schemas")
                            .clone(),
                    },
                )
            })
            .collect();
        steps.push(SiteArchitectPreviewStepV1 {
            instance: instance.clone(),
            operation: node.operation.clone(),
            owner: contract.owner.clone(),
            contract_hash: contract
                .fingerprint()
                .expect("native V2 preflight checked contract fingerprint"),
            owner_route: route.clone(),
            command_payload_hash: expected
                .command_hashes
                .get(instance)
                .expect("exact checked owner command set")
                .clone(),
            inputs,
            outputs,
            after: node.after.clone(),
            permitted_effects: node.permitted_effects.clone(),
            idempotency: contract.idempotency,
            owner_validate_only: "required_not_observed",
            effect_dispatch: "not_started",
        });
    }
    Ok(SiteArchitectUnadmittedStartPreviewV1 {
        schema_revision: SITE_ARCHITECT_START_PREVIEW_SCHEMA_REVISION_V1,
        preflight,
        execution_order,
        steps,
    })
}

#[cfg(test)]
mod single_operation_tests {
    use super::*;
    use crate::{
        AdmittedArtifactV1, ArtifactTypeId, ExecutionPlan, ExternalArtifact, OperationContract,
        OperationId, OperationNode, OwnerCommandWireV1, RunScope,
        OPERATION_DAG_RUN_SCHEMA_REVISION_V6, OWNER_OPERATION_CHILD_TYPE_V1, PLAN_SCHEMA_REVISION,
    };

    fn fixture(
        effectful: bool,
    ) -> (
        OperationDagRunV6,
        TrustedRouteCatalog,
        SiteArchitectExpectedStartV1,
    ) {
        let scope = RunScope {
            tenant_id: "tenant-a".into(),
            run_id: "00000000-0000-0000-0000-000000000001".into(),
        };
        let source_schema = ArtifactSchema {
            id: ArtifactTypeId::from("web.public_fetch_request"),
            revision: 1,
        };
        let output_schema = ArtifactSchema {
            id: ArtifactTypeId::from("web.page_snapshot"),
            revision: 1,
        };
        let effects = if effectful {
            BTreeSet::from([EffectId::from("web.public_page.read")])
        } else {
            BTreeSet::new()
        };
        let contract = OperationContract {
            reference: OperationRef {
                id: OperationId::from("web.public_page.fetch"),
                revision: 1,
            },
            owner: OperationOwnerId::from("web_fetch.owner"),
            inputs: BTreeMap::from([(PortId::from("request"), source_schema.clone())]),
            outputs: BTreeMap::from([(PortId::from("snapshot"), output_schema)]),
            invalidates: BTreeSet::new(),
            effects: effects.clone(),
            idempotency: if effectful {
                IdempotencyPolicy::AtMostOnce
            } else {
                IdempotencyPolicy::Pure
            },
        };
        let contract_hash = contract.fingerprint().unwrap();
        let mut routes = TrustedRouteCatalog::default();
        routes
            .register(
                contract.clone(),
                PinnedOwnerRouteV1 {
                    operation: contract.reference.clone(),
                    owner: contract.owner.clone(),
                    contract_hash: contract_hash.clone(),
                    workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.into(),
                    task_queue: "web-fetch-isolated-owner".into(),
                    build_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                },
            )
            .unwrap();

        let source_id = ArtifactId::from("single-input");
        let instance = OperationInstanceId::from("fetch-one-page");
        let plan = ExecutionPlan {
            schema_revision: PLAN_SCHEMA_REVISION,
            scope: scope.clone(),
            allowed_effects: effects.clone(),
            external_artifacts: BTreeMap::from([(
                source_id.clone(),
                ExternalArtifact {
                    schema: source_schema.clone(),
                    scope: scope.clone(),
                    provenance: "operator-owned-url-request".into(),
                },
            )]),
            pinned_contracts: Vec::new(),
            nodes: vec![OperationNode {
                instance: instance.clone(),
                operation: contract.reference.clone(),
                inputs: BTreeMap::from([(
                    PortId::from("request"),
                    ArtifactBinding::External {
                        artifact: source_id.clone(),
                    },
                )]),
                outputs: BTreeMap::from([(
                    PortId::from("snapshot"),
                    ArtifactId::from("single-output"),
                )]),
                after: BTreeSet::new(),
                permitted_effects: effects,
            }],
            hash: PlanHash::from("unsealed"),
        }
        .seal(routes.catalog())
        .unwrap();
        let source_payload = b"typed-url-input-not-authorized".to_vec();
        let command_bytes = b"typed-provider-command-not-authorized".to_vec();
        let run = OperationDagRunV6 {
            schema_revision: OPERATION_DAG_RUN_SCHEMA_REVISION_V6,
            plan,
            admitted: BTreeMap::from([(
                source_id.clone(),
                AdmittedArtifactV1 {
                    schema: source_schema,
                    scope,
                    provenance: "operator-owned-url-request".into(),
                    payload: source_payload.clone(),
                },
            )]),
            owner_commands: BTreeMap::from([(
                instance.clone(),
                OwnerCommandWireV1 {
                    operation: contract.reference.clone(),
                    owner: contract.owner,
                    contract_hash,
                    command_bytes: command_bytes.clone(),
                },
            )]),
            pinned_routes: routes.trusted_snapshot_for([&contract.reference]).unwrap(),
        };
        let expected = SiteArchitectExpectedStartV1 {
            schema_revision: SITE_ARCHITECT_START_PREFLIGHT_SCHEMA_REVISION_V1,
            plan_hash: run.plan.hash.clone(),
            command_hashes: BTreeMap::from([(instance, owner_content_hash(&command_bytes))]),
            source_payload_hashes: BTreeMap::from([(
                source_id,
                owner_content_hash(&source_payload),
            )]),
        };
        (run, routes, expected)
    }

    #[test]
    fn one_step_is_the_actual_v2_dag_and_remains_unadmitted() {
        let (run, routes, expected) = fixture(true);
        let preview = preview_unadmitted_single_operation_v2_start(&run, &routes, &expected)
            .expect("native V2 authoring check supports an effectful single-operation plan");
        assert_eq!(preview.execution_order.len(), 1);
        assert_eq!(preview.steps.len(), 1);
        assert_eq!(preview.preflight.execution, "not_started");
        assert_eq!(preview.preflight.owner_admission, "not_issued");
        assert_eq!(preview.preflight.source_authentication, "not_checked");
        assert_eq!(
            preview.steps[0].owner_validate_only,
            "required_not_observed"
        );
        assert_eq!(preview.steps[0].effect_dispatch, "not_started");
        assert_eq!(preview.steps[0].idempotency, IdempotencyPolicy::AtMostOnce);
        assert_eq!(preview.steps[0].permitted_effects.len(), 1);
        let safe_diagnostics = serde_json::to_string(&preview).unwrap();
        assert!(!safe_diagnostics.contains("typed-provider-command-not-authorized"));
        assert!(!safe_diagnostics.contains("typed-url-input-not-authorized"));
        assert!(!safe_diagnostics.contains("operator-owned-url-request"));
    }

    #[test]
    fn one_step_rejects_empty_or_multi_step_v2_plans() {
        let (mut run, routes, expected) = fixture(false);
        run.plan.nodes.clear();
        assert!(matches!(
            preflight_unadmitted_single_operation_v2_start(&run, &routes, &expected),
            Err(SiteArchitectSingleOperationPreflightError::StepCount { count: 0 })
        ));
        let (mut run, routes, expected) = fixture(false);
        run.plan.nodes.push(run.plan.nodes[0].clone());
        assert!(matches!(
            preview_unadmitted_single_operation_v2_start(&run, &routes, &expected),
            Err(SiteArchitectSingleOperationPreflightError::StepCount { count: 2 })
        ));
    }

    #[test]
    fn one_step_rejects_mutated_bytes_routes_and_expected_pins() {
        let (run, routes, expected) = fixture(false);
        let mut changed_command = run.clone();
        changed_command
            .owner_commands
            .get_mut(&OperationInstanceId::from("fetch-one-page"))
            .unwrap()
            .command_bytes
            .push(b'!');
        assert!(matches!(
            preflight_unadmitted_single_operation_v2_start(&changed_command, &routes, &expected),
            Err(SiteArchitectSingleOperationPreflightError::NativePreflight(
                SiteArchitectStartPreflightError::CommandDigestMismatch(_)
            ))
        ));

        let mut changed_source = run.clone();
        changed_source
            .admitted
            .get_mut(&ArtifactId::from("single-input"))
            .unwrap()
            .payload
            .push(b'!');
        assert!(matches!(
            preview_unadmitted_single_operation_v2_start(&changed_source, &routes, &expected),
            Err(SiteArchitectSingleOperationPreflightError::NativePreflight(
                SiteArchitectStartPreflightError::SourceDigestMismatch(_)
            ))
        ));

        let mut forged_route = run.clone();
        forged_route.pinned_routes[0].task_queue = "attacker-queue".into();
        assert!(matches!(
            preflight_unadmitted_single_operation_v2_start(&forged_route, &routes, &expected),
            Err(SiteArchitectSingleOperationPreflightError::NativePreflight(
                SiteArchitectStartPreflightError::InvalidRun(_)
            ))
        ));

        let mut forged_expectation = expected;
        forged_expectation.plan_hash = PlanHash::from("forged-plan-hash");
        assert!(matches!(
            preflight_unadmitted_single_operation_v2_start(&run, &routes, &forged_expectation),
            Err(SiteArchitectSingleOperationPreflightError::NativePreflight(
                SiteArchitectStartPreflightError::InvalidExpectation
            ))
        ));

        let unknown_routes = TrustedRouteCatalog::default();
        let locally_computed_expectation = site_architect_expected_start(&run);
        assert!(matches!(
            preview_unadmitted_single_operation_v2_start(
                &run,
                &unknown_routes,
                &locally_computed_expectation,
            ),
            Err(SiteArchitectSingleOperationPreflightError::NativePreflight(
                SiteArchitectStartPreflightError::InvalidRun(_)
            ))
        ));
    }

    fn site_architect_expected_start(run: &OperationDagRunV6) -> SiteArchitectExpectedStartV1 {
        SiteArchitectExpectedStartV1 {
            schema_revision: SITE_ARCHITECT_START_PREFLIGHT_SCHEMA_REVISION_V1,
            plan_hash: run.plan.hash.clone(),
            command_hashes: run
                .owner_commands
                .iter()
                .map(|(instance, command)| {
                    (instance.clone(), owner_content_hash(&command.command_bytes))
                })
                .collect(),
            source_payload_hashes: run
                .admitted
                .iter()
                .map(|(id, source)| (id.clone(), owner_content_hash(&source.payload)))
                .collect(),
        }
    }
}
