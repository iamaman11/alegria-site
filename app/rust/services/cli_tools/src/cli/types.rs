use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use contracts::generated::alegria::temporal::v1::{
    CrawlSourcesOutputPayload, FactExtractionInputPayload, ProjectionBarrierAuditOutputPayload,
    RawKnowledgeIngestionInputPayload, RawKnowledgeIngestionOutputPayload, ValidationInputPayload,
};
use infrastructure::adapters::seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository;
use infrastructure::adapters::seo_workflow_control_adapter::TemporalSeoWorkflowControlAdapter;
use infrastructure::adapters::sqlx_reconcile_adapter::{
    reconcile_target_system_default, ReconcileOptionsRecord,
};
use infrastructure::adapters::{
    reqwest_adapter::new_default_client,
    sqlx_adapter::connect_pg,
    sqlx_static_site_adapter::{load_static_site_snapshot, StaticCmsLinkRow, StaticCmsPageRow},
    tonic_adapter::AnalyticsClient,
};
use primitives::fact_verifier_json::{verify_fact_json, verify_numeric_rule_json};
use prost::Message;
use pulldown_cmark::{html, Options as MarkdownOptions, Parser as MarkdownParser};
use seo_application::cms_review::{apply_human_review_decision, ApplyHumanReviewDecisionInput};
use seo_application::crawl_ingest::run_crawl_sources;
use seo_application::execution::{run_mode_for_scenario, SeoRunPolicy};
use seo_application::registration::register_site_build_input;
use seo_application::scenario::{
    execute_site_build_scenario, SeoExecutionMode, SeoScenarioKind, SeoScenarioRequest,
};
use seo_ports::SeoSiteBuildRegistrationRequest;
use serde_json::{json, Value};
use sqlx::{types::Json, Row};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "cli_tools")]
#[command(about = "Alegria Rust CLI tools", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    CheckRustMigrationContract {
        #[arg(long, default_value = ".")]
        root: String,
        #[arg(
            long,
            default_value = "automation/reports/rust_migration_contract.rust.json"
        )]
        report_json: String,
        #[arg(long, default_value_t = false)]
        strict: bool,
    },
    CheckFactVerifierParity {
        #[arg(long, default_value = ".")]
        root: String,
        #[arg(
            long,
            default_value = "automation/reports/fact_verifier_parity.rust.json"
        )]
        report_json: String,
    },
    ComputeContentHash {
        #[arg(long)]
        text: String,
    },
    ComputeBytesHash {
        #[arg(long)]
        hex: String,
    },
    ComputeStableRuleId {
        #[arg(long, num_args = 1..)]
        parts: Vec<String>,
    },
    EncodeFactInput {
        #[arg(long)]
        sections_json: String,
    },
    EncodeValidationInput {
        #[arg(long)]
        required_links_json: String,
        #[arg(long)]
        required_keys_json: String,
        #[arg(long)]
        used_rule_keys_json: String,
        #[arg(long)]
        used_fact_keys_json: String,
        #[arg(long)]
        url_norm: String,
    },
    ReconcileTargetSystem {
        #[arg(long)]
        target_system: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long, default_value_t = 10)]
        max_retry_count: i32,
        #[arg(long, default_value_t = 500)]
        batch_limit: i64,
        #[arg(long, default_value_t = 0)]
        requeue_base_delay_sec: i64,
        #[arg(long, default_value_t = 15)]
        requeue_jitter_sec: i64,
    },
    BuildStaticSite {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long, default_value = "app/rust/dist/static-site")]
        output_dir: String,
        #[arg(long, default_value = "https://example.com")]
        base_url: String,
    },
    CrawlPendingSources {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long, default_value = "")]
        run_id: String,
        #[arg(long, default_value = "")]
        query_batch_key: String,
        #[arg(long, default_value_t = 25)]
        limit: i64,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        emit_qdrant: bool,
    },
    CmsReviewList {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
    CmsReviewShow {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        page_node_key: String,
    },
    CmsPublishStatus {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        page_node_key: String,
    },
    CmsTraceabilityInspect {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        page_node_key: String,
    },
    CmsBlockersInspect {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        page_node_key: String,
    },
    SeoRebuildBacklogInspect {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        page_node_key: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
    SeoSupportBundleInspect {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        context_key: String,
    },
    SeoPostPublishFeedbackProbe {
        #[arg(long, default_value = "http://127.0.0.1:50051")]
        analytics_addr: String,
        #[arg(long, default_value_t = false)]
        require_gsc: bool,
        #[arg(long)]
        report_json: Option<String>,
    },
    SeoReleaseRestoreGate {
        #[arg(long, default_value = ".")]
        root: String,
        #[arg(long, default_value_t = false)]
        run_ci_verify: bool,
        #[arg(long, default_value_t = false)]
        run_temporal_gate: bool,
        #[arg(long, default_value_t = false)]
        run_restore_drill: bool,
        #[arg(long)]
        report_json: Option<String>,
    },
    SeoCutoverShadowVerify {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        legacy_run_id: String,
        #[arg(long)]
        cutover_run_id: String,
        #[arg(long, default_value_t = false)]
        strict: bool,
        #[arg(long)]
        report_json: Option<String>,
    },
    CmsApprovePublish {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        page_node_key: String,
        #[arg(long)]
        actor_role: String,
        #[arg(long, default_value = "")]
        reason: String,
        #[arg(long, default_value = "app/rust/dist/static-site")]
        output_dir: String,
        #[arg(long, default_value = "https://example.com")]
        base_url: String,
    },
    CmsBlock {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        page_node_key: String,
        #[arg(long)]
        actor_role: String,
        #[arg(long, default_value = "")]
        reason: String,
    },
    CmsReopen {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        page_node_key: String,
        #[arg(long)]
        actor_role: String,
        #[arg(long, default_value = "")]
        reason: String,
    },
    SeoRun {
        #[command(subcommand)]
        scenario: SeoRunCommand,
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        run_id: Option<String>,
        #[arg(long)]
        context_key: Option<String>,
        #[arg(long, default_value = "alegria-site")]
        market: String,
        #[arg(long, default_value = "ru-RU")]
        locale: String,
        #[arg(long, default_value = "ES")]
        country_code: String,
        #[arg(long, default_value = "tourist")]
        visa_type: String,
        #[arg(long)]
        visa_subtype: Option<String>,
        #[arg(long, default_value = "standard")]
        applicant_profile: String,
        #[arg(long, default_value = "RU")]
        citizenship_code: String,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        bootstrap_context: bool,
        #[arg(long = "query")]
        queries: Vec<String>,
        #[arg(long)]
        query_batch_key: Option<String>,
        #[arg(long, default_value = "app/rust/dist/static-site")]
        output_dir: String,
        #[arg(long, default_value = "https://example.com")]
        base_url: String,
        #[arg(long, default_value_t = false)]
        publish: bool,
        #[arg(long, default_value_t = false)]
        require_preapproved_decision: bool,
        #[arg(long, default_value_t = false)]
        warn_only_projections: bool,
    },
}

#[derive(Subcommand, Debug, Clone, Copy)]
enum SeoRunCommand {
    Planning,
    Drafting,
    Publish,
    Full,
    Rebuild,
    CrawlIngest,
}

#[derive(Debug)]
struct Finding {
    level: &'static str,
    code: &'static str,
    message: String,
}

#[derive(Debug)]
struct StepPayloadBlob {
    payload_type: String,
    payload_bytes: Vec<u8>,
}

const CUTOVER_PHASE_M1_LEDGER_STEPS: &[&str] = &[
    "seo_preflight",
    "serp_ingest",
    "crawl_sources",
    "whole_page_semantic_pass",
    "page_utility_classifier",
    "dom_block_relevance_filter",
    "sectioning",
    "sectioning_contract_gate",
    "cas_gate",
    "raw_evidence_register",
    "projection_barrier(raw_evidence)",
    "layer_router",
    "subspan_layer_router",
    "entity_span_detection",
    "canonical_mapping",
    "ontology_intake_gate",
    "procedural_extraction",
    "operational_extraction",
    "editorial_extraction",
    "seo_signal_extraction",
    "commercial_signal_extraction",
    "extraction_schema_validate",
    "candidate_validation",
    "triple_builder",
    "completeness_judge",
    "resolution_loop",
    "contradiction_gate",
    "truth_adjudication",
    "verified_truth_write",
    "graph_admissibility_gate",
    "retrieval_admissibility_gate",
    "neo4j_sync",
    "voyage_qdrant_sync",
    "projection_barrier(semantic_projection)",
];

const CUTOVER_PHASE_M1_NON_LEDGERED_ACTIVITY_STEPS: &[&str] = &[
    "load_verified_support_bundle.initial",
    "load_verified_support_bundle.refresh",
];

const CUTOVER_PHASE_M2_LEDGER_STEPS: &[&str] = &[
    "serp_normalize",
    "opportunity_build",
    "ia_build",
    "link_recommend",
    "global_site_reconcile",
    "projection_barrier(global_site_reconcile)",
    "truth_admissibility_gate",
    "draft_assemble",
    "editorial_draft_generate",
    "draft_normalize",
    "content_contract_validate",
    "draft_qa",
    "cms_request_review",
    "human_approval_wait",
    "load_cms_approval_decision",
    "cms_publish_approved",
    "publish_materialize",
    "render_preview_validate",
    "finalize_publish",
    "projection_barrier(publish)",
    "rebuild_detect",
];

const LEGACY_PHASE_M2_LEDGER_STEPS: &[&str] = &[
    "serp_normalize",
    "opportunity_build",
    "ia_build",
    "link_recommend",
    "global_site_reconcile",
    "draft_assemble",
    "editorial_draft_generate",
    "draft_normalize",
    "content_contract_validate",
    "draft_qa",
    "cms_publish",
    "load_cms_approval_decision",
    "publish_materialize",
    "render_preview_validate",
    "finalize_publish",
    "rebuild_detect",
];

