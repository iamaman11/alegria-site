//! Native V2 owner for the existing pure semantic extraction operations.
//! Every execution uses a read-only History Activity before running its DTO
//! codec. Its receipt covers the transformation and preserves unverified
//! external provenance.

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::{ensure, Context, Result};
use operation_protocol::{
    operation_dag_parent_workflow_id_v2, owner_child_workflow_id_v2, owner_content_hash,
    owner_validation_child_workflow_id_v1, AdmittedArtifactV1, ArtifactBinding, IdempotencyPolicy,
    OperationContract, OperationRef, OwnerInvocationV1, OwnerResultV1, OwnerValidationRequestV1,
    OwnerValidationResultV1, PinnedOwnerOperationSchemaV3, PinnedOwnerRouteV1, PortId,
    TemporalExecutionIdentityV4, TrustedRouteCatalog, OWNER_OPERATION_CHILD_TYPE_V1,
};
use seo_application::operation_adapter::{
    execute_semantic_operation, semantic_extraction_owner_schemas, semantic_extraction_registry,
    validate_semantic_operation_input,
};
use serde::Serialize;
use temporalio_client::{Client, ClientOptions, Connection, ConnectionOptions, WorkflowHistory};
use temporalio_common::RetryPolicy;
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use temporalio_sdk::runtime::RuntimeOptions;
use temporalio_sdk::workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions};
use temporalio_sdk::{
    ActivityCloseTimeouts, ActivityOptions, ApplicationFailure, Runtime, Worker, WorkerOptions,
    WorkflowContext, WorkflowContextView, WorkflowResult, WorkflowTermination,
};

mod produced_history;

const OWNER_QUEUE: &str = "alegria-semantic-pure-owner-v1";
const OWNER_OPT_IN: &str = "ALEGRIA_ENABLE_SEMANTIC_PURE_OWNER_V1";
const ENTITY_SPANS_PRODUCED_PATCH: &str = "semantic-entity-spans-produced-v1";
static OWNER_CATALOG: OnceLock<TrustedRouteCatalog> = OnceLock::new();

fn fail(error: impl std::fmt::Display) -> WorkflowTermination {
    WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
        std::io::Error::other(format!("semantic_owner_validation_rejected: {error}")),
    ))
}

fn installed_catalog() -> Result<&'static TrustedRouteCatalog> {
    OWNER_CATALOG
        .get()
        .context("semantic owner catalog is unavailable")
}

fn is_entity_spans(operation: &OperationRef) -> bool {
    operation.revision == 1 && operation.id.as_str() == "semantic.entity_spans"
}

fn supports_produced(operation: &OperationRef, spans_produced_enabled: bool) -> bool {
    (operation.revision == 1 && operation.id.as_str() == "semantic.canonicalize_mentions")
        || (spans_produced_enabled && is_entity_spans(operation))
}

fn verify_validation(
    catalog: &TrustedRouteCatalog,
    request: &OwnerValidationRequestV1,
    spans_produced_enabled: bool,
) -> Result<()> {
    request.validate_shape()?;
    let contract = catalog
        .catalog()
        .contract(&request.operation)
        .context("unknown semantic owner operation")?;
    ensure!(
        request.parent.workflow_id == operation_dag_parent_workflow_id_v2(&request.scope.run_id)
            && contract.owner.as_str() == "semantic_extraction"
            && request.owner == contract.owner
            && request.contract_hash == contract.fingerprint()?
            && request.input_schemas == contract.inputs
            && request.command_bytes == b"{}"
            && (request.produced_input_ports.is_empty()
                || supports_produced(&request.operation, spans_produced_enabled)),
        "semantic ValidateOnly rejected its pinned contract, settings or input binding"
    );
    for artifact in request.external_inputs.values() {
        validate_semantic_operation_input(&request.operation, &artifact.payload)?;
    }
    Ok(())
}

