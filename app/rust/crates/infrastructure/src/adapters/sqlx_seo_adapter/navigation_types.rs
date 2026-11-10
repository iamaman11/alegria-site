#[derive(Debug, Clone)]
struct ScopeFields {
    scope_signature: String,
    market: String,
    locale: String,
    country_code: Option<String>,
    visa_type: Option<String>,
    applicant_profile: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct EditorialExtractionSweepOutputBlob {
    sections: Vec<EditorialExtractionSectionBlob>,
}

#[derive(Debug, serde::Deserialize)]
struct EditorialExtractionSectionBlob {
    topics: Vec<EditorialTopicBlob>,
}

#[derive(Debug, serde::Deserialize)]
struct EditorialTopicBlob {
    topic_type: String,
    topic_key_candidate: String,
    #[allow(dead_code)]
    confidence: f32,
}

#[derive(Debug, serde::Deserialize)]
struct TripleBuilderSweepOutputBlob {
    sections: Vec<TripleBuilderSectionBlob>,
}

#[derive(Debug, serde::Deserialize)]
struct TripleBuilderSectionBlob {
    triples: Vec<TripleSignalBlob>,
}

#[derive(Debug, serde::Deserialize)]
struct TripleSignalBlob {
    triple_id: String,
    subject_key: String,
    relation_type: String,
    object_key: String,
    evidence_section_id: String,
    confidence: f32,
}

#[derive(Debug, Clone)]
pub struct SeoScopeBootstrapReport {
    pub context_key: String,
    pub created_context: bool,
    pub seeded_registry_count: u64,
}

#[derive(Debug, Clone, Default)]
pub struct GlobalNavigationReport {
    pub navigation_tree_key: String,
    pub scope_count: u64,
    pub page_item_count: u64,
    pub silo_group_count: u64,
    pub rebuild_plan_count: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectionSyncStatus {
    pub target_system: String,
    pub pending_events: i64,
    pub processing_events: i64,
    pub failed_events: i64,
    pub done_events: i64,
    pub max_open_lag_ms: i64,
    pub oldest_open_event_id: Option<String>,
    pub oldest_open_aggregate_key: Option<String>,
    pub oldest_open_event_type: Option<String>,
    pub latest_failed_aggregate_key: Option<String>,
    pub latest_failed_event_type: Option<String>,
    pub latest_failed_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct QdrantCollectionStatus {
    pub collection_name: String,
    pub point_count: i64,
    pub last_materialized_at: Option<String>,
    pub lag_seconds: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct GraphProjectionStatus {
    pub artifact_name: String,
    pub point_count: i64,
    pub last_materialized_at: Option<String>,
    pub lag_seconds: Option<i64>,
}

impl ProjectionSyncStatus {
    pub fn open_event_count(&self) -> i64 {
        self.pending_events + self.processing_events
    }

    pub fn blocking_event_count(&self) -> i64 {
        self.open_event_count() + self.failed_events
    }
}

pub async fn read_qdrant_collection_statuses(
    pg: &PgPool,
    expected_collections: &[&str],
) -> Result<Vec<QdrantCollectionStatus>, DomainError> {
    let rows = sqlx::query(
        r#"
        WITH expected(collection_name) AS (
            SELECT unnest($1::text[])
        ),
        aggregated AS (
            SELECT
                collection_name,
                count(*)::bigint AS point_count,
                max(updated_at) AS last_materialized_at
            FROM kb.qdrant_points
            WHERE collection_name = ANY($1)
            GROUP BY collection_name
        )
        SELECT
            expected.collection_name,
            coalesce(aggregated.point_count, 0)::bigint AS point_count,
            aggregated.last_materialized_at::text AS last_materialized_at,
            CASE
                WHEN aggregated.last_materialized_at IS NULL THEN NULL
                ELSE greatest(extract(epoch from (now() - aggregated.last_materialized_at))::bigint, 0)
            END AS lag_seconds
        FROM expected
        LEFT JOIN aggregated
            ON aggregated.collection_name = expected.collection_name
        ORDER BY expected.collection_name
        "#,
    )
    .bind(expected_collections)
    .fetch_all(pg)
    .await
    .map_err(classify_sqlx)?;

    Ok(rows
        .into_iter()
        .map(|row| QdrantCollectionStatus {
            collection_name: row.get("collection_name"),
            point_count: row.get("point_count"),
            last_materialized_at: row.get("last_materialized_at"),
            lag_seconds: row.get("lag_seconds"),
        })
        .collect())
}

pub async fn read_graph_projection_statuses(
    pg: &PgPool,
    expected_artifacts: &[&str],
) -> Result<Vec<GraphProjectionStatus>, DomainError> {
    read_graph_projection_statuses_filtered(pg, expected_artifacts, None).await
}

/// Scope-specific source readiness cannot be satisfied by a different site's rows.
pub async fn read_graph_projection_statuses_for_scope(
    pg: &PgPool,
    expected_artifacts: &[&str],
    scope_signature: &str,
) -> Result<Vec<GraphProjectionStatus>, DomainError> {
    read_graph_projection_statuses_filtered(pg, expected_artifacts, Some(scope_signature)).await
}

async fn read_graph_projection_statuses_filtered(
    pg: &PgPool,
    expected_artifacts: &[&str],
    scope_signature: Option<&str>,
) -> Result<Vec<GraphProjectionStatus>, DomainError> {
    let rows = sqlx::query(
        r#"
        WITH expected(artifact_name) AS (
            SELECT unnest($1::text[])
        ),
        aggregated AS (
            SELECT 'keyword_cluster'::text AS artifact_name, count(*)::bigint AS point_count, max(updated_at) AS last_materialized_at
              FROM site.keyword_clusters
             WHERE ($2::text IS NULL OR scope_signature = $2)
            UNION ALL
            SELECT 'serp_pattern'::text AS artifact_name, count(*)::bigint AS point_count, max(updated_at) AS last_materialized_at
              FROM serp.serp_patterns
             WHERE ($2::text IS NULL OR scope_signature = $2)
            UNION ALL
            SELECT 'page_blueprint'::text AS artifact_name, count(*)::bigint AS point_count, max(updated_at) AS last_materialized_at
              FROM site.page_blueprints bp
             WHERE ($2::text IS NULL OR EXISTS (
                 SELECT 1 FROM site.page_nodes pn
                  WHERE pn.blueprint_key = bp.blueprint_key AND pn.scope_signature = $2
             ))
            UNION ALL
            SELECT 'page_node'::text AS artifact_name, count(*)::bigint AS point_count, max(updated_at) AS last_materialized_at
              FROM site.page_nodes
             WHERE ($2::text IS NULL OR scope_signature = $2)
            UNION ALL
            SELECT 'content_gap'::text AS artifact_name, count(*)::bigint AS point_count, max(updated_at) AS last_materialized_at
              FROM site.content_gaps
             WHERE ($2::text IS NULL OR scope_signature = $2)
            UNION ALL
            SELECT 'link_recommendation'::text AS artifact_name, count(*)::bigint AS point_count, max(updated_at) AS last_materialized_at
              FROM site.link_recommendations
             WHERE ($2::text IS NULL OR scope_signature = $2)
            UNION ALL
            SELECT 'page_brief'::text AS artifact_name, count(*)::bigint AS point_count, max(updated_at) AS last_materialized_at
              FROM site.page_briefs pb
             WHERE ($2::text IS NULL OR EXISTS (
                 SELECT 1 FROM site.page_nodes pn
                  WHERE pn.page_node_key = pb.page_node_key AND pn.scope_signature = $2
             ))
        )
        SELECT
            expected.artifact_name,
            coalesce(aggregated.point_count, 0)::bigint AS point_count,
            aggregated.last_materialized_at::text AS last_materialized_at,
            CASE
                WHEN aggregated.last_materialized_at IS NULL THEN NULL
                ELSE greatest(extract(epoch from (now() - aggregated.last_materialized_at))::bigint, 0)
            END AS lag_seconds
        FROM expected
        LEFT JOIN aggregated
            ON aggregated.artifact_name = expected.artifact_name
        ORDER BY expected.artifact_name
        "#,
    )
    .bind(expected_artifacts)
    .bind(scope_signature)
    .fetch_all(pg)
    .await
    .map_err(classify_sqlx)?;

    Ok(rows
        .into_iter()
        .map(|row| GraphProjectionStatus {
            artifact_name: row.get("artifact_name"),
            point_count: row.get("point_count"),
            last_materialized_at: row.get("last_materialized_at"),
            lag_seconds: row.get("lag_seconds"),
        })
        .collect())
}

#[derive(Debug, Clone)]
struct ActivePageForNavigation {
    page_node_key: String,
    scope_signature: String,
    parent_page_node_key: String,
    page_type_key: String,
    canonical_slug: String,
    canonical_url_path: String,
    hierarchy_depth: i32,
    menu_group: String,
    market: String,
    locale: String,
    country_code: Option<String>,
    visa_type: Option<String>,
    applicant_profile: Option<String>,
}
