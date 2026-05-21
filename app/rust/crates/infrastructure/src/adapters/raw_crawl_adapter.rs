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

use primitives::hash::blake3_hex;
use primitives::hash::content_hash_v1;
use primitives::html_sections::{extract_meta_typed, extract_sections_typed};
use primitives::qdrant_point_id::qdrant_point_id_v1;
use primitives::truth_candidates::{
    adjudicate_truth_candidates, validate_truth_candidate, TruthCandidateRuntime, TruthParamValue,
    TruthStructuredCandidate,
};
use primitives::url_norm::domain_norm;

use super::proto_runtime_payload_store::classify_sqlx;
use super::reqwest_adapter;
use super::semantic_search_adapter;
use super::sqlx_outbox_adapter::OutboxEnvelope;
use super::sqlx_runtime_outbox_adapter::outbox_emit_many;
use super::truth_extraction_llm_adapter;
use super::voyage_api_adapter::VoyageClient;

const CRAWL_USER_AGENT: &str = "AlegriaBot/1.0 (+https://alegria.local/seo-research)";
const MAX_REDIRECT_HOPS: usize = 10;
const MAX_CRAWL_ATTEMPTS: i32 = 4;

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

pub async fn claim_pending_crawl_batch(
    pool: &PgPool,
    run_id: &str,
    query_batch_key: &str,
    limit: i64,
) -> std::result::Result<Vec<CrawlQueueItem>, primitives::errors::DomainError> {
    let rows = sqlx::query(
        "WITH per_domain AS (
             SELECT DISTINCT ON (coalesce(nullif(source_domain, ''), url_norm)) url_norm, first_seen_at
             FROM serp.crawl_queue
             WHERE status = 'pending'
               AND ($2 = '' OR first_seen_run_id = $2)
               AND ($3 = '' OR query_batch_key = $3)
               AND next_attempt_at <= now()
               AND (locked_until IS NULL OR locked_until <= now())
             ORDER BY coalesce(nullif(source_domain, ''), url_norm), first_seen_at
         ),
         candidates AS (
             SELECT q.url_norm
             FROM serp.crawl_queue q
             JOIN per_domain d ON d.url_norm = q.url_norm
             ORDER BY q.first_seen_at
             LIMIT $1
             FOR UPDATE OF q SKIP LOCKED
         )
         UPDATE serp.crawl_queue q
         SET status = 'processing',
             crawl_attempt_count = q.crawl_attempt_count + 1,
             locked_until = now() + interval '10 minutes',
             notes = concat_ws('; ', nullif(q.notes, ''), 'claimed_by=raw_crawl_adapter')
         FROM candidates c
         WHERE q.url_norm = c.url_norm
         RETURNING q.url, q.url_norm, coalesce(q.source_domain, '') AS source_domain,
                   q.source_type, coalesce(q.dtype, '') AS dtype, q.crawl_attempt_count",
    )
    .bind(limit.max(1))
    .bind(run_id)
    .bind(query_batch_key)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    Ok(rows
        .into_iter()
        .map(|row| CrawlQueueItem {
            url: row.get("url"),
            url_norm: row.get("url_norm"),
            source_domain: row.get("source_domain"),
            source_type: row.get("source_type"),
            dtype: row.get("dtype"),
            attempt_count: row.get("crawl_attempt_count"),
        })
        .collect())
}

