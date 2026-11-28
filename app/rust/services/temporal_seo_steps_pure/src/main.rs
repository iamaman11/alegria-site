//! Independently deployable native Temporal pure SEO steps owner.
//!
//! Implements the frozen OwnerValidateV1 and OwnerOperationV1 child ABI on a
//! distinct queue. No Activity, SQL, crawler, provider SDK or network effect
//! is reachable from these Workflow bodies. The generic V2 DAG parent owns
//! the business RunScope and owner route pin; this worker checks the exact
//! compiled owner DTO and checked-dispatch bytes again. It does NOT attest
//! parent/producer server History or external-source authenticity. Produced
//! inputs are denied until an independent History-attestation path exists.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use seo_steps::operation_owner_schema_json::{
    seo_steps_owner_schemas, execute_seo_steps_pure, validate_seo_steps_pure_input,
};
use seo_steps::owner_pure_operations::{
    owner_pure_contracts, BuildSeoOpportunities, ExtractProceduralRules, NormalizeSerpQueries,
};
use anyhow::{ensure, Context, Result};
use operation_protocol::{
    operation_dag_parent_workflow_id_v2, owner_content_hash, AdmittedArtifactV1, ArtifactBinding,
    Operation, OperationContract, OperationRef, OwnerInvocationV1, OwnerResultV1,
    OwnerValidationRequestV1, OwnerValidationResultV1, PinnedOwnerRouteV1, PortId,
    TrustedRouteCatalog, OWNER_OPERATION_CHILD_TYPE_V1,
};
use temporalio_client::{Client, ClientOptions, Connection, ConnectionOptions, WorkflowHistory};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::runtime::RuntimeOptions;
use temporalio_sdk::workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions};
use temporalio_sdk::{
    ApplicationFailure, Runtime, Worker, WorkerOptions, WorkflowContext, WorkflowContextView,
    WorkflowResult, WorkflowTermination,
};
use url::Url;

const OWNER_QUEUE_V1: &str = "alegria-seo-steps-pure-owner-v1";
const OWNER_LIVE_OPT_IN: &str = "ALEGRIA_ENABLE_SEO_STEPS_PURE_OWNER_V1";
static TRUSTED_SEO_STEPS: OnceLock<TrustedRouteCatalog> = OnceLock::new();

fn invalid_input(reason: impl std::fmt::Display) -> WorkflowTermination {
    WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
        std::io::Error::other(format!(
            "SEO steps pure owner denied typed input: {reason}"
        )),
    ))
}

fn contract(operation: &OperationRef) -> Result<OperationContract> {
    owner_pure_contracts()?
        .contract(operation)
        .cloned()
        .context("unregistered SEO steps pure operation")
}

/// The command is deliberately *only* an empty owner-authored settings DTO;
/// all actual operation input is the immutable typed artifact on port input.
/// No untrusted run can choose another owner, effect, route, or provider URL.
fn verify_owner_validation(request: &OwnerValidationRequestV1) -> Result<()> {
    request.validate_shape()?;
    let installed = contract(&request.operation)?;
    ensure!(
        request.owner == installed.owner
            && request.contract_hash == installed.fingerprint()?
            && request.input_schemas == installed.inputs
            && request.command_bytes == b"{}",
        "SEO steps ValidateOnly rejects foreign contract or settings"
    );
    // The operation Child deliberately refuses Produced inputs until an
    // independent reader can attest the producer's actual Temporal History.
    // ValidateOnly must report the same restriction before the parent starts
    // any Child: accepting an impossible recipe is misleading and unsafe.
    ensure!(
        request.produced_input_ports.is_empty(),
        "SEO steps Produced input requires independently attested Temporal History"
    );
    for (port, external) in &request.external_inputs {
        ensure!(port == &PortId::from("input"), "foreign SEO steps port");
        validate_seo_steps_pure_input(&request.operation, &external.payload)
            .map_err(|error| anyhow::anyhow!("invalid SEO steps typed input: {error:?}"))?;
    }
    Ok(())
}