fn checked_input<'a>(
    catalog: &TrustedRouteCatalog,
    invocation: &'a OwnerInvocationV1,
    spans_produced_enabled: bool,
) -> Result<&'a AdmittedArtifactV1> {
    invocation.verify_integrity(catalog)?;
    let checked = &invocation.checked_dispatch;
    ensure!(
        invocation.parent.workflow_id == operation_dag_parent_workflow_id_v2(&checked.scope.run_id)
            && checked.owner.as_str() == "semantic_extraction"
            && catalog.route_for_dispatch(checked)?.task_queue == OWNER_QUEUE
            && checked.effects.is_empty()
            && checked.idempotency == IdempotencyPolicy::Pure
            && invocation.command_bytes == b"{}",
        "semantic owner expects a pure V2 invocation with empty settings"
    );
    let input = invocation
        .bound_inputs
        .get(&PortId::from("input"))
        .context("semantic input port is absent")?;
    ensure!(
        matches!(input.source, ArtifactBinding::External { .. })
            || supports_produced(&checked.operation, spans_produced_enabled),
        "semantic owner cannot accept this Produced binding"
    );
    Ok(&input.artifact)
}

fn execute_checked(
    catalog: &TrustedRouteCatalog,
    invocation: &OwnerInvocationV1,
    input: &AdmittedArtifactV1,
) -> Result<OwnerResultV1> {
    let dispatch = &invocation.checked_dispatch;
    let output = PortId::from("output");
    let schema = catalog
        .catalog()
        .contract(&dispatch.operation)
        .and_then(|contract| contract.outputs.get(&output))
        .context("semantic output schema is absent")?;
    let payload = execute_semantic_operation(&dispatch.operation, &input.payload)?;
    let receipt = serde_json::to_vec(&(
        1u16,
        &dispatch.operation,
        &dispatch.scope,
        &dispatch.execution_key,
        owner_content_hash(&payload),
    ))?;
    let result = OwnerResultV1 {
        dispatch_execution_key: dispatch.execution_key.clone(),
        outputs: BTreeMap::from([(
            output,
            AdmittedArtifactV1 {
                schema: schema.clone(),
                scope: dispatch.scope.clone(),
                provenance: "semantic_extraction.pure.v1:unattested_external_source".to_owned(),
                payload,
            },
        )]),
        receipt_bytes: receipt,
    };
    result.verify_integrity(dispatch, catalog)?;
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
        let expected = owner_validation_child_workflow_id_v1(
            &request.parent,
            &request.plan_hash,
            &request.instance,
        );
        if ctx.info().workflow_id() != expected {
            return Err(fail("unexpected ValidateOnly identity"));
        }
        let spans_produced_enabled = is_entity_spans(&request.operation)
            && !request.produced_input_ports.is_empty()
            && ctx.patched(ENTITY_SPANS_PRODUCED_PATCH);
        verify_validation(
            installed_catalog().map_err(fail)?,
            &request,
            spans_produced_enabled,
        )
        .map_err(fail)?;
        OwnerValidationResultV1::accepted(&request).map_err(fail)
    }
}

#[workflow]
struct AlegriaOwnerOperationV1Workflow {
    invocation: OwnerInvocationV1,
}

#[workflow_methods]
impl AlegriaOwnerOperationV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, invocation: OwnerInvocationV1) -> Self {
        Self { invocation }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<OwnerResultV1> {
        let invocation = ctx.state(|state| state.invocation.clone());
        let catalog = installed_catalog().map_err(fail)?;
        let spans_produced_enabled = is_entity_spans(&invocation.checked_dispatch.operation)
            && invocation
                .bound_inputs
                .values()
                .any(|bound| matches!(bound.source, ArtifactBinding::Produced { .. }))
            && ctx.patched(ENTITY_SPANS_PRODUCED_PATCH);
        let input = checked_input(catalog, &invocation, spans_produced_enabled).map_err(fail)?;
        if ctx.info().workflow_id()
            != owner_child_workflow_id_v2(&invocation.checked_dispatch.execution_key)
        {
            return Err(fail("unexpected operation Child identity"));
        }
        let mut options = ActivityOptions::start_to_close_timeout(Duration::from_secs(15));
        options.close_timeouts = ActivityCloseTimeouts::ScheduleAndStartToClose {
            schedule_to_close: Duration::from_secs(45),
            start_to_close: Duration::from_secs(15),
        };
        // A Child can reach its Activity before the parent observes its start.
        options.retry_policy = Some(RetryPolicy::builder().maximum_attempts(3).build());
        ctx.execute_activity(
            SemanticHistoryActivities::attest_semantic_owner_history_v1,
            invocation.clone(),
            options,
        )
        .await?;
        execute_checked(catalog, &invocation, input).map_err(fail)
    }
}

