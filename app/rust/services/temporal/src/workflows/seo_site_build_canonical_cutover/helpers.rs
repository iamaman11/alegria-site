fn support_request(run_id: &str, site_input: &SeoSiteBuildInputPayload) -> WorkflowResult<String> {
    let scope = site_input
        .scope
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("SeoSiteBuildInputPayload.scope is required"))?;
    Ok(serde_json::to_string(&VerifiedSupportBundleRequest {
        run_id: run_id.to_string(),
        context_key: site_input.context_key.clone(),
        scope_signature: scope.scope_signature.clone(),
        applicant_profile: scope.applicant_profile.clone(),
    })
    .map_err(anyhow::Error::from)?)
}

fn phase_with_ordinal(key: SeoPhaseKey, page_index: usize, page_total: usize) -> String {
    format!("{}:{}/{}", phase_label(key), page_index + 1, page_total)
}

fn blocked_execution_plan_invariant_status(reason: &str) -> String {
    format!("blocked:execution_plan_invariant:{reason}")
}

fn block_execution_plan_invariant(
    ctx: &mut WorkflowContext<SeoSiteBuildCanonicalCutoverWorkflow>,
    run_id: &str,
    reason: &str,
) -> String {
    let status = blocked_execution_plan_invariant_status(reason);
    ctx.state_mut(|s| s.phase = status.clone());
    format!("seo_site_build_canonical_cutover_blocked run_id={run_id} status={status}")
}

async fn sync_target(
    ctx: &mut WorkflowContext<SeoSiteBuildCanonicalCutoverWorkflow>,
    run_id: &str,
    step_name: &str,
    target_system: &str,
) -> WorkflowResult<ProjectionSyncOutput> {
    Ok(ctx
        .start_activity(
            AlegriaActivities::run_projection_sync_step,
            ProjectionSyncInput {
                run_id: run_id.to_string(),
                step_name: step_name.to_string(),
                target_system: target_system.to_string(),
                batch_limit: 500,
                lease_seconds: 120,
            },
            db_opts(120),
        )
        .await?)
}
