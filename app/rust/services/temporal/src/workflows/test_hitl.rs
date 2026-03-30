use infrastructure::adapters::temporalio_sdk_adapter::{
    workflow, workflow_methods, SyncWorkflowContext, WorkerOptions, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};

use crate::activities::AlegriaActivities;
use crate::metrics;

use super::runtime::{test_opts, BasicWorkflowStatus};

#[workflow]
#[derive(Default)]
struct TestHitlWorkflow {
    paused: bool,
    waiting_hitl: bool,
    resume_requested: bool,
    phase: String,
}

pub(crate) fn register(opts: &mut WorkerOptions) {
    opts.register_workflow::<TestHitlWorkflow>();
}

#[workflow_methods]
impl TestHitlWorkflow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        metrics::global()
            .workflow_starts_total
            .with_label_values(&["TestHitlWorkflow"])
            .inc();
        let workflow_id: String = ctx.workflow_initial_info().workflow_id.clone();

        ctx.state_mut(|s| s.phase = "test_step_prepare".to_string());
        let prepared: String = ctx
            .start_activity(
                AlegriaActivities::test_step_prepare,
                workflow_id,
                test_opts(10),
            )
            .await?;

        ctx.state_mut(|s| {
            s.waiting_hitl = true;
            if s.resume_requested {
                s.paused = false;
                s.phase = "hitl_wait_skipped_resume_queued".to_string();
            } else {
                s.paused = true;
                s.phase = "hitl_wait".to_string();
            }
        });
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| {
            s.waiting_hitl = false;
            s.resume_requested = false;
            s.phase = "test_step_finalize".to_string();
        });
        let out: String = ctx
            .start_activity(
                AlegriaActivities::test_step_finalize,
                prepared,
                test_opts(10),
            )
            .await?;

        ctx.state_mut(|s| s.phase = "done".to_string());
        metrics::global()
            .workflow_completions_total
            .with_label_values(&["TestHitlWorkflow"])
            .inc();
        Ok(out)
    }

    #[signal(name = "pause")]
    fn pause(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.paused = true;
        self.resume_requested = false;
        self.phase = "paused".to_string();
    }

    #[signal(name = "resume")]
    fn resume(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.paused = false;
        self.resume_requested = true;
        if self.waiting_hitl {
            self.phase = "resumed_hitl".to_string();
        } else {
            self.phase = "resume_queued".to_string();
        }
    }

    #[query(name = "status")]
    fn status(&self, _ctx: &WorkflowContextView) -> BasicWorkflowStatus {
        BasicWorkflowStatus {
            phase: self.phase.clone(),
            paused: self.paused,
        }
    }
}