pub async fn fetch_html(url: &str) -> Result<FetchedHtml> {
    let source_url = url.to_string();
    let redirect_chain = Arc::new(Mutex::new(vec![source_url.clone()]));
    let redirect_chain_for_policy = Arc::clone(&redirect_chain);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .tcp_keepalive(std::time::Duration::from_secs(60))
        .redirect(Policy::custom(move |attempt| {
            if let Ok(mut chain) = redirect_chain_for_policy.lock() {
                chain.clear();
                chain.extend(
                    attempt
                        .previous()
                        .iter()
                        .map(|previous| previous.as_str().to_string()),
                );
                chain.push(attempt.url().as_str().to_string());
            }
            if attempt.previous().len() > MAX_REDIRECT_HOPS {
                attempt.error("too many redirects")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .context("build crawl client")?;
    let response = client
        .get(&source_url)
        .header(reqwest::header::USER_AGENT, CRAWL_USER_AGENT)
        .send()
        .await
        .with_context(|| format!("fetch source HTML: {source_url}"))?;
    let final_url = response.url().to_string();
    let status_code = response.status().as_u16() as i32;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/html")
        .to_string();
    let body = response
        .text()
        .await
        .with_context(|| format!("read source HTML body: {source_url}"))?;
    let mut redirect_chain = redirect_chain
        .lock()
        .map(|chain| chain.clone())
        .unwrap_or_else(|_| vec![source_url.clone()]);
    if redirect_chain.is_empty() {
        redirect_chain.push(source_url.clone());
    }
    if redirect_chain.last().map(|value| value.as_str()) != Some(final_url.as_str()) {
        redirect_chain.push(final_url.clone());
    }
    Ok(FetchedHtml {
        source_url,
        final_url,
        status_code,
        content_type,
        body,
        redirect_chain,
    })
}

pub async fn evaluate_robots_policy(source_url: &str) -> Result<RobotsDecisionTrace> {
    let source =
        Url::parse(source_url).with_context(|| format!("parse source url: {source_url}"))?;
    let mut robots_url = source.clone();
    robots_url.set_path("/robots.txt");
    robots_url.set_query(None);
    robots_url.set_fragment(None);

    let mut trace = RobotsDecisionTrace {
        source_url: source_url.to_string(),
        robots_url: robots_url.as_str().to_string(),
        user_agent: CRAWL_USER_AGENT.to_string(),
        allowed: true,
        ..RobotsDecisionTrace::default()
    };

    let client = reqwest_adapter::new_default_client(20)?;
    let response = match client
        .get(robots_url.as_str())
        .header(reqwest::header::USER_AGENT, CRAWL_USER_AGENT)
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            trace.allowed = false;
            trace.fetch_error = Some(format!("robots fetch failed: {err}"));
            return Ok(trace);
        }
    };

    let status = response.status();
    trace.robots_status = Some(status.as_u16() as i32);
    if status.as_u16() == 404 || status.as_u16() == 410 {
        return Ok(trace);
    }
    if !status.is_success() {
        trace.allowed = false;
        trace.fetch_error = Some(format!("robots http_status={}", status.as_u16()));
        return Ok(trace);
    }

    let body = response
        .text()
        .await
        .with_context(|| format!("read robots body: {robots_url}"))?;
    let parsed = parse_robots_rules(&body, CRAWL_USER_AGENT, source.path());
    trace.allow_rules = parsed.allow_rules;
    trace.disallow_rules = parsed.disallow_rules;
    trace.allowed = parsed.allowed;
    trace.matched_rule = parsed.matched_rule;
    Ok(trace)
}

pub async fn save_crawled_html(
    pool: &PgPool,
    source_url: &str,
    final_url: &str,
    dtype: &str,
    status_code: i32,
    content_type: &str,
    raw_html: &str,
    redirect_chain: &[String],
    robots_trace: &RobotsDecisionTrace,
) -> std::result::Result<SavedRawPage, primitives::errors::DomainError> {
    let meta = extract_meta_typed(raw_html);
    let sections = extract_sections_typed(raw_html, source_url);
    let content_hash = content_hash_v1(raw_html);
    let domain = domain_norm(source_url);
    let dtype = if dtype.trim().is_empty() {
        "organic_competitor"
    } else {
        dtype.trim()
    };
    let content = json!({
        "extractor": "primitives::html_sections@1",
        "source_url": source_url,
        "final_url": final_url,
        "redirect_chain": redirect_chain,
        "robots_trace": robots_trace,
        "section_count": sections.len(),
        "content_hash": content_hash,
    });
    let trace = CrawlObservationTrace {
        source_url: source_url.to_string(),
        final_url: final_url.to_string(),
        redirect_hops: redirect_chain.len().saturating_sub(1),
        redirect_chain: redirect_chain.to_vec(),
        robots: robots_trace.clone(),
    };
    let trace_json = serde_json::to_value(&trace).map_err(|err| {
        primitives::errors::DomainError::InfraUnavailable {
            message: format!("serialize crawl trace: {err}"),
        }
    })?;
    let redirect_chain_json = Json(json!(redirect_chain));
    let robots_trace_json = Json(robots_trace.clone());
    let source_observation_json = Json(trace_json.clone());

    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    let existing = sqlx::query(
        "SELECT id
         FROM raw.pages
         WHERE url = $1
           AND CAST(snapshot_at AT TIME ZONE 'UTC' AS DATE) = CAST(now() AT TIME ZONE 'UTC' AS DATE)
         ORDER BY id DESC
         LIMIT 1",
    )
    .bind(source_url)
    .fetch_optional(&mut *tx)
    .await
    .map_err(classify_sqlx)?;

    let page_id = if let Some(row) = existing {
        let page_id: i64 = row.get("id");
        sqlx::query(
            "UPDATE raw.pages
             SET domain = $2,
                 dtype = $3,
                 status_code = $4,
                 content_type = $5,
                 crawled_at = now(),
                 final_url = $6,
                 redirect_chain = $7,
                 robots_trace = $8,
                 source_observation = $9,
                 title = $10,
                 meta_desc = $11,
                 canonical = $12,
                 word_count = $13,
                 content = $14,
                 raw_html = $15,
                 raw_html_bytes = $16,
                 content_hash = $17,
                 processed = false
             WHERE id = $1",
        )
        .bind(page_id)
        .bind(&domain)
        .bind(dtype)
        .bind(status_code as i16)
        .bind(content_type)
        .bind(final_url)
        .bind(redirect_chain_json.clone())
        .bind(robots_trace_json.clone())
        .bind(source_observation_json.clone())
        .bind(&meta.title)
        .bind(&meta.meta_desc)
        .bind(&meta.canonical)
        .bind(meta.word_count as i32)
        .bind(Json(content.clone()))
        .bind(raw_html)
        .bind(raw_html.len() as i32)
        .bind(&content_hash)
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
        page_id
    } else {
        let row = sqlx::query(
            "INSERT INTO raw.pages
             (url, domain, dtype, status_code, content_type, crawled_at, final_url, redirect_chain, robots_trace,
              source_observation, title, meta_desc, canonical, word_count, content, raw_html, raw_html_bytes,
              content_hash, processed)
             VALUES ($1, $2, $3, $4, $5, now(), $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, false)
             RETURNING id",
        )
        .bind(source_url)
        .bind(&domain)
        .bind(dtype)
        .bind(status_code as i16)
        .bind(content_type)
        .bind(final_url)
        .bind(redirect_chain_json.clone())
        .bind(robots_trace_json.clone())
        .bind(source_observation_json.clone())
        .bind(&meta.title)
        .bind(&meta.meta_desc)
        .bind(&meta.canonical)
        .bind(meta.word_count as i32)
        .bind(Json(content.clone()))
        .bind(raw_html)
        .bind(raw_html.len() as i32)
        .bind(&content_hash)
        .fetch_one(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
        row.get("id")
    };

    sqlx::query("DELETE FROM raw.sections WHERE page_id = $1")
        .bind(page_id)
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;

    for section in &sections {
        sqlx::query(
            "INSERT INTO raw.sections
             (page_id, heading_path, heading_level, section_order, section_type, content_md, content_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(page_id)
        .bind(&section.heading_path)
        .bind(section.heading_level as i16)
        .bind(section.section_order as i32)
        .bind(&section.section_type)
        .bind(&section.content_md)
        .bind(&section.content_hash)
        .execute(&mut *tx)
        .await
        .map_err(classify_sqlx)?;
    }

    tx.commit().await.map_err(classify_sqlx)?;
    let _ = record_content_hash_alias_linkage(pool, page_id, source_url, final_url, &content_hash)
        .await;
    Ok(SavedRawPage {
        page_id,
        section_count: sections.len(),
        content_hash,
    })
}

pub async fn record_content_hash_alias_linkage(
    pool: &PgPool,
    alias_page_id: i64,
    source_url: &str,
    final_url: &str,
    content_hash: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    if content_hash.trim().is_empty() {
        return Ok(());
    }
    let Some(row) = sqlx::query(
        "SELECT id
         FROM raw.pages
         WHERE content_hash = $1
           AND id <> $2
         ORDER BY id ASC
         LIMIT 1",
    )
    .bind(content_hash)
    .bind(alias_page_id)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    else {
        return Ok(());
    };
    let canonical_page_id: i64 = row.get("id");
    if canonical_page_id == alias_page_id {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO raw.page_content_aliases
         (alias_page_id, canonical_page_id, source_url, final_url, content_hash)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (alias_page_id) DO UPDATE
         SET canonical_page_id = EXCLUDED.canonical_page_id,
             source_url = EXCLUDED.source_url,
             final_url = EXCLUDED.final_url,
             content_hash = EXCLUDED.content_hash,
             updated_at = now()",
    )
    .bind(alias_page_id)
    .bind(canonical_page_id)
    .bind(source_url)
    .bind(final_url)
    .bind(content_hash)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

struct RobotsRuleSet {
    allowed: bool,
    matched_rule: Option<String>,
    allow_rules: Vec<String>,
    disallow_rules: Vec<String>,
}

fn parse_robots_rules(body: &str, user_agent: &str, path: &str) -> RobotsRuleSet {
    let mut current_block_matches = false;
    let mut allow_rules = Vec::new();
    let mut disallow_rules = Vec::new();
    let target_agent = user_agent.to_ascii_lowercase();

    for raw_line in body.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            current_block_matches = false;
            continue;
        }
        if let Some(value) = line.strip_prefix("User-agent:") {
            let agent = value.trim().to_ascii_lowercase();
            current_block_matches |= agent == "*" || agent == target_agent;
            continue;
        }
        if !current_block_matches {
            continue;
        }
        if let Some(value) = line.strip_prefix("Allow:") {
            let rule = value.trim().to_string();
            if !rule.is_empty() {
                allow_rules.push(rule);
            }
        } else if let Some(value) = line.strip_prefix("Disallow:") {
            let rule = value.trim().to_string();
            if !rule.is_empty() {
                disallow_rules.push(rule);
            }
        }
    }

    let best_allow = best_matching_rule(&allow_rules, path);
    let best_disallow = best_matching_rule(&disallow_rules, path);
    let allowed = match (&best_allow, &best_disallow) {
        (Some(allow), Some(disallow)) => allow.len() >= disallow.len(),
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => true,
    };
    let matched_rule = if allowed {
        best_allow.or(best_disallow)
    } else {
        best_disallow
    };

    RobotsRuleSet {
        allowed,
        matched_rule,
        allow_rules,
        disallow_rules,
    }
}

fn best_matching_rule(rules: &[String], path: &str) -> Option<String> {
    rules
        .iter()
        .filter(|rule| path.starts_with(rule.as_str()) || rule.as_str() == "/")
        .max_by_key(|rule| rule.len())
        .cloned()
}

pub async fn mark_crawl_done(
    pool: &PgPool,
    url_norm: &str,
    http_status: i32,
    notes: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    sqlx::query(
        "UPDATE serp.crawl_queue
         SET status = 'done',
             http_status = $2,
             next_attempt_at = now(),
             locked_until = NULL,
             last_error = NULL,
             notes = $3
         WHERE url_norm = $1",
    )
    .bind(url_norm)
    .bind(http_status)
    .bind(notes)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub async fn mark_crawl_failed(
    pool: &PgPool,
    url_norm: &str,
    http_status: Option<i32>,
    error: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    sqlx::query(
        "UPDATE serp.crawl_queue
         SET status = 'failed',
             http_status = $2,
             next_attempt_at = now(),
             locked_until = NULL,
             last_error = left($3, 2000),
             notes = left($3, 2000)
         WHERE url_norm = $1",
    )
    .bind(url_norm)
    .bind(http_status)
    .bind(error)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub async fn mark_crawl_retry(
    pool: &PgPool,
    url_norm: &str,
    http_status: Option<i32>,
    error: &str,
    attempt_count: i32,
) -> std::result::Result<(), primitives::errors::DomainError> {
    let delay_sec = next_crawl_retry_delay_sec(attempt_count);
    sqlx::query(
        "UPDATE serp.crawl_queue
         SET status = 'pending',
             http_status = $2,
             next_attempt_at = now() + make_interval(secs => $3::int),
             locked_until = NULL,
             last_error = left($4, 2000),
             notes = concat_ws('; ', nullif(notes, ''), left($4, 1800), concat('retry_in_sec=', $3::text))
         WHERE url_norm = $1",
    )
    .bind(url_norm)
    .bind(http_status)
    .bind(delay_sec)
    .bind(error)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub fn should_retry_http_status(status_code: i32) -> bool {
    status_code == 408
        || status_code == 409
        || status_code == 425
        || status_code == 429
        || status_code >= 500
}

pub fn should_retry_crawl_attempt(attempt_count: i32) -> bool {
    attempt_count > 0 && attempt_count < MAX_CRAWL_ATTEMPTS
}

pub fn next_crawl_retry_delay_sec(attempt_count: i32) -> i32 {
    match attempt_count {
        0 | 1 => 60,
        2 => 300,
        3 => 1800,
        _ => 7200,
    }
}

pub async fn load_raw_sections_by_page_ids(
    pool: &PgPool,
    page_ids: &[i64],
) -> std::result::Result<Vec<RawSectionRecord>, primitives::errors::DomainError> {
    if page_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT s.id, s.page_id, p.url AS source_url, p.domain AS source_domain,
                coalesce(p.dtype, '') AS source_dtype,
                coalesce(s.heading_path, '') AS heading_path,
                coalesce(s.section_type, '') AS section_type,
                s.content_md, coalesce(s.content_hash, '') AS content_hash
         FROM raw.sections s
         JOIN raw.pages p ON p.id = s.page_id
         WHERE s.page_id = ANY($1)
         ORDER BY array_position($1, s.page_id), s.section_order, s.id",
    )
    .bind(page_ids)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|row| RawSectionRecord {
            id: row.get("id"),
            page_id: row.get("page_id"),
            source_url: row.get("source_url"),
            source_domain: row.get("source_domain"),
            source_dtype: row.get("source_dtype"),
            heading_path: row.get("heading_path"),
            section_type: row.get("section_type"),
            content_md: row.get("content_md"),
            content_hash: row.get("content_hash"),
        })
        .collect())
}

pub async fn ingest_raw_pages_into_verified(
    pool: &PgPool,
    _run_id: &str,
    context_key: &str,
    raw_page_ids: &[i64],
) -> std::result::Result<RawKnowledgeIngestionReport, primitives::errors::DomainError> {
    if raw_page_ids.is_empty() {
        return Ok(RawKnowledgeIngestionReport::default());
    }
    let mut report = RawKnowledgeIngestionReport {
        raw_page_count: raw_page_ids.len(),
        ..RawKnowledgeIngestionReport::default()
    };
    let sections = load_raw_sections_by_page_ids(pool, raw_page_ids).await?;
    report.raw_section_count = sections.len();
    let expert_core_report =
        super::expert_extraction_core::run_expert_extraction_core(_run_id, context_key, &sections);
    report.expert_blocked_section_count = expert_core_report.blocked_section_count;
    report.expert_needs_hitl_section_count = expert_core_report.needs_hitl_section_count;
    report.expert_verified_ready_section_count = expert_core_report.verified_ready_section_count;
    report.expert_triple_count = expert_core_report.triple_count;
    if let Ok(summary_json) = serde_json::to_string(&expert_core_report) {
        tracing::info!(
            run_id = _run_id,
            context_key,
            raw_section_count = report.raw_section_count,
            expert_extraction_core = %summary_json,
            "evaluated expert extraction core inside raw_knowledge_ingestion"
        );
    }

    for section in &sections {
        if section.content_md.trim().is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO raw.section_context_candidates
             (raw_section_id, context_key, confidence, source_type, mapping_reason, status)
             VALUES ($1, $2, 0.7500, $3, 'raw_knowledge_ingestion@1', 'accepted')
             ON CONFLICT (raw_section_id, context_key) DO UPDATE
             SET confidence = GREATEST(raw.section_context_candidates.confidence, EXCLUDED.confidence),
                 source_type = EXCLUDED.source_type,
                 mapping_reason = EXCLUDED.mapping_reason,
                 status = EXCLUDED.status,
                 updated_at = now()",
        )
        .bind(section.id)
        .bind(context_key)
        .bind(source_type_for_section(section))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        ensure_source(pool, section).await?;

        let Some(extraction) = truth_extraction_llm_adapter::extract_rule_candidates(
            &truth_extraction_llm_adapter::TruthExtractionInput {
                context_key: context_key.to_string(),
                raw_section_id: section.id,
                source_url: section.source_url.clone(),
                source_domain: section.source_domain.clone(),
                heading_path: section.heading_path.clone(),
                raw_text: section.content_md.clone(),
                source_snapshot_hash: section.content_hash.clone(),
            },
        )
        .await?
        else {
            report.extraction_provider_unavailable = true;
            continue;
        };
        if extraction.candidates.is_empty() {
            continue;
        }

        let written =
            persist_extracted_rule_candidates(pool, context_key, section, &extraction).await?;
        report.extracted_rule_count += written;
    }

    let section_ids: Vec<i64> = sections.iter().map(|section| section.id).collect();
    let adjudication =
        adjudicate_persisted_rule_candidates(pool, context_key, &section_ids).await?;
    report.verified_rule_count = adjudication.verified_rule_count;
    report.needs_hitl_candidate_count = adjudication.needs_hitl_candidate_count;
    report
        .changed_truth_keys
        .extend(adjudication.changed_truth_keys);

    report.changed_truth_keys.sort();
    report.changed_truth_keys.dedup();
    Ok(report)
}

async fn ensure_source(
    pool: &PgPool,
    section: &RawSectionRecord,
) -> std::result::Result<(), primitives::errors::DomainError> {
    let source_label = if section.source_domain.trim().is_empty() {
        section.source_url.clone()
    } else {
        section.source_domain.clone()
    };
    sqlx::query(
        "INSERT INTO kb.sources
         (source_key, source_type, source_label, base_url, trust_level, status)
         VALUES ($1, $2, $3, $4, $5, 'active')
         ON CONFLICT (source_key) DO UPDATE
         SET source_label = EXCLUDED.source_label,
             base_url = EXCLUDED.base_url,
             status = 'active',
             updated_at = now()",
    )
    .bind(&section.source_url)
    .bind(source_type_for_section(section))
    .bind(source_label)
    .bind(&section.source_url)
    .bind(source_trust_level(section))
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

fn source_type_for_section(section: &RawSectionRecord) -> &'static str {
    let dtype = section.source_dtype.to_ascii_lowercase();
    let domain = section.source_domain.to_ascii_lowercase();
    if dtype.contains("government")
        || dtype.contains("official")
        || domain.contains(".gov")
        || domain.starts_with("gov.")
        || domain.contains("mfa.")
        || domain.contains("mid.")
        || domain.contains("embassy")
        || domain.contains("consulate")
    {
        "government"
    } else if dtype.contains("vfs") || domain.contains("vfsglobal") {
        "vfs"
    } else if dtype.contains("agency") {
        "niche_agency"
    } else if dtype.contains("forum")
        || domain.contains("forum")
        || domain.contains("reddit.")
        || domain.contains("quora.")
        || domain.contains("stackexchange")
    {
        "forum"
    } else if dtype.contains("editorial")
        || domain.contains("news")
        || domain.contains("blog")
        || domain.contains("media")
        || domain.contains("magazine")
        || domain.contains("medium.")
    {
        "editorial"
    } else {
        "low_trust"
    }
}

fn source_trust_level(section: &RawSectionRecord) -> i32 {
    match source_type_for_section(section) {
        "government" => 5,
        "vfs" => 4,
        "editorial" => 3,
        "niche_agency" => 2,
        "forum" | "low_trust" => 1,
        _ => 1,
    }
}

async fn persist_extracted_rule_candidates(
    pool: &PgPool,
    context_key: &str,
    section: &RawSectionRecord,
    extraction: &truth_extraction_llm_adapter::TruthExtractionResponse,
) -> std::result::Result<usize, primitives::errors::DomainError> {
    for (ordinal, rule) in extraction.candidates.iter().enumerate() {
        let candidate_id = blake3_hex(
            format!(
                "{}|{}|{}|{}|{}|{}",
                context_key,
                section.id,
                ordinal,
                rule.role,
                rule.concept_canonical_key,
                rule.evidence_quote
            )
            .as_bytes(),
        );
        let source_snapshot_hash = if section.content_hash.trim().is_empty() {
            content_hash_v1(&section.content_md)
        } else {
            section.content_hash.clone()
        };
        let validation = validate_truth_candidate(
            &TruthCandidateRuntime {
                rule_candidate_id: candidate_id.clone(),
                context_key: context_key.to_string(),
                role: rule.role.clone(),
                concept_canonical_key: rule.concept_canonical_key.clone(),
                raw_mention: rule.raw_mention.clone(),
                params: truth_param_value_from_json(&rule.params),
                scope: truth_param_value_from_json(&rule.scope),
                severity: rule.severity.clone(),
                derivation_type: rule.derivation_type.clone(),
                confidence: rule.confidence,
                evidence_section_id: rule.evidence_section_id.unwrap_or(section.id),
                evidence_quote: rule.evidence_quote.clone(),
                span_start: rule.span_start,
                span_end: rule.span_end,
                source_key: section.source_url.clone(),
                source_tier: source_type_for_section(section).to_string(),
                source_snapshot_hash: source_snapshot_hash.clone(),
                is_numeric: rule.is_numeric,
                is_range: rule.is_range,
                is_incomplete: rule.is_incomplete,
                uncertainty_flags: rule.uncertainty_flags.clone(),
            },
            &section.content_md,
        );
        let mut uncertainty_flags = rule.uncertainty_flags.clone();
        for issue in &validation.issues {
            let flag = format!("validator:{}", issue.code);
            if !uncertainty_flags.iter().any(|existing| existing == &flag) {
                uncertainty_flags.push(flag);
            }
        }
        sqlx::query(
            "INSERT INTO extracted.rule_candidates (
                 rule_candidate_id,
                 context_key,
                 raw_section_id,
                 role,
                 concept_canonical_key,
                 raw_mention,
                 params,
                 scope,
                 severity,
                 applies_to_profiles,
                 exceptions_raw,
                 conditions_raw,
                 alternatives,
                 modality_raw,
                 derivation_type,
                 is_numeric,
                 is_range,
                 is_incomplete,
                 confidence,
                 evidence_section_id,
                 evidence_quote,
                 span_start,
                 span_end,
                 source_key,
                 source_snapshot_hash,
                 llm_provider,
                 llm_model,
                 prompt_version,
                 epistemic_status,
                 uncertainty_flags
             )
             VALUES (
                 $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
                 $21, $22, $23, $24, $25, $26, $27, $28, $29, $30
             )
             ON CONFLICT (rule_candidate_id) DO UPDATE
             SET concept_canonical_key = EXCLUDED.concept_canonical_key,
                 raw_mention = EXCLUDED.raw_mention,
                 params = EXCLUDED.params,
                 scope = EXCLUDED.scope,
                 severity = EXCLUDED.severity,
                 applies_to_profiles = EXCLUDED.applies_to_profiles,
                 exceptions_raw = EXCLUDED.exceptions_raw,
                 conditions_raw = EXCLUDED.conditions_raw,
                 alternatives = EXCLUDED.alternatives,
                 modality_raw = EXCLUDED.modality_raw,
                 derivation_type = EXCLUDED.derivation_type,
                 is_numeric = EXCLUDED.is_numeric,
                 is_range = EXCLUDED.is_range,
                 is_incomplete = EXCLUDED.is_incomplete,
                 confidence = EXCLUDED.confidence,
                 evidence_quote = EXCLUDED.evidence_quote,
                 span_start = EXCLUDED.span_start,
                 span_end = EXCLUDED.span_end,
                 source_key = EXCLUDED.source_key,
                 source_snapshot_hash = EXCLUDED.source_snapshot_hash,
                 llm_provider = EXCLUDED.llm_provider,
                 llm_model = EXCLUDED.llm_model,
                 prompt_version = EXCLUDED.prompt_version,
                 epistemic_status = EXCLUDED.epistemic_status,
                 uncertainty_flags = EXCLUDED.uncertainty_flags,
                 updated_at = now()",
        )
        .bind(candidate_id)
        .bind(context_key)
        .bind(section.id)
        .bind(&rule.role)
        .bind(&rule.concept_canonical_key)
        .bind(&rule.raw_mention)
        .bind(Json::<Value>(rule.params.clone()))
        .bind(Json::<Value>(rule.scope.clone()))
        .bind(&rule.severity)
        .bind(Json::<Value>(json!(rule.applies_to_profiles)))
        .bind(rule.exceptions_raw.clone().unwrap_or_default())
        .bind(rule.conditions_raw.clone().unwrap_or_default())
        .bind(Json::<Value>(rule.alternatives.clone()))
        .bind(rule.modality_raw.clone().unwrap_or_default())
        .bind(&rule.derivation_type)
        .bind(rule.is_numeric)
        .bind(rule.is_range)
        .bind(rule.is_incomplete)
        .bind(rule.confidence)
        .bind(rule.evidence_section_id.unwrap_or(section.id))
        .bind(&rule.evidence_quote)
        .bind(rule.span_start as i32)
        .bind(rule.span_end as i32)
        .bind(&section.source_url)
        .bind(&source_snapshot_hash)
        .bind(&extraction.provider_key)
        .bind(&extraction.model_key)
        .bind(&extraction.prompt_version)
        .bind(&validation.epistemic_status)
        .bind(Json::<Value>(json!(uncertainty_flags)))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }
    Ok(extraction.candidates.len())
}

#[derive(Debug, Clone, Default)]
struct TruthAdjudicationPersistReport {
    verified_rule_count: usize,
    needs_hitl_candidate_count: usize,
    changed_truth_keys: Vec<String>,
}

async fn adjudicate_persisted_rule_candidates(
    pool: &PgPool,
    context_key: &str,
    section_ids: &[i64],
) -> std::result::Result<TruthAdjudicationPersistReport, primitives::errors::DomainError> {
    if section_ids.is_empty() {
        return Ok(TruthAdjudicationPersistReport::default());
    }

    let candidates =
        load_persisted_candidates_for_adjudication(pool, context_key, section_ids).await?;
    if candidates.is_empty() {
        return Ok(TruthAdjudicationPersistReport::default());
    }

    let mut report = TruthAdjudicationPersistReport::default();
    let mut groups: BTreeMap<(String, String, String), Vec<PersistedCandidateForAdjudication>> =
        BTreeMap::new();
    for candidate in candidates {
        groups
            .entry((
                candidate.context_key.clone(),
                candidate.role.clone(),
                candidate.concept_canonical_key.clone(),
            ))
            .or_default()
            .push(candidate);
    }

    for ((group_context_key, group_role, group_concept), group_candidates) in groups {
        let semantic_slot_id =
            semantic_rule_instance_id(&group_context_key, &group_role, &group_concept);
        let structured: Vec<TruthStructuredCandidate> = group_candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "structured")
            .map(to_truth_structured_candidate)
            .collect();
        let needs_hitl_ids: Vec<String> = group_candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "needs_hitl")
            .map(|candidate| candidate.rule_candidate_id.clone())
            .collect();
        let rejected_ids: Vec<String> = group_candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "rejected")
            .map(|candidate| candidate.rule_candidate_id.clone())
            .collect();

        if structured.is_empty() {
            if !needs_hitl_ids.is_empty() {
                for candidate_id in &needs_hitl_ids {
                    update_candidate_epistemic_status(pool, candidate_id, "needs_hitl").await?;
                }
                report.needs_hitl_candidate_count += needs_hitl_ids.len();
                demote_verified_semantic_slot(
                    pool,
                    &semantic_slot_id,
                    "disputed",
                    "needs_hitl",
                    "truth_adjudication@1",
                    "no_structured_candidates_for_semantic_identity",
                )
                .await?;
                report
                    .changed_truth_keys
                    .push(format!("verified.rule_instance:{semantic_slot_id}"));
            } else if !rejected_ids.is_empty() {
                for candidate_id in &rejected_ids {
                    update_candidate_epistemic_status(pool, candidate_id, "rejected").await?;
                }
                demote_verified_semantic_slot(
                    pool,
                    &semantic_slot_id,
                    "deprecated",
                    "not_admissible",
                    "truth_adjudication@1",
                    "all_candidates_rejected_for_semantic_identity",
                )
                .await?;
                report
                    .changed_truth_keys
                    .push(format!("verified.rule_instance:{semantic_slot_id}"));
            }
            continue;
        }

        let adjudication = adjudicate_truth_candidates(&structured);
        match adjudication.overall_status.as_str() {
            "verified" => {
                for decision in &adjudication.decisions {
                    if decision.decision == "verified" {
                        update_candidate_epistemic_status(
                            pool,
                            &decision.rule_candidate_id,
                            "verified",
                        )
                        .await?;
                    }
                }
                let canonical = pick_canonical_candidate(&group_candidates, &adjudication)?;
                upsert_verified_rule_instance_from_candidate(
                    pool,
                    &semantic_slot_id,
                    canonical,
                    "admissible",
                    &canonical_verification_method(&adjudication),
                    &canonical_adjudication_reason(&adjudication),
                )
                .await?;
                report.verified_rule_count += 1;
                report
                    .changed_truth_keys
                    .push(format!("verified.rule_instance:{semantic_slot_id}"));
            }
            "needs_hitl" => {
                for decision in &adjudication.decisions {
                    if decision.decision == "needs_hitl" {
                        update_candidate_epistemic_status(
                            pool,
                            &decision.rule_candidate_id,
                            "needs_hitl",
                        )
                        .await?;
                    }
                }
                report.needs_hitl_candidate_count += adjudication
                    .decisions
                    .iter()
                    .filter(|decision| decision.decision == "needs_hitl")
                    .count();
                demote_verified_semantic_slot(
                    pool,
                    &semantic_slot_id,
                    "disputed",
                    "needs_hitl",
                    "truth_adjudication@1",
                    &canonical_adjudication_reason(&adjudication),
                )
                .await?;
                report
                    .changed_truth_keys
                    .push(format!("verified.rule_instance:{semantic_slot_id}"));
            }
            _ => {
                for decision in &adjudication.decisions {
                    update_candidate_epistemic_status(
                        pool,
                        &decision.rule_candidate_id,
                        "rejected",
                    )
                    .await?;
                }
                demote_verified_semantic_slot(
                    pool,
                    &semantic_slot_id,
                    "deprecated",
                    "not_admissible",
                    "truth_adjudication@1",
                    &canonical_adjudication_reason(&adjudication),
                )
                .await?;
                report
                    .changed_truth_keys
                    .push(format!("verified.rule_instance:{semantic_slot_id}"));
            }
        }
    }

    report.changed_truth_keys.sort();
    report.changed_truth_keys.dedup();
    Ok(report)
}

