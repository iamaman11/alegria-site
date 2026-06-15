use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use contracts::generated::alegria::sync::v1::QdrantUpsertCommand;
use contracts::generated::alegria::temporal::v1::SourceContextChunkState;
use prost::Message;
use reqwest::redirect::Policy;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{types::Json, PgPool, Row};

use policies::truth_governance::{
    adjudicate_truth_candidates_with_governance, SourceGovernanceRecord,
};
use primitives::hash::blake3_hex;
use primitives::hash::content_hash_v1;
use primitives::html_sections::{extract_meta_typed, extract_sections_typed};
use primitives::qdrant_point_id::qdrant_point_id_v1;
use primitives::truth_candidates::{
    validate_truth_candidate, TruthCandidateRuntime, TruthParamValue, TruthStructuredCandidate,
};
use primitives::url_norm::domain_norm;

use super::proto_runtime_payload_store::classify_sqlx;
use super::reqwest_adapter;
use super::semantic_search_adapter;
use super::sqlx_outbox_adapter::OutboxEnvelope;
use super::sqlx_runtime_outbox_adapter::outbox_emit_many;
use super::truth_extraction_llm_adapter;
use super::voyage_api_adapter::{
    VoyageClient, VoyageEmbeddingOptions, VoyageInputType, VoyageOutputDtype,
};

const CRAWL_USER_AGENT: &str = "AlegriaBot/1.0 (+https://alegria.local/seo-research)";
const MAX_REDIRECT_HOPS: usize = 10;
const MAX_CRAWL_ATTEMPTS: i32 = 4;
const RAW_CHUNKS_STANDARD_COLLECTION: &str = "raw_chunks_4";
const RAW_CHUNKS_CONTEXT_COLLECTION: &str = "raw_chunks_ctx";
const DEFAULT_VOYAGE_STANDARD_MODEL: &str = "voyage-4-large";
const DEFAULT_VOYAGE_CONTEXT_MODEL: &str = "voyage-context-3";
const DEFAULT_VOYAGE_DIMENSION: u32 = 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RobotsDecisionTrace {
    pub source_url: String,
    pub robots_url: String,
    pub user_agent: String,
    pub robots_status: Option<i32>,
    pub allowed: bool,
    pub matched_rule: Option<String>,
    pub allow_rules: Vec<String>,
    pub disallow_rules: Vec<String>,
    pub fetch_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CrawlObservationTrace {
    pub source_url: String,
    pub final_url: String,
    pub redirect_hops: usize,
    pub redirect_chain: Vec<String>,
    pub robots: RobotsDecisionTrace,
}

#[derive(Debug, Clone)]
pub struct CrawlQueueItem {
    pub url: String,
    pub url_norm: String,
    pub source_domain: String,
    pub source_type: String,
    pub dtype: String,
    pub attempt_count: i32,
}

#[derive(Debug, Clone)]
pub struct FetchedHtml {
    pub source_url: String,
    pub final_url: String,
    pub status_code: i32,
    pub content_type: String,
    pub body: String,
    pub redirect_chain: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SavedRawPage {
    pub page_id: i64,
    pub section_count: usize,
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub struct RawSectionRecord {
    pub id: i64,
    pub page_id: i64,
    pub source_url: String,
    pub source_domain: String,
    pub source_dtype: String,
    pub heading_path: String,
    pub section_type: String,
    pub content_md: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Default)]
pub struct RawKnowledgeIngestionReport {
    pub raw_page_count: usize,
    pub raw_section_count: usize,
    pub extracted_rule_count: usize,
    pub verified_rule_count: usize,
    pub needs_hitl_candidate_count: usize,
    pub expert_blocked_section_count: usize,
    pub expert_needs_hitl_section_count: usize,
    pub expert_verified_ready_section_count: usize,
    pub expert_triple_count: usize,
    pub outbox_event_count: usize,
    pub changed_truth_keys: Vec<String>,
    pub extraction_provider_unavailable: bool,
}

#[derive(Debug, Clone)]
struct PersistedCandidateForAdjudication {
    rule_candidate_id: String,
    context_key: String,
    role: String,
    concept_canonical_key: String,
    params: TruthParamValue,
    source_key: String,
    source_tier: String,
    confidence: f64,
    freshness_class: String,
    completeness_class: String,
    evidence_section_id: i64,
    evidence_quote: String,
    span_start: i32,
    span_end: i32,
    source_snapshot_hash: String,
    prompt_version: String,
    model_version: String,
    epistemic_status: String,
}

