use async_trait::async_trait;
use contracts::generated::alegria::read_api::v1::{
    citation_fact, AppointmentRuleParams, CitationFact, ContextBundle, DocumentRequiredParams,
    EligibilityRuleParams, FeeItemParams, FormRequiredParams, RuleRoleTypeV1, SourceCitation,
    StepParams, TimelineItemParams, WhereToApplyParams,
};
use primitives::errors::DomainError;
use runtime_models::{GraphPlanningContext, RuleParams};
use seo_domain::identity;
use seo_ports::{
    CmsReviewDecisionOutcome, CmsReviewDecisionPort, CmsReviewDecisionRequest, CmsReviewPort,
    ContextBundleRepository, CrawlIngestRepository, DraftRepository, EditorialGenerationPort,
    GlobalNavigationPersistReport, GraphCapabilityPort, GraphCoverageEvaluation,
    GraphNeighborhoodHit, GraphReasoningPort, HitlQueuePort, OrganicSerpResponse,
    OrganicSerpResult, PlanningRepository, ProjectionBarrierStatus, ProjectionStatusRepository,
    PublishArtifactRepository, RebuildDependencyEvidence, RebuildRepository,
    SectionTemplateRepository, SemanticDemandCluster, SemanticLinkCandidate,
    SemanticLinkSearchPort, SeoBuildInputRepository, SeoBuildRegistrationRepository,
    SeoSiteBuildRegistrationRequest, SerpSearchPort, SourceContextRepository,
    VerifiedSupportBundleRequest, VerifiedSupportRepository,
};

use super::{
    dataforseo_serp_adapter, editorial_llm_adapter, graph_capability_adapter, neo4rs_adapter,
    raw_crawl_adapter, semantic_search_adapter,
    sqlx_adapter::AlegriaPgPool,
    sqlx_context_bundle_adapter, sqlx_hitl_adapter, sqlx_seo_adapter, sqlx_seo_cms_adapter,
    sqlx_serp_adapter,
    voyage_api_adapter::{VoyageClient, VoyageEmbeddingOptions, VoyageOutputDtype},
};
use contracts::generated::alegria::temporal::v1::{
    CmsApprovalDecision, CmsPublishInputPayload, CmsPublishOutputPayload,
    ContentContractValidateInputPayload, ContentContractValidateOutputPayload,
    CrawlSourcesInputPayload, CrawlSourcesOutputPayload, DraftAssembleOutputPayload,
    DraftNormalizeInputPayload, DraftNormalizeOutputPayload, DraftQaInputPayload,
    DraftQaOutputPayload, EditorialDraftGenerateInputPayload, EditorialDraftGenerateOutputPayload,
    FinalizePublishInputPayload, FinalizePublishOutputPayload, GlobalSiteReconcileInputPayload,
    GlobalSiteReconcileOutputPayload, HitlDecision, HitlTaskContext, IaBuildInputPayload,
    IaBuildOutputPayload, LinkRecommendInputPayload, LinkRecommendOutputPayload,
    OpportunityBuildInputPayload, OpportunityBuildOutputPayload, PageNodeState,
    PublishMaterializeInputPayload, PublishMaterializeOutputPayload,
    RawKnowledgeIngestionInputPayload, RawKnowledgeIngestionOutputPayload,
    RebuildDetectInputPayload, RebuildDetectOutputPayload, SectionTemplateBinding,
    SeoSiteBuildInputPayload, SeoVerifiedFactSupportState, SerpIngestInputPayload,
    SerpIngestOutputPayload, SerpNormalizeInputPayload, SerpNormalizeOutputPayload,
    SourceContextChunkState,
};
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
    collection_name: &str,
    record: semantic_search_adapter::SearchResultRecord,
) -> SourceContextChunkState {
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
    SourceContextChunkState {
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
        retrieval_score: format!("{:.4}", record.score),
        usage_policy: match collection_name {
            "verified_rules_4" => "verified_fact_support".to_string(),
            "raw_chunks_ctx" => "contextual_source_neighborhood_not_fact_support".to_string(),
            "editorial_topics_4" => "editorial_topic_support_not_fact_support".to_string(),
            "raw_chunks_4" => "standard_source_neighborhood_not_fact_support".to_string(),
            _ => "supplemental_context_not_fact_support".to_string(),
        },
    }
}

