//! Independently deployable native Temporal pure Acquisition owner.
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

use acquisition::operation_owner_schema_json::{
    acquisition_owner_schemas, execute_acquisition_pure, validate_acquisition_pure_input,
};
use acquisition::operations::{acquisition_pure_contracts, ClassifySourcePage, EvaluateDomBlocks};
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

const OWNER_QUEUE_V1: &str = "alegria-acquisition-pure-owner-v1";
const OWNER_LIVE_OPT_IN: &str = "ALEGRIA_ENABLE_ACQUISITION_PURE_OWNER_V1";
static TRUSTED_ACQUISITION: OnceLock<TrustedRouteCatalog> = OnceLock::new();

fn invalid_input(reason: impl std::fmt::Display) -> WorkflowTermination {
    WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
        std::io::Error::other(format!(
            "Acquisition pure owner denied typed input: {reason}"
        )),
    ))
}

fn contract(operation: &OperationRef) -> Result<OperationContract> {
    acquisition_pure_contracts()?
        .contract(operation)
        .cloned()
        .context("unregistered Acquisition pure operation")
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
        "Acquisition ValidateOnly rejects foreign contract or settings"
    );
    // The operation Child deliberately refuses Produced inputs until an
    // independent reader can attest the producer's actual Temporal History.
    // ValidateOnly must report the same restriction before the parent starts
    // any Child: accepting an impossible recipe is misleading and unsafe.
    ensure!(
        request.produced_input_ports.is_empty(),
        "Acquisition Produced input requires independently attested Temporal History"
    );
    for (port, external) in &request.external_inputs {
        ensure!(port == &PortId::from("input"), "foreign Acquisition port");
        validate_acquisition_pure_input(&request.operation, &external.payload)
            .map_err(|error| anyhow::anyhow!("invalid Acquisition typed input: {error:?}"))?;
    }
    Ok(())
}