struct SemanticHistoryActivities {
    client: Client,
    namespace: String,
}

#[activities]
impl SemanticHistoryActivities {
    #[activity]
    async fn attest_semantic_owner_history_v1(
        self: Arc<Self>,
        ctx: ActivityContext,
        invocation: OwnerInvocationV1,
    ) -> Result<(), ActivityError> {
        self.verify_history(&ctx, &invocation).await.map_err(|_| {
            ActivityError::application(ApplicationFailure::new(std::io::Error::other(
                "semantic_owner_history_unverified",
            )))
        })
    }
}

impl SemanticHistoryActivities {
    async fn verify_history(
        &self,
        ctx: &ActivityContext,
        invocation: &OwnerInvocationV1,
    ) -> Result<()> {
        let catalog = installed_catalog()?;
        // The operation Workflow records the patch before it schedules this Activity.
        checked_input(catalog, invocation, true)?;
        let info = ctx.info();
        ensure!(
            info.workflow_type.as_deref() == Some(OWNER_OPERATION_CHILD_TYPE_V1)
                && info.task_queue == OWNER_QUEUE,
            "History attestation belongs to the semantic owner Child"
        );
        let execution = TemporalExecutionIdentityV4 {
            workflow_id: info
                .workflow_id
                .clone()
                .context("Child Workflow ID missing")?,
            run_id: info
                .workflow_run_id
                .clone()
                .context("Child Run ID missing")?,
        };
        produced_history::attest_semantic_invocation(
            &self.client,
            &self.namespace,
            invocation,
            &execution,
            catalog,
        )
        .await
    }
}

#[derive(Serialize)]
struct CatalogDocument {
    schema_revision: u16,
    entries: Vec<CatalogEntry>,
}

