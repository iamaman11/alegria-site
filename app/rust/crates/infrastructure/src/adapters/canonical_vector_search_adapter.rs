//! Qdrant/Voyage retrieval implementation for canonical concept suggestions.
//! It discovers candidates; it does not assign mappings or verify Truth.

use async_trait::async_trait;
use primitives::errors::DomainError;
use seo_ports::{
    knowledge::{CanonicalVectorCandidate, CanonicalVectorLookup},
    CanonicalVectorSearchPort,
};

use super::semantic_search_adapter::{self, SearchResultRecord, VoyageSearchSurface};

pub struct CanonicalVectorSearchAdapter;

fn env_flag(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn canonical_candidate_text(record: &SearchResultRecord) -> String {
    ["label_ru", "aliases", "concept_key", "entity_key"]
        .into_iter()
        .filter_map(|key| record.payload.get(key))
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

fn canonical_candidate(record: SearchResultRecord) -> CanonicalVectorCandidate {
    let key = record
        .payload
        .get("concept_key")
        .or_else(|| record.payload.get("entity_key"))
        .cloned()
        .filter(|key| !key.trim().is_empty());
    CanonicalVectorCandidate {
        canonical_key: key,
        score: record.score,
    }
}

#[async_trait]
impl CanonicalVectorSearchPort for CanonicalVectorSearchAdapter {
    async fn search_canonical_concepts(
        &self,
        raw_text: &str,
    ) -> Result<CanonicalVectorLookup, DomainError> {
        let canonical_required = env_flag("CANONICAL_VECTOR_RETRIEVAL_REQUIRED");
        let rerank_required = env_flag("VOYAGE_RERANK_REQUIRED");
        if std::env::var("VOYAGE_API_KEY").is_err() {
            if canonical_required {
                return Err(DomainError::InfraUnavailable {
                    message:
                        "canonical vector retrieval is required, but VOYAGE_API_KEY is not set"
                            .to_string(),
                });
            }
            return Ok(CanonicalVectorLookup::Unavailable);
        }
        let collection = std::env::var("VOYAGE_CANONICAL_COLLECTION")
            .unwrap_or_else(|_| "kb_canonical_4".to_string());
        let mut results = match semantic_search_adapter::search_by_text_with_surface(
            raw_text,
            &collection,
            5,
            VoyageSearchSurface::Standard,
        )
        .await
        {
            Ok(results) => results,
            Err(err) if canonical_required => {
                return Err(DomainError::InfraUnavailable {
                    message: format!(
                        "canonical vector retrieval is required, but `{collection}` search failed: {err}"
                    ),
                });
            }
            Err(_) => return Ok(CanonicalVectorLookup::Unavailable),
        };
        if results.is_empty() {
            return Ok(CanonicalVectorLookup::Candidates(Vec::new()));
        }
        for record in &mut results {
            if !record.payload.contains_key("retrieval_text") {
                let text = canonical_candidate_text(record);
                if !text.trim().is_empty() {
                    record.payload.insert("retrieval_text".to_string(), text);
                }
            }
        }
        let candidates =
            match semantic_search_adapter::rerank_records(raw_text, results.clone(), Some(3)).await
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
        Ok(CanonicalVectorLookup::Candidates(
            candidates.into_iter().map(canonical_candidate).collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn vector_candidate_keeps_identity_and_score_without_truth_status() {
        let record = SearchResultRecord {
            entity_key: "qdrant-point".into(),
            score: 0.93,
            payload: BTreeMap::from([
                ("concept_key".into(), "canonical:passport".into()),
                ("label_ru".into(), "Паспорт".into()),
            ]),
        };
        assert_eq!(
            canonical_candidate(record),
            CanonicalVectorCandidate {
                canonical_key: Some("canonical:passport".into()),
                score: 0.93,
            }
        );
    }
}
