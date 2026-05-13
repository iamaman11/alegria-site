use super::temporalio_sdk_adapter::{
    connect_client, RawValue, UntypedSignal, UntypedWorkflow, WorkflowSignalOptions,
};
use primitives::errors::DomainError;
use seo_ports::SeoWorkflowControlPort;

fn empty_payload() -> RawValue {
    RawValue::default()
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
}

#[async_trait::async_trait]
impl SeoWorkflowControlPort for TemporalSeoWorkflowControlAdapter {
    async fn signal_resume(&self, workflow_id: &str) -> Result<(), DomainError> {
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
                UntypedSignal::<UntypedWorkflow>::new("resume"),
                empty_payload(),
                WorkflowSignalOptions::default(),
            )
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("failed to send resume signal: {err}"),
            })?;
        Ok(())
    }
}
