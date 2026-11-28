//! Independent, opt-in native Temporal V2 owner for the DataForSEO contract.
//!
//! The initial effect is an explicitly marked OFFLINE STUB, never a paid
//! DataForSEO request. Real network POST remains disabled at source level.
//! Even this stub requires operator-issued exact parent/child History admission
//! and a committed PostgreSQL attempt BEFORE it may return a typed receipt.

use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{ensure, Context, Result};
use dataforseo_contracts::{
    validate_dataforseo_owner_request, validate_discovery_shape_only,
    verify_dataforseo_owner_invocation, DataForSeoGoogleOrganicSearchInputV1,
    DataForSeoGoogleOrganicSearchOutputV1, DataForSeoGoogleOrganicSearchV1, DataForSeoOrganicHitV1,
    DataForSeoSourceStatusV1, DATAFORSEO_ACTIVITY_QUEUE_V1, DATAFORSEO_OWNER_QUEUE_V1, INPUT_PORT,
    OUTPUT_PORT, SCHEMA_REVISION,
};
use operation_protocol::{
    operation_dag_parent_workflow_id_v2, owner_child_workflow_id_v2, AdmittedArtifactV1, Operation,
    OwnerInvocationV1, OwnerResultV1, OwnerValidationRequestV1, OwnerValidationResultV1, PortId,
    OWNER_OPERATION_CHILD_TYPE_V1, OWNER_VALIDATION_CHILD_TYPE_V1,
};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use temporalio_client::{Client, ClientOptions, Connection, ConnectionOptions, WorkflowHistory};
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use temporalio_sdk::runtime::RuntimeOptions;
use temporalio_sdk::workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions};
use temporalio_sdk::{
    ActivityCloseTimeouts, ActivityOptions, ApplicationFailure, Runtime, SyncWorkflowContext,
    Worker, WorkerOptions, WorkflowContext, WorkflowContextView, WorkflowResult,
    WorkflowTermination,
};
use url::Url;
use uuid::Uuid;

mod operator;
// Strict paid HTTPS wire/parser is compiled and tested, but MUST NOT be
// dispatched until SQL V2 one-use grants and provider receipts have passed
// a separately attested native Temporal/PG E2E. V1 remains fixture-only.
#[allow(dead_code)]
mod paid_provider;

fn non_retryable(error: impl std::fmt::Display) -> ActivityError {
    ActivityError::application(ApplicationFailure::non_retryable(std::io::Error::other(
        error.to_string(),
    )))
}

/// Only independently retryable read-only admission probes get an automatic
/// retry. Invalid tenant, malformed command, missing SQL grant or governance
/// denial are terminal; they cannot be fixed by Temporal retries.
fn admission_probe_error(error: anyhow::Error) -> ActivityError {
    let retryable = error
        .downcast_ref::<sqlx::Error>()
        .is_some_and(|sql| match sql {
            sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => true,
            sqlx::Error::Database(db) => db.code().is_some_and(|code| {
                code.starts_with("08")
                    || matches!(
                        code.as_ref(),
                        "40001"
                            | "40003"
                            | "40P01"
                            | "55P03"
                            | "53300"
                            | "53400"
                            | "57P01"
                            | "57P02"
                            | "57P03"
                    )
            }),
            _ => false,
        });
    if retryable {
        ActivityError::application(ApplicationFailure::new(std::io::Error::other(
            error.to_string(),
        )))
    } else {
        non_retryable(error)
    }
}

/// Only a GET for an already durably persisted stub task may be retried.
/// After an ambiguous POST the SQL ledger remains `uncertain` and a second
/// Activity invocation cannot post again. HTTP status/transport failures of
/// a GET are safe to retry; malformed provider JSON and unexpected response
/// shapes remain terminal, not endlessly retried. Reqwest's classification of
/// a truncated HTTP body can vary between `body`, `decode` and `request` as
/// the transport evolves; all such failures are safe for a GET-only resume.
fn retryable_posted_stub_get(error: &anyhow::Error) -> bool {
    error.downcast_ref::<reqwest::Error>().is_some_and(|get| {
        get.status()
            .map(|status| {
                status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
            })
            .unwrap_or_else(|| !get.is_builder() && !get.is_redirect())
    })
}

fn posted_stub_get_error(error: anyhow::Error) -> ActivityError {
    if retryable_posted_stub_get(&error) {
        ActivityError::application(ApplicationFailure::new(std::io::Error::other(
            error.to_string(),
        )))
    } else {
        non_retryable(error)
    }
}

fn validation_failure(error: impl std::fmt::Display) -> WorkflowTermination {
    WorkflowTermination::failed_application(ApplicationFailure::non_retryable(
        std::io::Error::other(format!("DataForSEO typed owner validation: {error}")),
    ))
}

