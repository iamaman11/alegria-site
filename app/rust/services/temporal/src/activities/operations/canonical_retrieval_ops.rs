fn canonical_candidate_key(payload: &BTreeMap<String, String>) -> Option<String> {
    payload
        .get("concept_key")
        .cloned()
        .or_else(|| payload.get("entity_key").cloned())
        .filter(|value| !value.trim().is_empty())
}

fn canonical_candidate_text(payload: &BTreeMap<String, String>) -> String {
    [
        payload.get("label_ru"),
        payload.get("aliases"),
        payload.get("concept_key"),
        payload.get("entity_key"),
    ]
    .into_iter()
    .flatten()
    .map(|value| value.as_str())
    .collect::<Vec<_>>()
    .join(" ")
}

async fn resolve_canonical_vector_mappings(
    mappings: Vec<seo_steps::canonical_mapping_step::MappingResult>,
) -> Result<Vec<seo_steps::canonical_mapping_step::MappingResult>, DomainError> {
    let mut resolved = Vec::with_capacity(mappings.len());
    for mapping in mappings {
        if mapping.matching_stage != seo_steps::canonical_mapping_step::MatchingStage::VectorQdrant
        {
            resolved.push(mapping);
            continue;
        }
        let upgraded = match resolve_single_canonical_vector_mapping(&mapping).await? {
            Some(value) => value,
            None => mapping,
        };
        resolved.push(upgraded);
    }
    Ok(resolved)
}

async fn resolve_single_canonical_vector_mapping(
    mapping: &seo_steps::canonical_mapping_step::MappingResult,
) -> Result<Option<seo_steps::canonical_mapping_step::MappingResult>, DomainError> {
    let canonical_required = env_flag("CANONICAL_VECTOR_RETRIEVAL_REQUIRED");
    let rerank_required = env_flag("VOYAGE_RERANK_REQUIRED");
    if mapping.raw_text.trim().is_empty() {
        return Ok(None);
    }
    if std::env::var("VOYAGE_API_KEY").is_err() {
        if canonical_required {
            return Err(DomainError::InfraUnavailable {
                message: "canonical vector retrieval is required, but VOYAGE_API_KEY is not set"
                    .to_string(),
            });
        }
        return Ok(None);
    }
    let primary_collection = std::env::var("VOYAGE_CANONICAL_COLLECTION")
        .unwrap_or_else(|_| "kb_canonical_4".to_string());
    let mut results = match semantic_search_adapter::search_by_text_with_surface(
        &mapping.raw_text,
        &primary_collection,
        5,
        semantic_search_adapter::VoyageSearchSurface::Standard,
    )
    .await
    {
        Ok(found) if !found.is_empty() => found,
        Ok(_) if canonical_required => {
            return Err(DomainError::InfraUnavailable {
                message: format!(
                    "canonical vector retrieval is required, but `{primary_collection}` returned no candidates"
                ),
            });
        }
        Ok(_) => return Ok(None),
        Err(err) if canonical_required => {
            return Err(DomainError::InfraUnavailable {
                message: format!(
                    "canonical vector retrieval is required, but `{primary_collection}` search failed: {err}"
                ),
            });
        }
        Err(_) => return Ok(None),
    };
    if results.is_empty() {
        return Ok(None);
    }
    for record in &mut results {
        if !record.payload.contains_key("retrieval_text") {
            let text = canonical_candidate_text(&record.payload);
            if !text.trim().is_empty() {
                record.payload.insert("retrieval_text".to_string(), text);
            }
        }
    }
    let candidates =
        match semantic_search_adapter::rerank_records(&mapping.raw_text, results.clone(), Some(3))
            .await
        {
            Ok(reranked) if !reranked.is_empty() => reranked,
            Err(err) if rerank_required => {
                return Err(DomainError::InfraUnavailable {
                    message: format!(
                        "canonical vector retrieval requires rerank, but rerank failed: {err}"
                    ),
                });
            }
            _ => results,
        };
    let top = candidates.first().cloned();
    let Some(top) = top else {
        return Ok(None);
    };
    let top_key = canonical_candidate_key(&top.payload);
    let margin = if candidates.len() > 1 {
        top.score - candidates[1].score
    } else {
        top.score
    };
    let (mapping_type, needs_hitl, canonical_key) = if top.score >= 0.88 && margin >= 0.03 {
        ("auto_map".to_string(), false, top_key)
    } else if top.score >= 0.75 {
        ("review".to_string(), true, top_key)
    } else {
        ("new_candidate".to_string(), true, None)
    };
    Ok(Some(seo_steps::canonical_mapping_step::MappingResult {
        raw_text: mapping.raw_text.clone(),
        canonical_key,
        mapping_type,
        match_method: "qdrant_retrieval".to_string(),
        matching_stage: seo_steps::canonical_mapping_step::MatchingStage::VectorQdrant,
        qdrant_score: Some(top.score),
        confidence: top.score,
        needs_hitl,
    }))
}

