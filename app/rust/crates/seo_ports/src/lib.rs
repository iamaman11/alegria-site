use async_trait::async_trait;
pub mod knowledge;

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
use knowledge::{
    CanonicalVectorLookup, SemanticNeighborCandidate, SemanticNeighborCollection,
    SourceContextChunk, SourceContextCollection, VerifiedRuleEmbeddingBatch,
    VerifiedRuleUpsertBatch, VerifiedRulesQuery, VerifiedRulesView,
};
use primitives::errors::DomainError;
use runtime_models::GraphPlanningContext;
use seo_domain::rebuild::{RebuildDetectRequest, RebuildDetectResult};
pub use seo_domain::review_decision::{CmsApproval, HumanReviewDecision};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Exact bytes and public route of one file in an immutable approved release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicReleaseFile {
    pub relative_path: String,
    pub public_route: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseDeliveryStage {
    Preflight,
    Upload,
    Readback,
}

#[derive(Debug)]
pub struct ReleaseDeliveryError {
    pub stage: ReleaseDeliveryStage,
    pub detail: String,
}

impl std::fmt::Display for ReleaseDeliveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "public release {:?} failed: {}", self.stage, self.detail)
    }
}

impl std::error::Error for ReleaseDeliveryError {}

/// One replaceable external effect: deploy an immutable release and read back
/// its actual public bytes. The caller owns approval, PostgreSQL and Outbox.
#[async_trait]
pub trait PublicReleaseDelivery: Send + Sync {
    fn preflight(&self) -> Result<(), ReleaseDeliveryError>;

    /// Synchronous so an activation guard can safely restore the previous
    /// release during Drop without spawning a detached deployment.
    fn deploy(&self, release: &Path, previous: Option<&Path>) -> Result<(), ReleaseDeliveryError>;

    async fn verify(
        &self,
        origin: &str,
        files: &[PublicReleaseFile],
    ) -> Result<(), ReleaseDeliveryError>;
}

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