fn verify_owner_invocation(invocation: &OwnerInvocationV1) -> Result<OwnerResultV1> {
    let catalog = TRUSTED_SEO_STEPS
        .get()
        .context("SEO steps owner catalog is not installed")?;
    invocation.verify_integrity(catalog)?;
    // The generic dispatch hash proves only self-consistency. A Produced
    // artifact must be attested against the *actual* preceding child result
    // and owner receipt observed in server History. No such separately
    // credentialed History reader is installed for this pure owner yet.
    // Fail closed rather than laundering a caller-provided producer proof.
    ensure!(
        invocation
            .bound_inputs
            .values()
            .all(|bound| matches!(&bound.source, ArtifactBinding::External { .. })),
        "SEO steps Produced input requires independently attested Temporal History"
    );
    let checked = &invocation.checked_dispatch;
    let installed = contract(&checked.operation)?;
    ensure!(
        checked.owner == installed.owner
            && checked.scope.tenant_id.trim() != ""
            && checked.scope.run_id.trim() != ""
            && invocation.parent.workflow_id
                == operation_dag_parent_workflow_id_v2(&checked.scope.run_id)
            && checked.effects.is_empty()
            && checked.idempotency == operation_protocol::IdempotencyPolicy::Pure
            && invocation.command_bytes == b"{}",
        "SEO steps owner refuses foreign scope, owner or settings"
    );
    let source = invocation
        .bound_inputs
        .get(&PortId::from("input"))
        .context("SEO steps owner requires one typed input artifact")?;
    let output_bytes = execute_seo_steps_pure(&checked.operation, &source.artifact.payload)
        .map_err(|error| anyhow::anyhow!("SEO steps pure execution rejected: {error:?}"))?;
    let output_hash = owner_content_hash(&output_bytes);
    // A deterministic *pure* receipt, not a durable provider receipt or
    // attestation of the authenticity of an external source document.
    let receipt_bytes = serde_json::to_vec(&(
        1u16,
        &checked.operation,
        &checked.scope,
        &checked.execution_key,
        &output_hash,
    ))?;
    let output_schema = installed
        .outputs
        .get(&PortId::from("output"))
        .context("SEO steps output schema missing")?;
    let result = OwnerResultV1 {
        dispatch_execution_key: checked.execution_key.clone(),
        outputs: BTreeMap::from([(
            PortId::from("output"),
            AdmittedArtifactV1 {
                schema: output_schema.clone(),
                scope: checked.scope.clone(),
                provenance: "seo_steps.pure.v1:unattested_source".to_owned(),
                payload: output_bytes,
            },
        )]),
        receipt_bytes,
    };
    result.verify_integrity(checked, catalog)?;
    Ok(result)
}

#[workflow]
struct AlegriaOwnerValidateV1Workflow {
    request: OwnerValidationRequestV1,
}

#[workflow_methods]
impl AlegriaOwnerValidateV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, request: OwnerValidationRequestV1) -> Self {
        Self { request }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<OwnerValidationResultV1> {
        let request = ctx.state(|state| state.request.clone());
        verify_owner_validation(&request).map_err(invalid_input)?;
        OwnerValidationResultV1::accepted(&request).map_err(invalid_input)
    }
}

#[workflow]
struct AlegriaOwnerOperationV1Workflow {
    immutable_invocation: OwnerInvocationV1,
}

