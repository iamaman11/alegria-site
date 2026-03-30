use contracts::generated::alegria::temporal::v1::{
    HitlDecision, HitlPauseInfo, HitlResolutionInput, StepContractMeta,
};
use infrastructure::adapters::temporalio_sdk_adapter::{
    workflow, workflow_methods, SyncWorkflowContext, WorkerOptions, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};

use crate::activities::AlegriaActivities;
use crate::metrics;

use super::runtime::{db_opts, fast_opts, llm_opts, WorkflowStatus};

#[workflow]
#[derive(Default)]
struct FactExtractionWorkflow {
    paused: bool,
    waiting_hitl: bool,
    hitl_task_id: Option<i64>,
    hitl_resolution: Option<HitlDecision>,
    phase: String,
}

pub(crate) fn register(opts: &mut WorkerOptions) {
    opts.register_workflow::<FactExtractionWorkflow>();
}

#[workflow_methods]
impl FactExtractionWorkflow {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<String> {
        metrics::global()
            .workflow_starts_total
            .with_label_values(&["FactExtractionWorkflow"])
            .inc();
        let run_id: String = ctx.workflow_initial_info().workflow_id.clone();

        ctx.state_mut(|s| s.phase = "extract_facts".to_string());
        let run_id: String = ctx
            .start_activity(AlegriaActivities::extract_facts, run_id, llm_opts(180))
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "verify_rules".to_string());
        let run_id: String = ctx
            .start_activity(AlegriaActivities::verify_rules, run_id, fast_opts(30))
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "prepare_hitl_pause".to_string());
        let hitl_info: HitlPauseInfo = ctx
            .start_activity(
                AlegriaActivities::prepare_hitl_pause,
                run_id.clone(),
                db_opts(30),
            )
            .await?;

        if hitl_info.requires_hitl {
            let task_id = hitl_info.task_id;
            ctx.state_mut(|s| {
                s.waiting_hitl = true;
                s.hitl_task_id = Some(task_id);
                s.paused = true;
                s.phase = "hitl_wait".to_string();
            });
            ctx.wait_condition(|s| !s.paused && s.hitl_resolution.is_some())
                .await;

            let resolution_json = ctx
                .state(|s| s.hitl_resolution.clone())
                .expect("missing HITL resolution after HITL wait_condition");
            ctx.state_mut(|s| s.phase = "apply_hitl_resolution".to_string());
            let run_id: String = ctx
                .start_activity(
                    AlegriaActivities::apply_hitl_resolution,
                    HitlResolutionInput {
                        meta: Some(StepContractMeta {
                            run_id: run_id.clone(),
                            step_name: "apply_hitl_resolution".to_string(),
                            schema_version: 1,
                            input_hash: String::new(),
                            output_hash: String::new(),
                            idempotency_key: String::new(),
                            requires_hitl: true,
                            prompt_version: String::new(),
                            model_version: String::new(),
                            registry_version: String::new(),
                            error_class: String::new(),
                        }),
                        task_id,
                        resolution: Some(resolution_json),
                    },
                    db_opts(30),
                )
                .await?;
            ctx.state_mut(|s| {
                s.waiting_hitl = false;
                s.hitl_task_id = None;
                s.hitl_resolution = None;
                s.phase = "hitl_resolved".to_string();
            });
            ctx.wait_condition(|s| !s.paused).await;
            let _ = run_id;
        }

        ctx.state_mut(|s| s.phase = "persist_and_emit".to_string());
        let run_id: String = ctx
            .start_activity(AlegriaActivities::persist_and_emit, run_id, db_opts(30))
            .await?;
        ctx.wait_condition(|s| !s.paused).await;

        ctx.state_mut(|s| s.phase = "finalize_run".to_string());
        let run_id: String = ctx
            .start_activity(AlegriaActivities::finalize_run, run_id, db_opts(10))
            .await?;
        ctx.state_mut(|s| s.phase = "done".to_string());
        metrics::global()
            .workflow_completions_total
            .with_label_values(&["FactExtractionWorkflow"])
            .inc();

        Ok(run_id)
    }

    #[signal(name = "pause")]
    fn pause(&mut self, _ctx: &mut SyncWorkflowContext<Self>, reason: Option<String>) {
        self.paused = true;
        self.phase = format!("paused:{}", reason.unwrap_or_else(|| "manual".to_string()));
    }

    #[signal(name = "resume")]
    fn resume(&mut self, _ctx: &mut SyncWorkflowContext<Self>, resolution: Option<HitlDecision>) {
        self.paused = false;
        if self.waiting_hitl {
            self.hitl_resolution = resolution;
            self.phase = if self.hitl_resolution.is_some() {
                "resumed_hitl".to_string()
            } else {
                "hitl_resolution_missing".to_string()
            };
        } else {
            self.phase = "resumed".to_string();
        }
    }

    #[query(name = "status")]
    fn status(&self, _ctx: &WorkflowContextView) -> WorkflowStatus {
        WorkflowStatus {
            phase: self.phase.clone(),
            paused: self.paused,
            waiting_hitl: self.waiting_hitl,
            hitl_task_id: self.hitl_task_id,
            has_hitl_resolution: self.hitl_resolution.is_some(),
        }
    }

    #[update(name = "set_pause")]
    fn set_pause(&mut self, _ctx: &mut SyncWorkflowContext<Self>, paused: bool) -> bool {
        self.paused = paused;
        if paused {
            self.phase = "paused:update".to_string();
        } else if self.phase.starts_with("paused") {
            self.phase = "resumed:update".to_string();
        }
        self.paused
    }
}
