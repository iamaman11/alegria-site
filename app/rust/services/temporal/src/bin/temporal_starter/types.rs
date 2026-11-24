use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use temporal_gateway::connect_client;
use temporalio_client::{
    Client, UntypedSignal, UntypedWorkflow, WorkflowDescribeOptions,
    WorkflowExecutionStatus as WorkflowClientStatus, WorkflowGetResultOptions,
    WorkflowSignalOptions, WorkflowStartOptions,
};
use temporalio_common::data_converters::{PayloadConverter, RawValue};
use infrastructure::adapters::{
    dataforseo_serp_adapter,
    sqlx_execution_runs_adapter, sqlx_serp_adapter,
    seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository,
    sqlx_adapter::connect_pg,
    sqlx_seo_adapter,
    voyage_api_adapter::{
        VoyageClient, VoyageEmbeddingOptions, VoyageInputType, VoyageOutputDtype,
        VoyageRerankOptions,
    },
};
use graph_neo4j as neo4rs_adapter;
use vector_qdrant as qdrant_client_adapter;
use primitives::{hash::blake3_hex, qdrant_point_id::qdrant_point_id_v1};
use seo_application::execution::{normalize_run_mode, SeoDurablePlan, SeoDurableStep};
use seo_domain::identity;
use seo_ports::{SeoBuildRegistrationRepository, SeoSiteBuildRegistrationRequest};
use serde_json::{json, Value};
use sqlx::{types::Json, Row};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "temporal_starter")]
#[command(about = "Temporal starter CLI for Alegria workflows")]
struct Cli {
    /// Temporal endpoint, example: http://localhost:7233
    #[arg(long, default_value = "http://localhost:7233")]
    temporal_url: String,
    /// Temporal namespace
    #[arg(long, default_value = "default")]
    namespace: String,
    /// Task queue (must match worker queue)
    #[arg(long, default_value = "alegria-pipeline")]
    task_queue: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Connectivity check: open client and print namespace/task queue
    Ping,
    /// Start workflow execution
    Start {
        #[arg(long, value_enum)]
        workflow: WorkflowKind,
        /// Optional workflow id; if omitted a deterministic prefix + UUID is used
        #[arg(long)]
        workflow_id: Option<String>,
        /// Postgres URL for SEO site-build input registration.
        #[arg(long)]
        database_url: Option<String>,
        /// Authoritative kb.visa_contexts.context_key for SEO site-build.
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
        /// SEO query batch. Repeat --query for multiple fixed queries.
        #[arg(long = "query")]
        queries: Vec<String>,
        /// Verified primary URLs for the SAME canonical source crawl.
        /// Repeat --source-url; these never acquire a fabricated SERP rank.
        #[arg(long = "source-url")]
        source_urls: Vec<String>,
        #[arg(long)]
        query_batch_key: Option<String>,
        /// Reprocess the exact acquired corpus of a terminal canonical run. No new search or crawl.
        #[arg(long, conflicts_with_all = ["queries", "source_urls", "query_batch_key"])]
        reuse_corpus_from: Option<String>,
        #[arg(long, default_value = "publish_with_hitl")]
        run_mode: String,
        /// Explicit V2 source-acquisition-only step order (not full crawl ingest).
        #[arg(long, value_enum)]
        v2_order: Option<V2Order>,
        /// Await workflow completion, including genuine external HITL decision.
        #[arg(long, default_value_t = false)]
        wait: bool,
    },
    /// Validate and optionally bootstrap a SEO site-build scope without starting Temporal.
    SeoPreflight {
        #[arg(long)]
        database_url: Option<String>,
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
        /// Fail preflight when Neo4j/Qdrant/CMS projection outbox is not fully drained.
        #[arg(long, default_value_t = false)]
        strict_projections: bool,
        /// Warn when pending/processing projection events are older than this many milliseconds.
        #[arg(long, default_value_t = 300_000)]
        projection_max_lag_ms: i64,
        #[arg(long, default_value = "app/rust/dist/static-site")]
        output_dir: String,
        #[arg(long)]
        report_json: Option<String>,
    },
    /// Consume queued rebuild backlog rows and start scoped rebuild runs.
    RebuildDispatch {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long, default_value_t = 10)]
        limit: i64,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long)]
        report_json: Option<String>,
    },
    /// Plan ontology backfill/reindex work and optionally materialize concepts into Neo4j.
    OntologyBackfillPlan {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long)]
        concept_key: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: i64,
        #[arg(long, default_value_t = false)]
        apply_neo4j: bool,
        #[arg(long, default_value_t = false)]
        apply_qdrant: bool,
        #[arg(long)]
        report_json: Option<String>,
    },
    /// End-to-end test workflow with HITL pause/resume
    DemoHitl {
        /// Optional workflow id; if omitted generated
        #[arg(long)]
        workflow_id: Option<String>,
        /// Delay before sending resume signal
        #[arg(long, default_value_t = 700)]
        wait_before_resume_ms: u64,
    },
    /// Send a resume signal to a paused workflow.
    WorkflowResume {
        #[arg(long)]
        workflow_id: String,
    },
    /// Read-only reattach to an existing canonical site workflow. Never starts another run.
    WorkflowWait {
        #[arg(long)]
        workflow_id: String,
    },
    /// Compare unfinished business runs with Temporal and close only verified terminal runs.
    RunReconcile {
        #[arg(long)]
        database_url: Option<String>,
    },
}

