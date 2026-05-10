use contracts::generated::alegria::temporal::v1::{
    PublishArtifact, PublishMaterializeInputPayload, PublishMaterializeOutputPayload,
};

pub fn execute(input: &PublishMaterializeInputPayload) -> PublishMaterializeOutputPayload {
    let mut blocking_reasons = Vec::new();
    if input.page_node_key.trim().is_empty() {
        blocking_reasons.push("missing_page_node_key".to_string());
    }
    if input.revision_id.trim().is_empty() {
        blocking_reasons.push("missing_revision_id".to_string());
    }
    if input.canonical_url_path.trim().is_empty() {
        blocking_reasons.push("missing_canonical_url_path".to_string());
    }
    if input.output_dir.trim().is_empty() {
        blocking_reasons.push("missing_output_dir".to_string());
    }
    if input.base_url.trim().is_empty() {
        blocking_reasons.push("missing_base_url".to_string());
    }
    let artifact = input.publish_artifact.clone().unwrap_or(PublishArtifact {
        artifact_key: primitives::seo::seo_artifact_key(
            "publish_artifact",
            &[
                &input.page_node_key,
                &input.revision_id,
                "headless_snapshot",
            ],
        ),
        page_node_key: input.page_node_key.clone(),
        revision_id: input.revision_id.clone(),
        artifact_type: "headless_snapshot".to_string(),
        artifact_uri: String::new(),
        manifest_json: "{}".to_string(),
        status: "pending_materialization".to_string(),
    });
    PublishMaterializeOutputPayload {
        publish_artifact: Some(artifact),
        preview_pages: Vec::new(),
        materialization_status: if blocking_reasons.is_empty() {
            "ready_to_build".to_string()
        } else {
            "blocked".to_string()
        },
        blocking_reasons,
    }
}