fn verify_owner_invocation(invocation: &OwnerInvocationV1) -> Result<OwnerResultV1> {
    let catalog = TRUSTED_ACQUISITION
        .get()
        .context("Acquisition owner catalog is not installed")?;
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
        "Acquisition Produced input requires independently attested Temporal History"
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
        "Acquisition owner refuses foreign scope, owner or settings"
    );
    let source = invocation
        .bound_inputs
        .get(&PortId::from("input"))
        .context("Acquisition owner requires one typed input artifact")?;
    let output_bytes = execute_acquisition_pure(&checked.operation, &source.artifact.payload)
        .map_err(|error| anyhow::anyhow!("Acquisition pure execution rejected: {error:?}"))?;
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
        .context("Acquisition output schema missing")?;
    let result = OwnerResultV1 {
        dispatch_execution_key: checked.execution_key.clone(),
        outputs: BTreeMap::from([(
            PortId::from("output"),
            AdmittedArtifactV1 {
                schema: output_schema.clone(),
                scope: checked.scope.clone(),
                provenance: "acquisition.pure.v1:unattested_source".to_owned(),
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

/// No caller-authored route catalog is loaded. Only the two compiled
/// acquisition contracts are routed to this worker's fixed Task Queue.
fn trusted_owner_catalog(build_id: &str) -> Result<TrustedRouteCatalog> {
    ensure!(
        immutable_build_id(build_id),
        "WORKER_BUILD_ID must be 40-hex Git SHA"
    );
    let schemas = acquisition_owner_schemas()?;
    ensure!(
        schemas.len() == 2,
        "both acquisition owner schemas are required"
    );
    let mut trusted = TrustedRouteCatalog::default();
    for operation in [
        ClassifySourcePage::contract().reference,
        EvaluateDomBlocks::contract().reference,
    ] {
        let installed = contract(&operation)?;
        schemas
            .get(&operation)
            .context("compiled Acquisition structural schema missing")?
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
    let schemas = acquisition_owner_schemas()?;
    let mut entries = Vec::with_capacity(2);
    for operation in [
        ClassifySourcePage::contract().reference,
        EvaluateDomBlocks::contract().reference,
    ] {
        let installed = routes
            .catalog()
            .contract(&operation)
            .context("compiled Acquisition contract missing from trusted routes")?;
        let route = routes
            .trusted_snapshot_for([&operation])?
            .into_iter()
            .next()
            .context("compiled Acquisition route missing")?;
        let owner_schema = schemas
            .get(&operation)
            .context("compiled Acquisition owner DTO schema missing")?;
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
        "unexpected Acquisition owner DTO schema count"
    );
    Ok(serde_json::json!({"schema_revision": 3, "entries": entries}))
}

fn install_catalog() -> Result<()> {
    let build_id = std::env::var("WORKER_BUILD_ID")?;
    TRUSTED_ACQUISITION
        .set(trusted_owner_catalog(&build_id)?)
        .map_err(|_| anyhow::anyhow!("Acquisition owner catalog already installed"))
}

async fn connect_client(endpoint: &str, namespace: &str) -> Result<Client> {
    let options = ConnectionOptions::new(Url::parse(endpoint)?)
        .identity(format!(
            "alegria-acquisition-pure-owner:{}",
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
        "Acquisition pure owner requires explicit opt-in"
    );
    if let Ok(queue) = std::env::var("TEMPORAL_ACQUISITION_OWNER_QUEUE") {
        ensure!(queue == OWNER_QUEUE_V1, "untrusted Acquisition Task Queue");
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
        .map_err(|error| anyhow::anyhow!("invalid Acquisition owner History: {error}"))?;
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
            "unknown Acquisition owner command"
        );
        let path = args.next().context("missing Acquisition History path")?;
        ensure!(
            args.next().is_none(),
            "unexpected Acquisition replay argument"
        );
        replay_history(Path::new(&path)).await?;
        println!("CI_TEMPORAL_ACQUISITION_PURE_OWNER_REPLAY=PASS");
        return Ok(());
    }
    run_worker().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use operation_protocol::{
        OperationInstanceId, OperationOwnerId, PlanHash, RunScope, TemporalExecutionIdentityV4,
        OWNER_VALIDATION_SCHEMA_REVISION_V1,
    };

    fn valid_owner_validation() -> OwnerValidationRequestV1 {
        let installed = ClassifySourcePage::contract();
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
            instance: OperationInstanceId::from("classify"),
            operation: installed.reference.clone(),
            owner: installed.owner.clone(),
            contract_hash: installed.fingerprint().unwrap(),
            command_bytes: b"{}".to_vec(),
            input_schemas: installed.inputs.clone(),
            external_inputs: BTreeMap::from([(
                PortId::from("input"),
                AdmittedArtifactV1 {
                    schema: installed.inputs[&PortId::from("input")].clone(),
                    scope,
                    provenance: "unattested-test-source".to_owned(),
                    payload: br#"{"url":"https://example.org","title":"Content","raw_text":"Visa fee EUR 25"}"#.to_vec(),
                },
            )]),
            produced_input_ports: Default::default(),
        }
    }

    #[test]
    fn validate_only_checks_exact_owner_command_tenant_and_nested_dto() {
        let good = valid_owner_validation();
        verify_owner_validation(&good).unwrap();
        let mut changed = good.clone();
        changed.command_bytes = br#"{"unexpected":true}"#.to_vec();
        assert!(verify_owner_validation(&changed).is_err());
        let mut changed = good.clone();
        changed.owner = OperationOwnerId::from("seo_domain");
        assert!(verify_owner_validation(&changed).is_err());
        let mut changed = good.clone();
        changed.scope.tenant_id = "foreign-tenant".to_owned();
        assert!(verify_owner_validation(&changed).is_err());
        let mut changed = good;
        changed
            .external_inputs
            .get_mut(&PortId::from("input"))
            .unwrap()
            .payload =
            br#"{"url":"https://example.org","title":"Content","raw_text":"x","secret":true}"#
                .to_vec();
        assert!(verify_owner_validation(&changed).is_err());
    }

    #[test]
    fn validate_only_rejects_produced_input_before_child_start() {
        let mut request = valid_owner_validation();
        request.external_inputs.clear();
        request.produced_input_ports.insert(PortId::from("input"));
        // The generic V1 wire considers this an otherwise valid schema-only
        // request. This particular owner cannot yet attest its producer.
        request.validate_shape().unwrap();
        assert!(verify_owner_validation(&request).is_err());
    }

    #[test]
    fn owner_route_installation_is_exact_and_rejects_forged_build_id() {
        assert!(trusted_owner_catalog("dev").is_err());
        let installed = trusted_owner_catalog(&"a".repeat(40)).unwrap();
        assert_eq!(
            installed
                .catalog()
                .contract(&ClassifySourcePage::contract().reference)
                .unwrap()
                .owner
                .as_str(),
            "acquisition"
        );
        assert_eq!(
            installed
                .catalog()
                .contract(&EvaluateDomBlocks::contract().reference)
                .unwrap()
                .owner
                .as_str(),
            "acquisition"
        );
        assert!(installed
            .catalog()
            .contract(&OperationRef {
                id: "acquisition.fetch_remote".into(),
                revision: 1,
            })
            .is_none());
    }

    #[test]
    fn catalog_export_reuses_compiled_contracts_routes_and_schemas() {
        assert!(trusted_catalog_document_v3("dev").is_err());
        assert!(trusted_catalog_document_v3(&"0".repeat(40)).is_err());
        let build_id = "a".repeat(40);
        let document = trusted_catalog_document_v3(&build_id).unwrap();
        assert_eq!(document["schema_revision"], 3);
        let entries = document["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        for operation in [
            ClassifySourcePage::contract().reference,
            EvaluateDomBlocks::contract().reference,
        ] {
            let contract = contract(&operation).unwrap();
            let entry = entries
                .iter()
                .find(|entry| entry["contract"]["reference"]["id"] == operation.id.as_str())
                .unwrap();
            assert_eq!(entry["contract"], serde_json::to_value(&contract).unwrap());
            assert_eq!(entry["route"]["build_id"], build_id.as_str());
            assert_eq!(
                entry["route"]["workflow_type"],
                OWNER_OPERATION_CHILD_TYPE_V1
            );
            assert_eq!(entry["route"]["task_queue"], OWNER_QUEUE_V1);
            assert_eq!(
                entry["route"]["contract_hash"],
                serde_json::to_value(contract.fingerprint().unwrap()).unwrap()
            );
            assert_eq!(
                entry["owner_schema"],
                serde_json::to_value(&acquisition_owner_schemas().unwrap()[&operation]).unwrap()
            );
            assert_eq!(
                entry["parameter_schema"],
                serde_json::json!({
                    "type": "object", "additionalProperties": false
                })
            );
        }
    }
}
