use anyhow::{anyhow, Result};

pub use temporalio_client::{Client, ClientOptions, Connection, ConnectionOptions, WorkflowListOptions};
pub use temporalio_client::{
    UntypedQuery, UntypedSignal, UntypedUpdate, UntypedWorkflow, WorkflowExecuteUpdateOptions,
    WorkflowGetResultOptions, WorkflowQueryOptions, WorkflowSignalOptions, WorkflowStartOptions,
};
pub use temporalio_common;
pub use temporalio_common::data_converters::RawValue;
pub use temporalio_common::protos::temporal::api::enums::v1::WorkflowExecutionStatus;
pub use temporalio_common::protos::coresdk::AsJsonPayloadExt;
pub use temporalio_common::worker::{WorkerDeploymentOptions, WorkerDeploymentVersion};
pub use temporalio_macros::{
    activities, query, run, signal, update, update_validator, workflow, workflow_methods,
};
pub use temporalio_sdk::activities::{ActivityContext, ActivityError};
pub use temporalio_sdk::{
    ActivityOptions, SyncWorkflowContext, Worker, WorkerOptions, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};
pub use temporalio_sdk_core::{CoreRuntime, RuntimeOptions, Url};

pub fn build_runtime() -> Result<CoreRuntime> {
    let opts = RuntimeOptions::builder()
        .build()
        .map_err(|e| anyhow!(e))?;
    Ok(CoreRuntime::new_assume_tokio(opts)?)
}

pub async fn connect_client(temporal_url: &str, identity: String, namespace: &str) -> Result<Client> {
    let connection_opts = ConnectionOptions::new(Url::parse(temporal_url)?)
        .identity(identity)
        .build();
    let connection = Connection::connect(connection_opts).await?;
    Ok(Client::new(connection, ClientOptions::new(namespace).build())
        .map_err(|e| anyhow!("{e}"))?)
}
