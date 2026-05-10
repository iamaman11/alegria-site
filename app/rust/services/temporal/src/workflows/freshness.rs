use infrastructure::adapters::temporalio_sdk_adapter::{
    workflow, workflow_methods, SyncWorkflowContext, WorkerOptions, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};

use crate::activities::AlegriaActivities;
use crate::metrics;

use super::runtime::{db_opts, BasicWorkflowStatus};

#[workflow]
#[derive(Default)]
struct FreshnessCheckWorkflow {
    paused: bool,
    phase: String,
}

pub(crate) fn register(opts: &mut WorkerOptions) {
    opts.register_workflow::<FreshnessCheckWorkflow>();
}

#[workflow_methods]
impl FreshnessCheckWorkflow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        metrics::global()
            .workflow_starts_total
            .with_label_values(&["FreshnessCheckWorkflow"])
            .inc();
        ctx.state_mut(|s| s.phase = "check_data_freshness".to_string());
        let threshold = "24".to_string();
        let report: String = ctx
            .start_activity(
                AlegriaActivities::check_data_freshness,
                threshold,
                db_opts(30),
            )
            .await?;
        ctx.wait_condition(|s| !s.paused).await;
        ctx.state_mut(|s| s.phase = "done".to_string());
        metrics::global()
            .workflow_completions_total
            .with_label_values(&["FreshnessCheckWorkflow"])
            .inc();
        Ok(report)
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