async fn load_persisted_candidates_for_adjudication(
    pool: &PgPool,
    context_key: &str,
    section_ids: &[i64],
) -> std::result::Result<Vec<PersistedCandidateForAdjudication>, primitives::errors::DomainError> {
    let rows = sqlx::query(
        "SELECT c.rule_candidate_id,
                c.context_key,
                c.role,
                c.concept_canonical_key,
                c.params,
                c.source_key,
                coalesce(src.source_type, '') AS source_tier,
                c.confidence::float8 AS confidence,
                c.evidence_section_id,
                c.evidence_quote,
                c.span_start,
                c.span_end,
                c.source_snapshot_hash,
                c.prompt_version,
                c.llm_model,
                c.epistemic_status,
                c.uncertainty_flags,
                EXISTS (
                    SELECT 1
                    FROM kb.concepts concept
                    WHERE concept.concept_key = c.concept_canonical_key
                      AND concept.status = 'active'
                ) AS concept_exists
         FROM extracted.rule_candidates c
         LEFT JOIN kb.sources src ON src.source_key = c.source_key
         WHERE c.context_key = $1
           AND c.raw_section_id = ANY($2)
         ORDER BY c.created_at, c.rule_candidate_id",
    )
    .bind(context_key)
    .bind(section_ids)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut candidates = Vec::with_capacity(rows.len());
    for row in rows {
        let uncertainty_flags: Vec<String> = row
            .try_get::<Json<Vec<String>>, _>("uncertainty_flags")
            .map(|json| json.0)
            .unwrap_or_default();
        let mut epistemic_status: String = row.get("epistemic_status");
        let concept_exists: bool = row.get("concept_exists");
        if epistemic_status != "rejected" && !concept_exists {
            epistemic_status = "needs_hitl".to_string();
            if let Ok(rule_candidate_id) = row.try_get::<String, _>("rule_candidate_id") {
                update_candidate_epistemic_status(pool, &rule_candidate_id, &epistemic_status)
                    .await?;
            }
        }
        let freshness_class = classify_candidate_freshness(&uncertainty_flags);
        let completeness_class =
            classify_candidate_completeness(&epistemic_status, &uncertainty_flags);
        if epistemic_status == "structured"
            && (freshness_class != "fresh" || completeness_class != "complete")
        {
            epistemic_status = "needs_hitl".to_string();
            if let Ok(rule_candidate_id) = row.try_get::<String, _>("rule_candidate_id") {
                update_candidate_epistemic_status(pool, &rule_candidate_id, &epistemic_status)
                    .await?;
            }
        }
        candidates.push(PersistedCandidateForAdjudication {
            rule_candidate_id: row.get("rule_candidate_id"),
            context_key: row.get("context_key"),
            role: row.get("role"),
            concept_canonical_key: row.get("concept_canonical_key"),
            params: row
                .try_get::<Json<Value>, _>("params")
                .map(|json| truth_param_value_from_json(&json.0))
                .unwrap_or_default(),
            source_key: row.get("source_key"),
            source_tier: row.get("source_tier"),
            confidence: row.get::<f64, _>("confidence"),
            freshness_class,
            completeness_class,
            evidence_section_id: row.get("evidence_section_id"),
            evidence_quote: row.get("evidence_quote"),
            span_start: row.get("span_start"),
            span_end: row.get("span_end"),
            source_snapshot_hash: row.get("source_snapshot_hash"),
            prompt_version: row.get("prompt_version"),
            model_version: row.get("llm_model"),
            epistemic_status,
        });
    }
    Ok(candidates)
}

