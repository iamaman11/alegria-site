//! PostgreSQL-owned implementations of SEO application repository ports.
//!
//! Provider dispatch is deliberately absent: callers that need Graph,
//! Voyage, live SERP or LLM effects use runtime_composition's local wrapper.
//! Pure PG E2E can compile these same SQL implementations without linking
//! Neo4j/Qdrant/Temporal SDK packages.

use async_trait::async_trait;
use contracts::generated::alegria::temporal::v1::{
    CmsPublishInputPayload, CmsPublishOutputPayload, ContentContractValidateInputPayload,
    ContentContractValidateOutputPayload, DraftAssembleOutputPayload, DraftNormalizeInputPayload,
    DraftNormalizeOutputPayload, DraftQaInputPayload, DraftQaOutputPayload,
    FinalizePublishInputPayload, FinalizePublishOutputPayload, GlobalSiteReconcileInputPayload,
    GlobalSiteReconcileOutputPayload, IaBuildInputPayload, IaBuildOutputPayload,
    LinkRecommendInputPayload, LinkRecommendOutputPayload, OpportunityBuildInputPayload,
    OpportunityBuildOutputPayload, PublishMaterializeInputPayload, PublishMaterializeOutputPayload,
    SeoSiteBuildInputPayload, SeoVerifiedFactSupportState, SerpIngestInputPayload,
    SerpIngestOutputPayload, SerpNormalizeInputPayload, SerpNormalizeOutputPayload,
};
use infrastructure::adapters::{
    dataforseo_serp_adapter, sqlx_adapter::AlegriaPgPool, sqlx_context_bundle_adapter,
    sqlx_seo_adapter, sqlx_seo_cms_adapter, sqlx_serp_adapter,
};
use primitives::errors::DomainError;
use runtime_models::RuleRoleType;
use seo_domain::identity;
use seo_ports::knowledge::{VerifiedRule, VerifiedRulesQuery, VerifiedRulesView};
use seo_ports::{
    CmsApproval, CmsReviewDecisionOutcome, CmsReviewDecisionPort, CmsReviewDecisionRequest,
    CmsReviewPort, DraftRepository, OrganicSerpResponse, PlanningRepository,
    ProjectionBarrierStatus, ProjectionStatusRepository, PublishArtifactRepository,
    SectionTemplate, SectionTemplateRepository, SeoBuildInputRepository,
    SeoBuildRegistrationRepository, SeoSiteBuildRegistrationRequest, SerpIngestRepository,
    VerifiedRulesReader, VerifiedSupportBundleRequest, VerifiedSupportRepository,
};
use sqlx::Row;
use uuid::Uuid;

/// Exactly one SQL implementation per port. No provider connection is created
/// or inferred by constructing this repository.
pub struct SqlxSeoPgRepository<'a> {
    pool: &'a AlegriaPgPool,
}

impl<'a> SqlxSeoPgRepository<'a> {
    pub fn new(pool: &'a AlegriaPgPool) -> Self {
        Self { pool }
    }
}

include!("build_and_support_ports.rs");
include!("planning_ports.rs");
include!("draft_publish_ports.rs");
include!("projection_status_port.rs");
