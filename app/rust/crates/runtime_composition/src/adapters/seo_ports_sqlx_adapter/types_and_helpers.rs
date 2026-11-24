use async_trait::async_trait;
use primitives::errors::DomainError;
use runtime_models::{GraphPlanningContext, RuleRoleType};
use seo_domain::{
    identity,
    rebuild::{RebuildDetectRequest, RebuildDetectResult},
};
use seo_ports::knowledge::{
    SourceContextChunk, SourceContextCollection, VerifiedRule, VerifiedRulesQuery,
    VerifiedRulesView,
};
use seo_ports::{
    CmsApproval, CmsReviewDecisionOutcome, CmsReviewDecisionPort, CmsReviewDecisionRequest,
    CmsReviewPort, CrawlIngestRepository, DraftRepository, EditorialGenerationPort,
    GraphCapabilityPort, GraphReasoningPort, OrganicSerpResponse, OrganicSerpResult,
    PlanningRepository, ProjectionBarrierStatus, ProjectionStatusRepository,
    PublishArtifactRepository, RebuildDependencyEvidence, RebuildRepository, SectionTemplate,
    SectionTemplateRepository, SemanticDemandCluster, SemanticLinkCandidate,
    SemanticLinkSearchPort, SeoBuildInputRepository, SeoBuildRegistrationRepository,
    SeoSiteBuildRegistrationRequest, SerpSearchPort, SourceContextSearchPort, VerifiedRulesReader,
    VerifiedSupportBundleRequest, VerifiedSupportRepository,
};

use super::{graph_capability_adapter, semantic_search_adapter};
use infrastructure::adapters::{
    dataforseo_serp_adapter, editorial_llm_adapter, manual_llm_queue, raw_crawl_adapter,
    sqlx_adapter::AlegriaPgPool,
    sqlx_context_bundle_adapter, sqlx_seo_adapter, sqlx_seo_cms_adapter, sqlx_serp_adapter,
    voyage_api_adapter::{VoyageClient, VoyageEmbeddingOptions, VoyageOutputDtype},
};
use contracts::generated::alegria::temporal::v1::{
    CmsPublishInputPayload, CmsPublishOutputPayload, ContentContractValidateInputPayload,
    ContentContractValidateOutputPayload, CrawlSourcesInputPayload, CrawlSourcesOutputPayload,
    DraftAssembleOutputPayload, DraftNormalizeInputPayload, DraftNormalizeOutputPayload,
    DraftQaInputPayload, DraftQaOutputPayload, EditorialDraftGenerateInputPayload,
    EditorialDraftGenerateOutputPayload, FinalizePublishInputPayload, FinalizePublishOutputPayload,
    GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload, IaBuildInputPayload,
    IaBuildOutputPayload, LinkRecommendInputPayload, LinkRecommendOutputPayload,
    OpportunityBuildInputPayload, OpportunityBuildOutputPayload, PublishMaterializeInputPayload,
    PublishMaterializeOutputPayload, RawKnowledgeIngestionInputPayload,
    RawKnowledgeIngestionOutputPayload, SeoSiteBuildInputPayload, SeoVerifiedFactSupportState,
    SerpIngestInputPayload, SerpIngestOutputPayload, SerpNormalizeInputPayload,
    SerpNormalizeOutputPayload,
};
use graph_neo4j;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

pub struct SqlxSeoRuntimeRepository<'a> {
    pool: &'a AlegriaPgPool,
}

impl<'a> SqlxSeoRuntimeRepository<'a> {
    pub fn new(pool: &'a AlegriaPgPool) -> Self {
        Self { pool }
    }
}

async fn ensure_graph_contract_if_required(context_key: &str) -> Result<(), DomainError> {
    graph_capability_adapter::ensure_graph_contract_if_required(context_key).await
}

fn source_chunk_from_search_result(
    collection: SourceContextCollection,
    record: semantic_search_adapter::SearchResultRecord,
) -> SourceContextChunk {
    let collection_name = collection.name();
    let payload = record.payload;
    let retrieval_text = payload
        .get("retrieval_text")
        .cloned()
        .or_else(|| payload.get("body_markdown").cloned())
        .or_else(|| payload.get("fragment_text").cloned())
        .unwrap_or_default();
    let entity_key = if record.entity_key.trim().is_empty() {
        payload
            .get("entity_key")
            .cloned()
            .or_else(|| payload.get("artifact_key").cloned())
            .unwrap_or_default()
    } else {
        record.entity_key
    };
    SourceContextChunk {
        collection,
        chunk_key: format!("{collection_name}:{entity_key}"),
        source_url: payload.get("source_url").cloned().unwrap_or_default(),
        source_domain: payload.get("source_domain").cloned().unwrap_or_default(),
        heading_path: payload
            .get("heading_path")
            .cloned()
            .or_else(|| payload.get("canonical_url_path").cloned())
            .unwrap_or_default(),
        section_type: payload
            .get("section_type")
            .cloned()
            .or_else(|| payload.get("artifact_type").cloned())
            .unwrap_or_else(|| collection_name.to_string()),
        content_md: retrieval_text,
        retrieval_score: record.score,
    }
}

