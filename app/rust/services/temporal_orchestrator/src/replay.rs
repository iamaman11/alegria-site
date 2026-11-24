//! Offline official Temporal SDK history replay for the provider-free package.
//! No DB, provider SDK, Activity registration, poller or network connection.

use std::path::Path;

use anyhow::{Context, Result};
use temporal_gateway::PURE_DAG_ORCHESTRATOR_QUEUE;
use temporalio_client::WorkflowHistory;
use temporalio_sdk::workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions};

pub(crate) async fn replay_history(path: &Path) -> Result<()> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("read actual Temporal history {}", path.display()))?;
    let history = WorkflowHistory::from_json(&bytes)
        .map_err(|error| anyhow::anyhow!("decode Temporal history: {error}"))?;
    let mut options = WorkflowReplayerOptions::new()
        .namespace("default".to_string())
        .task_queue(PURE_DAG_ORCHESTRATOR_QUEUE.to_string())
        .build();
    super::workflow::register_for_replay(&mut options)?;
    WorkflowReplayer::new(options)
        .map_err(|error| anyhow::anyhow!("construct Temporal replayer: {error}"))?
        .replay_workflow(history)
        .await
        .map_err(|error| anyhow::anyhow!("Temporal history replay failed: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_or_missing_history_fails_offline() {
        let missing =
            std::env::temp_dir().join(format!("alegria-pure-dag-missing-{}", std::process::id()));
        assert!(replay_history(&missing).await.is_err());
        let corrupt = std::env::temp_dir().join(format!(
            "alegria-pure-dag-corrupt-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&corrupt, b"not Temporal History").unwrap();
        let error = replay_history(&corrupt).await.unwrap_err();
        std::fs::remove_file(&corrupt).unwrap();
        assert!(error.to_string().contains("decode Temporal history"));
    }
}
