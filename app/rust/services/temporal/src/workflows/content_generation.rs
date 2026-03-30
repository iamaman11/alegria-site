use infrastructure::adapters::temporalio_sdk_adapter::{
    workflow, workflow_methods, SyncWorkflowContext, WorkerOptions, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};

use crate::activities::AlegriaActivities;
use crate::metrics;

use super::runtime::{db_opts, fast_opts, llm_opts, BasicWorkflowStatus};

#[workflow]
#[derive(Default)]
struct ContentGenerationWorkflow {
    paused: bool,
    phase: String,
}

pub(crate) fn register(opts: &mut WorkerOptions) {
    opts.register_workflow::<ContentGenerationWorkflow>();
}

#[workflow_methods]
impl ContentGenerationWorkflow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        metrics::global()
            .workflow_starts_total
            .with_label_values(&["ContentGenerationWorkflow"])
            .inc();
        let run_id: String = ctx.workflow_initial_info().workflow_id.clone();

        ctx.state_mut(|s| s.phase = "generate_content".to_string());
        let run_id: String = ctx
            .start_activity(AlegriaActivities::generate_content, run_id, llm_opts(120))
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "validate_blocks".to_string());
        let run_id: String = ctx
            .start_activity(AlegriaActivities::validate_blocks, run_id, fast_opts(30))
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "finalize_run".to_string());
        let run_id: String = ctx
            .start_activity(AlegriaActivities::finalize_run, run_id, db_opts(10))
            .await?;
        ctx.state_mut(|s| s.phase = "done".to_string());
        metrics::global()
            .workflow_completions_total
            .with_label_values(&["ContentGenerationWorkflow"])
            .inc();

        Ok(run_id)
    }

    #[signal(name = "pause")]
    fn pause(&mut self, _ctx: &mut SyncWorkflowContext<Self>, reason: Option<String>) {
        self.paused = true;
        self.phase = format!("paused:{}", reason.unwrap_or_else(|| "manual".to_string()));
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

    #[update(name = "set_pause")]
    fn set_pause(&mut self, _ctx: &mut SyncWorkflowContext<Self>, paused: bool) -> bool {
        self.paused = paused;
        self.paused
    }
}
