//! Replay saved Temporal server history against the actually registered code.
//!
//! This is an offline acceptance boundary: it must not start PostgreSQL,
//! metrics, Activity workers, or any network client. History comes from a
//! completed real Temporal run and is read verbatim from the test fixture.

use std::path::Path;

use anyhow::{Context, Result};
use temporalio_client::WorkflowHistory;
use temporalio_sdk::workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions};

pub(crate) async fn replay_history(path: &Path) -> Result<()> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("read actual Temporal history {}", path.display()))?;
    let history = WorkflowHistory::from_json(&bytes)
        .map_err(|error| anyhow::anyhow!("decode Temporal history: {error}"))?;

    // Both workflow definitions live in this binary. Keeping V1 registered is
    // necessary for histories started before the V2 command-plan rollout.
    let mut options = WorkflowReplayerOptions::new()
        .namespace("default".to_string())
        .task_queue("alegria-pipeline".to_string())
        .build();
    super::seo_site_build_canonical_cutover::register_for_replay(&mut options)
        .context("register canonical V1 workflow for history replay")?;
    super::seo_site_build_composable_v2::register_for_replay(&mut options)
        .context("register composable V2 workflow for history replay")?;
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
    async fn absent_history_fails_before_any_runtime_connection() {
        let missing = std::env::temp_dir().join(format!(
            "alegria-missing-history-{}-{}.json",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let error = replay_history(&missing).await.unwrap_err();
        assert!(error.to_string().contains("read actual Temporal history"));
    }

    #[tokio::test]
    async fn corrupt_history_is_rejected_without_starting_worker() {
        let path = std::env::temp_dir().join(format!(
            "alegria-corrupt-history-{}-{}.json",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, b"not a Temporal workflow history").unwrap();
        let error = replay_history(&path).await.unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(error.to_string().contains("decode Temporal history"));
    }
}