/// Section template loaded from the editorial catalog. The repository contract
/// does not depend on the Temporal payload used to orchestrate draft assembly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionTemplate {
    pub template_key: String,
    pub page_type_key: String,
    pub dominant_intent: String,
    pub section_role: String,
    pub heading: String,
    pub template_body: String,
    pub template_version: u32,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmsReviewDecisionRequest {
    pub page_node_key: String,
    /// Exact immutable review candidate. Page-only review is not supported:
    /// the current published pointer may differ from the pending candidate.
    pub revision_id: String,
    pub actor_role: String,
    pub decision: HumanReviewDecision,
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
    #[serde(default)]
    pub retained_source_run_id: Option<String>,
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
pub trait SourceContextSearchPort: Send + Sync {
    /// Runtime configuration is an effect; the application owns required/optional behavior.
    fn provider_configured(&self) -> bool;
    fn retrieval_required(&self) -> bool;

    async fn search_source_context_collection(
        &self,
        collection: SourceContextCollection,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SourceContextChunk>, DomainError>;
}

/// Embedding is the only external operation needed before the canonical Truth
/// write. SQL registry and Outbox effects belong to the Truth transaction,
/// rather than a second port that could commit independently.
#[async_trait]
pub trait VerifiedRuleEmbeddingPort: Send + Sync {
    fn retrieval_required(&self) -> bool;

    async fn embed_rule_texts(
        &self,
        run_id: &str,
        texts: &[String],
    ) -> Result<Option<VerifiedRuleEmbeddingBatch>, DomainError>;
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
        input: &RebuildDetectRequest,
        output: &RebuildDetectResult,
    ) -> Result<(), DomainError>;

    async fn semantic_neighbor_impacts(
        &self,
        changed_truth_keys: &[String],
        page_node_keys: &[String],
    ) -> Result<Vec<RebuildDependencyEvidence>, DomainError>;
}

#[async_trait]
pub trait SerpSearchPort: Send + Sync {
    async fn fetch_google_organic_standard(
        &self,
        run_id: &str,
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

/// Candidate discovery only; canonical mapping policy belongs to Knowledge.
#[async_trait]
pub trait CanonicalVectorSearchPort: Send + Sync {
    async fn search_canonical_concepts(
        &self,
        raw_text: &str,
    ) -> Result<CanonicalVectorLookup, DomainError>;
}

/// Replaceable Qdrant/Voyage effects for Truth's read-only retrieval diagnostics.
/// Keeping search and rerank separate allows the use case to distinguish a
/// best-effort search failure from a failed evidence-bearing rerank.
#[async_trait]
pub trait SemanticNeighborSearchPort: Send + Sync {
    fn provider_configured(&self) -> bool;

    async fn search_semantic_neighbors(
        &self,
        query: &str,
        collection: SemanticNeighborCollection,
        limit: usize,
    ) -> Result<Vec<SemanticNeighborCandidate>, DomainError>;

    async fn rerank_semantic_neighbors(
        &self,
        query: &str,
        candidates: Vec<SemanticNeighborCandidate>,
        limit: usize,
    ) -> Result<Vec<SemanticNeighborCandidate>, DomainError>;
}

#[async_trait]
pub trait GraphReasoningPort: Send + Sync {
    async fn load_planning_graph_context(
        &self,
        scope_signature: &str,
        run_id: &str,
    ) -> Result<GraphPlanningContext, DomainError>;
}

#[async_trait]
pub trait GraphCapabilityPort: Send + Sync {
    async fn ensure_graph_contract(&self, context_key: &str) -> Result<(), DomainError>;

    /// One error boundary for the mandatory graph capability shared by
    /// planning, drafting and rebuild. The adapter owns the connection check;
    /// the application identifies the phase that cannot proceed.
    async fn require_graph_contract(
        &self,
        phase_family: &str,
        phase: &str,
    ) -> Result<(), DomainError> {
        self.ensure_graph_contract(phase)
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!(
                    "graph capability contract failed for {phase_family} phase `{phase}`: {err}"
                ),
            })
    }
}

#[async_trait]
pub trait SerpIngestRepository: Send + Sync {
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
}

#[async_trait]
pub trait PlanningRepository: Send + Sync {
    async fn persist_serp_normalize_output(
        &self,
        input: &SerpNormalizeInputPayload,
        output: &SerpNormalizeOutputPayload,
    ) -> Result<(), DomainError>;

    async fn persist_opportunity_build_output(
        &self,
        input: &OpportunityBuildInputPayload,
        output: &OpportunityBuildOutputPayload,
    ) -> Result<OpportunityBuildOutputPayload, DomainError>;

    /// The same keyword cluster retains one page identity when its URL changes.
    async fn existing_page_owners_by_cluster(
        &self,
        scope_signature: &str,
        cluster_keys: &[String],
    ) -> Result<Vec<(String, String)>, DomainError>;

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
    ) -> Result<(), DomainError>;
}

#[async_trait]
pub trait SectionTemplateRepository: Send + Sync {
    async fn load_section_templates(
        &self,
        page_type_key: &str,
        dominant_intent: &str,
    ) -> Result<Vec<SectionTemplate>, DomainError>;
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
    ) -> Result<Option<CmsApproval>, DomainError>;
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
    /// Resolve deployment-only settings before the pure materialization gate.
    /// Unconfigured values stay empty so the gate fails closed.
    fn resolve_publish_materialize_input(
        &self,
        input: &PublishMaterializeInputPayload,
    ) -> PublishMaterializeInputPayload;

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
pub trait VerifiedRulesReader: Send + Sync {
    /// None means the requested context is not active. An active context
    /// with no admissible verified rules is returned with an empty rules list.
    async fn read_verified_rules(
        &self,
        query: &VerifiedRulesQuery,
    ) -> Result<Option<VerifiedRulesView>, DomainError>;
}
