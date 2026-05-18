use contracts::generated::alegria::temporal::v1::{
    CrawlSourcesInputPayload, CrawlSourcesOutputPayload, RawKnowledgeIngestionInputPayload,
    RawKnowledgeIngestionOutputPayload,
};
use primitives::errors::DomainError;
use seo_ports::CrawlIngestRepository;

pub async fn run_crawl_sources<R: CrawlIngestRepository>(
    repo: &R,
    input: &CrawlSourcesInputPayload,
) -> Result<CrawlSourcesOutputPayload, DomainError> {
    repo.crawl_sources(input).await
}

pub async fn run_raw_knowledge_ingestion<R: CrawlIngestRepository>(
    repo: &R,
    input: &RawKnowledgeIngestionInputPayload,
) -> Result<RawKnowledgeIngestionOutputPayload, DomainError> {
    repo.ingest_raw_knowledge(input).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct FakeRepo;

    #[async_trait]
    impl CrawlIngestRepository for FakeRepo {
        async fn crawl_sources(
            &self,
            _input: &CrawlSourcesInputPayload,
        ) -> Result<CrawlSourcesOutputPayload, DomainError> {
            Ok(CrawlSourcesOutputPayload {
                claimed_count: 1,
                crawled_count: 1,
                failed_count: 0,
                raw_page_count: 1,
                raw_section_count: 2,
                qdrant_event_count: 0,
                status: "done".to_string(),
                raw_page_ids: vec![101],
                failed_urls: Vec::new(),
            })
        }

        async fn ingest_raw_knowledge(
            &self,
            _input: &RawKnowledgeIngestionInputPayload,
        ) -> Result<RawKnowledgeIngestionOutputPayload, DomainError> {
            Ok(RawKnowledgeIngestionOutputPayload {
                raw_page_count: 1,
                raw_section_count: 2,
                extracted_rule_count: 1,
                verified_rule_count: 1,
                outbox_event_count: 1,
                changed_truth_keys: vec!["verified.rule_instance:passport".to_string()],
                status: "done".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn crawl_ingest_routes_through_repo() {
        let repo = FakeRepo;
        let crawled = run_crawl_sources(
            &repo,
            &CrawlSourcesInputPayload {
                run_id: "run-1".to_string(),
                query_batch_key: "batch-1".to_string(),
                limit: 25,
                emit_qdrant: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(crawled.raw_page_ids, vec![101]);

        let ingested = run_raw_knowledge_ingestion(
            &repo,
            &RawKnowledgeIngestionInputPayload {
                run_id: "run-1".to_string(),
                context_key: "ES|tourist||BY".to_string(),
                query_batch_key: "batch-1".to_string(),
                raw_page_ids: vec![101],
                source_policy: "candidate_only_truth_extraction@1".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(ingested.verified_rule_count, 1);
    }
}
