//! Temporal client connectivity and workflow control.
//!
//! Workflow and Activity execution belong to individual registered worker
//! packages (`temporal_worker` for legacy, `temporal_orchestrator` for pure
//! DAGs). This crate owns only independently used client operations (CLI,
//! reconcile and E2E), so storage adapters do not acquire worker handlers.

use anyhow::{anyhow, Result};
use futures::StreamExt;
use primitives::errors::DomainError;
use seo_ports::SeoWorkflowControlPort;
use std::time::{Duration, SystemTime};
use temporalio_client::{
    Client, ClientOptions, Connection, ConnectionOptions, UntypedSignal, UntypedWorkflow,
    WorkflowListOptions, WorkflowSignalOptions,
};
use temporalio_common::data_converters::RawValue;
use temporalio_sdk_core::Url;

/// Provider-free pure-operation Workflow consumer. Never share the existing
/// `alegria-pipeline` queue with partial Activity sets or legacy V1/V2 workers.
/// Changing this name requires a versioned cutover of durable Temporal histories.
pub const PURE_DAG_ORCHESTRATOR_QUEUE: &str = "alegria-pure-dag-v3";

/// Distinct opt-in parent Task Queue for admitted mixed-operation Workflows.
/// This owner-free worker starts only versioned PG child Workflows, never
/// executes their PostgreSQL Activities or bypasses independent admissions.
pub const EFFECT_DAG_ORCHESTRATOR_V4_QUEUE: &str = "alegria-effect-dag-v4";
/// V5 admitted effect DAG fanout/fanin parent; never poll V4 histories here.
pub const EFFECT_DAG_ORCHESTRATOR_V5_QUEUE: &str = "alegria-effect-dag-v5";

pub async fn connect_client(
    temporal_url: &str,
    identity: String,
    namespace: &str,
) -> Result<Client> {
    let connection_opts = ConnectionOptions::new(Url::parse(temporal_url)?)
        .identity(identity)
        .build();
    let connection = Connection::connect(connection_opts).await?;
    Client::new(connection, ClientOptions::new(namespace).build()).map_err(|e| anyhow!("{e}"))
}

/// Read Temporal liveness only. The caller owns the decision whether a stale
/// run is legitimate (for example, blocked on a human review in PostgreSQL).
pub async fn running_workflow_ids_older_than(
    client: &Client,
    task_queue: &str,
    threshold: Duration,
) -> Result<Vec<String>> {
    if task_queue.is_empty()
        || !task_queue
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return Err(anyhow!("invalid Temporal task queue name"));
    }
    let query = format!("ExecutionStatus = 'Running' AND TaskQueue = '{task_queue}'");
    let mut stream =
        client.list_workflows(&query, WorkflowListOptions::builder().limit(200).build());
    let mut workflows = Vec::new();
    while let Some(item) = stream.next().await {
        let execution = item?;
        let started_at = execution.start_time().unwrap_or(SystemTime::UNIX_EPOCH);
        if SystemTime::now()
            .duration_since(started_at)
            .unwrap_or_default()
            >= threshold
        {
            workflows.push(execution.id().to_string());
        }
    }
    Ok(workflows)
}

pub struct TemporalSeoWorkflowControlAdapter {
    temporal_url: String,
    client_name: String,
    namespace: String,
}

impl TemporalSeoWorkflowControlAdapter {
    pub fn new(temporal_url: String, client_name: String, namespace: String) -> Self {
        Self {
            temporal_url,
            client_name,
            namespace,
        }
    }

    async fn signal_named(&self, workflow_id: &str, signal_name: &str) -> Result<(), DomainError> {
        let client = connect_client(
            &self.temporal_url,
            self.client_name.clone(),
            &self.namespace,
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("failed to connect temporal client: {err}"),
        })?;
        let handle = client.get_workflow_handle::<UntypedWorkflow>(workflow_id.to_string());
        handle
            .signal(
                UntypedSignal::<UntypedWorkflow>::new(signal_name),
                RawValue::default(),
                WorkflowSignalOptions::default(),
            )
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("failed to send {signal_name} signal: {err}"),
            })?;
        Ok(())
    }

    pub async fn signal_llm_result_ready(&self, workflow_id: &str) -> Result<(), DomainError> {
        self.signal_named(workflow_id, "llm_result_ready").await
    }
}

#[async_trait::async_trait]
impl SeoWorkflowControlPort for TemporalSeoWorkflowControlAdapter {
    async fn signal_resume(&self, workflow_id: &str) -> Result<(), DomainError> {
        self.signal_named(workflow_id, "resume").await
    }
}
