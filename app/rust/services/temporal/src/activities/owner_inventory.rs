//! Exhaustive logical ownership of the 73 *registered* legacy Activities.
//!
//! All entries still run on `alegria-pipeline` inside `temporal_worker`.
//! A logical owner is the package responsible for the operation's contract,
//! not a claim of physical isolation or permission to change an existing
//! Activity's name/queue. `LegacyMixedRuntime` marks an extraction blocker.
//! `automation/check_temporal_activity_owner_inventory.py` compares this list
//! against every `#[activity]` method in the registered SDK impl.

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogicalOwner {
    Acquisition,
    TruthDomain,
    SeoDomain,
    SeoApplication,
    PostgresStore,
    ProjectionDelivery,
    LegacyMixedRuntime,
    LegacyTestRuntime,
}

#[allow(dead_code)]
impl LogicalOwner {
    pub(crate) const fn package(self) -> &'static str {
        match self {
            Self::Acquisition => "acquisition",
            Self::TruthDomain => "truth_domain",
            Self::SeoDomain => "seo_domain",
            Self::SeoApplication => "seo_application",
            Self::PostgresStore => "postgres_store",
            Self::ProjectionDelivery => "projection_delivery",
            Self::LegacyMixedRuntime | Self::LegacyTestRuntime => "temporal_worker",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LegacyActivityRoute {
    pub(crate) method: &'static str,
    pub(crate) logical_owner: LogicalOwner,
    pub(crate) deployed_task_queue: &'static str,
}

pub(crate) const LEGACY_TASK_QUEUE: &str = "alegria-pipeline";

macro_rules! route {
    ($method:ident => $owner:ident) => {
        LegacyActivityRoute {
            method: stringify!($method),
            logical_owner: LogicalOwner::$owner,
            deployed_task_queue: LEGACY_TASK_QUEUE,
        }
    };
}

#[allow(dead_code)]
pub(crate) const REGISTERED_ACTIVITY_ROUTES: &[LegacyActivityRoute] = &[
    route!(run_page_utility_classifier_step => Acquisition),
    route!(run_dom_block_relevance_step => Acquisition),
    route!(run_layer_router_step => SeoDomain),
    route!(run_entity_span_detection_step => SeoDomain),
    route!(run_canonical_mapping_step => SeoDomain),
    route!(run_procedural_extraction_step => SeoDomain),
    route!(run_operational_extraction_step => SeoDomain),
    route!(run_editorial_extraction_step => SeoDomain),
    route!(run_completeness_judge_step => TruthDomain),
    route!(run_triple_builder_step => SeoDomain),
    route!(run_contradiction_gate_step => TruthDomain),
    route!(run_hitl_decision_step => TruthDomain),
    route!(run_projection_reconcile_step => ProjectionDelivery),
    route!(run_projection_sync_step => PostgresStore),
    route!(run_projection_barrier_audit_step => PostgresStore),
    route!(load_semantic_section_sample_step => PostgresStore),
    route!(run_seo_preflight_step => SeoApplication),
    route!(run_whole_page_semantic_pass_step => Acquisition),
    route!(run_sectioning_step => Acquisition),
    route!(run_page_utility_sweep_step => Acquisition),
    route!(run_dom_block_relevance_sweep_step => Acquisition),
    route!(run_sectioning_contract_gate_step => Acquisition),
    route!(run_cas_gate_step => Acquisition),
    route!(run_raw_evidence_register_step => PostgresStore),
    route!(run_layer_router_sweep_step => SeoDomain),
    route!(run_subspan_layer_router_step => SeoDomain),
    route!(run_entity_span_sweep_step => SeoDomain),
    route!(run_canonical_mapping_sweep_step => SeoDomain),
    route!(run_ontology_intake_gate_step => TruthDomain),
    route!(run_procedural_extraction_sweep_step => SeoDomain),
    route!(run_operational_extraction_sweep_step => SeoDomain),
    route!(run_editorial_extraction_sweep_step => SeoDomain),
    route!(run_seo_signal_extraction_step => SeoDomain),
    route!(run_commercial_signal_extraction_step => SeoDomain),
    route!(run_extraction_schema_validate_step => TruthDomain),
    route!(prepare_manual_truth_candidates => PostgresStore),
    route!(load_manual_truth_status => PostgresStore),
    route!(run_candidate_validation_step => TruthDomain),
    route!(run_triple_builder_sweep_step => SeoDomain),
    route!(run_completeness_judge_sweep_step => TruthDomain),
    route!(run_resolution_loop_step => TruthDomain),
    route!(run_contradiction_gate_sweep_step => TruthDomain),
    route!(run_truth_adjudication_step => LegacyMixedRuntime),
    route!(run_verified_truth_write_step => PostgresStore),
    route!(load_seo_site_build_input => PostgresStore),
    route!(load_verified_support_bundle => PostgresStore),
    route!(run_serp_ingest_step => Acquisition),
    route!(run_crawl_sources_step => Acquisition),
    route!(run_raw_knowledge_ingestion_step => LegacyMixedRuntime),
    route!(run_serp_normalize_step => SeoDomain),
    route!(run_opportunity_build_step => SeoApplication),
    route!(run_ia_build_step => SeoApplication),
    route!(run_link_recommend_step => LegacyMixedRuntime),
    route!(run_global_site_reconcile_step => LegacyMixedRuntime),
    route!(run_draft_assemble_step => SeoApplication),
    route!(run_draft_normalize_step => SeoApplication),
    route!(run_editorial_draft_generate => LegacyMixedRuntime),
    route!(load_manual_editorial_result => PostgresStore),
    route!(run_content_contract_validate_step => SeoApplication),
    route!(run_draft_qa_step => SeoApplication),
    route!(run_truth_admissibility_gate_step => TruthDomain),
    route!(run_cms_request_review_step => PostgresStore),
    route!(run_human_approval_wait_step => PostgresStore),
    route!(run_cms_publish_approved_step => LegacyMixedRuntime),
    route!(load_cms_approval_decision => PostgresStore),
    route!(run_rebuild_detect_step => SeoApplication),
    route!(run_publish_materialize_step => LegacyMixedRuntime),
    route!(run_render_preview_validate_step => SeoApplication),
    route!(run_finalize_publish_step => PostgresStore),
    route!(test_step_prepare => LegacyTestRuntime),
    route!(test_step_finalize => LegacyTestRuntime),
    route!(finalize_run => PostgresStore),
    route!(check_data_freshness => PostgresStore),
];
