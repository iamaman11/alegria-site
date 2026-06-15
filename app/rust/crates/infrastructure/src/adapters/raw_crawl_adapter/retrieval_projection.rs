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
    let retrieval_required = std::env::var("RETRIEVAL_CAPABILITY_REQUIRED")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    let voyage_api_key = match std::env::var("VOYAGE_API_KEY") {
        Ok(value) => value,
        Err(_) if retrieval_required => {
            return Err(primitives::errors::DomainError::InfraUnavailable {
                message: "raw chunk dual projection requires VOYAGE_API_KEY under hard-required retrieval contract"
                    .to_string(),
            })
        }
        Err(_) => return Ok(0),
    };
    let voyage_model =
        std::env::var("VOYAGE_MODEL").unwrap_or_else(|_| DEFAULT_VOYAGE_STANDARD_MODEL.to_string());
    let voyage_context_model = std::env::var("VOYAGE_CONTEXT_MODEL")
        .unwrap_or_else(|_| DEFAULT_VOYAGE_CONTEXT_MODEL.to_string());
    let standard_vectors = if texts.is_empty() {
        Vec::new()
    } else {
        VoyageClient::new(voyage_api_key.clone(), voyage_model.clone())
            .embed_all_with_settings(
                &texts,
                &VoyageEmbeddingOptions {
                    input_type: Some(VoyageInputType::Document),
                    output_dimension: Some(DEFAULT_VOYAGE_DIMENSION),
                    output_dtype: Some(VoyageOutputDtype::Float),
                    truncation: Some(true),
                },
            )
            .await
            .map_err(|err| primitives::errors::DomainError::InfraUnavailable {
                message: format!("Voyage raw section embedding failed: {err}"),
            })?
    };
    let contextual_vectors = if texts.is_empty() {
        Vec::new()
    } else {
        let grouped = vec![texts.iter().map(String::as_str).collect::<Vec<_>>()];
        VoyageClient::new(voyage_api_key, voyage_context_model.clone())
            .contextualized_embed(
                &grouped,
                &VoyageEmbeddingOptions {
                    input_type: Some(VoyageInputType::Document),
                    output_dimension: Some(DEFAULT_VOYAGE_DIMENSION),
                    output_dtype: Some(VoyageOutputDtype::Float),
                    truncation: Some(true),
                },
                Some(&voyage_context_model),
            )
            .await
            .map_err(|err| primitives::errors::DomainError::InfraUnavailable {
                message: format!("Voyage contextualized raw section embedding failed: {err}"),
            })?
            .into_iter()
            .next()
            .unwrap_or_default()
    };
    if standard_vectors.len() != texts.len() {
        return Err(primitives::errors::DomainError::InfraUnavailable {
            message: format!(
                "raw_chunks_4 embedding count mismatch: expected {}, got {}",
                texts.len(),
                standard_vectors.len()
            ),
        });
    }
    if contextual_vectors.len() != texts.len() {
        return Err(primitives::errors::DomainError::InfraUnavailable {
            message: format!(
                "raw_chunks_ctx embedding count mismatch: expected {}, got {}",
                texts.len(),
                contextual_vectors.len()
            ),
        });
    }
    let mut standard_vector_iter = standard_vectors.into_iter();
    let mut contextual_vector_iter = contextual_vectors.into_iter();
    let mut events = Vec::with_capacity(sections.len());
    for section in &sections {
        if section.content_md.trim().is_empty() {
            continue;
        }
        let entity_key = format!("raw_section:{}", section.id);
        let standard_vector = standard_vector_iter.next().ok_or_else(|| {
            primitives::errors::DomainError::InfraUnavailable {
                message: "raw_chunks_4 projection missing Voyage embedding".to_string(),
            }
        })?;
        let contextual_vector = contextual_vector_iter.next().ok_or_else(|| {
            primitives::errors::DomainError::InfraUnavailable {
                message: "raw_chunks_ctx projection missing Voyage embedding".to_string(),
            }
        })?;
        let contextual_vector = if contextual_vector.is_empty() {
            standard_vector.clone()
        } else {
            contextual_vector
        };
        let standard_point = (
            standard_vector,
            voyage_model.as_str(),
            "raw_section_voyage_4@1",
            RAW_CHUNKS_STANDARD_COLLECTION,
        );
        let contextual_point = (
            contextual_vector,
            voyage_context_model.as_str(),
            "raw_section_context_voyage@1",
            RAW_CHUNKS_CONTEXT_COLLECTION,
        );

        for (vector, embedding_model, embedding_version, collection_name) in
            [standard_point, contextual_point]
        {
            let point_id = qdrant_point_id_v1(collection_name, "raw_section", &entity_key);
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
            metadata.insert("retrieval_text".to_string(), section.content_md.clone());
            let vector_size = vector.len() as u32;
            let payload_bytes = QdrantUpsertCommand {
                event_id: String::new(),
                collection_name: collection_name.to_string(),
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
                aggregate_key: format!("{entity_key}|{collection_name}"),
                target_system: "qdrant".to_string(),
                event_type: "QdrantUpsertCommand".to_string(),
                payload_type: "alegria.outbox.qdrant_upsert_command.v1".to_string(),
                schema_version: 1,
                idempotency_key: content_hash_v1(&format!(
                    "{entity_key}|{collection_name}|QdrantUpsertCommand|{}",
                    primitives::hash::blake3_hex(&payload_bytes)
                )),
                payload_bytes,
            });

            sqlx::query(
                "INSERT INTO kb.qdrant_points
                 (point_id, entity_type, entity_key, collection_name, embedding_model, embedding_version)
                 VALUES ($1, 'raw_section', $2, $3, $4, $5)
                 ON CONFLICT (entity_type, entity_key, collection_name) DO UPDATE
                 SET point_id = EXCLUDED.point_id,
                     embedding_model = EXCLUDED.embedding_model,
                     embedding_version = EXCLUDED.embedding_version,
                     updated_at = now()",
            )
            .bind(&point_id)
            .bind(&entity_key)
            .bind(collection_name)
            .bind(embedding_model)
            .bind(embedding_version)
            .execute(pool)
            .await
            .map_err(classify_sqlx)?;
        }
    }
    let emitted = outbox_emit_many(pool, &events).await?;
    Ok(emitted as usize)
}