#[workflow_methods]
impl AlegriaOwnerOperationV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, invocation: OwnerInvocationV1) -> Self {
        Self {
            immutable_invocation: invocation,
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<OwnerResultV1> {
        let invocation = ctx.state(|state| state.immutable_invocation.clone());
        verify_owner_invocation(&invocation).map_err(invalid_input)
    }
}

fn immutable_build_id(value: &str) -> bool {
    value.len() == 40
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && value.bytes().any(|byte| byte != b'0')
}

/// No caller-authored route catalog is loaded. Only three compiled
/// SEO steps contracts are routed to this worker's fixed Task Queue.
fn trusted_owner_catalog(build_id: &str) -> Result<TrustedRouteCatalog> {
    ensure!(
        immutable_build_id(build_id),
        "WORKER_BUILD_ID must be 40-hex Git SHA"
    );
    let schemas = seo_steps_owner_schemas()?;
    ensure!(
        schemas.len() == 3,
        "all three SEO steps owner schemas are required"
    );
    let mut trusted = TrustedRouteCatalog::default();
    for operation in [
        ExtractProceduralRules::contract().reference,
        NormalizeSerpQueries::contract().reference,
        BuildSeoOpportunities::contract().reference,
    ] {
        let installed = contract(&operation)?;
        schemas
            .get(&operation)
            .context("compiled SEO steps structural schema missing")?
            .validate(&installed)?;
        trusted.register(
            installed.clone(),
            PinnedOwnerRouteV1 {
                operation,
                owner: installed.owner.clone(),
                contract_hash: installed.fingerprint()?,
                workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.to_owned(),
                task_queue: OWNER_QUEUE_V1.to_owned(),
                build_id: build_id.to_owned(),
            },
        )?;
    }
    Ok(trusted)
}

/// Export exactly this binary's compiled owner contracts, routes and DTO
/// schemas. Printing a catalog does not install it or authorize execution:
/// the operator must independently install it under the trusted root.
fn trusted_catalog_document_v3(build_id: &str) -> Result<serde_json::Value> {
    let routes = trusted_owner_catalog(build_id)?;
    let schemas = seo_steps_owner_schemas()?;
    let mut entries = Vec::with_capacity(3);
    for operation in [
        ExtractProceduralRules::contract().reference,
        NormalizeSerpQueries::contract().reference,
        BuildSeoOpportunities::contract().reference,
    ] {
        let installed = routes
            .catalog()
            .contract(&operation)
            .context("compiled SEO steps contract missing from trusted routes")?;
        let route = routes
            .trusted_snapshot_for([&operation])?
            .into_iter()
            .next()
            .context("compiled SEO steps route missing")?;
        let owner_schema = schemas
            .get(&operation)
            .context("compiled SEO steps owner DTO schema missing")?;
        owner_schema.validate(installed)?;
        entries.push(serde_json::json!({
            "contract": installed,
            "route": route,
            "parameter_schema": {"type": "object", "additionalProperties": false},
            "owner_schema": owner_schema,
        }));
    }
    ensure!(
        entries.len() == schemas.len(),
        "unexpected SEO steps owner DTO schema count"
    );
    Ok(serde_json::json!({"schema_revision": 3, "entries": entries}))
}

fn install_catalog() -> Result<()> {
    let build_id = std::env::var("WORKER_BUILD_ID")?;
    TRUSTED_SEO_STEPS
        .set(trusted_owner_catalog(&build_id)?)
        .map_err(|_| anyhow::anyhow!("SEO steps owner catalog already installed"))
}

async fn connect_client(endpoint: &str, namespace: &str) -> Result<Client> {
    let options = ConnectionOptions::new(Url::parse(endpoint)?)
        .identity(format!(
            "alegria-seo-steps-pure-owner:{}",
            std::process::id()
        ))
        .build();
    let connection = Connection::connect(options).await?;
    Client::new(connection, ClientOptions::new(namespace).build())
        .map_err(|error| anyhow::anyhow!("{error}"))
}

async fn run_worker() -> Result<()> {
    ensure!(
        std::env::var(OWNER_LIVE_OPT_IN).as_deref() == Ok("true"),
        "SEO steps pure owner requires explicit opt-in"
    );
    if let Ok(queue) = std::env::var("TEMPORAL_SEO_STEPS_OWNER_QUEUE") {
        ensure!(queue == OWNER_QUEUE_V1, "untrusted SEO steps Task Queue");
    }
    install_catalog()?;
    let endpoint = std::env::var("TEMPORAL_URL")?;
    let namespace = std::env::var("TEMPORAL_NAMESPACE")?;
    ensure!(!endpoint.is_empty() && !namespace.is_empty());
    let runtime = Runtime::from_current_tokio(
        RuntimeOptions::builder()
            .build()
            .map_err(|error| anyhow::anyhow!("{error}"))?,
    )?;
    let client = connect_client(&endpoint, &namespace).await?;
    let mut options = WorkerOptions::new(OWNER_QUEUE_V1).build();
    options.register_workflow::<AlegriaOwnerValidateV1Workflow>()?;
    options.register_workflow::<AlegriaOwnerOperationV1Workflow>()?;
    let mut worker =
        Worker::new(&runtime, client, options).map_err(|error| anyhow::anyhow!("{error}"))?;
    worker.run().await?;
    Ok(())
}

async fn replay_history(path: &Path) -> Result<()> {
    install_catalog()?;
    let history = WorkflowHistory::from_json(&std::fs::read(path)?)
        .map_err(|error| anyhow::anyhow!("invalid SEO steps owner History: {error}"))?;
    let mut options = WorkflowReplayerOptions::new()
        .namespace("default".to_owned())
        .task_queue(OWNER_QUEUE_V1.to_owned())
        .build();
    options.register_workflow::<AlegriaOwnerValidateV1Workflow>()?;
    options.register_workflow::<AlegriaOwnerOperationV1Workflow>()?;
    WorkflowReplayer::new(options)
        .map_err(|error| anyhow::anyhow!("{error}"))?
        .replay_workflow(history)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    if let Some(argument) = args.next() {
        if argument == "--export-trusted-catalog-v3" {
            ensure!(args.next().is_none(), "unexpected catalog export arguments");
            let build_id = std::env::var("WORKER_BUILD_ID")?;
            println!("{}", trusted_catalog_document_v3(&build_id)?);
            return Ok(());
        }
        ensure!(
            argument == "--replay-history",
            "unknown SEO steps owner command"
        );
        let path = args.next().context("missing SEO steps History path")?;
        ensure!(
            args.next().is_none(),
            "unexpected SEO steps replay argument"
        );
        replay_history(Path::new(&path)).await?;
        println!("CI_TEMPORAL_SEO_STEPS_PURE_OWNER_REPLAY=PASS");
        return Ok(());
    }
    run_worker().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use operation_protocol::{
        canonical_owner_dispatch_input_bytes, checked_execution_key, ArtifactId, BoundArtifactV1,
        CheckedDispatch, OperationInstanceId, OperationOwnerId, PlanHash, RunScope,
        TemporalExecutionIdentityV4, OWNER_INVOCATION_SCHEMA_REVISION,
        OWNER_VALIDATION_SCHEMA_REVISION_V1,
    };
    use std::collections::BTreeSet;

    fn sample_operations() -> [(OperationRef, &'static [u8]); 3] {
        [
            (
                ExtractProceduralRules::contract().reference,
                br#"{"section_id":"section-1","raw_text":"Visa fee EUR 25","mentions":[]}"#,
            ),
            (
                NormalizeSerpQueries::contract().reference,
                br#"{"run_id":"business-run","queries":["tourist visa fee"]}"#,
            ),
            (
                BuildSeoOpportunities::contract().reference,
                br#"{"serp_patterns":[]}"#,
            ),
        ]
    }

    fn valid_owner_validation(
        operation: &OperationRef,
        payload: &[u8],
    ) -> OwnerValidationRequestV1 {
        let installed = contract(operation).unwrap();
        let scope = RunScope {
            tenant_id: "tenant-a".to_owned(),
            run_id: "business-run".to_owned(),
        };
        OwnerValidationRequestV1 {
            schema_revision: OWNER_VALIDATION_SCHEMA_REVISION_V1,
            parent: TemporalExecutionIdentityV4 {
                workflow_id: operation_dag_parent_workflow_id_v2(&scope.run_id),
                run_id: "server-run-id".to_owned(),
            },
            plan_hash: PlanHash("sealed-plan".to_owned()),
            scope: scope.clone(),
            instance: OperationInstanceId::from("single-seo-action"),
            operation: operation.clone(),
            owner: installed.owner.clone(),
            contract_hash: installed.fingerprint().unwrap(),
            command_bytes: b"{}".to_vec(),
            input_schemas: installed.inputs.clone(),
            external_inputs: BTreeMap::from([(
                PortId::from("input"),
                AdmittedArtifactV1 {
                    schema: installed.inputs[&PortId::from("input")].clone(),
                    scope,
                    provenance: "test-only-unattested-external".to_owned(),
                    payload: payload.to_vec(),
                },
            )]),
            produced_input_ports: Default::default(),
        }
    }

    #[test]
    fn validates_each_compiled_operation_independently_and_rejects_foreign_inputs() {
        for (operation, payload) in sample_operations() {
            let request = valid_owner_validation(&operation, payload);
            verify_owner_validation(&request).unwrap();
            let mut foreign = request.clone();
            foreign.owner = OperationOwnerId::from("qdrant");
            assert!(verify_owner_validation(&foreign).is_err());
            let mut wrong_command = request.clone();
            wrong_command.command_bytes = br#"{"provider_url":"https://example.org"}"#.to_vec();
            assert!(verify_owner_validation(&wrong_command).is_err());
            let mut unknown_field = request.clone();
            unknown_field.external_inputs.get_mut(&PortId::from("input")).unwrap().payload =
                br#"{"unrecognized":"payload"}"#.to_vec();
            assert!(verify_owner_validation(&unknown_field).is_err());
            let mut produced = request;
            produced.external_inputs.clear();
            produced.produced_input_ports.insert(PortId::from("input"));
            produced.validate_shape().unwrap();
            assert!(verify_owner_validation(&produced).is_err());
        }
    }

    #[test]
    fn validate_only_denies_semantically_invalid_domain_input_before_child() {
        let op = NormalizeSerpQueries::contract().reference;
        let request = valid_owner_validation(&op, br#"{"run_id":"  ","queries":[]}"#);
        let err = verify_owner_validation(&request).unwrap_err();
        assert!(err.to_string().contains("DomainFailure"));
    }

    fn admitted_one_node_invocation(operation: &OperationRef, input: &[u8]) -> OwnerInvocationV1 {
        let installed = contract(operation).unwrap();
        let scope = RunScope {
            tenant_id: "tenant-a".to_owned(),
            run_id: "business-run".to_owned(),
        };
        let artifact_id = ArtifactId::from("external-one-node-source");
        let bound_inputs = BTreeMap::from([(
            PortId::from("input"),
            BoundArtifactV1 {
                artifact_id: artifact_id.clone(),
                source: ArtifactBinding::External {
                    artifact: artifact_id,
                },
                artifact: AdmittedArtifactV1 {
                    schema: installed.inputs[&PortId::from("input")].clone(),
                    scope: scope.clone(),
                    provenance: "explicit-external-test-fixture-no-source-attestation".to_owned(),
                    payload: input.to_vec(),
                },
                produced_evidence: None,
            },
        )]);
        let mut checked_dispatch = CheckedDispatch {
            plan_hash: PlanHash("sealed-plan".to_owned()),
            scope: scope.clone(),
            instance: OperationInstanceId::from("seo-single-action"),
            operation: operation.clone(),
            owner: installed.owner,
            effects: BTreeSet::new(),
            idempotency: operation_protocol::IdempotencyPolicy::Pure,
            input_hash: String::new(),
            execution_key: String::new(),
        };
        let command_bytes = b"{}".to_vec();
        let dispatch_bytes = canonical_owner_dispatch_input_bytes(&bound_inputs, &command_bytes)
            .unwrap();
        checked_dispatch.input_hash = owner_content_hash(&dispatch_bytes);
        checked_dispatch.execution_key =
            checked_execution_key(&checked_dispatch, &checked_dispatch.input_hash).unwrap();
        OwnerInvocationV1 {
            schema_revision: OWNER_INVOCATION_SCHEMA_REVISION,
            parent: TemporalExecutionIdentityV4 {
                workflow_id: operation_dag_parent_workflow_id_v2(&scope.run_id),
                run_id: "native-parent-server-run-id".to_owned(),
            },
            checked_dispatch,
            command_bytes,
            bound_inputs,
        }
    }

    #[test]
    fn three_independent_one_node_dispatches_produce_typed_receipts() {
        TRUSTED_SEO_STEPS.get_or_init(|| trusted_owner_catalog(&"a".repeat(40)).unwrap());
        for (operation, payload) in sample_operations() {
            let invocation = admitted_one_node_invocation(&operation, payload);
            let first = verify_owner_invocation(&invocation).unwrap();
            let repeated = verify_owner_invocation(&invocation).unwrap();
            assert_eq!(first.receipt_bytes, repeated.receipt_bytes);
            assert_eq!(first.outputs.len(), 1);
            assert_eq!(first.outputs[&PortId::from("output")].payload,
                repeated.outputs[&PortId::from("output")].payload);
            assert!(serde_json::from_slice::<serde_json::Value>(
                &first.outputs[&PortId::from("output")].payload
            ).unwrap().is_object());

            let mut changed = invocation.clone();
            changed.command_bytes = br#"{"unsafe":true}"#.to_vec();
            assert!(verify_owner_invocation(&changed).is_err());
            let mut foreign_tenant = invocation;
            foreign_tenant.checked_dispatch.scope.tenant_id = "foreign-tenant".into();
            assert!(verify_owner_invocation(&foreign_tenant).is_err());
        }
    }

    #[test]
    fn trusted_catalog_exactly_matches_three_real_owner_schemas() {
        assert!(trusted_owner_catalog("dev").is_err());
        assert!(trusted_owner_catalog(&"0".repeat(40)).is_err());
        let build_id = "a".repeat(40);
        let installed = trusted_owner_catalog(&build_id).unwrap();
        let document = trusted_catalog_document_v3(&build_id).unwrap();
        assert_eq!(document["schema_revision"], 3);
        let entries = document["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        for (operation, _) in sample_operations() {
            let contract = contract(&operation).unwrap();
            assert_eq!(
                installed.catalog().contract(&operation).unwrap().owner.as_str(),
                "seo_steps"
            );
            let entry = entries.iter().find(|entry| {
                entry["contract"]["reference"]["id"] == operation.id.as_str()
            }).unwrap();
            assert_eq!(entry["contract"], serde_json::to_value(&contract).unwrap());
            assert_eq!(entry["route"]["build_id"], build_id.as_str());
            assert_eq!(entry["route"]["task_queue"], OWNER_QUEUE_V1);
            assert_eq!(entry["route"]["workflow_type"], OWNER_OPERATION_CHILD_TYPE_V1);
            assert_eq!(
                entry["owner_schema"],
                serde_json::to_value(&seo_steps_owner_schemas().unwrap()[&operation]).unwrap()
            );
        }
        assert!(installed.catalog().contract(&OperationRef {
            id: "seo_steps.untrusted_network_call".into(),
            revision: 1,
        }).is_none());
    }
}
