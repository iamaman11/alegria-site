use super::*;

/// This gate is independent of editorial HITL. Its durable source of truth is
/// the persisted candidate job, not the wakeup signal carried by Temporal.
pub(super) async fn await_manual_truth_candidates(
    ctx: &mut WorkflowContext<SeoSiteBuildCanonicalCutoverWorkflow>,
    run_id: &str,
    validation_input: &CandidateValidationInput,
) -> WorkflowResult<()> {
    ctx.state_mut(|s| s.phase = "manual_truth_prepare".to_string());
    let manual_state: String = ctx
        .start_activity(
            AlegriaActivities::prepare_manual_truth_candidates,
            validation_input.clone(),
            db_opts(60),
        )
        .await?;
    if manual_state == "awaiting_manual" {
        ctx.state_mut(|s| s.phase = "manual_truth_wait".to_string());
        loop {
            // Saved task state wins over an ephemeral wakeup signal.
            // Replay/Reset must resume completed manual work immediately.
            let status: String = ctx
                .start_activity(
                    AlegriaActivities::load_manual_truth_status,
                    run_id.to_string(),
                    db_opts(30),
                )
                .await?;
            if status == "done" {
                break;
            }
            if status != "awaiting_manual" {
                return Err(super::super::runtime::workflow_contract_failure(
                    "manual truth job status invalid",
                ));
            }
            ctx.wait_condition(|s| !s.paused && s.llm_result_ready)
                .await?;
            ctx.state_mut(|s| s.llm_result_ready = false);
        }
    } else if manual_state != "done" {
        return Err(super::super::runtime::workflow_contract_failure(
            "manual truth preparation status invalid",
        ));
    }
    ctx.wait_condition(|s| !s.paused).await?;
    Ok(())
}