#[derive(Serialize)]
struct CatalogEntry {
    contract: OperationContract,
    route: PinnedOwnerRouteV1,
    parameter_schema: EmptyParameterSchema,
    owner_schema: PinnedOwnerOperationSchemaV3,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EmptyParameterSchema {
    #[serde(rename = "type")]
    kind: &'static str,
    additional_properties: bool,
}

fn build_catalog(build_id: &str) -> Result<(TrustedRouteCatalog, CatalogDocument)> {
    ensure!(
        build_id.len() == 40
            && build_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            && build_id.bytes().any(|byte| byte != b'0'),
        "WORKER_BUILD_ID must be a nonzero 40-hex source SHA"
    );
    let registry = semantic_extraction_registry()?;
    let mut schemas = semantic_extraction_owner_schemas()?;
    let mut catalog = TrustedRouteCatalog::default();
    let mut entries = Vec::new();
    for contract in registry.catalog().contracts() {
        let owner_schema = schemas
            .remove(&contract.reference)
            .context("compiled semantic DTO schema missing")?;
        owner_schema.validate(contract)?;
        let route = PinnedOwnerRouteV1 {
            operation: contract.reference.clone(),
            owner: contract.owner.clone(),
            contract_hash: contract.fingerprint()?,
            workflow_type: OWNER_OPERATION_CHILD_TYPE_V1.to_owned(),
            task_queue: OWNER_QUEUE.to_owned(),
            build_id: build_id.to_owned(),
        };
        catalog.register(contract.clone(), route.clone())?;
        entries.push(CatalogEntry {
            contract: contract.clone(),
            route,
            parameter_schema: EmptyParameterSchema {
                kind: "object",
                additional_properties: false,
            },
            owner_schema,
        });
    }
    ensure!(
        schemas.is_empty(),
        "semantic owner schema is not in the compiled catalog"
    );
    Ok((
        catalog,
        CatalogDocument {
            schema_revision: 3,
            entries,
        },
    ))
}

async fn run_worker() -> Result<()> {
    ensure!(
        std::env::var(OWNER_OPT_IN).as_deref() == Ok("true"),
        "semantic owner requires its explicit opt-in"
    );
    let namespace = std::env::var("TEMPORAL_NAMESPACE")?;
    ensure!(
        !namespace.is_empty() && namespace.trim() == namespace,
        "invalid Temporal namespace"
    );
    let endpoint = url::Url::parse(&std::env::var("TEMPORAL_URL")?)?;
    let connection = Connection::connect(
        ConnectionOptions::new(endpoint)
            .identity(format!(
                "alegria-semantic-pure-owner:{}",
                std::process::id()
            ))
            .build(),
    )
    .await?;
    let client = Client::new(connection, ClientOptions::new(namespace.clone()).build())
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let runtime = Runtime::from_current_tokio(
        RuntimeOptions::builder()
            .build()
            .map_err(|error| anyhow::anyhow!("{error}"))?,
    )?;
    let mut options = WorkerOptions::new(OWNER_QUEUE)
        .register_activities(SemanticHistoryActivities {
            client: client.clone(),
            namespace,
        })
        .build();
    options.register_workflow::<AlegriaOwnerValidateV1Workflow>()?;
    options.register_workflow::<AlegriaOwnerOperationV1Workflow>()?;
    Worker::new(&runtime, client, options)
        .map_err(|error| anyhow::anyhow!("{error}"))?
        .run()
        .await?;
    Ok(())
}

fn runtime_catalog(
    compiled: TrustedRouteCatalog,
    document: &CatalogDocument,
) -> Result<TrustedRouteCatalog> {
    let Some(path) = std::env::var_os("TEMPORAL_OPERATION_DAG_CATALOG_PATH") else {
        return Ok(compiled);
    };
    let bundle = trusted_owner_catalog::load_trusted_owner_bundle(Path::new(&path))?;
    let mut installed = 0;
    for contract in bundle.routes.catalog().contracts() {
        let route = bundle
            .routes
            .trusted_snapshot_for([&contract.reference])?
            .into_iter()
            .next()
            .context("installed route missing")?;
        if route.task_queue != OWNER_QUEUE && contract.owner.as_str() != "semantic_extraction" {
            continue;
        }
        let local = document
            .entries
            .iter()
            .find(|entry| entry.contract.reference == contract.reference)
            .context("semantic queue assigned an operation absent from this build")?;
        ensure!(
            local.contract == *contract && local.route == route,
            "semantic owner route differs from its compiled contract or build"
        );
        bundle.verify_caller_schema(&contract.reference, &local.owner_schema)?;
        installed += 1;
    }
    ensure!(
        installed > 0,
        "trusted catalog contains no route for this semantic owner"
    );
    Ok(bundle.routes)
}

async fn replay_history(path: &Path) -> Result<()> {
    let history = WorkflowHistory::from_json(&std::fs::read(path)?)
        .map_err(|error| anyhow::anyhow!("invalid semantic owner History: {error}"))?;
    let mut options = WorkflowReplayerOptions::new()
        .namespace("default".to_owned())
        .task_queue(OWNER_QUEUE.to_owned())
        .build();
    options.register_workflow::<AlegriaOwnerValidateV1Workflow>()?;
    options.register_workflow::<AlegriaOwnerOperationV1Workflow>()?;
    WorkflowReplayer::new(options)
        .map_err(|error| anyhow::anyhow!("{error}"))?
        .replay_workflow(history)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    println!("CI_TEMPORAL_SEMANTIC_PURE_OWNER_REPLAY=PASS");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let (catalog, document) = build_catalog(&std::env::var("WORKER_BUILD_ID")?)?;
    let mut args = std::env::args_os().skip(1);
    let flag = args.next();
    if flag.as_deref() == Some(std::ffi::OsStr::new("--export-trusted-catalog-v3")) {
        ensure!(args.next().is_none(), "unexpected catalog export argument");
        println!("{}", serde_json::to_string(&document)?);
        return Ok(());
    }
    OWNER_CATALOG
        .set(runtime_catalog(catalog, &document)?)
        .map_err(|_| anyhow::anyhow!("semantic catalog already initialized"))?;
    if let Some(flag) = flag {
        ensure!(flag == "--replay-history", "unknown semantic owner command");
        let path = args.next().context("missing History path")?;
        ensure!(args.next().is_none(), "unexpected replay argument");
        return replay_history(Path::new(&path)).await;
    }
    run_worker().await
}