pub async fn retrieve_source_context_chunks(
    pool: &PgPool,
    query: &str,
    limit: u64,
) -> std::result::Result<Vec<SourceContextChunkState>, primitives::errors::DomainError> {
    let contextual_required = std::env::var("CONTEXTUAL_RAW_CHUNK_RETRIEVAL_REQUIRED")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    if std::env::var("VOYAGE_API_KEY").is_err() {
        if contextual_required {
            return Err(primitives::errors::DomainError::InfraUnavailable {
                message:
                    "contextual raw-chunk retrieval is required, but VOYAGE_API_KEY is not set"
                        .to_string(),
            });
        }
        return Ok(Vec::new());
    }
    let results = match semantic_search_adapter::search_by_text_with_surface(
        query,
        RAW_CHUNKS_CONTEXT_COLLECTION,
        limit,
        semantic_search_adapter::VoyageSearchSurface::Contextualized,
    )
    .await
    {
        Ok(found) if !found.is_empty() => found,
        Ok(_) if !contextual_required => return Ok(Vec::new()),
        Ok(_) => {
            return Err(primitives::errors::DomainError::InfraUnavailable {
                message: "contextual raw-chunk retrieval is required, but raw_chunks_ctx returned no candidates"
                    .to_string(),
            });
        }
        Err(_) if !contextual_required => return Ok(Vec::new()),
        Err(err) => {
            return Err(primitives::errors::DomainError::InfraUnavailable {
                message: format!(
                    "contextual raw-chunk retrieval is required, and raw_chunks_ctx search failed: {err}"
                ),
            });
        }
    };
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