fn to_truth_structured_candidate(
    candidate: &PersistedCandidateForAdjudication,
) -> TruthStructuredCandidate {
    TruthStructuredCandidate {
        rule_candidate_id: candidate.rule_candidate_id.clone(),
        context_key: candidate.context_key.clone(),
        role: candidate.role.clone(),
        concept_canonical_key: candidate.concept_canonical_key.clone(),
        params: candidate.params.clone(),
        source_key: candidate.source_key.clone(),
        source_tier: candidate.source_tier.clone(),
        confidence: candidate.confidence,
        freshness_class: candidate.freshness_class.clone(),
        completeness_class: candidate.completeness_class.clone(),
        evidence_quote: candidate.evidence_quote.clone(),
        epistemic_status: candidate.epistemic_status.clone(),
    }
}

fn truth_param_value_from_json(value: &Value) -> TruthParamValue {
    match value {
        Value::Null => TruthParamValue::Null,
        Value::Bool(value) => TruthParamValue::Bool(*value),
        Value::Number(value) => {
            if let Some(integer) = value.as_i64() {
                TruthParamValue::Integer(integer)
            } else if let Some(decimal) = value.as_f64() {
                TruthParamValue::Decimal(decimal)
            } else {
                TruthParamValue::Null
            }
        }
        Value::String(value) => TruthParamValue::Text(value.clone()),
        Value::Array(values) => TruthParamValue::List(
            values
                .iter()
                .map(truth_param_value_from_json)
                .collect::<Vec<_>>(),
        ),
        Value::Object(values) => TruthParamValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), truth_param_value_from_json(value)))
                .collect(),
        ),
    }
}