async fn search_context_collection(
    collection_name: &str,
    query: &str,
    limit: usize,
    surface: semantic_search_adapter::VoyageSearchSurface,
) -> Result<Vec<SourceContextChunkState>, DomainError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
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
        .map(|record| source_chunk_from_search_result(collection_name, record))
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
            let cluster_key = primitives::seo::seo_artifact_key(
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

fn read_role_type(role_type: &str) -> i32 {
    match role_type {
        "must_provide" => RuleRoleTypeV1::MustProvide as i32,
        "must_pay" => RuleRoleTypeV1::MustPay as i32,
        "must_satisfy" => RuleRoleTypeV1::MustSatisfy as i32,
        "allows" => RuleRoleTypeV1::Allows as i32,
        "forbids" => RuleRoleTypeV1::Forbids as i32,
        "timeline" => RuleRoleTypeV1::Timeline as i32,
        "document_required" => RuleRoleTypeV1::DocumentRequired as i32,
        "eligibility_rule" => RuleRoleTypeV1::EligibilityRule as i32,
        "fee_item" => RuleRoleTypeV1::FeeItem as i32,
        "timeline_item" => RuleRoleTypeV1::TimelineItem as i32,
        "where_to_apply" => RuleRoleTypeV1::WhereToApply as i32,
        "appointment_rule" => RuleRoleTypeV1::AppointmentRule as i32,
        "form_required" => RuleRoleTypeV1::FormRequired as i32,
        "step" => RuleRoleTypeV1::Step as i32,
        _ => RuleRoleTypeV1::Unspecified as i32,
    }
}

fn map_params(params: &RuleParams) -> Option<citation_fact::Params> {
    match params {
        RuleParams::None => None,
        RuleParams::Document {
            severity,
            subtype,
            notarization_required,
            translation_required,
            accepts_alternatives,
            conditions_key,
        } => Some(citation_fact::Params::DocParams(DocumentRequiredParams {
            severity: severity.clone(),
            subtype: subtype.clone().unwrap_or_default(),
            notarization_required: *notarization_required,
            translation_required: *translation_required,
            accepts_alternatives: *accepts_alternatives,
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::Fee {
            amount,
            currency,
            severity,
            channel,
            conditions_key,
        } => Some(citation_fact::Params::FeeParams(FeeItemParams {
            amount: *amount,
            currency: currency.clone(),
            severity: severity.clone(),
            channel: channel.clone().unwrap_or_default(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::Timeline {
            days,
            subtype,
            severity,
            conditions_key,
        } => Some(citation_fact::Params::TimelineParams(TimelineItemParams {
            days: *days,
            subtype: subtype.clone().unwrap_or_default(),
            severity: severity.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::WhereToApply {
            location_key,
            channel,
            conditions_key,
        } => Some(citation_fact::Params::WhereParams(WhereToApplyParams {
            location_key: location_key.clone(),
            channel: channel.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::EligibilityRule {
            subtype,
            severity,
            conditions_key,
        } => Some(citation_fact::Params::EligibilityParams(
            EligibilityRuleParams {
                subtype: subtype.clone(),
                severity: severity.clone(),
                conditions_key: conditions_key.clone(),
            },
        )),
        RuleParams::AppointmentRule {
            subtype,
            advance_days,
            conditions_key,
        } => Some(citation_fact::Params::AppointmentParams(
            AppointmentRuleParams {
                subtype: subtype.clone(),
                advance_days: *advance_days,
                conditions_key: conditions_key.clone(),
            },
        )),
        RuleParams::FormRequired {
            form_id,
            severity,
            conditions_key,
        } => Some(citation_fact::Params::FormParams(FormRequiredParams {
            form_id: form_id.clone(),
            severity: severity.clone(),
            conditions_key: conditions_key.clone(),
        })),
        RuleParams::Step {
            step_index,
            subtype,
            conditions_key,
        } => Some(citation_fact::Params::StepParams(StepParams {
            step_index: *step_index,
            subtype: subtype.clone(),
            conditions_key: conditions_key.clone(),
        })),
    }
}

