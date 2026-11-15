
#[derive(Debug, Clone)]
pub struct RunVerifiedRulePoint {
    pub point_id: String,
    pub rule_instance_id: String,
    pub context_key: String,
    pub concept_key: String,
    pub source_key: String,
}

/// A verified fact is eligible for retrieval only if its canonical SQL state,
/// current business-run outbox receipt, and Qdrant point all agree.
pub async fn read_run_verified_rule_points(
    pool: &PgPool,
    run_id: &str,
    context_key: &str,
) -> Result<Vec<RunVerifiedRulePoint>, DomainError> {
    let rows = sqlx::query(
        r#"
        SELECT ri.rule_instance_id, ri.context_key, ri.concept_key, ri.source_key,
               q.point_id,
               EXISTS (
                   SELECT 1 FROM system.sync_outbox o
                   WHERE o.run_id = $1 AND o.target_system = 'qdrant'
                     AND o.aggregate_type = 'rule_instance'
                     AND o.aggregate_key = ri.rule_instance_id
                     AND o.event_type = 'QdrantUpsertCommand'
                     AND o.status = 'done'
               ) AS delivered
        FROM verified.rule_instances ri
        LEFT JOIN kb.qdrant_points q
          ON q.collection_name = 'verified_rules_4'
         AND q.entity_type = 'rule_instance'
         AND q.entity_key = ri.rule_instance_id
        WHERE ri.context_key = $2 AND ri.status = 'verified'
        ORDER BY ri.rule_instance_id
        "#,
    )
    .bind(run_id)
    .bind(context_key)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut verified = Vec::with_capacity(rows.len());
    for row in rows {
        let delivered: bool = row.get("delivered");
        let point_id: Option<String> = row.get("point_id");
        if !delivered || point_id.is_none() {
            return Ok(Vec::new());
        }
        verified.push(RunVerifiedRulePoint {
            point_id: point_id.unwrap(),
            rule_instance_id: row.get("rule_instance_id"),
            context_key: row.get("context_key"),
            concept_key: row.get("concept_key"),
            source_key: row.get("source_key"),
        });
    }
    Ok(verified)
}

/// A completed graph projection cannot become stale merely because time passes.
/// It can become stale if the source changed after the last successful receipt.
/// Live Neo4j scope readback and pending/failed outbox checks are separate gates.
pub async fn read_run_graph_source_synced(
    pool: &PgPool,
    run_id: &str,
    scope_signature: &str,
) -> Result<bool, DomainError> {
    sqlx::query_scalar::<_, bool>(
        r#"
        WITH source_updates(changed_at) AS (
            SELECT max(updated_at) FROM site.keyword_clusters WHERE scope_signature = $2
            UNION ALL SELECT max(updated_at) FROM serp.serp_patterns WHERE scope_signature = $2
            UNION ALL SELECT max(bp.updated_at) FROM site.page_blueprints bp
              WHERE EXISTS (SELECT 1 FROM site.page_nodes pn
                             WHERE pn.blueprint_key = bp.blueprint_key AND pn.scope_signature = $2)
            UNION ALL SELECT max(updated_at) FROM site.page_nodes WHERE scope_signature = $2
            UNION ALL SELECT max(updated_at) FROM site.content_gaps WHERE scope_signature = $2
            UNION ALL SELECT max(updated_at) FROM site.link_recommendations WHERE scope_signature = $2
            UNION ALL SELECT max(pb.updated_at) FROM site.page_briefs pb
              JOIN site.page_nodes pn ON pn.page_node_key = pb.page_node_key
             WHERE pn.scope_signature = $2
        )
        SELECT COALESCE(
            (SELECT max(o.updated_at) FROM system.sync_outbox o
             WHERE o.run_id = $1 AND o.target_system = 'neo4j' AND o.status = 'done')
            >= (SELECT max(changed_at) FROM source_updates),
            FALSE
        )
        AND NOT EXISTS (
            SELECT 1 FROM system.sync_outbox o
            WHERE o.run_id = $1 AND o.target_system = 'neo4j'
              AND o.status IN ('pending', 'processing', 'failed')
        )
        "#,
    )
    .bind(run_id)
    .bind(scope_signature)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)
}

/// Immutable raw evidence is fresh when this business run's outbox has
/// completed and every indexed section still agrees with the stored source.
/// Wall-clock age of the last embedding is not evidence of staleness.
#[derive(Debug, Clone)]
pub struct RunRawChunkPoint {
    pub collection_name: String,
    pub point_id: String,
    pub section_id: String,
    pub content_hash: String,
}

pub async fn read_run_raw_chunk_points(
    pool: &PgPool,
    run_id: &str,
) -> Result<Vec<RunRawChunkPoint>, DomainError> {
    let rows = sqlx::query(
        r#"
        SELECT split_part(o.aggregate_key, '|', 2) AS collection_name,
               o.status,
               s.id::TEXT AS section_id,
               s.content_hash,
               q.point_id
        FROM system.sync_outbox o
        LEFT JOIN raw.sections s
            ON s.id = split_part(split_part(o.aggregate_key, '|', 1), ':', 2)::BIGINT
        LEFT JOIN kb.qdrant_points q
            ON q.collection_name = split_part(o.aggregate_key, '|', 2)
           AND q.entity_type = 'raw_section'
           AND q.entity_key = split_part(o.aggregate_key, '|', 1)
        WHERE o.run_id = $1
          AND o.target_system = 'qdrant'
          AND o.aggregate_type = 'raw_section'
          AND split_part(o.aggregate_key, '|', 2) IN ('raw_chunks_4', 'raw_chunks_ctx')
        ORDER BY collection_name, section_id
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut points = Vec::with_capacity(rows.len());
    for row in rows {
        let status: String = row.get("status");
        let section_id: Option<String> = row.get("section_id");
        let content_hash: Option<String> = row.get("content_hash");
        let point_id: Option<String> = row.get("point_id");
        // A missing source, SQL point, or unfinished outbox lease fails closed.
        if status != "done"
            || section_id.is_none()
            || content_hash.as_ref().is_none_or(String::is_empty)
            || point_id.is_none()
        {
            return Ok(Vec::new());
        }
        points.push(RunRawChunkPoint {
            collection_name: row.get("collection_name"),
            point_id: point_id.unwrap(),
            section_id: section_id.unwrap(),
            content_hash: content_hash.unwrap(),
        });
    }
    Ok(points)
}

/// The step journal is durable producer evidence: an empty optional graph
/// output is valid only when the producer finished for this exact run.
pub async fn read_completed_seo_producers(
    pool: &PgPool,
    run_id: &str,
) -> Result<Vec<String>, DomainError> {
    sqlx::query_scalar::<_, String>(
        r#"
        SELECT DISTINCT step_name
        FROM pipeline.step_executions
        WHERE run_id = $1::uuid
          AND status = 'done'
          AND step_name = ANY($2::text[])
        "#,
    )
    .bind(run_id)
    .bind(&[
        "opportunity_build",
        "link_recommend",
        "global_site_reconcile",
        "canonical_mapping",
        "whole_page_semantic_pass",
    ])
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)
}

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

