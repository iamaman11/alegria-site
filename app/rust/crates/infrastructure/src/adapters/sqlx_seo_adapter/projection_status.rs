async fn emit_projection_events_for_run(
    pool: &PgPool,
    run_id: &str,
    events: Vec<OutboxEnvelope>,
) -> Result<(), DomainError> {
    let events = events
        .into_iter()
        .map(|mut event| {
            if event.run_id.trim().is_empty() {
                event.run_id = run_id.to_string();
            }
            event
        })
        .collect::<Vec<_>>();
    let _ = outbox_emit_many(pool, &events).await?;
    Ok(())
}

pub async fn read_projection_sync_status_for_run(
    pool: &PgPool,
    run_id: &str,
) -> Result<Vec<ProjectionSyncStatus>, DomainError> {
    let rows = sqlx::query(
        r#"
        WITH target_systems(target_system) AS (
            VALUES ('neo4j'), ('qdrant'), ('cms')
        )
        SELECT
            targets.target_system,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'pending')::BIGINT AS pending_events,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'processing')::BIGINT AS processing_events,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'failed')::BIGINT AS failed_events,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'done')::BIGINT AS done_events,
            COALESCE(
                MAX(
                    CASE
                        WHEN outbox.status IN ('pending', 'processing') THEN
                            GREATEST(
                                0,
                                FLOOR(EXTRACT(EPOCH FROM (now() - outbox.created_at)) * 1000)
                            )::BIGINT
                        ELSE 0
                    END
                ),
                0
            )::BIGINT AS max_open_lag_ms,
            oldest.event_id AS oldest_open_event_id,
            oldest.aggregate_key AS oldest_open_aggregate_key,
            oldest.event_type AS oldest_open_event_type,
            latest_failed.aggregate_key AS latest_failed_aggregate_key,
            latest_failed.event_type AS latest_failed_event_type,
            latest_failed.last_error AS latest_failed_error
        FROM target_systems targets
        LEFT JOIN system.sync_outbox outbox
            ON outbox.target_system = targets.target_system
           AND outbox.run_id = $1
        LEFT JOIN LATERAL (
            SELECT
                event_id::TEXT AS event_id,
                aggregate_key,
                event_type
            FROM system.sync_outbox
            WHERE target_system = targets.target_system
              AND run_id = $1
              AND status IN ('pending', 'processing')
            ORDER BY created_at ASC
            LIMIT 1
        ) oldest ON TRUE
        LEFT JOIN LATERAL (
            SELECT
                aggregate_key,
                event_type,
                last_error
            FROM system.sync_outbox
            WHERE target_system = targets.target_system
              AND run_id = $1
              AND status = 'failed'
            ORDER BY updated_at DESC
            LIMIT 1
        ) latest_failed ON TRUE
        GROUP BY
            targets.target_system,
            oldest.event_id,
            oldest.aggregate_key,
            oldest.event_type,
            latest_failed.aggregate_key,
            latest_failed.event_type,
            latest_failed.last_error
        ORDER BY targets.target_system
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|row| ProjectionSyncStatus {
            target_system: row.get("target_system"),
            pending_events: row.get("pending_events"),
            processing_events: row.get("processing_events"),
            failed_events: row.get("failed_events"),
            done_events: row.get("done_events"),
            max_open_lag_ms: row.get("max_open_lag_ms"),
            oldest_open_event_id: row.get("oldest_open_event_id"),
            oldest_open_aggregate_key: row.get("oldest_open_aggregate_key"),
            oldest_open_event_type: row.get("oldest_open_event_type"),
            latest_failed_aggregate_key: row.get("latest_failed_aggregate_key"),
            latest_failed_event_type: row.get("latest_failed_event_type"),
            latest_failed_error: row.get("latest_failed_error"),
        })
        .collect())
}

pub async fn read_projection_sync_status(
    pool: &PgPool,
) -> Result<Vec<ProjectionSyncStatus>, DomainError> {
    let rows = sqlx::query(
        r#"
        WITH target_systems(target_system) AS (
            VALUES ('neo4j'), ('qdrant'), ('cms')
        )
        SELECT
            targets.target_system,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'pending')::BIGINT AS pending_events,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'processing')::BIGINT AS processing_events,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'failed')::BIGINT AS failed_events,
            COUNT(outbox.event_id) FILTER (WHERE outbox.status = 'done')::BIGINT AS done_events,
            COALESCE(
                MAX(
                    CASE
                        WHEN outbox.status IN ('pending', 'processing') THEN
                            GREATEST(
                                0,
                                FLOOR(EXTRACT(EPOCH FROM (now() - outbox.created_at)) * 1000)
                            )::BIGINT
                        ELSE 0
                    END
                ),
                0
            )::BIGINT AS max_open_lag_ms,
            oldest.event_id AS oldest_open_event_id,
            oldest.aggregate_key AS oldest_open_aggregate_key,
            oldest.event_type AS oldest_open_event_type,
            latest_failed.aggregate_key AS latest_failed_aggregate_key,
            latest_failed.event_type AS latest_failed_event_type,
            latest_failed.last_error AS latest_failed_error
        FROM target_systems targets
        LEFT JOIN system.sync_outbox outbox
            ON outbox.target_system = targets.target_system
        LEFT JOIN LATERAL (
            SELECT
                event_id::TEXT AS event_id,
                aggregate_key,
                event_type
            FROM system.sync_outbox
            WHERE target_system = targets.target_system
              AND status IN ('pending', 'processing')
            ORDER BY created_at ASC
            LIMIT 1
        ) oldest ON TRUE
        LEFT JOIN LATERAL (
            SELECT
                aggregate_key,
                event_type,
                last_error
            FROM system.sync_outbox
            WHERE target_system = targets.target_system
              AND status = 'failed'
            ORDER BY updated_at DESC
            LIMIT 1
        ) latest_failed ON TRUE
        GROUP BY
            targets.target_system,
            oldest.event_id,
            oldest.aggregate_key,
            oldest.event_type,
            latest_failed.aggregate_key,
            latest_failed.event_type,
            latest_failed.last_error
        ORDER BY targets.target_system
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    Ok(rows
        .into_iter()
        .map(|row| ProjectionSyncStatus {
            target_system: row.get("target_system"),
            pending_events: row.get("pending_events"),
            processing_events: row.get("processing_events"),
            failed_events: row.get("failed_events"),
            done_events: row.get("done_events"),
            max_open_lag_ms: row.get("max_open_lag_ms"),
            oldest_open_event_id: row.get("oldest_open_event_id"),
            oldest_open_aggregate_key: row.get("oldest_open_aggregate_key"),
            oldest_open_event_type: row.get("oldest_open_event_type"),
            latest_failed_aggregate_key: row.get("latest_failed_aggregate_key"),
            latest_failed_event_type: row.get("latest_failed_event_type"),
            latest_failed_error: row.get("latest_failed_error"),
        })
        .collect())
}