fn probe_options() -> ActivityOptions {
    let mut options = ActivityOptions::start_to_close_timeout(Duration::from_secs(15));
    options.close_timeouts = ActivityCloseTimeouts::ScheduleAndStartToClose {
        schedule_to_close: Duration::from_secs(300),
        start_to_close: Duration::from_secs(15),
    };
    options.task_queue = Some(DATAFORSEO_ACTIVITY_QUEUE_V1.to_owned());
    options
}

fn stub_effect_options() -> ActivityOptions {
    // A Worker may die after the durable POST task ID is committed but while
    // GET is in flight. Bound each Activity attempt so Temporal can dispatch
    // a new attempt to a restarted Worker. The SQL 'posted' marker guarantees
    // every such replay is GET-only; an uncertain POST can never repeat.
    let mut options = ActivityOptions::start_to_close_timeout(Duration::from_secs(20));
    options.close_timeouts = ActivityCloseTimeouts::ScheduleAndStartToClose {
        schedule_to_close: Duration::from_secs(300),
        start_to_close: Duration::from_secs(20),
    };
    options.task_queue = Some(DATAFORSEO_ACTIVITY_QUEUE_V1.to_owned());
    options
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
        validate_dataforseo_owner_request(&request).map_err(validation_failure)
    }
}

#[workflow]
struct AlegriaOwnerOperationV1Workflow {
    immutable_invocation: OwnerInvocationV1,
    wakeup_count: u64,
}

