use contracts::generated::alegria::temporal::v1::{
    FinalizePublishInputPayload, FinalizePublishOutputPayload,
};

pub fn execute(input: &FinalizePublishInputPayload) -> FinalizePublishOutputPayload {
    let validation = input.render_validation.clone().unwrap_or_default();
    let publish_artifact = input.publish_artifact.clone();
    // A render-ready artifact is not yet a published revision. The repository
    // must independently verify public HTTPS bytes before returning success.
    let gate = seo_domain::publishing::ReleaseReadiness::from_render_validation(
        &validation.verdict,
        &validation.blocking_reasons,
    );
    FinalizePublishOutputPayload {
        verdict: gate.verdict().to_string(),
        current_status: gate.current_status().to_string(),
        blocking_reasons: validation.blocking_reasons,
        publish_artifact,
    }
}
