use async_trait::async_trait;
use contracts::generated::alegria::read_api::v1::ContextBundle;
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
use primitives::errors::DomainError;
use runtime_models::GraphPlanningContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifiedSupportBundleRequest {
    pub run_id: String,
    pub context_key: String,
    pub scope_signature: String,
    pub applicant_profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RebuildDependencyEvidence {
    pub page_node_key: String,
    pub reason_package: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrganicSerpResult {
    pub rank: i32,
    pub title: String,
    pub url: String,
    pub url_norm: String,
    pub domain_norm: String,
    pub source_tier: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrganicSerpResponse {
    pub raw_payload_utf8: String,
    pub organic_results: Vec<OrganicSerpResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticLinkCandidate {
    pub entity_key: String,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticDemandCluster {
    pub cluster_key: String,
    pub member_queries: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GlobalNavigationPersistReport {
    pub navigation_tree_key: String,
    pub scope_count: u64,
    pub page_item_count: u64,
    pub silo_group_count: u64,
    pub rebuild_plan_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmsReviewDecisionRequest {
    pub page_node_key: String,
    pub actor_role: String,
    pub decision: String,
    pub reason: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmsReviewDecisionOutcome {
    pub decision_key: String,
    pub revision_id: String,
    pub workflow_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeoSiteBuildRegistrationRequest {
    pub run_id: String,
    pub context_key: Option<String>,
    pub market: String,
    pub locale: String,
    pub country_code: String,
    pub visa_type: String,
    pub visa_subtype: Option<String>,
    pub applicant_profile: String,
    pub citizenship_code: String,
    pub bootstrap_context: bool,
    pub queries: Vec<String>,
    pub query_batch_key: Option<String>,
    pub run_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProjectionBarrierStatus {
    pub blocked_events: i64,
    pub max_open_lag_ms: i64,
}

#[async_trait]
pub trait SeoBuildInputRepository: Send + Sync {
    async fn load_site_build_input(
        &self,
        run_id: &str,
    ) -> Result<SeoSiteBuildInputPayload, DomainError>;
}

#[async_trait]
pub trait SeoBuildRegistrationRepository: Send + Sync {
    async fn register_site_build_input(
        &self,
        request: &SeoSiteBuildRegistrationRequest,
    ) -> Result<SeoSiteBuildInputPayload, DomainError>;
}

#[async_trait]
pub trait VerifiedSupportRepository: Send + Sync {
    async fn load_verified_support_bundle(
        &self,
        request: &VerifiedSupportBundleRequest,
    ) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError>;
}

#[async_trait]
pub trait CrawlIngestRepository: Send + Sync {
    async fn crawl_sources(
        &self,
        input: &CrawlSourcesInputPayload,
    ) -> Result<CrawlSourcesOutputPayload, DomainError>;

    async fn ingest_raw_knowledge(
        &self,
        input: &RawKnowledgeIngestionInputPayload,
    ) -> Result<RawKnowledgeIngestionOutputPayload, DomainError>;
}

#[async_trait]
pub trait SourceContextRepository: Send + Sync {
    async fn load_source_context_chunks(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SourceContextChunkState>, DomainError>;
}

#[async_trait]
pub trait ProjectionStatusRepository: Send + Sync {
    async fn load_projection_barrier_status(
        &self,
        run_id: &str,
    ) -> Result<ProjectionBarrierStatus, DomainError>;
}

#[async_trait]
pub trait RebuildRepository: Send + Sync {
    async fn narrow_rebuild_impacts(
        &self,
        changed_truth_keys: &[String],
    ) -> Result<Vec<RebuildDependencyEvidence>, DomainError>;

    async fn persist_rebuild_detect_output(
        &self,
        input: &RebuildDetectInputPayload,
        output: &RebuildDetectOutputPayload,
    ) -> Result<(), DomainError>;

    async fn semantic_neighbor_impacts(
        &self,
        changed_truth_keys: &[String],
        page_nodes: &[PageNodeState],
    ) -> Result<Vec<RebuildDependencyEvidence>, DomainError>;
}

#[async_trait]
pub trait SerpSearchPort: Send + Sync {
    async fn fetch_google_organic_live_advanced(
        &self,
        locale: Option<&str>,
        query: &str,
    ) -> Result<Option<OrganicSerpResponse>, DomainError>;
}

#[async_trait]
pub trait SemanticLinkSearchPort: Send + Sync {
    async fn search_link_targets(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SemanticLinkCandidate>, DomainError>;

    async fn search_keyword_clusters(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SemanticLinkCandidate>, DomainError>;

    async fn cluster_demand_queries(
        &self,
        queries: &[String],
    ) -> Result<Vec<SemanticDemandCluster>, DomainError>;
}

#[async_trait]
pub trait PlanningRepository: Send + Sync {
    async fn load_graph_planning_context(
        &self,
        run_id: &str,
        scope_signature: &str,
    ) -> Result<GraphPlanningContext, DomainError>;

    async fn persist_serp_ingest_output(
        &self,
        input: &SerpIngestInputPayload,
        output: &SerpIngestOutputPayload,
    ) -> Result<(), DomainError>;

    async fn persist_live_serp_query_results(
        &self,
        run_id: &str,
        query_batch_key: &str,
        ordinal: usize,
        query: &str,
        response: &OrganicSerpResponse,
    ) -> Result<(), DomainError>;

    async fn persist_serp_normalize_output(
        &self,
        input: &SerpNormalizeInputPayload,
        output: &SerpNormalizeOutputPayload,
    ) -> Result<(), DomainError>;

    async fn persist_opportunity_build_output(
        &self,
        input: &OpportunityBuildInputPayload,
        output: &OpportunityBuildOutputPayload,
    ) -> Result<(), DomainError>;

    async fn persist_ia_build_output(
        &self,
        input: &IaBuildInputPayload,
        output: &IaBuildOutputPayload,
    ) -> Result<(), DomainError>;

    async fn persist_link_recommend_output(
        &self,
        input: &LinkRecommendInputPayload,
        output: &LinkRecommendOutputPayload,
    ) -> Result<(), DomainError>;

    async fn persist_global_site_reconcile_output(
        &self,
        input: &GlobalSiteReconcileInputPayload,
        output: &GlobalSiteReconcileOutputPayload,
    ) -> Result<GlobalNavigationPersistReport, DomainError>;
}

#[async_trait]
pub trait SectionTemplateRepository: Send + Sync {
    async fn load_section_templates(
        &self,
        page_type_key: &str,
        dominant_intent: &str,
    ) -> Result<Vec<SectionTemplateBinding>, DomainError>;
}

#[async_trait]
pub trait DraftRepository: Send + Sync {
    async fn persist_draft_assemble_output(
        &self,
        run_id: &str,
        output: &DraftAssembleOutputPayload,
    ) -> Result<(), DomainError>;

    async fn persist_draft_normalize_output(
        &self,
        input: &DraftNormalizeInputPayload,
        output: &DraftNormalizeOutputPayload,
    ) -> Result<(), DomainError>;

    async fn persist_content_contract_validate_output(
        &self,
        input: &ContentContractValidateInputPayload,
        output: &ContentContractValidateOutputPayload,
    ) -> Result<(), DomainError>;

    async fn persist_draft_qa_output(
        &self,
        input: &DraftQaInputPayload,
        output: &DraftQaOutputPayload,
    ) -> Result<(), DomainError>;
}

#[async_trait]
pub trait EditorialGenerationPort: Send + Sync {
    async fn generate_editorial_draft(
        &self,
        input: &EditorialDraftGenerateInputPayload,
    ) -> Result<EditorialDraftGenerateOutputPayload, DomainError>;
}

#[async_trait]
pub trait CmsReviewPort: Send + Sync {
    async fn persist_cms_publish_output(
        &self,
        input: &CmsPublishInputPayload,
        output: &CmsPublishOutputPayload,
    ) -> Result<CmsPublishOutputPayload, DomainError>;

    async fn load_latest_approval_decision(
        &self,
        page_node_key: &str,
        revision_id: &str,
    ) -> Result<Option<CmsApprovalDecision>, DomainError>;
}

#[async_trait]
pub trait CmsReviewDecisionPort: Send + Sync {
    async fn apply_human_review_decision(
        &self,
        request: &CmsReviewDecisionRequest,
    ) -> Result<CmsReviewDecisionOutcome, DomainError>;
}

#[async_trait]
pub trait SeoWorkflowControlPort: Send + Sync {
    async fn signal_resume(&self, workflow_id: &str) -> Result<(), DomainError>;
}

#[async_trait]
pub trait PublishArtifactRepository: Send + Sync {
    async fn persist_publish_materialize_output(
        &self,
        input: &PublishMaterializeInputPayload,
        output: &PublishMaterializeOutputPayload,
    ) -> Result<PublishMaterializeOutputPayload, DomainError>;

    async fn persist_finalize_publish_output(
        &self,
        input: &FinalizePublishInputPayload,
        output: &FinalizePublishOutputPayload,
    ) -> Result<FinalizePublishOutputPayload, DomainError>;
}

#[async_trait]
pub trait ContextBundleRepository: Send + Sync {
    async fn load_context_bundle(&self, context_key: &str) -> Result<ContextBundle, DomainError>;
}

#[async_trait]
pub trait HitlQueuePort: Send + Sync {
    async fn enqueue_hitl_task(
        &self,
        task_type: &str,
        diagnostics: &HitlTaskContext,
        priority: i32,
    ) -> Result<i64, DomainError>;

    async fn resolve_hitl_task(
        &self,
        task_id: i64,
        resolution: &HitlDecision,
    ) -> Result<(), DomainError>;
}