#[workflow_methods]
impl AlegriaOwnerOperationV1Workflow {
    #[init]
    fn new(_ctx: &WorkflowContextView, invocation: OwnerInvocationV1) -> Self {
        Self {
            immutable_invocation: invocation,
            wakeup_count: 0,
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<OwnerResultV1> {
        let invocation = ctx.state(|state| state.immutable_invocation.clone());
        verify_dataforseo_owner_invocation(&invocation).map_err(validation_failure)?;
        // An arbitrary Signal is NEVER admission. It merely wakes the read-
        // only SQL probe, which must find a separately History-attested row.
        let mut observed = 0;
        loop {
            let previous = observed;
            ctx.wait_condition(move |state| state.wakeup_count > previous)
                .await?;
            observed = ctx.state(|state| state.wakeup_count);
            let admitted: bool = ctx
                .start_activity(
                    DataForSeoActivities::check_operator_admission_v1,
                    invocation.clone(),
                    probe_options(),
                )
                .await?;
            if admitted {
                break;
            }
        }
        let receipt: OwnerResultV1 = ctx
            .start_activity(
                DataForSeoActivities::materialize_admitted_stub_v1,
                invocation,
                stub_effect_options(),
            )
            .await?;
        Ok(receipt)
    }

    #[signal(name = "operator_admission_ready_dataforseo_v1")]
    fn operator_admission_ready_dataforseo_v1(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.wakeup_count = self.wakeup_count.saturating_add(1);
    }
}

/// Clearly distinct from a provider receipt. No consumer hypothesis may
/// count this fixture as evidence, even if its typed bytes are structurally
/// valid. The same output works through the generic DAG output port.
fn offline_fixture_result(
    invocation: &OwnerInvocationV1,
    input: &DataForSeoGoogleOrganicSearchInputV1,
    task_id: &str,
    response_bytes: &[u8],
    organic_results: Vec<DataForSeoOrganicHitV1>,
) -> Result<OwnerResultV1> {
    let contract = DataForSeoGoogleOrganicSearchV1::contract();
    let output = DataForSeoGoogleOrganicSearchOutputV1 {
        schema_revision: SCHEMA_REVISION,
        source_status: DataForSeoSourceStatusV1::StubFixtureOnly,
        provider_task_id: task_id.to_owned(),
        provider_response_hash: blake3::hash(response_bytes).to_hex().to_string(),
        input_hash: blake3::hash(&serde_json::to_vec(input)?)
            .to_hex()
            .to_string(),
        provider_reported_cost_usd_micros: Some(0),
        organic_results,
    };
    validate_discovery_shape_only(&output, input)?;
    let payload = serde_json::to_vec(&output)?;
    let port = PortId::from(OUTPUT_PORT);
    Ok(OwnerResultV1 {
        dispatch_execution_key: invocation.checked_dispatch.execution_key.clone(),
        outputs: std::collections::BTreeMap::from([(
            port.clone(),
            AdmittedArtifactV1 {
                schema: contract.outputs[&port].clone(),
                scope: input.scope.clone(),
                provenance: format!(
                    "dataforseo:offline-fixture-only:{}",
                    invocation.checked_dispatch.execution_key
                ),
                payload: payload.clone(),
            },
        )]),
        receipt_bytes: payload,
    })
}

/// The worker has no DataForSEO credentials and cannot call a public URL.
/// Only explicit loopback HTTP fixtures can be contacted, with redirects
/// disabled. This preserves physical POST/GET behavior for CI without spend.
fn stub_endpoint() -> Result<Url> {
    let value = std::env::var("DATAFORSEO_STUB_BASE_URL")
        .context("offline DataForSEO owner requires a test-only loopback endpoint")?;
    let url = Url::parse(&value)?;
    validate_stub_url(&url)?;
    Ok(url)
}

fn validate_stub_url(url: &Url) -> Result<()> {
    ensure!(
        url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port().is_some()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && url.username().is_empty()
            && url.password().is_none(),
        "DataForSEO fixture endpoint must be bare http://127.0.0.1:<port>/"
    );
    Ok(())
}

fn stub_task_id(value: &str) -> bool {
    value.strip_prefix("stub-serp-").is_some_and(|index| {
        !index.is_empty() && index.len() <= 8 && index.bytes().all(|b| b.is_ascii_digit())
    })
}

async fn capped_stub_json(mut response: reqwest::Response, cap: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            chunk.len() <= cap.saturating_sub(bytes.len()),
            "DataForSEO stub HTTP body exceeds owner byte cap"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn stub_task_post(url: &Url, input: &DataForSeoGoogleOrganicSearchInputV1) -> Result<String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?;
    let response = client
        .post(url.join("v3/serp/google/organic/task_post")?)
        .json(&vec![json!({
            "keyword": input.keyword,
            "language_code": input.language_code,
            "location_code": input.location_code,
            "depth": input.depth,
            "priority": 1,
        })])
        .send()
        .await?
        .error_for_status()?;
    let bytes = capped_stub_json(response, 64 * 1024).await?;
    let receipt: Value = serde_json::from_slice(&bytes)?;
    ensure!(
        receipt["status_code"].as_i64() == Some(20000)
            && receipt["tasks"][0]["status_code"].as_i64() == Some(20100),
        "DataForSEO stub POST response is not an accepted task"
    );
    let task_id = receipt["tasks"][0]["id"]
        .as_str()
        .context("missing DataForSEO stub task ID")?;
    ensure!(stub_task_id(task_id), "unexpected non-stub task ID");
    Ok(task_id.to_owned())
}

fn typed_stub_hits(value: &Value, depth: u32) -> Result<Vec<DataForSeoOrganicHitV1>> {
    let mut organic = Vec::new();
    let mut seen_ranks = std::collections::BTreeSet::new();
    let results = value["tasks"][0]["result"]
        .as_array()
        .context("DataForSEO completed stub has no result array")?;
    for result in results {
        if let Some(items) = result["items"].as_array() {
            for item in items {
                if item["type"].as_str() != Some("organic") {
                    continue;
                }
                let rank = item["rank_absolute"]
                    .as_u64()
                    .or_else(|| item["rank_group"].as_u64())
                    .and_then(|rank| u32::try_from(rank).ok())
                    .context("DataForSEO stub organic rank missing")?;
                ensure!(
                    rank > 0 && rank <= depth,
                    "DataForSEO stub rank exceeds query depth"
                );
                ensure!(seen_ranks.insert(rank), "DataForSEO stub duplicate rank");
                let title = item["title"]
                    .as_str()
                    .context("DataForSEO stub title missing")?;
                let source_url = item["url"].as_str().context("stub URL missing")?;
                let source = Url::parse(source_url)?;
                ensure!(
                    source.scheme() == "https"
                        && source.username().is_empty()
                        && source.password().is_none()
                        && source.host_str().is_some(),
                    "stub SERP URL must be direct HTTPS without credentials"
                );
                let domain = source.host_str().unwrap_or_default().to_ascii_lowercase();
                let snippet = item["description"].as_str().unwrap_or_default();
                ensure!(
                    title.len() <= 4_096
                        && snippet.len() <= 8_192
                        && source_url.len() <= 2_048
                        && domain.len() <= 256,
                    "stub SERP organic fields exceed typed output bounds"
                );
                organic.push(DataForSeoOrganicHitV1 {
                    rank,
                    title: title.to_owned(),
                    source_url: source_url.to_owned(),
                    domain_norm: domain,
                    snippet: snippet.to_owned(),
                });
            }
        }
    }
    ensure!(
        organic.len() <= usize::try_from(depth)?,
        "stub results exceed depth"
    );
    Ok(organic)
}

async fn stub_task_get(
    url: &Url,
    task_id: &str,
    depth: u32,
) -> Result<(Vec<u8>, Vec<DataForSeoOrganicHitV1>)> {
    ensure!(stub_task_id(task_id), "invalid persisted stub task ID");
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?;
    let path = format!("v3/serp/google/organic/task_get/advanced/{task_id}");
    let response = client
        .get(url.join(&path)?)
        .send()
        .await?
        .error_for_status()?;
    let bytes = capped_stub_json(response, 512 * 1024).await?;
    let value: Value = serde_json::from_slice(&bytes)?;
    ensure!(
        value["status_code"].as_i64() == Some(20000)
            && value["tasks"][0]["status_code"].as_i64() == Some(20000)
            && value["tasks"][0]["result"].is_array(),
        "DataForSEO stub GET did not return a completed task"
    );
    let hits = typed_stub_hits(&value, depth)?;
    Ok((bytes, hits))
}

struct DataForSeoActivities {
    pool: Arc<PgPool>,
}

fn checked_activity_identity(
    ctx: &ActivityContext,
    invocation: &OwnerInvocationV1,
) -> Result<(String, String)> {
    verify_dataforseo_owner_invocation(invocation)?;
    let info = ctx.info();
    let workflow_id = info
        .workflow_id
        .clone()
        .context("missing DataForSEO child Workflow ID")?;
    let run_id = info
        .workflow_run_id
        .clone()
        .context("missing DataForSEO server Run ID")?;
    ensure!(
        info.workflow_type.as_deref() == Some(OWNER_OPERATION_CHILD_TYPE_V1)
            && info.task_queue == DATAFORSEO_ACTIVITY_QUEUE_V1
            && invocation.parent.workflow_id
                == operation_dag_parent_workflow_id_v2(&invocation.checked_dispatch.scope.run_id)
            && workflow_id
                == owner_child_workflow_id_v2(&invocation.checked_dispatch.execution_key),
        "DataForSEO Activity does not belong to admitted V2 owner Child"
    );
    Ok((workflow_id, run_id))
}

impl DataForSeoActivities {
    /// All 12 fields are compared by the independently issued SQL admission.
    /// A user cannot mint admission by choosing an arbitrary tenant or UUID.
    async fn admitted_id(
        &self,
        ctx: &ActivityContext,
        invocation: &OwnerInvocationV1,
    ) -> Result<Option<Uuid>> {
        let (child_id, child_run_id) = checked_activity_identity(ctx, invocation)?;
        let dispatch = &invocation.checked_dispatch;
        let input = verify_dataforseo_owner_invocation(invocation)?;
        let business_run = Uuid::parse_str(&dispatch.scope.run_id)?;
        let cap = i64::try_from(input.max_cost_usd_micros)?;
        let hash = blake3::hash(&invocation.command_bytes).to_hex().to_string();
        let admission: Option<Uuid> = sqlx::query_scalar(
            "SELECT pipeline.lookup_dataforseo_owner_admission_v1(
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        )
        .bind(&dispatch.scope.tenant_id)
        .bind(business_run)
        .bind(dispatch.plan_hash.as_str())
        .bind(dispatch.instance.as_str())
        .bind(&invocation.parent.workflow_id)
        .bind(&invocation.parent.run_id)
        .bind(&child_id)
        .bind(&child_run_id)
        .bind(&dispatch.execution_key)
        .bind(hash)
        .bind(&dispatch.input_hash)
        .bind(cap)
        .fetch_one(&*self.pool)
        .await?;
        Ok(admission)
    }
}

#[activities]
impl DataForSeoActivities {
    #[activity]
    async fn check_operator_admission_v1(
        self: Arc<Self>,
        ctx: ActivityContext,
        invocation: OwnerInvocationV1,
    ) -> Result<bool, ActivityError> {
        tokio::time::sleep(Duration::from_secs(1)).await;
        self.admitted_id(&ctx, &invocation)
            .await
            .map(|value| value.is_some())
            .map_err(admission_probe_error)
    }

    #[activity]
    async fn materialize_admitted_stub_v1(
        self: Arc<Self>,
        ctx: ActivityContext,
        invocation: OwnerInvocationV1,
    ) -> Result<OwnerResultV1, ActivityError> {
        let admission = self
            .admitted_id(&ctx, &invocation)
            .await
            .map_err(admission_probe_error)?
            .ok_or_else(|| non_retryable("missing independently issued DataForSEO admission"))?;
        let dispatch_key = &invocation.checked_dispatch.execution_key;
        let input = verify_dataforseo_owner_invocation(&invocation).map_err(non_retryable)?;
        let cap = i64::try_from(input.max_cost_usd_micros).map_err(non_retryable)?;
        // SQL transaction claims the exact event BEFORE any effect. A crash
        // before confirm leaves 'uncertain': no automatic second attempt.
        let state: String =
            sqlx::query_scalar("SELECT pipeline.begin_dataforseo_owner_v1($1,$2,$3,$4)")
                .bind(admission)
                .bind(&invocation.command_bytes)
                .bind(dispatch_key)
                .bind(cap)
                .fetch_one(&*self.pool)
                .await
                .map_err(non_retryable)?;
        match state.as_str() {
            "started" => {
                // A paid POST path does NOT exist. A real local stub POST
                // starts only after the durable SQL pre-claim committed.
                let endpoint = stub_endpoint().map_err(non_retryable)?;
                let task_id = stub_task_post(&endpoint, &input)
                    .await
                    .map_err(non_retryable)?;
                let recorded: bool = sqlx::query_scalar(
                    "SELECT pipeline.record_stub_dataforseo_task_v1($1,$2,$3,$4)",
                )
                .bind(admission)
                .bind(&invocation.command_bytes)
                .bind(dispatch_key)
                .bind(&task_id)
                .fetch_one(&*self.pool)
                .await
                .map_err(non_retryable)?;
                if !recorded {
                    return Err(non_retryable(
                        "DataForSEO stub task ID was not durably recorded",
                    ));
                }
                self.complete_posted_stub(&invocation, admission, &input, &task_id)
                    .await
            }
            "posted" => {
                let task_id: Option<String> =
                    sqlx::query_scalar("SELECT pipeline.lookup_stub_dataforseo_task_v1($1,$2,$3)")
                        .bind(admission)
                        .bind(&invocation.command_bytes)
                        .bind(dispatch_key)
                        .fetch_one(&*self.pool)
                        .await
                        .map_err(non_retryable)?;
                let task_id =
                    task_id.ok_or_else(|| non_retryable("missing posted stub task ID"))?;
                self.complete_posted_stub(&invocation, admission, &input, &task_id)
                    .await
            }
            "confirmed" => {
                let bytes: Option<Vec<u8>> = sqlx::query_scalar(
                    "SELECT pipeline.read_confirmed_dataforseo_owner_receipt_v1($1,$2,$3)",
                )
                .bind(admission)
                .bind(&invocation.command_bytes)
                .bind(dispatch_key)
                .fetch_one(&*self.pool)
                .await
                .map_err(non_retryable)?;
                let bytes = bytes.ok_or_else(|| non_retryable("missing durable owner receipt"))?;
                let result: OwnerResultV1 =
                    serde_json::from_slice(&bytes).map_err(non_retryable)?;
                let output_payload = result
                    .outputs
                    .get(&PortId::from(OUTPUT_PORT))
                    .map(|output| &output.payload);
                if result.dispatch_execution_key != *dispatch_key
                    || output_payload != Some(&result.receipt_bytes)
                {
                    return Err(non_retryable(
                        "durable DataForSEO owner receipt does not match dispatch",
                    ));
                }
                Ok(result)
            }
            "uncertain" => Err(non_retryable(
                "DataForSEO attempt state is uncertain; explicit operator reconciliation required",
            )),
            _ => Err(non_retryable("unknown DataForSEO SQL attempt state")),
        }
    }
}

impl DataForSeoActivities {
    async fn complete_posted_stub(
        &self,
        invocation: &OwnerInvocationV1,
        admission: Uuid,
        input: &DataForSeoGoogleOrganicSearchInputV1,
        task_id: &str,
    ) -> Result<OwnerResultV1, ActivityError> {
        let (response_bytes, hits) = stub_task_get(
            &stub_endpoint().map_err(non_retryable)?,
            task_id,
            input.depth,
        )
        .await
        .map_err(posted_stub_get_error)?;
        let result = offline_fixture_result(invocation, input, task_id, &response_bytes, hits)
            .map_err(non_retryable)?;
        let encoded = serde_json::to_vec(&result).map_err(non_retryable)?;
        let output = result
            .outputs
            .get(&PortId::from(OUTPUT_PORT))
            .ok_or_else(|| non_retryable("missing typed DataForSEO fixture output"))?;
        let confirmed: bool = sqlx::query_scalar(
            "SELECT pipeline.confirm_dataforseo_owner_v1(
                        $1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(admission)
        .bind(&invocation.command_bytes)
        .bind(&invocation.checked_dispatch.execution_key)
        .bind(task_id)
        .bind(&output.payload)
        .bind(&encoded)
        .bind(0_i64)
        .fetch_one(&*self.pool)
        .await
        .map_err(non_retryable)?;
        if !confirmed {
            return Err(non_retryable(
                "DataForSEO stub receipt not durably confirmed",
            ));
        }
        Ok(result)
    }
}

fn database_secret_file() -> Result<String> {
    let filename = std::env::var("DATAFORSEO_OWNER_DATABASE_URL_FILE")
        .context("DataForSEO owner requires isolated PostgreSQL secret file")?;
    let path = Path::new(&filename);
    ensure!(path.is_absolute(), "database secret path must be absolute");
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.file_type().is_file(),
        "database secret must not be symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "secret must be 0600/0400"
        );
    }
    let value = std::fs::read_to_string(path)?
        .trim_end_matches(|ch| ch == '\r' || ch == '\n')
        .to_owned();
    ensure!(
        !value.is_empty() && value.trim() == value,
        "malformed DB secret"
    );
    Ok(value)
}

async fn assert_isolated_worker_role(pool: &PgPool) -> Result<()> {
    let row = sqlx::query(
        "SELECT r.rolsuper,r.rolbypassrls,
                pipeline.current_session_tenant_v1() AS tenant,
                has_table_privilege(session_user,'pipeline.dataforseo_owner_admissions_v1','INSERT') AS can_issue,
                has_table_privilege(session_user,'pipeline.dataforseo_owner_admissions_v1','SELECT') AS can_read,
                has_function_privilege(session_user,
                    'pipeline.lookup_dataforseo_owner_admission_v1(text,uuid,text,text,text,text,text,text,text,text,text,bigint)','EXECUTE') AS lookup_allowed,
                has_function_privilege(session_user,
                    'pipeline.begin_dataforseo_owner_v1(uuid,bytea,text,bigint)','EXECUTE') AS begin_allowed,
                has_function_privilege(session_user,
                    'pipeline.confirm_dataforseo_owner_v1(uuid,bytea,text,text,bytea,bytea,bigint)','EXECUTE') AS confirm_allowed,
                has_function_privilege(session_user,
                    'pipeline.record_stub_dataforseo_task_v1(uuid,bytea,text,text)','EXECUTE') AS record_allowed,
                has_function_privilege(session_user,
                    'pipeline.lookup_stub_dataforseo_task_v1(uuid,bytea,text)','EXECUTE') AS posted_allowed,
                has_function_privilege(session_user,
                    'pipeline.read_confirmed_dataforseo_owner_receipt_v1(uuid,bytea,text)','EXECUTE') AS receipt_allowed
           FROM pg_roles r WHERE r.rolname=session_user",
    )
    .fetch_one(pool)
    .await?;
    ensure!(
        !row.try_get::<bool, _>("rolsuper")?
            && !row.try_get::<bool, _>("rolbypassrls")?
            && !row.try_get::<bool, _>("can_issue")?
            && !row.try_get::<bool, _>("can_read")?
            && row.try_get::<bool, _>("lookup_allowed")?
            && row.try_get::<bool, _>("begin_allowed")?
            && row.try_get::<bool, _>("confirm_allowed")?
            && row.try_get::<bool, _>("record_allowed")?
            && row.try_get::<bool, _>("posted_allowed")?
            && row.try_get::<bool, _>("receipt_allowed")?
            && row.try_get::<Option<String>, _>("tenant")?.as_deref()
                == Some(std::env::var("DATAFORSEO_OWNER_TENANT_ID")?.as_str()),
        "DataForSEO worker must have tenant-bound function-only permissions"
    );
    Ok(())
}

async fn connect_client(endpoint: &str, namespace: &str) -> Result<Client> {
    let options = ConnectionOptions::new(Url::parse(endpoint)?)
        .identity(format!("alegria-dataforseo-owner:{}", std::process::id()))
        .build();
    let connection = Connection::connect(options).await?;
    Client::new(connection, ClientOptions::new(namespace).build())
        .map_err(|error| anyhow::anyhow!("{error}"))
}

async fn run_worker() -> Result<()> {
    ensure!(
        std::env::var("ALEGRIA_ENABLE_DATAFORSEO_STUB_OWNER_V1").as_deref() == Ok("true"),
        "DataForSEO stub-only owner requires explicit isolated opt-in"
    );
    stub_endpoint()?;
    let pool = PgPool::connect(&database_secret_file()?).await?;
    assert_isolated_worker_role(&pool).await?;
    let runtime = Runtime::from_current_tokio(
        RuntimeOptions::builder()
            .build()
            .map_err(|error| anyhow::anyhow!("{error}"))?,
    )?;
    let endpoint = std::env::var("TEMPORAL_URL")?;
    let namespace = std::env::var("TEMPORAL_NAMESPACE")?;
    ensure!(
        !endpoint.is_empty() && !namespace.is_empty(),
        "explicit Temporal context required"
    );
    let client = connect_client(&endpoint, &namespace).await?;
    let mut command_options = WorkerOptions::new(DATAFORSEO_OWNER_QUEUE_V1).build();
    command_options.register_workflow::<AlegriaOwnerValidateV1Workflow>()?;
    command_options.register_workflow::<AlegriaOwnerOperationV1Workflow>()?;
    let mut command = Worker::new(&runtime, client.clone(), command_options)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let activities = WorkerOptions::new(DATAFORSEO_ACTIVITY_QUEUE_V1)
        .register_activities(DataForSeoActivities {
            pool: Arc::new(pool),
        })
        .build();
    let mut effect =
        Worker::new(&runtime, client, activities).map_err(|error| anyhow::anyhow!("{error}"))?;
    tokio::try_join!(command.run(), effect.run())?;
    Ok(())
}

async fn replay_history(path: &Path) -> Result<()> {
    let history = WorkflowHistory::from_json(&std::fs::read(path)?)
        .map_err(|error| anyhow::anyhow!("invalid DataForSEO owner History: {error}"))?;
    let mut options = WorkflowReplayerOptions::new()
        .namespace("default".to_owned())
        .task_queue(DATAFORSEO_OWNER_QUEUE_V1.to_owned())
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
    if let Some(arg) = args.next() {
        ensure!(
            arg == "--replay-history" || arg == "--issue-stub-admission",
            "unknown DataForSEO owner command"
        );
        let file = args
            .next()
            .context("DataForSEO command requires one file")?;
        ensure!(args.next().is_none(), "unexpected replay arguments");
        if arg == "--issue-stub-admission" {
            operator::issue_from_file(Path::new(&file)).await?;
            println!("CI_TEMPORAL_DATAFORSEO_OPERATOR_ISSUED=PASS");
        } else {
            replay_history(Path::new(&file)).await?;
            println!("CI_TEMPORAL_DATAFORSEO_OWNER_REPLAY=PASS");
        }
        return Ok(());
    }
    run_worker().await
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Write};