fn truth_param_value_to_json(value: &TruthParamValue) -> Value {
    match value {
        TruthParamValue::Null => Value::Null,
        TruthParamValue::Bool(value) => Value::Bool(*value),
        TruthParamValue::Integer(value) => json!(value),
        TruthParamValue::Decimal(value) => json!(value),
        TruthParamValue::Text(value) => Value::String(value.clone()),
        TruthParamValue::List(values) => {
            Value::Array(values.iter().map(truth_param_value_to_json).collect())
        }
        TruthParamValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), truth_param_value_to_json(value)))
                .collect(),
        ),
    }
}

fn classify_candidate_freshness(uncertainty_flags: &[String]) -> String {
    if uncertainty_flags.iter().any(|flag| {
        flag == "stale_source" || flag == "validator:freshness_or_temporality_ambiguous"
    }) {
        "stale".to_string()
    } else if uncertainty_flags
        .iter()
        .any(|flag| flag == "freshness_ambiguous" || flag == "temporal_ambiguous")
    {
        "watch".to_string()
    } else {
        "fresh".to_string()
    }
}

fn classify_candidate_completeness(epistemic_status: &str, uncertainty_flags: &[String]) -> String {
    if uncertainty_flags.iter().any(|flag| {
        matches!(
            flag.as_str(),
            "validator:fee_item_incomplete"
                | "validator:timeline_item_incomplete"
                | "validator:where_to_apply_incomplete"
                | "validator:role_specific_params_missing"
                | "validator:missing_numeric_params"
                | "validator:missing_range_bounds"
                | "validator:candidate_marked_incomplete"
        )
    }) {
        "incomplete".to_string()
    } else if epistemic_status == "needs_hitl" {
        "partial".to_string()
    } else {
        "complete".to_string()
    }
}