#[derive(serde::Serialize)]
struct SeoPreflightCollectionReport {
    collection_name: String,
    exists: bool,
    fresh: bool,
    projection_complete: bool,
    point_count: i64,
    last_materialized_at: Option<String>,
    last_source_change_at: Option<String>,
    lag_seconds: Option<i64>,
    qdrant_collection_exists: bool,
}

#[derive(serde::Serialize)]
struct SeoPreflightGraphProjectionReport {
    projection_name: String,
    exists: bool,
    fresh: bool,
    projection_complete: bool,
    point_count: i64,
    last_materialized_at: Option<String>,
    last_source_change_at: Option<String>,
    lag_seconds: Option<i64>,
}

#[derive(serde::Serialize)]
struct SeoPreflightReport {
    artifact_id: String,
    status: String,
    context_key: String,
    normalized_profile: String,
    retrieval_capability_required: bool,
    canonical_vector_retrieval_required: bool,
    contextual_raw_chunk_retrieval_required: bool,
    voyage_rerank_required: bool,
    voyage_embeddings_ready: bool,
    voyage_contextualized_ready: bool,
    voyage_rerank_ready: bool,
    qdrant_ready: bool,
    qdrant_collection_contract_ready: bool,
    neo4j_ready: bool,
    graph_query_ready: bool,
    graph_gds_ready: bool,
    graph_projection_contract_ready: bool,
    graph_capability_required: bool,
    neo4j_sync_required: bool,
    graph_query_required: bool,
    graph_gds_required: bool,
    projection_blocked: bool,
    retrieval_block_reason: Option<String>,
    graph_contract_status: String,
    graph_block_reason: Option<String>,
    required_collections: Vec<SeoPreflightCollectionReport>,
    required_graph_projections: Vec<SeoPreflightGraphProjectionReport>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum WorkflowKind {
    ExpertDecomposedExtraction,
    ExpertExtraction,
    ExpertProjection,
    ExpertSemanticSlice,
    FreshnessCheck,
    ProjectionReconcile,
    SeoSiteBuildCanonicalCutover,
    SeoSiteBuildComposableV2,
    TestHitl,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum V2Order {
    SupportFirst,
    AcquisitionFirst,
}

impl V2Order {
    fn durable_plan(self) -> anyhow::Result<SeoDurablePlan> {
        use SeoDurableStep::{LoadVerifiedSupport, SourceAcquisition};
        let ordered = match self {
            Self::SupportFirst => vec![LoadVerifiedSupport, SourceAcquisition],
            Self::AcquisitionFirst => vec![SourceAcquisition, LoadVerifiedSupport],
        };
        SeoDurablePlan::source_acquisition_only(ordered).map_err(Into::into)
    }
}

impl WorkflowKind {
    fn workflow_type(self) -> &'static str {
        match self {
            WorkflowKind::ExpertDecomposedExtraction => "ExpertDecomposedExtractionWorkflow",
            WorkflowKind::ExpertExtraction => "ExpertExtractionWorkflow",
            WorkflowKind::ExpertProjection => "ExpertProjectionWorkflow",
            WorkflowKind::ExpertSemanticSlice => "ExpertSemanticSliceWorkflow",
            WorkflowKind::FreshnessCheck => "FreshnessCheckWorkflow",
            WorkflowKind::ProjectionReconcile => "ProjectionReconcileWorkflow",
            WorkflowKind::SeoSiteBuildCanonicalCutover => "SeoSiteBuildCanonicalCutoverWorkflow",
            WorkflowKind::SeoSiteBuildComposableV2 => "SeoSiteBuildComposableV2Workflow",
            WorkflowKind::TestHitl => "TestHitlWorkflow",
        }
    }

    fn id_prefix(self) -> &'static str {
        match self {
            WorkflowKind::ExpertDecomposedExtraction => "expert-decomposed-extraction",
            WorkflowKind::ExpertExtraction => "expert-extraction",
            WorkflowKind::ExpertProjection => "expert-projection",
            WorkflowKind::ExpertSemanticSlice => "expert-semantic-slice",
            WorkflowKind::FreshnessCheck => "freshness-check",
            WorkflowKind::ProjectionReconcile => "projection-reconcile",
            WorkflowKind::SeoSiteBuildCanonicalCutover => "seo-site-build-canonical-cutover",
            WorkflowKind::SeoSiteBuildComposableV2 => "seo-site-build-composable-v2",
            WorkflowKind::TestHitl => "test-hitl",
        }
    }

