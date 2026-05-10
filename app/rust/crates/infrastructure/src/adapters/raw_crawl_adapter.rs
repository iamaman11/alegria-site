use std::collections::HashMap;

use anyhow::{Context, Result};
use contracts::generated::alegria::sync::v1::QdrantUpsertCommand;
use contracts::generated::alegria::temporal::v1::SourceContextChunkState;
use prost::Message;
use serde_json::json;
use sqlx::{types::Json, PgPool, Row};

use primitives::facts_extractor::{extract_facts_typed, ExtractionContext};
use primitives::hash::content_hash_v1;
use primitives::html_sections::{extract_meta_typed, extract_sections_typed};
use primitives::stable_id::stable_rule_instance_id;
use primitives::url_norm::domain_norm;
use runtime_models::PersistPipelineState;

use super::proto_runtime_payload_store::{build_extracted_payload_from_typed, classify_sqlx};
use super::reqwest_adapter;
use super::semantic_search_adapter;
use super::sqlx_outbox_adapter::OutboxEnvelope;
use super::sqlx_runtime_outbox_adapter::outbox_emit_many;
use super::sqlx_source_projection_adapter;
use super::voyage_api_adapter::VoyageClient;

#[derive(Debug, Clone)]
pub struct CrawlQueueItem {
    pub url: String,
    pub url_norm: String,
    pub source_type: String,
    pub dtype: String,
}

#[derive(Debug, Clone)]
pub struct FetchedHtml {
    pub final_url: String,
    pub status_code: i32,
    pub content_type: String,
    pub body: String,
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
    pub outbox_event_count: usize,
    pub changed_truth_keys: Vec<String>,
}

pub async fn claim_pending_crawl_batch(
    pool: &PgPool,
    run_id: &str,
    query_batch_key: &str,
    limit: i64,
) -> std::result::Result<Vec<CrawlQueueItem>, primitives::errors::DomainError> {
    let rows = sqlx::query(
        "WITH candidates AS (
             SELECT url_norm
             FROM serp.crawl_queue
             WHERE status = 'pending'
               AND ($2 = '' OR first_seen_run_id = $2)
               AND ($3 = '' OR query_batch_key = $3)
               AND (locked_until IS NULL OR locked_until <= now())
             ORDER BY first_seen_at
             LIMIT $1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE serp.crawl_queue q
         SET status = 'processing',
             crawl_attempt_count = q.crawl_attempt_count + 1,
             locked_until = now() + interval '10 minutes',
             notes = concat_ws('; ', nullif(q.notes, ''), 'claimed_by=raw_crawl_adapter')
         FROM candidates c
         WHERE q.url_norm = c.url_norm
         RETURNING q.url, q.url_norm, q.source_type, coalesce(q.dtype, '') AS dtype",
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
            source_type: row.get("source_type"),
            dtype: row.get("dtype"),
        })
        .collect())
}

pub async fn fetch_html(url: &str) -> Result<FetchedHtml> {
    let client = reqwest_adapter::new_default_client(45)?;
    let response = client
        .get(url)
        .header(
            reqwest::header::USER_AGENT,
            "AlegriaBot/1.0 (+https://alegria.local/seo-research)",
        )
        .send()
        .await
        .with_context(|| format!("fetch source HTML: {url}"))?;
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
        .with_context(|| format!("read source HTML body: {url}"))?;
    Ok(FetchedHtml {
        final_url,
        status_code,
        content_type,
        body,
    })
}

