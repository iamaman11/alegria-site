use contracts::generated::alegria::temporal::v1::ReconcileTargetInputPayload;
use infrastructure::adapters::temporalio_sdk_adapter::{
    workflow, workflow_methods, SyncWorkflowContext, WorkerOptions, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};
use runtime_models::ReconcileTargetReportRecord;

use crate::activities::AlegriaActivities;
use crate::metrics;

use super::runtime::{db_opts, BasicWorkflowStatus};

#[workflow]
#[derive(Default)]
struct ProjectionReconcileWorkflow {
    paused: bool,
    phase: String,
}

pub(crate) fn register(opts: &mut WorkerOptions) {
    opts.register_workflow::<ProjectionReconcileWorkflow>();
}

async fn reconcile_target(
    ctx: &mut WorkflowContext<ProjectionReconcileWorkflow>,
    run_id: &str,
    target_system: &str,
) -> WorkflowResult<ReconcileTargetReportRecord> {
    Ok(ctx
        .start_activity(
            AlegriaActivities::run_projection_reconcile_step,
            ReconcileTargetInputPayload {
                run_id: run_id.to_string(),
                target_system: target_system.to_string(),
                dry_run: false,
                max_retry_count: 10,
                batch_limit: 500,
                requeue_base_delay_sec: 0,
                requeue_jitter_sec: 15,
            },
            db_opts(120),
        )
        .await?)
}

#[workflow_methods]
impl ProjectionReconcileWorkflow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        metrics::global()
            .workflow_starts_total
            .with_label_values(&["ProjectionReconcileWorkflow"])
            .inc();

        let run_id = ctx.workflow_initial_info().workflow_id.clone();
        ctx.state_mut(|s| s.phase = "projection_reconcile.neo4j".to_string());
        let neo4j = reconcile_target(ctx, &run_id, "neo4j").await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "projection_reconcile.qdrant".to_string());
        let qdrant = reconcile_target(ctx, &run_id, "qdrant").await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "done:projection_reconcile".to_string());
        metrics::global()
            .workflow_completions_total
            .with_label_values(&["ProjectionReconcileWorkflow"])
            .inc();

        Ok(format!(
            "projection_reconcile_ok run_id={} neo4j_failed={} qdrant_failed={} neo4j_requeued={} qdrant_requeued={}",
            run_id,
            neo4j.failed_candidates,
            qdrant.failed_candidates,
            neo4j.requeued_failed,
            qdrant.requeued_failed
        ))
    }

    #[signal(name = "pause")]
    fn pause(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.paused = true;
        self.phase = "paused".to_string();
    }

    #[signal(name = "resume")]
    fn resume(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.paused = false;
        self.phase = "resumed".to_string();
    }

    #[query(name = "status")]
    fn status(&self, _ctx: &WorkflowContextView) -> BasicWorkflowStatus {
        BasicWorkflowStatus {
            phase: self.phase.clone(),
            paused: self.paused,
        }
    }
}
