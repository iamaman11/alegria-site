#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neo4jBackwriteInput {
    pub target_system: String,
    pub dry_run: bool,
    pub max_retry_count: Option<i32>,
    pub batch_limit: Option<i64>,
    pub requeue_base_delay_sec: Option<i64>,
    pub requeue_jitter_sec: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neo4jBackwriteOutput {
    pub target_system: String,
    pub dry_run: bool,
    pub stale_candidates: i64,
    pub failed_candidates: i64,
    pub reset_stale_processing: i64,
    pub requeued_failed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionSyncInput {
    pub run_id: String,
    pub target_system: String,
    pub step_name: String,
    pub batch_limit: i64,
    pub lease_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionSyncOutput {
    pub run_id: String,
    pub target_system: String,
    pub processed_events: i64,
    pub retried_events: i64,
    pub failed_events: i64,
    pub remaining_pending: i64,
    pub remaining_failed: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSectionSampleInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSectionSampleOutput {
    pub section_id: String,
    pub page_id: i64,
    pub source_url: String,
    pub source_domain: String,
    pub heading_path: String,
    pub section_type: String,
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeoPreflightInput {
    pub run_id: String,
    pub context_key: String,
    pub scope: SeoScopePayload,
    pub projection_max_lag_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeoPreflightOutput {
    pub context_key: String,
    pub normalized_profile: String,
    pub page_type_count: i64,
    pub page_node_count: i64,
    pub navigation_item_count: i64,
    pub verified_rule_count: i64,
    pub pending_rule_count: i64,
    pub qdrant_point_count: i64,
    pub voyage_embeddings_ready: bool,
    pub voyage_contextualized_ready: bool,
    pub voyage_rerank_ready: bool,
    pub qdrant_ready: bool,
    pub qdrant_collection_contract_ready: bool,
    pub neo4j_ready: bool,
    pub graph_query_ready: bool,
    pub graph_gds_ready: bool,
    pub graph_projection_contract_ready: bool,
    pub retrieval_capability_required: bool,
    pub canonical_vector_retrieval_required: bool,
    pub contextual_raw_chunk_retrieval_required: bool,
    pub voyage_rerank_required: bool,
    pub graph_capability_required: bool,
    pub neo4j_sync_required: bool,
    pub graph_query_required: bool,
    pub graph_gds_required: bool,
    pub retrieval_contract_status: String,
    pub retrieval_block_reason: Option<String>,
    pub required_collection_statuses: Vec<SeoPreflightCollectionStatus>,
    pub graph_contract_status: String,
    pub graph_block_reason: Option<String>,
    pub required_graph_projection_statuses: Vec<SeoPreflightGraphProjectionStatus>,
    pub projection_blocked: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeoPreflightCollectionStatus {
    pub collection_name: String,
    pub exists: bool,
    pub fresh: bool,
    pub projection_complete: bool,
    pub point_count: i64,
    pub last_materialized_at: Option<String>,
    pub last_source_change_at: Option<String>,
    pub lag_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeoPreflightGraphProjectionStatus {
    pub projection_name: String,
    pub exists: bool,
    pub fresh: bool,
    pub projection_complete: bool,
    pub point_count: i64,
    pub last_materialized_at: Option<String>,
    pub last_source_change_at: Option<String>,
    pub lag_seconds: Option<i64>,
}

