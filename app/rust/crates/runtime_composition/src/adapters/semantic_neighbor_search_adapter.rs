//! Qdrant/Voyage effects for typed, non-authoritative Truth diagnostics.

use async_trait::async_trait;
use primitives::errors::DomainError;
use seo_ports::{
    knowledge::{SemanticNeighborCandidate, SemanticNeighborCollection},
    SemanticNeighborSearchPort,
};
use std::collections::BTreeMap;

use super::semantic_search_adapter::{self, SearchResultRecord, VoyageSearchSurface};

pub struct SemanticNeighborSearchAdapter;

fn to_candidate(record: SearchResultRecord) -> SemanticNeighborCandidate {
    let retrieval_text = [
        "retrieval_text",
        "label_ru",
        "aliases",
        "heading_path",
        "entity_key",
    ]
    .into_iter()
    .find_map(|key| record.payload.get(key))
    .cloned()
    .unwrap_or_default();
    SemanticNeighborCandidate {
        entity_key: record.entity_key,
        score: record.score,
        retrieval_text,
    }
}

fn to_record(candidate: SemanticNeighborCandidate) -> SearchResultRecord {
    SearchResultRecord {
        entity_key: candidate.entity_key,
        score: candidate.score,
        payload: BTreeMap::from([("retrieval_text".to_string(), candidate.retrieval_text)]),
    }
}

#[async_trait]
impl SemanticNeighborSearchPort for SemanticNeighborSearchAdapter {
    fn provider_configured(&self) -> bool {
        std::env::var("VOYAGE_API_KEY").is_ok()
    }

    async fn search_semantic_neighbors(
        &self,
        query: &str,
        collection: SemanticNeighborCollection,
        limit: usize,
    ) -> Result<Vec<SemanticNeighborCandidate>, DomainError> {
        let surface = if collection == SemanticNeighborCollection::RawContextual {
            VoyageSearchSurface::Contextualized
        } else {
            VoyageSearchSurface::Standard
        };
        let found = semantic_search_adapter::search_by_text_with_surface(
            query,
            collection.name(),
            u64::try_from(limit).map_err(|_| DomainError::ValidationFailure {
                message: format!("semantic neighbor limit too large: {limit}"),
            })?,
            surface,
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!(
                "{} semantic neighbor search failed: {err}",
                collection.name()
            ),
        })?;
        Ok(found.into_iter().map(to_candidate).collect())
    }

    async fn rerank_semantic_neighbors(
        &self,
        query: &str,
        candidates: Vec<SemanticNeighborCandidate>,
        limit: usize,
    ) -> Result<Vec<SemanticNeighborCandidate>, DomainError> {
        let records = candidates.into_iter().map(to_record).collect();
        let reranked = semantic_search_adapter::rerank_records(query, records, Some(limit))
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("Voyage semantic neighbor rerank failed: {err}"),
            })?;
        Ok(reranked.into_iter().map(to_candidate).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_qdrant_payload_to_read_only_candidate() {
        let candidate = to_candidate(SearchResultRecord {
            entity_key: "section:42".into(),
            score: 0.83,
            payload: BTreeMap::from([("heading_path".into(), "Government source".into())]),
        });
        assert_eq!(candidate.entity_key, "section:42");
        assert_eq!(candidate.retrieval_text, "Government source");
        assert_eq!(
            to_record(candidate).payload["retrieval_text"],
            "Government source"
        );
    }
}
