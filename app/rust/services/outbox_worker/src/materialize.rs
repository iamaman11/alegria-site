use anyhow::Result;

pub async fn dispatch_event(
    target_system: &str,
    event_type: &str,
    aggregate_key: &str,
    payload_type: &str,
    payload_bytes: &[u8],
) -> Result<()> {
    infrastructure::adapters::projection_materialize_adapter::dispatch_event(
        target_system,
        event_type,
        aggregate_key,
        payload_type,
        payload_bytes,
    )
    .await
}