    fn requires_uuid_run_id(self) -> bool {
        matches!(
            self,
            WorkflowKind::ExpertDecomposedExtraction
                | WorkflowKind::ExpertExtraction
                | WorkflowKind::ExpertProjection
                | WorkflowKind::ExpertSemanticSlice
                | WorkflowKind::SeoSiteBuildCanonicalCutover
                | WorkflowKind::SeoSiteBuildComposableV2
        )
    }
}

#[cfg(test)]
mod canonical_workflow_cli_tests {
    use super::*;

    #[test]
    fn starter_exposes_only_the_canonical_seo_workflow() {
        let valid = Cli::try_parse_from([
            "temporal_starter",
            "start",
            "--workflow",
            "seo-site-build-canonical-cutover",
        ])
        .expect("canonical workflow remains startable");
        assert!(matches!(
            valid.command,
            Command::Start {
                workflow: WorkflowKind::SeoSiteBuildCanonicalCutover,
                ..
            }
        ));

        for legacy in ["seo-site-build", "content-generation"] {
            assert!(
                Cli::try_parse_from(["temporal_starter", "start", "--workflow", legacy])
                    .is_err(),
                "retired workflow must not be startable: {legacy}"
            );
        }
    }

    #[test]
    fn rebuild_dispatch_always_starts_the_canonical_workflow() {
        assert!(matches!(
            Cli::try_parse_from(["temporal_starter", "rebuild-dispatch"])
                .expect("canonical rebuild dispatch")
                .command,
            Command::RebuildDispatch { .. }
        ));
        assert!(
            Cli::try_parse_from([
                "temporal_starter",
                "rebuild-dispatch",
                "--workflow",
                "seo-site-build-legacy-compat",
            ])
            .is_err(),
            "rebuild dispatch must not select a retired workflow"
        );
    }

    #[test]
    fn v2_is_explicitly_selectable_with_two_distinct_immutable_recipes() {
        let cli = Cli::try_parse_from([
            "temporal_starter",
            "start",
            "--workflow",
            "seo-site-build-composable-v2",
            "--run-mode",
            "crawl_only",
            "--v2-order",
            "acquisition-first",
        ])
        .expect("V2 has an explicit opt-in entrypoint");
        assert!(matches!(
            cli.command,
            Command::Start {
                workflow: WorkflowKind::SeoSiteBuildComposableV2,
                v2_order: Some(V2Order::AcquisitionFirst),
                ..
            }
        ));
        let a = V2Order::SupportFirst.durable_plan().unwrap();
        let b = V2Order::AcquisitionFirst.durable_plan().unwrap();
        assert_ne!(a.ordered_steps(), b.ordered_steps());
        assert_eq!(a.ordered_steps()[0], SeoDurableStep::LoadVerifiedSupport);
        assert_eq!(b.ordered_steps()[0], SeoDurableStep::SourceAcquisition);
    }
}