async fn update_candidate_epistemic_status(
    pool: &PgPool,
    rule_candidate_id: &str,
    epistemic_status: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    sqlx::query(
        "UPDATE extracted.rule_candidates
         SET epistemic_status = $2,
             updated_at = now()
         WHERE rule_candidate_id = $1",
    )
    .bind(rule_candidate_id)
    .bind(epistemic_status)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

fn semantic_rule_instance_id(context_key: &str, role: &str, concept_canonical_key: &str) -> String {
    blake3_hex(format!("{context_key}|{role}|{concept_canonical_key}").as_bytes())
}

fn canonical_role_key(role: &str) -> String {
    role.trim().to_ascii_lowercase()
}

fn pick_canonical_candidate<'a>(
    group_candidates: &'a [PersistedCandidateForAdjudication],
    adjudication: &primitives::truth_candidates::TruthAdjudicationResult,
) -> std::result::Result<&'a PersistedCandidateForAdjudication, primitives::errors::DomainError> {
    let verified_ids: Vec<&str> = adjudication
        .decisions
        .iter()
        .filter(|decision| decision.decision == "verified")
        .map(|decision| decision.rule_candidate_id.as_str())
        .collect();
    group_candidates
        .iter()
        .filter(|candidate| {
            verified_ids
                .iter()
                .any(|id| *id == candidate.rule_candidate_id)
        })
        .max_by(|left, right| {
            left.confidence
                .partial_cmp(&right.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .ok_or_else(|| primitives::errors::DomainError::UnexpectedBug {
            message: "truth adjudication produced verified verdict without canonical candidate"
                .to_string(),
        })
}

fn canonical_verification_method(
    adjudication: &primitives::truth_candidates::TruthAdjudicationResult,
) -> String {
    adjudication
        .decisions
        .iter()
        .find(|decision| decision.decision == "verified")
        .map(|decision| decision.verification_method.clone())
        .unwrap_or_else(|| "truth_adjudication@1".to_string())
}

fn canonical_adjudication_reason(
    adjudication: &primitives::truth_candidates::TruthAdjudicationResult,
) -> String {
    adjudication
        .decisions
        .first()
        .map(|decision| decision.adjudication_reason.clone())
        .unwrap_or_else(|| "truth_adjudication_result_missing".to_string())
}

async fn upsert_verified_rule_instance_from_candidate(
    pool: &PgPool,
    rule_instance_id: &str,
    candidate: &PersistedCandidateForAdjudication,
    publish_admissibility: &str,
    verification_method: &str,
    adjudication_reason: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    let role_key = canonical_role_key(&candidate.role);
    sqlx::query(
        "INSERT INTO verified.rule_instances (
             rule_instance_id,
             context_key,
             rule_type_key,
             concept_key,
             role_type,
             params,
             status,
             source_key,
             confidence,
             effective_from,
             rule_candidate_id,
             evidence_section_id,
             evidence_quote,
             span_start,
             span_end,
             source_snapshot_hash,
             verification_method,
             adjudication_reason,
             publish_admissibility,
             freshness_class,
             completeness_class,
             registry_version,
             prompt_version,
             model_version,
             pipeline_version
         )
         VALUES (
             $1, $2, $3, $4, $5, $6, 'verified', $7, $8, current_date,
             $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23
         )
         ON CONFLICT (rule_instance_id) DO UPDATE
         SET rule_type_key = EXCLUDED.rule_type_key,
             concept_key = EXCLUDED.concept_key,
             role_type = EXCLUDED.role_type,
             params = EXCLUDED.params,
             status = EXCLUDED.status,
             source_key = EXCLUDED.source_key,
             confidence = EXCLUDED.confidence,
             effective_from = EXCLUDED.effective_from,
             rule_candidate_id = EXCLUDED.rule_candidate_id,
             evidence_section_id = EXCLUDED.evidence_section_id,
             evidence_quote = EXCLUDED.evidence_quote,
             span_start = EXCLUDED.span_start,
             span_end = EXCLUDED.span_end,
             source_snapshot_hash = EXCLUDED.source_snapshot_hash,
             verification_method = EXCLUDED.verification_method,
             adjudication_reason = EXCLUDED.adjudication_reason,
             publish_admissibility = EXCLUDED.publish_admissibility,
             freshness_class = EXCLUDED.freshness_class,
             completeness_class = EXCLUDED.completeness_class,
             registry_version = EXCLUDED.registry_version,
             prompt_version = EXCLUDED.prompt_version,
             model_version = EXCLUDED.model_version,
             pipeline_version = EXCLUDED.pipeline_version,
             updated_at = now()",
    )
    .bind(rule_instance_id)
    .bind(&candidate.context_key)
    .bind(&role_key)
    .bind(&candidate.concept_canonical_key)
    .bind(&role_key)
    .bind(Json::<Value>(truth_param_value_to_json(&candidate.params)))
    .bind(&candidate.source_key)
    .bind(candidate.confidence)
    .bind(&candidate.rule_candidate_id)
    .bind(candidate.evidence_section_id)
    .bind(&candidate.evidence_quote)
    .bind(candidate.span_start)
    .bind(candidate.span_end)
    .bind(&candidate.source_snapshot_hash)
    .bind(verification_method)
    .bind(adjudication_reason)
    .bind(publish_admissibility)
    .bind(&candidate.freshness_class)
    .bind(&candidate.completeness_class)
    .bind("registry@1")
    .bind(&candidate.prompt_version)
    .bind(&candidate.model_version)
    .bind("truth_adjudication_runtime@1")
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

async fn demote_verified_semantic_slot(
    pool: &PgPool,
    rule_instance_id: &str,
    status: &str,
    publish_admissibility: &str,
    verification_method: &str,
    adjudication_reason: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    sqlx::query(
        "UPDATE verified.rule_instances
         SET status = $2,
             publish_admissibility = $3,
             verification_method = $4,
             adjudication_reason = $5,
             updated_at = now()
         WHERE rule_instance_id = $1",
    )
    .bind(rule_instance_id)
    .bind(status)
    .bind(publish_admissibility)
    .bind(verification_method)
    .bind(adjudication_reason)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub async fn load_raw_sections_for_page(
    pool: &PgPool,
    page_id: i64,
) -> std::result::Result<Vec<RawSectionRecord>, primitives::errors::DomainError> {
    let rows = sqlx::query(
        "SELECT s.id, s.page_id, p.url AS source_url, p.domain AS source_domain,
                coalesce(p.dtype, '') AS source_dtype,
                coalesce(s.heading_path, '') AS heading_path,
                coalesce(s.section_type, '') AS section_type,
                s.content_md, coalesce(s.content_hash, '') AS content_hash
         FROM raw.sections s
         JOIN raw.pages p ON p.id = s.page_id
         WHERE s.page_id = $1
         ORDER BY s.section_order, s.id",
    )
    .bind(page_id)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|row| RawSectionRecord {
            id: row.get("id"),
            page_id: row.get("page_id"),
            source_url: row.get("source_url"),
            source_domain: row.get("source_domain"),
            source_dtype: row.get("source_dtype"),
            heading_path: row.get("heading_path"),
            section_type: row.get("section_type"),
            content_md: row.get("content_md"),
            content_hash: row.get("content_hash"),
        })
        .collect())
}

pub async fn load_raw_sections_by_ids(
    pool: &PgPool,
    section_ids: &[i64],
) -> std::result::Result<Vec<RawSectionRecord>, primitives::errors::DomainError> {
    if section_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT s.id, s.page_id, p.url AS source_url, p.domain AS source_domain,
                coalesce(p.dtype, '') AS source_dtype,
                coalesce(s.heading_path, '') AS heading_path,
                coalesce(s.section_type, '') AS section_type,
                s.content_md, coalesce(s.content_hash, '') AS content_hash
         FROM raw.sections s
         JOIN raw.pages p ON p.id = s.page_id
         WHERE s.id = ANY($1)
         ORDER BY array_position($1, s.id)",
    )
    .bind(section_ids)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|row| RawSectionRecord {
            id: row.get("id"),
            page_id: row.get("page_id"),
            source_url: row.get("source_url"),
            source_domain: row.get("source_domain"),
            source_dtype: row.get("source_dtype"),
            heading_path: row.get("heading_path"),
            section_type: row.get("section_type"),
            content_md: row.get("content_md"),
            content_hash: row.get("content_hash"),
        })
        .collect())
}

