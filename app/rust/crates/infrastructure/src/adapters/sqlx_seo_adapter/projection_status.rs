#[derive(Debug, Clone)]
pub struct RunVerifiedRulePoint {
    pub point_id: String,
    pub rule_instance_id: String,
    pub context_key: String,
    pub concept_key: String,
    pub source_key: String,
}

/// The global retrieval index is reusable between runs. If this run emitted
/// rule projections, validate its exact owned projection set; another SQL
/// fact in the same context may legitimately be outside that run's output.
/// For a retained run with no rule events, validate existing global facts
/// against prior completed receipts and live Qdrant instead. Profile-specific
/// facts omitted from the global index have a separate SQL support gate.
pub async fn read_run_verified_rule_points(
    pool: &PgPool,
    run_id: &str,
    context_key: &str,
) -> Result<Vec<RunVerifiedRulePoint>, DomainError> {
    let owned_rows = sqlx::query(
        r#"
        SELECT ri.rule_instance_id, ri.context_key, ri.concept_key, ri.source_key,
               q.point_id, bool_and(o.status = 'done') AS delivered
        FROM system.sync_outbox o
        LEFT JOIN verified.rule_instances ri
          ON ri.rule_instance_id = o.aggregate_key
         AND ri.context_key = $2
         AND ri.status = 'verified'
        LEFT JOIN kb.qdrant_points q
          ON q.collection_name = 'verified_rules_4'
         AND q.entity_type = 'rule_instance'
         AND q.entity_key = o.aggregate_key
        WHERE o.run_id = $1
          AND o.target_system = 'qdrant'
          AND o.aggregate_type = 'rule_instance'
          AND o.event_type = 'QdrantUpsertCommand'
        GROUP BY o.aggregate_key, ri.rule_instance_id, ri.context_key,
                 ri.concept_key, ri.source_key, q.point_id
        ORDER BY o.aggregate_key
        "#,
    )
    .bind(run_id)
    .bind(context_key)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let rows = if owned_rows.is_empty() {
        sqlx::query(
            r#"
        SELECT ri.rule_instance_id, ri.context_key, ri.concept_key, ri.source_key,
               q.point_id,
               EXISTS (
                   SELECT 1 FROM system.sync_outbox o
                   WHERE o.aggregate_key = ri.rule_instance_id
                     AND o.aggregate_type = 'rule_instance'
                     AND o.target_system = 'qdrant'
                     AND o.event_type = 'QdrantUpsertCommand'
                     AND o.status = 'done'
               ) AS delivered
        FROM verified.rule_instances ri
        LEFT JOIN kb.qdrant_points q
          ON q.collection_name = 'verified_rules_4'
         AND q.entity_type = 'rule_instance'
         AND q.entity_key = ri.rule_instance_id
        WHERE ri.context_key = $2
          AND ri.status = 'verified'
          AND (
              NOT EXISTS (
                  SELECT 1 FROM verified.rule_instance_profiles rp
                  WHERE rp.rule_instance_id = ri.rule_instance_id
              )
              OR q.point_id IS NOT NULL
              OR EXISTS (
                  SELECT 1 FROM system.sync_outbox owned
                  WHERE owned.run_id = $1
                    AND owned.aggregate_key = ri.rule_instance_id
                    AND owned.aggregate_type = 'rule_instance'
                    AND owned.target_system = 'qdrant'
              )
          )
        ORDER BY ri.rule_instance_id
            "#,
        )
        .bind(run_id)
        .bind(context_key)
        .fetch_all(pool)
        .await
        .map_err(classify_sqlx)?
    } else {
        owned_rows
    };

    let mut verified = Vec::with_capacity(rows.len());
    for row in rows {
        let delivered: bool = row.get("delivered");
        let point_id: Option<String> = row.get("point_id");
        let rule_instance_id: Option<String> = row.get("rule_instance_id");
        let context_key: Option<String> = row.get("context_key");
        let concept_key: Option<String> = row.get("concept_key");
        let source_key: Option<String> = row.get("source_key");
        if !delivered
            || point_id.is_none()
            || rule_instance_id.is_none()
            || context_key.is_none()
            || concept_key.is_none()
            || source_key.is_none()
        {
            return Ok(Vec::new());
        }
        verified.push(RunVerifiedRulePoint {
            point_id: point_id.unwrap(),
            rule_instance_id: rule_instance_id.unwrap(),
            context_key: context_key.unwrap(),
            concept_key: concept_key.unwrap(),
            source_key: source_key.unwrap(),
        });
    }
    Ok(verified)
}

/// Graph freshness is certified by completed revision-aware outbox receipts
/// and the separate real Neo4j scope readback, not wall-clock materialization age.
/// An unfinished run-owned projection is always a blocker.
pub async fn read_run_graph_source_synced(
    pool: &PgPool,
    run_id: &str,
    scope_signature: &str,
) -> Result<bool, DomainError> {
    sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM site.page_nodes p WHERE p.scope_signature = $2
        )
        AND EXISTS(
            SELECT 1 FROM system.sync_outbox o
            WHERE o.run_id = $1 AND o.target_system = 'neo4j'
              AND o.status = 'done'
        )
        AND NOT EXISTS(
            SELECT 1 FROM system.sync_outbox o
            WHERE o.run_id = $1 AND o.target_system = 'neo4j'
              AND o.status <> 'done'
        )
        "#,
    )
    .bind(run_id)
    .bind(scope_signature)
    .fetch_one(pool)
    .await
    .map_err(classify_sqlx)
}

/// Immutable raw evidence remains valid across retained-corpus runs if every
/// nonempty section of the exact acquired pages has a delivered projection
/// receipt and its live Qdrant payload still matches the current source hash.
/// Wall-clock embedding age and the outbox's original run ID are irrelevant.
#[derive(Debug, Clone)]
pub struct RunRawChunkPoint {
    pub collection_name: String,
    pub point_id: String,
    pub section_id: String,
    pub content_hash: String,
}

pub async fn read_run_raw_chunk_points(
    pool: &PgPool,
    raw_page_ids: &[i64],
) -> Result<Vec<RunRawChunkPoint>, DomainError> {
    if raw_page_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        r#"
        SELECT c.collection_name,
               s.id::TEXT AS section_id,
               s.content_hash,
               q.point_id,
               EXISTS (
                   SELECT 1 FROM system.sync_outbox o
                   WHERE o.aggregate_type = 'raw_section'
                     AND o.aggregate_key = 'raw_section:' || s.id::TEXT || '|' || c.collection_name
                     AND o.target_system = 'qdrant'
                     AND o.event_type = 'QdrantUpsertCommand'
                     AND o.status = 'done'
               ) AS delivered
        FROM raw.sections s
        CROSS JOIN (VALUES ('raw_chunks_4'), ('raw_chunks_ctx')) AS c(collection_name)
        LEFT JOIN kb.qdrant_points q
            ON q.collection_name = c.collection_name
           AND q.entity_type = 'raw_section'
           AND q.entity_key = 'raw_section:' || s.id::TEXT
        WHERE s.page_id = ANY($1::BIGINT[])
          AND btrim(s.content_md) <> ''
        ORDER BY collection_name, section_id
        "#,
    )
    .bind(raw_page_ids)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut points = Vec::with_capacity(rows.len());
    for row in rows {
        let delivered: bool = row.get("delivered");
        let section_id: Option<String> = row.get("section_id");
        let content_hash: Option<String> = row.get("content_hash");
        let point_id: Option<String> = row.get("point_id");
        // Missing source, SQL point, or completed outbox proof fails closed.
        if !delivered
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