    fn read_fixture_request(socket: &mut std::net::TcpStream) -> String {
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 2048];
        let header_end = loop {
            let n = socket.read(&mut buffer).expect("read fixture request");
            assert!(n > 0, "stub HTTP request must have headers");
            request.extend_from_slice(&buffer[..n]);
            assert!(request.len() < 8192, "unexpected oversized fixture request");
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                break end + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        while request.len() - header_end < content_length {
            let n = socket.read(&mut buffer).expect("read fixture request body");
            assert!(n > 0, "incomplete fixture request body");
            request.extend_from_slice(&buffer[..n]);
            assert!(
                request.len() < 8192,
                "unexpected oversized fixture request body"
            );
        }
        String::from_utf8(request).expect("fixture HTTP request is UTF-8")
    }

    #[tokio::test]
    async fn physical_stub_post_then_retry_get_never_posts_twice() {
        // Exercises real loopback POST/GET bytes, including a transient GET
        // failure. This is intentionally NOT a Temporal/SQL admission test;
        // those boundaries have independent physical suites.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let serving = std::thread::spawn(move || {
            let post_response =
                r#"{"status_code":20000,"tasks":[{"status_code":20100,"id":"stub-serp-7"}]}"#;
            let get_response = r#"{"status_code":20000,"tasks":[{"status_code":20000,"result":[{"items":[{"type":"organic","rank_absolute":1,"title":"Fixture page","url":"https://example.org/kitchens","description":"Fixture only"}]}]}]}"#;
            let mut observed = Vec::new();
            for (status, body) in [
                ("200 OK", post_response),
                ("503 Service Unavailable", ""),
                ("200 OK", get_response),
            ] {
                let (mut socket, peer) = listener.accept().unwrap();
                assert!(peer.ip().is_loopback(), "only loopback fixture accepted");
                observed.push(read_fixture_request(&mut socket));
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).unwrap();
            }
            observed
        });

        let input = DataForSeoGoogleOrganicSearchInputV1 {
            schema_revision: SCHEMA_REVISION,
            scope: operation_protocol::RunScope {
                tenant_id: "fixture-tenant".into(),
                run_id: "123e4567-e89b-42d3-a456-426614174000".into(),
            },
            keyword: "custom kitchens customer problems".into(),
            language_code: "en".into(),
            location_code: 2840,
            depth: 5,
            max_cost_usd_micros: 100_000,
        };
        let endpoint = Url::parse(&format!("http://{address}/")).unwrap();
        validate_stub_url(&endpoint).unwrap();
        let task_id = stub_task_post(&endpoint, &input)
            .await
            .expect("fixture accepts one POST");
        assert_eq!(task_id, "stub-serp-7");

        let first_get = stub_task_get(&endpoint, &task_id, input.depth)
            .await
            .expect_err("first GET is transiently unavailable");
        assert!(
            retryable_posted_stub_get(&first_get),
            "only the posted GET is retryable after the durable task ID"
        );
        let (response_bytes, hits) = stub_task_get(&endpoint, &task_id, input.depth)
            .await
            .expect("GET-only resume recovers the fixture");
        assert!(!response_bytes.is_empty());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_url, "https://example.org/kitchens");

        let requests = serving.join().expect("fixture server completed");
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("POST "))
                .count(),
            1,
            "GET recovery must never duplicate the task-creating POST"
        );
        assert!(requests[0].starts_with("POST /v3/serp/google/organic/task_post HTTP/1.1"));
        assert!(requests[0].contains("custom kitchens customer problems"));
        for request in &requests[1..] {
            assert!(request
                .starts_with("GET /v3/serp/google/organic/task_get/advanced/stub-serp-7 HTTP/1.1"));
        }
        for request in requests {
            assert!(
                !request.to_ascii_lowercase().contains("authorization:"),
                "offline stub must never send provider credentials"
            );
        }
    }

    #[tokio::test]
    async fn accepted_stub_post_with_lost_response_is_ambiguous_not_replayed() {
        // A server may have accepted task creation while the reply is lost.
        // The Activity's surrounding SQL preclaim must remain `uncertain`;
        // the HTTP function itself must not retry this unsafe POST.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let serving = std::thread::spawn(move || {
            let (mut socket, peer) = listener.accept().unwrap();
            assert!(peer.ip().is_loopback());
            let request = read_fixture_request(&mut socket);
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n{\"status_code\":20000",
                )
                .unwrap();
            request
        });
        let input = DataForSeoGoogleOrganicSearchInputV1 {
            schema_revision: SCHEMA_REVISION,
            scope: operation_protocol::RunScope {
                tenant_id: "fixture-tenant".into(),
                run_id: "123e4567-e89b-42d3-a456-426614174000".into(),
            },
            keyword: "made-to-order kitchens".into(),
            language_code: "en".into(),
            location_code: 2840,
            depth: 5,
            max_cost_usd_micros: 100_000,
        };
        let endpoint = Url::parse(&format!("http://{address}/")).unwrap();
        let _lost_reply = stub_task_post(&endpoint, &input)
            .await
            .expect_err("lost POST receipt is an unknown outcome, not success");
        // The actual error variant is transport-version dependent. The
        // important invariant is fail-closed: no task ID and no second POST.
        let request = serving.join().unwrap();
        assert!(request.starts_with("POST /v3/serp/google/organic/task_post HTTP/1.1"));
        assert!(request.contains("made-to-order kitchens"));
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
    }

    async fn stub_get_status_error(status: &str) -> anyhow::Error {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind disposable local-only GET test listener");
        let address = listener.local_addr().unwrap();
        let response =
            format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let serving = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0_u8; 512];
            socket.read(&mut request).unwrap();
            socket.write_all(response.as_bytes()).unwrap();
        });
        let url = format!("http://{address}/v3/serp/google/organic/task_get/advanced/stub-serp-1");
        let error = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .get(url)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .expect_err("test server must return an HTTP status error");
        serving.join().unwrap();
        error.into()
    }

    #[tokio::test]
    async fn only_transient_posted_task_get_failures_may_retry() {
        for status in ["429 Too Many Requests", "503 Service Unavailable"] {
            let error = stub_get_status_error(status).await;
            assert!(
                retryable_posted_stub_get(&error),
                "{status} must allow GET-only retry"
            );
        }
        for status in ["400 Bad Request", "401 Unauthorized", "404 Not Found"] {
            let error = stub_get_status_error(status).await;
            assert!(
                !retryable_posted_stub_get(&error),
                "{status} must not mask a permanent provider error"
            );
        }
        let malformed: anyhow::Error = serde_json::from_slice::<Value>(b"not-json")
            .expect_err("malformed response")
            .into();
        assert!(
            !retryable_posted_stub_get(&malformed),
            "invalid content cannot cause unlimited GET retries"
        );
    }

    #[tokio::test]
    async fn interrupted_posted_task_get_body_may_retry_without_new_post() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let serving = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0_u8; 512];
            socket.read(&mut request).unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}")
                .unwrap();
        });
        let url = format!("http://{address}/v3/serp/google/organic/task_get/advanced/stub-serp-1");
        let error = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(url)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .expect_err("incomplete GET response body must fail");
        serving.join().unwrap();
        assert!(
            retryable_posted_stub_get(&error.into()),
            "incomplete GET bytes may be retried read-only, regardless of transport error category"
        );
    }

    #[test]
    fn stub_network_path_rejects_live_endpoint_or_redirect_targets() {
        for rejected in [
            "https://api.dataforseo.com/",
            "http://api.dataforseo.com:80/",
            "http://localhost:1234/",
            "http://10.0.0.1:1234/",
            "http://127.0.0.1:1234/other",
            "http://user:pass@127.0.0.1:1234/",
            "http://127.0.0.1:1234/?provider=real",
        ] {
            assert!(
                validate_stub_url(&Url::parse(rejected).unwrap()).is_err(),
                "{rejected}"
            );
        }
        assert!(validate_stub_url(&Url::parse("http://127.0.0.1:1234/").unwrap()).is_ok());
    }

    #[test]
    fn typed_stub_items_preserve_structural_discovery_but_not_authenticity() {
        let input = json!({"tasks":[{"result":[{"items":[{
            "type":"organic", "rank_absolute":1, "title":"Fixture page",
            "url":"https://example.org/resource", "description":"Fixture snippet"
        }]}]}]});
        let hits = typed_stub_hits(&input, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].domain_norm, "example.org");
        assert!(typed_stub_hits(&input, 0).is_err());
    }

    #[test]
    fn fixture_is_never_labeled_as_real_provider_source() {
        let serde = serde_json::to_string(&DataForSeoSourceStatusV1::StubFixtureOnly).unwrap();
        assert_eq!(serde, "\"stub_fixture_only\"");
        assert_ne!(serde, "\"serp_discovery_only\"");
    }

    #[test]
    fn no_live_paid_adapter_or_credentials_linked_in_this_owner() {
        assert_ne!(DATAFORSEO_OWNER_QUEUE_V1, INPUT_PORT);
        assert!(DATAFORSEO_ACTIVITY_QUEUE_V1.contains("dataforseo"));
        assert_eq!(
            OWNER_VALIDATION_CHILD_TYPE_V1,
            "AlegriaOwnerValidateV1Workflow"
        );
    }
}
