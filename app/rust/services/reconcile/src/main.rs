use anyhow::Result;
use futures::StreamExt;
use infrastructure::adapters::sqlx_adapter::connect_pg;
use infrastructure::adapters::sqlx_runtime_health_adapter;
use infrastructure::adapters::temporalio_sdk_adapter::{connect_client, WorkflowListOptions};
use tracing::info;
use use_cases::pipeline_runtime::{
    append_reconcile_summary_action, append_reconcile_target_action, begin_reconcile_run,
    finish_reconcile_run, ReconcileSummary,
    ReconcileTargetReportRecord,
};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().json().init();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres_password@localhost:5433/alegria".to_string());
    let temporal_url = std::env::var("TEMPORAL_URL")
        .unwrap_or_else(|_| "http://localhost:7233".to_string());
    let pool = connect_pg(&database_url).await?;
    let reconcile_run_id = begin_reconcile_run(&pool).await?;

    let graph_before = use_cases::reconcile_graph::read_graph_backlog().await?;
    let qdrant_before = use_cases::reconcile_qdrant::read_qdrant_backlog().await?;

    let graph_report = use_cases::reconcile_graph::reconcile_graph().await?;
    let qdrant_report = use_cases::reconcile_qdrant::reconcile_qdrant().await?;

    let graph_after = use_cases::reconcile_graph::read_graph_backlog().await?;
    let qdrant_after = use_cases::reconcile_qdrant::read_qdrant_backlog().await?;
    let open_dlq = sqlx_runtime_health_adapter::count_open_dead_letters(&pool).await?;
    let stale_runs = sqlx_runtime_health_adapter::count_stale_runs(&pool).await?;
    let pending_hitl_runs = sqlx_runtime_health_adapter::count_pending_hitl_runs(&pool).await?;
    let stuck_steps = sqlx_runtime_health_adapter::count_stuck_steps(&pool).await?;
    let temporal_client = connect_client(
        &temporal_url,
        format!("alegria-reconcile@{}", std::process::id()),
        "default",
    )
    .await?;
    let mut stuck_workflows: i64 = 0;
    let mut stream = temporal_client.list_workflows(
        "ExecutionStatus = 'Running' AND TaskQueue = 'alegria-pipeline'",
        WorkflowListOptions::builder().limit(200).build(),
    );
    while let Some(item) = stream.next().await {
        let execution = item?;
        let started_at = execution.start_time().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let age = std::time::SystemTime::now()
            .duration_since(started_at)
            .unwrap_or_default();
        if age.as_secs() < 12 * 60 * 60 {
            continue;
        }
        let wf_id = execution.id().to_string();
        let is_pending_hitl = match uuid::Uuid::parse_str(&wf_id) {
            Ok(run_id) => sqlx_runtime_health_adapter::is_run_pending_hitl(&pool, run_id)
                .await
                .unwrap_or(false),
            Err(_) => false,
        };
        if !is_pending_hitl {
            stuck_workflows += 1;
        }
    }
    let summary = ReconcileSummary {
        open_dlq,
        stale_runs,
        pending_hitl_runs,
        stuck_steps: stuck_steps + stuck_workflows,
    };

    append_reconcile_target_action(
        &pool,
        reconcile_run_id,
        "reconcile_target",
        Some("neo4j"),
        None,
        &ReconcileTargetReportRecord {
            target_system: graph_report.target_system.clone(),
            dry_run: graph_report.dry_run,
            stale_candidates: graph_report.stale_candidates,
            failed_candidates: graph_report.failed_candidates,
            reset_stale_processing: graph_report.reset_stale_processing,
            requeued_failed: graph_report.requeued_failed,
        },
    )
    .await?;
    append_reconcile_target_action(
        &pool,
        reconcile_run_id,
        "reconcile_target",
        Some("qdrant"),
        None,
        &ReconcileTargetReportRecord {
            target_system: qdrant_report.target_system.clone(),
            dry_run: qdrant_report.dry_run,
            stale_candidates: qdrant_report.stale_candidates,
            failed_candidates: qdrant_report.failed_candidates,
            reset_stale_processing: qdrant_report.reset_stale_processing,
            requeued_failed: qdrant_report.requeued_failed,
        },
    )
    .await?;
    append_reconcile_summary_action(
        &pool,
        reconcile_run_id,
        "temporal_visibility_scan",
        Some("temporal"),
        None,
        &summary,
    )
    .await?;
    append_reconcile_summary_action(
        &pool,
        reconcile_run_id,
        "runtime_fail_safe_snapshot",
        None,
        None,
        &summary,
    )
    .await?;
    finish_reconcile_run(&pool, reconcile_run_id, "done", &summary, None).await?;

    info!(
        reconcile_run_id,
        target_system = graph_report.target_system,
        dry_run = graph_report.dry_run,
        stale_candidates = graph_report.stale_candidates,
        failed_candidates = graph_report.failed_candidates,
        reset_stale_processing = graph_report.reset_stale_processing,
        requeued_failed = graph_report.requeued_failed,
        pending_before = graph_before.0,
        failed_before = graph_before.1,
        pending_after = graph_after.0,
        failed_after = graph_after.1,
        "reconcile report"
    );
    info!(
        reconcile_run_id,
        target_system = qdrant_report.target_system,
        dry_run = qdrant_report.dry_run,
        stale_candidates = qdrant_report.stale_candidates,
        failed_candidates = qdrant_report.failed_candidates,
        reset_stale_processing = qdrant_report.reset_stale_processing,
        requeued_failed = qdrant_report.requeued_failed,
        pending_before = qdrant_before.0,
        failed_before = qdrant_before.1,
        pending_after = qdrant_after.0,
        failed_after = qdrant_after.1,
        "reconcile report"
    );
    info!(
        reconcile_run_id,
        open_dlq,
        stale_runs,
        pending_hitl_runs,
        stuck_steps,
        "runtime fail-safe snapshot"
    );

    Ok(())
}
