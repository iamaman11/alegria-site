// Expert-stage persistence metadata, not semantic classification policy.
fn stage_record<T: Serialize, U: Serialize>(
    stage_name: &str,
    section: &RawSectionRecord,
    input: &T,
    output: &U,
    status: ExpertStageStatus,
    error_class: Option<&str>,
) -> ExpertStageRecord {
    let input_json = serde_json::to_vec(input).unwrap_or_default();
    let output_json = serde_json::to_vec(output).unwrap_or_default();
    ExpertStageRecord {
        stage_name: stage_name.to_string(),
        schema_version: EXPERT_CORE_SCHEMA_VERSION,
        idempotency_key: blake3_hex(
            format!(
                "expert_extraction_core|{}|{}|{}|{}",
                stage_name, section.id, section.content_hash, section.heading_path
            )
            .as_bytes(),
        ),
        input_hash: content_hash_v1(std::str::from_utf8(&input_json).unwrap_or("")),
        output_hash: content_hash_v1(std::str::from_utf8(&output_json).unwrap_or("")),
        status,
        error_class: error_class.map(str::to_string),
    }
}
