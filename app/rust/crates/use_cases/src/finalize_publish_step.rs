use contracts::generated::alegria::temporal::v1::{
    FinalizePublishInputPayload, FinalizePublishOutputPayload,
};

pub fn execute(input: &FinalizePublishInputPayload) -> FinalizePublishOutputPayload {
    let validation = input.render_validation.clone().unwrap_or_default();
    let publish_artifact = input.publish_artifact.clone();
    let verdict = if validation.verdict == "render_ready" {
        "published"
    } else {
        "blocked"
    };
    FinalizePublishOutputPayload {
        verdict: verdict.to_string(),
        current_status: if verdict == "published" {
            "published".to_string()
        } else {
            "blocked".to_string()
        },
        blocking_reasons: validation.blocking_reasons,
        publish_artifact,
    }
}