pub async fn emit_raw_section_qdrant_events(
    pool: &PgPool,
    run_id: &str,
    page_id: i64,
) -> std::result::Result<usize, primitives::errors::DomainError> {
    let sections = load_raw_sections_for_page(pool, page_id).await?;
    let texts = sections
        .iter()
        .filter(|section| !section.content_md.trim().is_empty())
        .map(|section| section.content_md.clone())
        .collect::<Vec<_>>();
    let voyage_api_key = std::env::var("VOYAGE_API_KEY").ok();
    let voyage_model =
        std::env::var("VOYAGE_MODEL").unwrap_or_else(|_| "voyage-3-large".to_string());
    let real_vectors = if let Some(api_key) = voyage_api_key {
        if texts.is_empty() {
            Vec::new()
        } else {
            VoyageClient::new(api_key, voyage_model.clone())
                .embed_all(&texts)
                .await
                .map_err(|err| primitives::errors::DomainError::InfraUnavailable {
                    message: format!("Voyage raw section embedding failed: {err}"),
                })?
        }
    } else {
        Vec::new()
    };
    let mut real_vector_iter = real_vectors.into_iter();
    let mut events = Vec::with_capacity(sections.len());
    for section in &sections {
        if section.content_md.trim().is_empty() {
            continue;
        }
        let (vector, embedding_model, embedding_version) =
            if let Some(vector) = real_vector_iter.next() {
                (vector, voyage_model.as_str(), "raw_section_voyage@1")
            } else {
                (
                    fingerprint_vector(&section.content_md),
                    "deterministic-fingerprint",
                    "raw_section_bootstrap@1",
                )
            };
        let entity_key = format!("raw_section:{}", section.id);
        let point_id = qdrant_point_id_v1("content_chunks", "raw_section", &entity_key);
        let mut metadata = HashMap::new();
        metadata.insert("page_id".to_string(), section.page_id.to_string());
        metadata.insert("section_id".to_string(), section.id.to_string());
        metadata.insert("heading_path".to_string(), section.heading_path.clone());
        metadata.insert("section_type".to_string(), section.section_type.clone());
        metadata.insert("content_hash".to_string(), section.content_hash.clone());
        metadata.insert("source_url".to_string(), section.source_url.clone());
        metadata.insert("source_domain".to_string(), section.source_domain.clone());
        metadata.insert("embedding_model".to_string(), embedding_model.to_string());
        metadata.insert(
            "embedding_version".to_string(),
            embedding_version.to_string(),
        );
        let vector_size = vector.len() as u32;
        let payload_bytes = QdrantUpsertCommand {
            event_id: String::new(),
            collection_name: "content_chunks".to_string(),
            entity_type: "raw_section".to_string(),
            entity_key: entity_key.clone(),
            point_id: point_id.clone(),
            vector,
            payload: None,
            distance: "cosine".to_string(),
            vector_size,
            metadata,
        }
        .encode_to_vec();
        events.push(OutboxEnvelope {
            run_id: run_id.to_string(),
            aggregate_type: "raw_section".to_string(),
            aggregate_key: entity_key.clone(),
            target_system: "qdrant".to_string(),
            event_type: "QdrantUpsertCommand".to_string(),
            payload_type: "alegria.outbox.qdrant_upsert_command.v1".to_string(),
            schema_version: 1,
            idempotency_key: content_hash_v1(&format!(
                "{entity_key}|QdrantUpsertCommand|{}",
                primitives::hash::blake3_hex(&payload_bytes)
            )),
            payload_bytes,
        });

        sqlx::query(
            "INSERT INTO kb.qdrant_points
             (point_id, entity_type, entity_key, collection_name, embedding_model, embedding_version)
             VALUES ($1, 'raw_section', $2, 'content_chunks', $3, $4)
             ON CONFLICT (entity_type, entity_key, collection_name) DO UPDATE
             SET point_id = EXCLUDED.point_id,
                 embedding_model = EXCLUDED.embedding_model,
                 embedding_version = EXCLUDED.embedding_version,
                 updated_at = now()",
        )
        .bind(&point_id)
        .bind(&entity_key)
        .bind(embedding_model)
        .bind(embedding_version)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }
    let emitted = outbox_emit_many(pool, &events).await?;
    Ok(emitted as usize)
}

