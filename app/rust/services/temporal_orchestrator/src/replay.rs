//! Offline official Temporal SDK history replay for the provider-free package.
//! No DB, provider SDK, Activity registration, poller or network connection.

use super::{
    EFFECT_DAG_ORCHESTRATOR_V4_QUEUE, EFFECT_DAG_ORCHESTRATOR_V5_QUEUE,
    PURE_DAG_ORCHESTRATOR_QUEUE, PURE_DAG_ORCHESTRATOR_V4_QUEUE,
    PURE_DAG_ORCHESTRATOR_V5_QUEUE,
};
use std::path::Path;

use anyhow::{Context, Result};
use operation_protocol::{OPERATION_DAG_PARENT_QUEUE, OPERATION_DAG_PARENT_QUEUE_V2};
#[cfg(feature = "legacy-qdrant-v1")]
use qdrant_contracts::QDRANT_DAG_PARENT_QUEUE_V1;
use temporalio_client::WorkflowHistory;
use temporalio_sdk::workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions};

#[cfg(all(test, feature = "legacy-workflows"))]
pub(crate) async fn replay_history(path: &Path) -> Result<()> {
    replay_history_for_queue(path, PURE_DAG_ORCHESTRATOR_QUEUE).await
}

pub(crate) async fn replay_history_for_queue(path: &Path, queue: &str) -> Result<()> {
    #[cfg(not(feature = "legacy-workflows"))]
    anyhow::ensure!(
        queue != PURE_DAG_ORCHESTRATOR_QUEUE
            && queue != EFFECT_DAG_ORCHESTRATOR_V4_QUEUE
            && queue != EFFECT_DAG_ORCHESTRATOR_V5_QUEUE,
        "historical site Workflow replay requires legacy-workflows binary feature"
    );
    #[cfg(not(feature = "legacy-workflows"))]
    anyhow::ensure!(
        queue != PURE_DAG_ORCHESTRATOR_V4_QUEUE
            && queue != PURE_DAG_ORCHESTRATOR_V5_QUEUE,
        "strict pure V4/V5 replay requires legacy-workflows binary feature"
    );
    #[cfg(feature = "legacy-qdrant-v1")]
    let qdrant_v1 = queue == QDRANT_DAG_PARENT_QUEUE_V1;
    #[cfg(not(feature = "legacy-qdrant-v1"))]
    let qdrant_v1 = false;
    anyhow::ensure!(
        queue == PURE_DAG_ORCHESTRATOR_QUEUE
            || queue == PURE_DAG_ORCHESTRATOR_V4_QUEUE
            || queue == PURE_DAG_ORCHESTRATOR_V5_QUEUE
            || queue == EFFECT_DAG_ORCHESTRATOR_V4_QUEUE
            || queue == EFFECT_DAG_ORCHESTRATOR_V5_QUEUE
            || qdrant_v1
            || queue == OPERATION_DAG_PARENT_QUEUE
            || queue == OPERATION_DAG_PARENT_QUEUE_V2,
        "unknown replay Task Queue"
    );
    let bytes = std::fs::read(path)
        .with_context(|| format!("read actual Temporal history {}", path.display()))?;
    let history = WorkflowHistory::from_json(&bytes)
        .map_err(|error| anyhow::anyhow!("decode Temporal history: {error}"))?;
    if queue == OPERATION_DAG_PARENT_QUEUE || queue == OPERATION_DAG_PARENT_QUEUE_V2 {
        // Replay must use the same trusted snapshot as execution. A different
        // process catalog is a *failure*, never authority to reroute effects.
        let path = std::env::var("TEMPORAL_OPERATION_DAG_CATALOG_PATH")
            .context("generic DAG replay requires a trusted route catalog path")?;
        let catalog = super::owner_catalog::load_trusted_owner_catalog(Path::new(&path))?;
        super::workflow::install_owner_catalog(catalog)?;
    }
    let mut options = WorkflowReplayerOptions::new()
        .namespace("default".to_string())
        .task_queue(queue.to_string())
        .build();
    if queue == PURE_DAG_ORCHESTRATOR_V5_QUEUE {
        #[cfg(feature = "legacy-workflows")]
        super::workflow::register_for_replay_pure_v5(&mut options)?;
        #[cfg(not(feature = "legacy-workflows"))]
        anyhow::bail!("strict pure V5 replay requires legacy-workflows binary feature");
    } else if queue == PURE_DAG_ORCHESTRATOR_V4_QUEUE {
        #[cfg(feature = "legacy-workflows")]
        super::workflow::register_for_replay_pure_v4(&mut options)?;
        #[cfg(not(feature = "legacy-workflows"))]
        anyhow::bail!("strict pure V4 replay requires legacy-workflows binary feature");
    } else if queue == OPERATION_DAG_PARENT_QUEUE_V2 {
        super::workflow::register_for_replay_operation_dag_v2(&mut options)?;
    } else if queue == OPERATION_DAG_PARENT_QUEUE {
        super::workflow::register_for_replay_operation_dag_v1(&mut options)?;
    } else if qdrant_v1 {
        #[cfg(feature = "legacy-qdrant-v1")]
        super::workflow::register_for_replay_qdrant_v1(&mut options)?;
    } else if queue == EFFECT_DAG_ORCHESTRATOR_V5_QUEUE {
        #[cfg(feature = "legacy-workflows")]
        super::workflow::register_for_replay_v5(&mut options)?;
        #[cfg(not(feature = "legacy-workflows"))]
        anyhow::bail!("historical V5 replay requires legacy-workflows binary feature");
    } else if queue == EFFECT_DAG_ORCHESTRATOR_V4_QUEUE {
        #[cfg(feature = "legacy-workflows")]
        super::workflow::register_for_replay_v4(&mut options)?;
        #[cfg(not(feature = "legacy-workflows"))]
        anyhow::bail!("historical V4 replay requires legacy-workflows binary feature");
    } else {
        #[cfg(feature = "legacy-workflows")]
        super::workflow::register_for_replay(&mut options)?;
        #[cfg(not(feature = "legacy-workflows"))]
        anyhow::bail!("historical pure DAG replay requires legacy-workflows binary feature");
    }
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

    #[cfg(not(feature = "legacy-workflows"))]
    #[tokio::test]
    async fn default_generic_replay_rejects_all_frozen_site_queues() {
        let missing = std::env::temp_dir().join("not-a-real-temporal-history.json");
        for queue in [
            PURE_DAG_ORCHESTRATOR_QUEUE,
            PURE_DAG_ORCHESTRATOR_V4_QUEUE,
            PURE_DAG_ORCHESTRATOR_V5_QUEUE,
            EFFECT_DAG_ORCHESTRATOR_V4_QUEUE,
            EFFECT_DAG_ORCHESTRATOR_V5_QUEUE,
        ] {
            let error = replay_history_for_queue(&missing, queue).await.unwrap_err();
            assert!(error.to_string().contains("legacy-workflows"), "{queue}");
        }
    }

    #[cfg(feature = "legacy-workflows")]
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

    #[tokio::test]
    async fn generic_replay_has_one_stable_queue_and_no_obsolete_mixed_v6_alias() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let missing = std::env::temp_dir().join(format!(
            "alegria-generic-dag-missing-{}-{unique}",
            std::process::id(),
        ));
        let obsolete = replay_history_for_queue(&missing, "alegria-mixed-owner-dag-v6")
            .await
            .unwrap_err();
        assert!(obsolete.to_string().contains("unknown replay Task Queue"));

        // The generic queue is recognized, but an actual SDK History is
        // mandatory. A missing source must never be replayed as an invented
        // successful result or reinterpreted as an old workflow version.
        let absent = replay_history_for_queue(&missing, OPERATION_DAG_PARENT_QUEUE)
            .await
            .unwrap_err();
        assert!(absent.to_string().contains("read actual Temporal history"));

        let absent_v2 = replay_history_for_queue(&missing, OPERATION_DAG_PARENT_QUEUE_V2)
            .await
            .unwrap_err();
        assert!(absent_v2
            .to_string()
            .contains("read actual Temporal history"));
    }
}