pub async fn save_crawled_html(
    pool: &PgPool,
    url: &str,
    dtype: &str,
    status_code: i32,
    content_type: &str,
    raw_html: &str,
) -> std::result::Result<SavedRawPage, primitives::errors::DomainError> {
    let meta = extract_meta_typed(raw_html);
    let sections = extract_sections_typed(raw_html, url);
    let content_hash = content_hash_v1(raw_html);
    let domain = domain_norm(url);
    let dtype = if dtype.trim().is_empty() {
        "organic_competitor"
    } else {
        dtype.trim()
    };
    let content = json!({
        "extractor": "primitives::html_sections@1",
        "section_count": sections.len(),
        "content_hash": content_hash,
    });

    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    let existing = sqlx::query(
        "SELECT id
         FROM raw.pages
         WHERE url = $1
           AND CAST(snapshot_at AT TIME ZONE 'UTC' AS DATE) = CAST(now() AT TIME ZONE 'UTC' AS DATE)
         ORDER BY id DESC
         LIMIT 1",
    )
    .bind(url)
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
                 title = $6,
                 meta_desc = $7,
                 canonical = $8,
                 word_count = $9,
                 content = $10,
                 raw_html = $11,
                 raw_html_bytes = $12,
                 content_hash = $13,
                 processed = false
             WHERE id = $1",
        )
        .bind(page_id)
        .bind(&domain)
        .bind(dtype)
        .bind(status_code as i16)
        .bind(content_type)
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
             (url, domain, dtype, status_code, content_type, crawled_at, title, meta_desc,
              canonical, word_count, content, raw_html, raw_html_bytes, content_hash, processed)
             VALUES ($1, $2, $3, $4, $5, now(), $6, $7, $8, $9, $10, $11, $12, $13, false)
             RETURNING id",
        )
        .bind(url)
        .bind(&domain)
        .bind(dtype)
        .bind(status_code as i16)
        .bind(content_type)
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
    Ok(SavedRawPage {
        page_id,
        section_count: sections.len(),
        content_hash,
    })
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

        let extraction_input = format!(
            "{}\n\n{}\n\nSource: {}",
            section.heading_path, section.content_md, section.source_url
        );
        let extracted_typed = extract_facts_typed(
            &extraction_input,
            &ExtractionContext {
                source_key: Some(section.source_url.clone()),
            },
        );
        let mut payload = build_extracted_payload_from_typed(extracted_typed);
        if payload.rule_instances.is_empty() {
            continue;
        }
        let auto_verified = source_allows_auto_verify(section);
        for rule in &mut payload.rule_instances {
            rule.status = if auto_verified { "verified" } else { "pending" }.to_string();
            if rule.source_key.is_none() {
                rule.source_key = Some(section.source_url.clone());
            }
        }
        ensure_extracted_concepts(pool, &payload).await?;
        report.extracted_rule_count += payload.rule_instances.len();
        for rule in &payload.rule_instances {
            if rule.status == "verified" {
                report.changed_truth_keys.push(stable_rule_instance_id(&[
                    context_key,
                    &rule.rule_type_key,
                    &rule.concept_key,
                    rule.role_type.as_str(),
                ]));
            }
        }
        let verified_in_payload = payload
            .rule_instances
            .iter()
            .filter(|rule| rule.status == "verified")
            .count();
        let (written, outbox_count) = sqlx_source_projection_adapter::persist_from_pipeline_state(
            pool,
            &PersistPipelineState {
                context_key: context_key.to_string(),
                extracted_payload: payload,
            },
        )
        .await?;
        let _ = written;
        report.verified_rule_count += verified_in_payload;
        report.outbox_event_count += outbox_count;
    }

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
    } else {
        "editorial"
    }
}

fn source_trust_level(section: &RawSectionRecord) -> i32 {
    match source_type_for_section(section) {
        "government" => 5,
        "vfs" => 4,
        "niche_agency" => 2,
        _ => 3,
    }
}

fn source_allows_auto_verify(section: &RawSectionRecord) -> bool {
    matches!(source_type_for_section(section), "government" | "vfs")
}

async fn ensure_extracted_concepts(
    pool: &PgPool,
    payload: &runtime_models::ExtractedPayload,
) -> std::result::Result<(), primitives::errors::DomainError> {
    let mut seen = std::collections::BTreeSet::new();
    for rule in &payload.rule_instances {
        if rule.concept_key.trim().is_empty() || !seen.insert(rule.concept_key.clone()) {
            continue;
        }
        let concept_type = match rule.role_type.as_str() {
            "must_pay" | "fee_item" => "fee",
            "timeline" | "timeline_item" => "timeline",
            "must_provide" | "document_required" | "form_required" => "document",
            "where_to_apply" | "appointment_rule" => "location",
            "step" => "process",
            _ => "rule",
        };
        let label = rule.concept_key.replace('_', " ");
        sqlx::query(
            "INSERT INTO kb.concepts
             (concept_key, concept_type, label_ru, status)
             VALUES ($1, $2, $3, 'active')
             ON CONFLICT (concept_key) DO UPDATE
             SET concept_type = CASE
                     WHEN kb.concepts.concept_type = '' THEN EXCLUDED.concept_type
                     ELSE kb.concepts.concept_type
                 END,
                 label_ru = CASE
                     WHEN kb.concepts.label_ru = '' THEN EXCLUDED.label_ru
                     ELSE kb.concepts.label_ru
                 END,
                 status = CASE
                     WHEN kb.concepts.status = 'deprecated' THEN 'active'
                     ELSE kb.concepts.status
                 END",
        )
        .bind(&rule.concept_key)
        .bind(concept_type)
        .bind(label)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }
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
        let point_id = content_hash_v1(&format!(
            "content_chunks|{}|{}|{}",
            section.page_id, section.id, section.content_hash
        ));
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