async fn search_context_collection(
    collection: SourceContextCollection,
    query: &str,
    limit: usize,
) -> Result<Vec<SourceContextChunk>, DomainError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let collection_name = collection.name();
    let surface = if collection == SourceContextCollection::RawContextual {
        semantic_search_adapter::VoyageSearchSurface::Contextualized
    } else {
        semantic_search_adapter::VoyageSearchSurface::Standard
    };
    let search_limit = u64::try_from(limit.saturating_mul(3).max(limit)).map_err(|_| {
        DomainError::ValidationFailure {
            message: format!("source context search limit too large: {limit}"),
        }
    })?;
    let found = semantic_search_adapter::search_by_text_with_surface(
        query,
        collection_name,
        search_limit,
        surface,
    )
    .await
    .map_err(|err| DomainError::InfraUnavailable {
        message: format!("{collection_name} source context retrieval failed: {err}"),
    })?;
    let reranked = semantic_search_adapter::rerank_records(query, found, Some(limit))
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("{collection_name} source context rerank failed: {err}"),
        })?;
    Ok(reranked
        .into_iter()
        .take(limit)
        .map(|record| source_chunk_from_search_result(collection, record))
        .collect())
}

fn cosine_similarity(lhs: &[f32], rhs: &[f32]) -> f32 {
    if lhs.len() != rhs.len() || lhs.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut lhs_norm = 0.0f64;
    let mut rhs_norm = 0.0f64;
    for (l, r) in lhs.iter().zip(rhs.iter()) {
        let lf = *l as f64;
        let rf = *r as f64;
        dot += lf * rf;
        lhs_norm += lf * lf;
        rhs_norm += rf * rf;
    }
    if lhs_norm == 0.0 || rhs_norm == 0.0 {
        return 0.0;
    }
    (dot / (lhs_norm.sqrt() * rhs_norm.sqrt())) as f32
}

fn cluster_queries_from_embeddings(
    queries: &[String],
    embeddings: &[Vec<f32>],
    threshold: f32,
) -> Vec<SemanticDemandCluster> {
    if queries.is_empty() || queries.len() != embeddings.len() {
        return Vec::new();
    }
    let mut parent = (0..queries.len()).collect::<Vec<_>>();
    fn find(parent: &mut [usize], x: usize) -> usize {
        if parent[x] != x {
            let root = find(parent, parent[x]);
            parent[x] = root;
        }
        parent[x]
    }
    fn union(parent: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[rb] = ra;
        }
    }
    for i in 0..embeddings.len() {
        for j in (i + 1)..embeddings.len() {
            if cosine_similarity(&embeddings[i], &embeddings[j]) >= threshold {
                union(&mut parent, i, j);
            }
        }
    }
    let mut groups = std::collections::BTreeMap::<usize, Vec<usize>>::new();
    for idx in 0..queries.len() {
        let root = find(&mut parent, idx);
        groups.entry(root).or_default().push(idx);
    }
    groups
        .into_values()
        .map(|members| {
            let member_queries = members
                .iter()
                .map(|idx| queries[*idx].clone())
                .collect::<Vec<_>>();
            let mut pair_scores = Vec::new();
            for i in 0..members.len() {
                for j in (i + 1)..members.len() {
                    pair_scores.push(cosine_similarity(
                        &embeddings[members[i]],
                        &embeddings[members[j]],
                    ));
                }
            }
            let confidence = if pair_scores.is_empty() {
                1.0
            } else {
                pair_scores.iter().sum::<f32>() / pair_scores.len() as f32
            };
            let cluster_key = seo_domain::artifact_identity::seo_artifact_key(
                "step0_demand_cluster",
                &[&member_queries.join("|"), "voyage_step0@1"],
            );
            SemanticDemandCluster {
                cluster_key,
                member_queries,
                confidence,
            }
        })
        .collect()
}