pub async fn retrieve_source_context_chunks(
    pool: &PgPool,
    query: &str,
    limit: u64,
) -> std::result::Result<Vec<SourceContextChunkState>, primitives::errors::DomainError> {
    if std::env::var("VOYAGE_API_KEY").is_err() {
        return Ok(Vec::new());
    }
    let results = semantic_search_adapter::search_by_text(query, "content_chunks", limit)
        .await
        .map_err(|err| primitives::errors::DomainError::InfraUnavailable {
            message: format!("source context semantic search failed: {err}"),
        })?;
    let mut section_ids = Vec::new();
    let mut score_by_section = HashMap::new();
    for result in results {
        let entity_key = result
            .payload
            .get("entity_key")
            .cloned()
            .unwrap_or(result.entity_key);
        let Some(id_raw) = entity_key.strip_prefix("raw_section:") else {
            continue;
        };
        let Ok(section_id) = id_raw.parse::<i64>() else {
            continue;
        };
        section_ids.push(section_id);
        score_by_section.insert(section_id, result.score);
    }
    let sections = load_raw_sections_by_ids(pool, &section_ids).await?;
    Ok(sections
        .into_iter()
        .map(|section| SourceContextChunkState {
            chunk_key: format!("raw_section:{}", section.id),
            source_url: section.source_url,
            source_domain: section.source_domain,
            heading_path: section.heading_path,
            section_type: section.section_type,
            content_md: section.content_md,
            retrieval_score: score_by_section
                .get(&section.id)
                .map(|score| format!("{score:.4}"))
                .unwrap_or_default(),
            usage_policy: "supplemental_context_not_fact_support".to_string(),
        })
        .collect())
}

fn fingerprint_vector(text: &str) -> Vec<f32> {
    let digest = primitives::hash::blake3_hex(text.as_bytes());
    let mut vector = Vec::with_capacity(16);
    for chunk in digest.as_bytes().chunks(2).take(16) {
        let Ok(hex) = std::str::from_utf8(chunk) else {
            continue;
        };
        let value = u8::from_str_radix(hex, 16).unwrap_or(0);
        vector.push((value as f32 / 127.5) - 1.0);
    }
    if vector.is_empty() {
        vector.push(0.0);
    }
    vector
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::hyper_adapter::{http1, service_fn, TokioIo};
    use anyhow::Result;
    use bytes::Bytes;
    use http_body_util::Full;
    use hyper::Response;
    use std::net::SocketAddr;
    use tokio::net::TcpListener;

    async fn spawn_test_server() -> Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr: SocketAddr = listener.local_addr()?;
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let service = service_fn(|req| async move {
                        let path = req.uri().path().to_string();
                        let response = match path.as_str() {
                            "/robots.txt" => Response::builder()
                                .status(200)
                                .body(Full::new(Bytes::from_static(
                                    b"User-agent: *\nDisallow: /blocked/\nAllow: /allowed/\n",
                                )))
                                .expect("robots response"),
                            "/redirect" => Response::builder()
                                .status(302)
                                .header("Location", "/allowed/final")
                                .body(Full::new(Bytes::new()))
                                .expect("redirect response"),
                            "/allowed/final" => Response::builder()
                                .status(200)
                                .header("Content-Type", "text/html; charset=utf-8")
                                .body(Full::new(Bytes::from_static(
                                    br#"<html><head><link rel="canonical" href="http://127.0.0.1/allowed/final"></head><body>ok</body></html>"#,
                                )))
                                .expect("final response"),
                            "/blocked/page" => Response::builder()
                                .status(200)
                                .header("Content-Type", "text/html; charset=utf-8")
                                .body(Full::new(Bytes::from_static(
                                    b"<html><body>blocked</body></html>",
                                )))
                                .expect("blocked response"),
                            _ => Response::builder()
                                .status(404)
                                .body(Full::new(Bytes::new()))
                                .expect("not found"),
                        };
                        Ok::<_, std::convert::Infallible>(response)
                    });
                    let io = TokioIo::new(stream);
                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        });
        Ok(format!("http://{}", addr))
    }

    #[tokio::test]
    async fn robots_policy_and_redirect_trace_smoke() -> Result<()> {
        let base_url = spawn_test_server().await?;
        let blocked = format!("{base_url}/blocked/page");
        let redirect = format!("{base_url}/redirect");
        let allowed = format!("{base_url}/allowed/final");

        let robots = evaluate_robots_policy(&blocked).await?;
        assert!(!robots.allowed);
        assert_eq!(robots.matched_rule.as_deref(), Some("/blocked/"));
        assert!(robots.disallow_rules.iter().any(|rule| rule == "/blocked/"));

        let fetched = fetch_html(&redirect).await?;
        assert_eq!(fetched.source_url, redirect);
        assert_eq!(fetched.final_url, allowed);
        assert!(fetched.redirect_chain.len() >= 2);
        assert_eq!(
            fetched.redirect_chain.first().map(|s| s.as_str()),
            Some(redirect.as_str())
        );
        assert_eq!(
            fetched.redirect_chain.last().map(|s| s.as_str()),
            Some(allowed.as_str())
        );

        Ok(())
    }

    #[test]
    fn crawl_retry_policy_staggers_attempts() {
        assert_eq!(next_crawl_retry_delay_sec(0), 60);
        assert_eq!(next_crawl_retry_delay_sec(1), 60);
        assert_eq!(next_crawl_retry_delay_sec(2), 300);
        assert_eq!(next_crawl_retry_delay_sec(3), 1800);
        assert_eq!(next_crawl_retry_delay_sec(4), 7200);
        assert!(should_retry_http_status(429));
        assert!(should_retry_http_status(500));
        assert!(!should_retry_http_status(404));
        assert!(should_retry_crawl_attempt(1));
        assert!(should_retry_crawl_attempt(3));
        assert!(!should_retry_crawl_attempt(4));
    }

    #[test]
    fn crawl_observation_trace_serializes_provenance_fields() {
        let trace = CrawlObservationTrace {
            source_url: "https://example.com/source".to_string(),
            final_url: "https://example.com/final".to_string(),
            redirect_hops: 1,
            redirect_chain: vec![
                "https://example.com/source".to_string(),
                "https://example.com/final".to_string(),
            ],
            robots: RobotsDecisionTrace {
                source_url: "https://example.com/source".to_string(),
                robots_url: "https://example.com/robots.txt".to_string(),
                user_agent: CRAWL_USER_AGENT.to_string(),
                robots_status: Some(200),
                allowed: true,
                matched_rule: Some("/".to_string()),
                allow_rules: vec!["/".to_string()],
                disallow_rules: vec![],
                fetch_error: None,
            },
        };
        let value = serde_json::to_value(trace).expect("serialize trace");
        assert_eq!(
            value.get("source_url").and_then(|v| v.as_str()),
            Some("https://example.com/source")
        );
        assert_eq!(
            value.get("final_url").and_then(|v| v.as_str()),
            Some("https://example.com/final")
        );
        assert!(value.get("redirect_chain").is_some());
        assert!(value.get("robots").is_some());
    }
}
