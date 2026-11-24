use postgres_store::sqlx_outbox_adapter::OutboxEvent;
use runtime_composition::adapters::projection_materialize_adapter::{
    DispatchDisposition, DispatchError,
};
use sqlx::PgPool;

pub async fn dispatch_event(
    pool: &PgPool,
    event: &OutboxEvent,
) -> Result<DispatchDisposition, DispatchError> {
    runtime_composition::adapters::projection_materialize_adapter::dispatch_leased_event(
        pool, event,
    )
    .await
}
