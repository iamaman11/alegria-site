use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use infrastructure::adapters::temporalio_sdk_adapter::{
    connect_client, RawValue, UntypedSignal, UntypedWorkflow, WorkflowGetResultOptions,
    WorkflowSignalOptions, WorkflowStartOptions,
};
use infrastructure::adapters::{
    neo4rs_adapter, qdrant_client_adapter, seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository,
    sqlx_adapter::connect_pg, sqlx_seo_adapter, voyage_api_adapter::VoyageClient,
};
use primitives::{hash::blake3_hex, qdrant_point_id::qdrant_point_id_v1};
use seo_application::execution::normalize_run_mode;
use seo_application::registration::register_site_build_input;
use seo_domain::identity;
use seo_ports::SeoSiteBuildRegistrationRequest;
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
        #[arg(long)]
        query_batch_key: Option<String>,
        #[arg(long, default_value = "publish_with_hitl")]
        run_mode: String,
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
    },
    /// Consume queued rebuild backlog rows and start scoped rebuild runs.
    RebuildDispatch {
        #[arg(long)]
        database_url: Option<String>,
        #[arg(long, default_value_t = 10)]
        limit: i64,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long, value_enum, default_value_t = RebuildDispatchWorkflowKind::SeoSiteBuild)]
        workflow: RebuildDispatchWorkflowKind,
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
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum WorkflowKind {
    ContentGeneration,
    ExpertDecomposedExtraction,
    ExpertExtraction,
    ExpertProjection,
    ExpertSemanticSlice,
    FreshnessCheck,
    ProjectionReconcile,
    SeoSiteBuildCanonicalCutover,
    SeoSiteBuild,
    TestHitl,
}

impl WorkflowKind {
    fn workflow_type(self) -> &'static str {
        match self {
            WorkflowKind::ContentGeneration => "ContentGenerationWorkflow",
            WorkflowKind::ExpertDecomposedExtraction => "ExpertDecomposedExtractionWorkflow",
            WorkflowKind::ExpertExtraction => "ExpertExtractionWorkflow",
            WorkflowKind::ExpertProjection => "ExpertProjectionWorkflow",
            WorkflowKind::ExpertSemanticSlice => "ExpertSemanticSliceWorkflow",
            WorkflowKind::FreshnessCheck => "FreshnessCheckWorkflow",
            WorkflowKind::ProjectionReconcile => "ProjectionReconcileWorkflow",
            WorkflowKind::SeoSiteBuildCanonicalCutover => "SeoSiteBuildCanonicalCutoverWorkflow",
            WorkflowKind::SeoSiteBuild => "SeoSiteBuildWorkflow",
            WorkflowKind::TestHitl => "TestHitlWorkflow",
        }
    }

    fn id_prefix(self) -> &'static str {
        match self {
            WorkflowKind::ContentGeneration => "content-gen",
            WorkflowKind::ExpertDecomposedExtraction => "expert-decomposed-extraction",
            WorkflowKind::ExpertExtraction => "expert-extraction",
            WorkflowKind::ExpertProjection => "expert-projection",
            WorkflowKind::ExpertSemanticSlice => "expert-semantic-slice",
            WorkflowKind::FreshnessCheck => "freshness-check",
            WorkflowKind::ProjectionReconcile => "projection-reconcile",
            WorkflowKind::SeoSiteBuildCanonicalCutover => "seo-site-build-canonical-cutover",
            WorkflowKind::SeoSiteBuild => "seo-site-build",
            WorkflowKind::TestHitl => "test-hitl",
        }
    }

    fn requires_uuid_run_id(self) -> bool {
        matches!(
            self,
            WorkflowKind::ContentGeneration
                | WorkflowKind::ExpertDecomposedExtraction
                | WorkflowKind::ExpertExtraction
                | WorkflowKind::ExpertProjection
                | WorkflowKind::ExpertSemanticSlice
                | WorkflowKind::SeoSiteBuildCanonicalCutover
                | WorkflowKind::SeoSiteBuild
        )
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum RebuildDispatchWorkflowKind {
    ExpertExtraction,
    SeoSiteBuild,
}

impl RebuildDispatchWorkflowKind {
    fn workflow_type(self) -> &'static str {
        match self {
            RebuildDispatchWorkflowKind::ExpertExtraction => "ExpertExtractionWorkflow",
            RebuildDispatchWorkflowKind::SeoSiteBuild => "SeoSiteBuildWorkflow",
        }
    }

    fn run_mode(self) -> &'static str {
        match self {
            RebuildDispatchWorkflowKind::ExpertExtraction => "draft_only",
            RebuildDispatchWorkflowKind::SeoSiteBuild => "publish_with_hitl",
        }
    }
}

fn empty_payload() -> RawValue {
    RawValue::default()
}

fn default_database_url() -> String {
    env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres_password@localhost:5433/alegria".into())
}

fn write_report(report_path: &str, payload: &Value) -> Result<PathBuf> {
    let out = PathBuf::from(report_path);
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create report dir failed: {}", parent.display()))?;
    }
    fs::write(&out, serde_json::to_vec_pretty(payload)?)
        .with_context(|| format!("write report failed: {}", out.display()))?;
    Ok(out)
}

fn parse_reason_json(reason: &str) -> Value {
    serde_json::from_str::<Value>(reason).unwrap_or_else(|_| json!({ "raw_reason": reason }))
}

fn fingerprint_vector(text: &str) -> Vec<f32> {
    let digest = blake3_hex(text.as_bytes());
    let mut vector = Vec::with_capacity(16);
    for chunk in digest.as_bytes().chunks(2).take(16) {
        let Ok(hex) = std::str::from_utf8(chunk) else {
            continue;
        };
        let value = u8::from_str_radix(hex, 16).unwrap_or(0);
        vector.push((value as f32 / 127.5) - 1.0);
    }
    if vector.is_empty() {
        vector.push(0.0);
    }
    vector
}

async fn persist_seo_site_build_input(
    database_url: Option<String>,
    run_id: &str,
    context_key: Option<String>,
    market: String,
    locale: String,
    country_code: String,
    visa_type: String,
    visa_subtype: Option<String>,
    applicant_profile: String,
    citizenship_code: String,
    bootstrap_context: bool,
    queries: Vec<String>,
    query_batch_key: Option<String>,
    run_mode: String,
) -> Result<()> {
    let pool = connect_pg(&database_url.unwrap_or_else(default_database_url)).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    register_site_build_input(
        &repo,
        &SeoSiteBuildRegistrationRequest {
            run_id: run_id.to_string(),
            context_key,
            market,
            locale,
            country_code,
            visa_type,
            visa_subtype,
            applicant_profile,
            citizenship_code,
            bootstrap_context,
            queries,
            query_batch_key,
            run_mode: Some(normalize_run_mode(&run_mode).to_string()),
        },
    )
    .await
    .map_err(|err| anyhow::anyhow!("{err}"))?;
    Ok(())
}

fn env_set(name: &str) -> bool {
    env::var(name)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn llm_configured() -> bool {
    env_set("OPENAI_API_KEY")
        || env_set("ANTHROPIC_API_KEY")
        || env_set("GEMINI_API_KEY")
        || env_set("GOOGLE_API_KEY")
        || env_set("SEO_LLM_LOCAL_ENDPOINT")
}

async fn probe_qdrant(qdrant_url: &str) -> Result<bool> {
    let client = qdrant_client_adapter::connect_qdrant(qdrant_url).await?;
    let exists = client.collection_exists("content_chunks").await?;
    Ok(exists)
}

async fn probe_neo4j(uri: &str, user: &str, password: &str) -> Result<()> {
    let graph = neo4rs_adapter::connect_neo4j(uri, user, password).await?;
    neo4rs_adapter::run_cypher(&graph, "RETURN 1 AS ok").await?;
    Ok(())
}

async fn run_seo_preflight(
    database_url: Option<String>,
    context_key: Option<String>,
    market: String,
    locale: String,
    country_code: String,
    visa_type: String,
    visa_subtype: Option<String>,
    applicant_profile: String,
    citizenship_code: String,
    bootstrap_context: bool,
    strict_projections: bool,
    projection_max_lag_ms: i64,
    output_dir: String,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = tokio::time::timeout(Duration::from_secs(10), connect_pg(&database_url))
        .await
        .map_err(|_| anyhow::anyhow!("database_connect timed out after 10s"))?
        .context("database_connect failed")?;
    sqlx_seo_adapter::ensure_seo_runtime_registries(&pool)
        .await
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    let scope = identity::derive_scope(
        &market,
        &locale,
        &country_code,
        &visa_type,
        &applicant_profile,
    )
    .map_err(|err| anyhow::anyhow!("{err}"))?;
    let normalized_profile =
        sqlx_seo_adapter::validate_applicant_profile_reference(&pool, &scope.applicant_profile)
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;
    println!("OK database_connect");
    println!("OK scope market={market} locale={locale} country={country_code} visa_type={visa_type} applicant_profile={normalized_profile}");

    let truth_identity = identity::derive_truth_identity(
        &country_code,
        &visa_type,
        visa_subtype.as_deref(),
        &citizenship_code,
    )
    .map_err(|err| anyhow::anyhow!("{err}"))?;
    let resolved_context_key = if bootstrap_context {
        let report = sqlx_seo_adapter::bootstrap_seo_scope(
            &pool,
            context_key.as_deref(),
            &truth_identity.country_code,
            &truth_identity.visa_family,
            if truth_identity.visa_subtype.is_empty() {
                None
            } else {
                Some(truth_identity.visa_subtype.as_str())
            },
            &truth_identity.citizenship_code,
        )
        .await
        .context("bootstrap_seo_scope failed")?;
        println!(
            "OK seo_scope context_key={} created_context={} seeded_registries={}",
            report.context_key, report.created_context, report.seeded_registry_count
        );
        report.context_key
    } else {
        context_key.unwrap_or_else(|| truth_identity.context_key.clone())
    };

    let context_row = sqlx::query(
        "SELECT count(*)::bigint AS count FROM kb.visa_contexts WHERE context_key = $1 AND status = 'active'",
    )
    .bind(&resolved_context_key)
    .fetch_one(&pool)
    .await
    .context("active context lookup failed")?;
    let context_count: i64 = sqlx::Row::get(&context_row, "count");
    if context_count == 0 {
        println!("FAIL active_context context_key={resolved_context_key}");
        return Ok(2);
    }
    println!("OK active_context context_key={resolved_context_key}");

    let page_type_row = sqlx::query(
        "SELECT count(*)::bigint AS count FROM site.registry_page_types WHERE status = 'active'",
    )
    .fetch_one(&pool)
    .await
    .context("page type registry count failed")?;
    let page_type_count: i64 = sqlx::Row::get(&page_type_row, "count");
    if page_type_count == 0 {
        println!("FAIL registry_page_types active_count=0");
        return Ok(2);
    }
    println!("OK registry_page_types active_count={page_type_count}");

    let page_node_row = sqlx::query("SELECT count(*)::bigint AS count FROM site.page_nodes")
        .fetch_one(&pool)
        .await
        .context("page node count failed")?;
    let page_node_count: i64 = sqlx::Row::get(&page_node_row, "count");
    let navigation_item_row =
        sqlx::query("SELECT count(*)::bigint AS count FROM site.navigation_items")
            .fetch_one(&pool)
            .await
            .context("navigation item count failed")?;
    let navigation_item_count: i64 = sqlx::Row::get(&navigation_item_row, "count");
    println!(
        "OK global_site_model page_nodes={} navigation_items={}",
        page_node_count, navigation_item_count
    );

    let verified_rule_row = sqlx::query(
        "SELECT count(*)::bigint AS count FROM verified.rule_instances WHERE context_key = $1 AND status = 'verified'",
    )
    .bind(&resolved_context_key)
    .fetch_one(&pool)
    .await
    .context("verified rule count failed")?;
    let verified_rule_count: i64 = sqlx::Row::get(&verified_rule_row, "count");
    let pending_rule_row = sqlx::query(
        "SELECT count(*)::bigint AS count FROM verified.rule_instances WHERE context_key = $1 AND status = 'pending'",
    )
    .bind(&resolved_context_key)
    .fetch_one(&pool)
    .await
    .context("pending rule count failed")?;
    let pending_rule_count: i64 = sqlx::Row::get(&pending_rule_row, "count");
    println!(
        "OK knowledge_state verified_rules={} pending_review_rules={}",
        verified_rule_count, pending_rule_count
    );

    let qdrant_point_row = sqlx::query("SELECT count(*)::bigint AS count FROM kb.qdrant_points")
        .fetch_one(&pool)
        .await
        .context("qdrant point ledger count failed")?;
    let qdrant_point_count: i64 = sqlx::Row::get(&qdrant_point_row, "count");
    println!("OK qdrant_point_ledger points={qdrant_point_count}");

    let mut projection_blocked = false;
    let projection_statuses = sqlx_seo_adapter::read_projection_sync_status(&pool)
        .await
        .context("projection sync status failed")?;
    for status in projection_statuses {
        let open_events = status.open_event_count();
        let blocked_events = status.blocking_event_count();
        if blocked_events > 0 {
            projection_blocked = true;
        }
        let state = if status.failed_events > 0 {
            "FAIL"
        } else if open_events > 0 && status.max_open_lag_ms > projection_max_lag_ms {
            "WARN"
        } else {
            "OK"
        };
        println!(
            "{} projection_status target={} pending={} processing={} failed={} done={} max_open_lag_ms={}",
            state,
            status.target_system,
            status.pending_events,
            status.processing_events,
            status.failed_events,
            status.done_events,
            status.max_open_lag_ms
        );
        if let Some(aggregate_key) = status.oldest_open_aggregate_key.as_deref() {
            println!(
                "INFO projection_oldest_open target={} event_id={} aggregate_key={} event_type={}",
                status.target_system,
                status.oldest_open_event_id.as_deref().unwrap_or(""),
                aggregate_key,
                status.oldest_open_event_type.as_deref().unwrap_or("")
            );
        }
        if let Some(aggregate_key) = status.latest_failed_aggregate_key.as_deref() {
            println!(
                "INFO projection_latest_failed target={} aggregate_key={} event_type={} error={}",
                status.target_system,
                aggregate_key,
                status.latest_failed_event_type.as_deref().unwrap_or(""),
                status.latest_failed_error.as_deref().unwrap_or("")
            );
        }
    }
    if strict_projections && projection_blocked {
        println!("FAIL projection_barrier strict=true blocked_events_present=true");
        return Ok(2);
    }
    if projection_blocked {
        println!("WARN projection_barrier strict=false blocked_events_present=true");
    } else {
        println!("OK projection_barrier all_targets_drained=true");
    }

    if env_set("DATAFORSEO_LOGIN") && env_set("DATAFORSEO_PASSWORD") {
        println!("OK dataforseo_credentials");
    } else {
        println!(
            "WARN dataforseo_credentials missing; live SERP discovery will not populate crawl queue"
        );
    }
    if env_set("VOYAGE_API_KEY") {
        println!("OK voyage_credentials");
    } else {
        println!("WARN voyage_credentials missing; semantic retrieval will be limited");
    }
    let qdrant_url = env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());
    match tokio::time::timeout(Duration::from_secs(3), probe_qdrant(&qdrant_url)).await {
        Ok(Ok(content_chunks_exists)) => {
            println!(
                "OK qdrant_connect url={} content_chunks_collection={}",
                qdrant_url, content_chunks_exists
            );
        }
        Ok(Err(err)) => {
            println!("WARN qdrant_connect url={} error={}", qdrant_url, err);
        }
        Err(_) => {
            println!("WARN qdrant_connect url={} error=timeout", qdrant_url);
        }
    }
    let neo4j_uri = env::var("NEO4J_URI").unwrap_or_else(|_| "127.0.0.1:7687".to_string());
    let neo4j_user = env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
    let neo4j_password =
        env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "neo4j_password".to_string());
    match tokio::time::timeout(
        Duration::from_secs(3),
        probe_neo4j(&neo4j_uri, &neo4j_user, &neo4j_password),
    )
    .await
    {
        Ok(Ok(())) => {
            println!("OK neo4j_connect uri={neo4j_uri}");
        }
        Ok(Err(err)) => {
            println!("WARN neo4j_connect uri={} error={}", neo4j_uri, err);
        }
        Err(_) => {
            println!("WARN neo4j_connect uri={} error=timeout", neo4j_uri);
        }
    }
    if llm_configured() {
        println!("OK llm_provider");
    } else {
        println!("WARN llm_provider missing; deterministic fallback cannot produce production-quality pages");
    }

    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("static output dir is not writable: {output_dir}"))?;
    println!("OK static_output_dir path={output_dir}");
    Ok(0)
}

async fn run_rebuild_dispatch(
    client: &infrastructure::adapters::temporalio_sdk_adapter::Client,
    task_queue: &str,
    database_url: Option<String>,
    limit: i64,
    dry_run: bool,
    workflow: RebuildDispatchWorkflowKind,
    report_json: Option<String>,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    let rows = sqlx::query(
        r#"
        SELECT
            b.rebuild_request_key,
            b.page_node_key,
            b.trigger_type,
            b.priority,
            b.reason,
            b.created_at::text AS created_at,
            p.scope_signature,
            k.seed_keyword,
            s.market,
            s.locale,
            s.country_code,
            s.visa_type,
            s.applicant_profile,
            s.context_key,
            ctx.visa_subtype,
            ctx.citizenship_code
        FROM monitoring.seo_rebuild_backlog b
        LEFT JOIN site.page_nodes p ON p.page_node_key = b.page_node_key
        LEFT JOIN site.keyword_clusters k ON k.cluster_key = p.keyword_cluster_key
        LEFT JOIN site.site_scopes s ON s.scope_signature = p.scope_signature
        LEFT JOIN kb.visa_contexts ctx ON ctx.context_key = s.context_key
        WHERE b.status = 'queued'
        ORDER BY b.priority ASC, b.created_at ASC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(&pool)
    .await?;

    let mut dispatched = Vec::new();
    let mut blocked = Vec::new();

    for row in rows {
        let rebuild_request_key: String = row.get("rebuild_request_key");
        let page_node_key = row.get::<Option<String>, _>("page_node_key");
        let trigger_type: String = row.get("trigger_type");
        let priority: i32 = row.get("priority");
        let created_at: String = row.get("created_at");
        let reason_text: String = row.get("reason");
        let reason_value = parse_reason_json(&reason_text);

        let scope_signature = row.get::<Option<String>, _>("scope_signature");
        let seed_keyword = row.get::<Option<String>, _>("seed_keyword");
        let market = row.get::<Option<String>, _>("market");
        let locale = row.get::<Option<String>, _>("locale");
        let country_code = row.get::<Option<String>, _>("country_code");
        let visa_type = row.get::<Option<String>, _>("visa_type");
        let applicant_profile = row.get::<Option<String>, _>("applicant_profile");
        let context_key = row.get::<Option<String>, _>("context_key");
        let visa_subtype = row.get::<Option<String>, _>("visa_subtype");
        let citizenship_code = row.get::<Option<String>, _>("citizenship_code");

        let mut missing = Vec::new();
        if page_node_key.as_deref().unwrap_or_default().is_empty() {
            missing.push("page_node_key");
        }
        if scope_signature.as_deref().unwrap_or_default().is_empty() {
            missing.push("scope_signature");
        }
        if seed_keyword.as_deref().unwrap_or_default().is_empty() {
            missing.push("seed_keyword");
        }
        if market.as_deref().unwrap_or_default().is_empty() {
            missing.push("market");
        }
        if locale.as_deref().unwrap_or_default().is_empty() {
            missing.push("locale");
        }
        if country_code.as_deref().unwrap_or_default().is_empty() {
            missing.push("country_code");
        }
        if visa_type.as_deref().unwrap_or_default().is_empty() {
            missing.push("visa_type");
        }
        if applicant_profile.as_deref().unwrap_or_default().is_empty() {
            missing.push("applicant_profile");
        }
        if context_key.as_deref().unwrap_or_default().is_empty() {
            missing.push("context_key");
        }
        if citizenship_code.as_deref().unwrap_or_default().is_empty() {
            missing.push("citizenship_code");
        }

        if !missing.is_empty() {
            let blocked_reason = json!({
                "status": "blocked",
                "failure_class": "rebuild_dispatch_missing_scope_data",
                "missing_fields": missing,
                "workflow_type": workflow.workflow_type(),
                "reason_package": reason_value,
            });
            if !dry_run {
                sqlx::query(
                    r#"
                    UPDATE monitoring.seo_rebuild_backlog
                    SET status = 'blocked',
                        reason = $2,
                        updated_at = now()
                    WHERE rebuild_request_key = $1
                    "#,
                )
                .bind(&rebuild_request_key)
                .bind(blocked_reason.to_string())
                .execute(&pool)
                .await?;
                if let Some(page_node_key) = page_node_key.as_deref() {
                    sqlx::query(
                        r#"
                        UPDATE site.global_rebuild_plan
                        SET status = 'blocked',
                            reason_payload = $3,
                            updated_at = now()
                        WHERE affected_page_node_key = $1
                          AND trigger_type = $2
                          AND status = 'queued'
                        "#,
                    )
                    .bind(page_node_key)
                    .bind(&trigger_type)
                    .bind(Json(blocked_reason.clone()))
                    .execute(&pool)
                    .await?;
                }
            }
            blocked.push(json!({
                "rebuild_request_key": rebuild_request_key,
                "page_node_key": page_node_key,
                "trigger_type": trigger_type,
                "priority": priority,
                "created_at": created_at,
                "status": if dry_run { "would_block" } else { "blocked" },
                "missing_fields": missing,
            }));
            continue;
        }

        let page_node_key = page_node_key.unwrap_or_default();
        let run_id = Uuid::new_v4().to_string();
        let query_batch_key = format!("rebuild:{}:{}", page_node_key, run_id);
        let dispatch_payload = json!({
            "status": if dry_run { "planned" } else { "running" },
            "workflow_type": workflow.workflow_type(),
            "workflow_id": run_id,
            "run_mode": workflow.run_mode(),
            "seed_keyword": seed_keyword.as_deref().unwrap_or_default(),
            "query_batch_key": query_batch_key,
            "reason_package": reason_value,
        });

        if !dry_run {
            register_site_build_input(
                &repo,
                &SeoSiteBuildRegistrationRequest {
                    run_id: run_id.clone(),
                    context_key: context_key.clone(),
                    market: market.clone().unwrap_or_default(),
                    locale: locale.clone().unwrap_or_default(),
                    country_code: country_code.clone().unwrap_or_default(),
                    visa_type: visa_type.clone().unwrap_or_default(),
                    visa_subtype: visa_subtype.clone().filter(|v| !v.trim().is_empty()),
                    applicant_profile: applicant_profile.clone().unwrap_or_default(),
                    citizenship_code: citizenship_code.clone().unwrap_or_default(),
                    bootstrap_context: false,
                    queries: vec![seed_keyword.clone().unwrap_or_default()],
                    query_batch_key: Some(query_batch_key.clone()),
                    run_mode: Some(workflow.run_mode().to_string()),
                },
            )
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;

            let options = WorkflowStartOptions::new(task_queue.to_string(), run_id.clone()).build();
            client
                .start_workflow(
                    UntypedWorkflow::new(workflow.workflow_type()),
                    empty_payload(),
                    options,
                )
                .await
                .with_context(|| {
                    format!(
                        "failed to start {} for rebuild_request_key={}",
                        workflow.workflow_type(),
                        rebuild_request_key
                    )
                })?;

            sqlx::query(
                r#"
                UPDATE monitoring.seo_rebuild_backlog
                SET status = 'running',
                    reason = $2,
                    updated_at = now()
                WHERE rebuild_request_key = $1
                "#,
            )
            .bind(&rebuild_request_key)
            .bind(dispatch_payload.to_string())
            .execute(&pool)
            .await?;

            sqlx::query(
                r#"
                UPDATE site.global_rebuild_plan
                SET status = 'running',
                    reason_payload = $3,
                    updated_at = now()
                WHERE affected_page_node_key = $1
                  AND trigger_type = $2
                  AND status = 'queued'
                "#,
            )
            .bind(&page_node_key)
            .bind(&trigger_type)
            .bind(Json(dispatch_payload.clone()))
            .execute(&pool)
            .await?;
        }

        dispatched.push(json!({
            "rebuild_request_key": rebuild_request_key,
            "page_node_key": page_node_key,
            "trigger_type": trigger_type,
            "priority": priority,
            "created_at": created_at,
            "workflow_type": workflow.workflow_type(),
            "workflow_id": run_id,
            "run_mode": workflow.run_mode(),
            "query_batch_key": query_batch_key,
            "seed_keyword": seed_keyword,
            "scope_signature": scope_signature,
            "status": if dry_run { "planned" } else { "running" },
        }));
    }

    let payload = json!({
        "status": "ok",
        "dry_run": dry_run,
        "workflow_type": workflow.workflow_type(),
        "dispatched_count": dispatched.len(),
        "blocked_count": blocked.len(),
        "dispatched": dispatched,
        "blocked": blocked,
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    if let Some(report_json) = report_json.as_deref() {
        let out = write_report(report_json, &payload)?;
        eprintln!("report: {}", out.display());
    }
    Ok(0)
}

async fn run_ontology_backfill_plan(
    database_url: Option<String>,
    concept_key: Option<String>,
    limit: i64,
    apply_neo4j: bool,
    apply_qdrant: bool,
    report_json: Option<String>,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let qdrant_url = env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());
    let voyage_api_key = env::var("VOYAGE_API_KEY").ok();
    let voyage_model = env::var("VOYAGE_MODEL").unwrap_or_else(|_| "voyage-3-large".to_string());
    let rows = sqlx::query(
        r#"
        SELECT
            c.concept_key,
            c.concept_type,
            c.status,
            c.reg_version,
            COALESCE(c.label_ru, '') AS label_ru,
            COALESCE(COUNT(a.alias_id), 0)::bigint AS alias_count,
            COUNT(r.rule_instance_id)::bigint AS verified_rule_count
        FROM kb.concepts c
        LEFT JOIN kb.concept_aliases a
          ON a.concept_key = c.concept_key
         AND a.status IN ('active','pending')
        LEFT JOIN verified.rule_instances r
          ON r.concept_key = c.concept_key
         AND r.status = 'verified'
        WHERE ($1::text IS NULL OR c.concept_key = $1)
          AND ($1::text IS NOT NULL OR c.status IN ('approved','active'))
        GROUP BY c.concept_key, c.concept_type, c.status, c.reg_version, c.label_ru, c.updated_at
        ORDER BY c.updated_at DESC, c.concept_key ASC
        LIMIT $2
        "#,
    )
    .bind(concept_key.clone())
    .bind(limit)
    .fetch_all(&pool)
    .await?;

    let mut concept_records = Vec::new();
    for row in rows {
        let concept_key: String = row.get("concept_key");
        let alias_rows = sqlx::query(
            r#"
            SELECT alias_text
            FROM kb.concept_aliases
            WHERE concept_key = $1
              AND status IN ('active','pending')
            ORDER BY confidence DESC NULLS LAST, alias_text ASC
            "#,
        )
        .bind(&concept_key)
        .fetch_all(&pool)
        .await?;
        let aliases = alias_rows
            .into_iter()
            .map(|alias_row| alias_row.get::<String, _>("alias_text"))
            .collect::<Vec<_>>();
        concept_records.push((
            concept_key,
            row.get::<String, _>("concept_type"),
            row.get::<String, _>("status"),
            row.get::<i32, _>("reg_version"),
            row.get::<String, _>("label_ru"),
            row.get::<i64, _>("alias_count"),
            row.get::<i64, _>("verified_rule_count"),
            aliases,
        ));
    }

    let voyage_vectors = if apply_qdrant {
        let texts = concept_records
            .iter()
            .map(
                |(
                    concept_key,
                    concept_type,
                    status,
                    _reg_version,
                    label_ru,
                    _alias_count,
                    _verified_rule_count,
                    aliases,
                )| {
                    format!(
                        "{} {} {} {} {}",
                        concept_key,
                        concept_type,
                        status,
                        label_ru,
                        aliases.join(" ")
                    )
                },
            )
            .collect::<Vec<_>>();
        if let Some(api_key) = voyage_api_key.clone() {
            if texts.is_empty() {
                Vec::new()
            } else {
                VoyageClient::new(api_key, voyage_model.clone())
                    .embed_all(&texts)
                    .await?
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let mut voyage_iter = voyage_vectors.into_iter();
    let qdrant_client = if apply_qdrant {
        Some(qdrant_client_adapter::connect_qdrant(&qdrant_url).await?)
    } else {
        None
    };

    let mut concepts = Vec::new();
    let mut failures = Vec::new();

    for (
        concept_key,
        concept_type,
        status,
        reg_version,
        label_ru,
        alias_count,
        verified_rule_count,
        aliases,
    ) in concept_records
    {
        let mut neo4j_result = json!({ "status": "skipped" });
        if apply_neo4j {
            match infrastructure::adapters::neo4j_materialization_adapter::materialize_concept(
                &concept_key,
            )
            .await
            {
                Ok(()) => {
                    neo4j_result = json!({ "status": "materialized" });
                }
                Err(err) => {
                    neo4j_result = json!({
                        "status": "failed",
                        "error": err.to_string(),
                    });
                    failures.push(json!({
                        "concept_key": concept_key,
                        "failure_class": "neo4j_materialization_failed",
                        "error": err.to_string(),
                    }));
                }
            }
        }

        let mut qdrant_result = json!({ "status": "skipped" });
        if let Some(client) = qdrant_client.as_ref() {
            let embedding_text = format!(
                "{} {} {} {} {}",
                concept_key,
                concept_type,
                status,
                label_ru,
                aliases.join(" ")
            )
            .trim()
            .to_string();
            let (vector, embedding_model, embedding_version) =
                if let Some(vector) = voyage_iter.next() {
                    (vector, voyage_model.as_str(), "ontology_voyage@1")
                } else {
                    (
                        fingerprint_vector(&embedding_text),
                        "deterministic-fingerprint",
                        "ontology_fingerprint@1",
                    )
                };
            let point_id = qdrant_point_id_v1("ontology", "concept", &concept_key);
            let mut payload = BTreeMap::new();
            payload.insert("concept_key".to_string(), concept_key.clone());
            payload.insert("concept_type".to_string(), concept_type.clone());
            payload.insert("status".to_string(), status.clone());
            payload.insert("label_ru".to_string(), label_ru.clone());
            payload.insert("aliases".to_string(), aliases.join(" | "));
            payload.insert("reg_version".to_string(), reg_version.to_string());
            payload.insert(
                "embedding_version".to_string(),
                embedding_version.to_string(),
            );

            match async {
                qdrant_client_adapter::ensure_default_dense_collection(
                    client,
                    "ontology",
                    vector.len() as u64,
                )
                .await?;
                qdrant_client_adapter::upsert_embedding_points(
                    client,
                    "ontology",
                    vec![qdrant_client_adapter::DenseEmbeddingPoint {
                        point_id: point_id.clone(),
                        vector,
                        payload,
                    }],
                )
                .await?;
                sqlx::query(
                    r#"
                    INSERT INTO kb.qdrant_points
                        (point_id, entity_type, entity_key, collection_name, embedding_model, embedding_version)
                    VALUES ($1, 'concept', $2, 'ontology', $3, $4)
                    ON CONFLICT (entity_type, entity_key, collection_name) DO UPDATE
                    SET point_id = EXCLUDED.point_id,
                        embedding_model = EXCLUDED.embedding_model,
                        embedding_version = EXCLUDED.embedding_version,
                        updated_at = now()
                    "#,
                )
                .bind(&point_id)
                .bind(&concept_key)
                .bind(embedding_model)
                .bind(embedding_version)
                .execute(&pool)
                .await?;
                Result::<()>::Ok(())
            }
            .await
            {
                Ok(()) => {
                    qdrant_result = json!({
                        "status": "materialized",
                        "collection_name": "ontology",
                        "point_id": point_id,
                        "embedding_model": embedding_model,
                        "embedding_version": embedding_version,
                    });
                }
                Err(err) => {
                    qdrant_result = json!({
                        "status": "failed",
                        "error": err.to_string(),
                    });
                    failures.push(json!({
                        "concept_key": concept_key,
                        "failure_class": "qdrant_materialization_failed",
                        "error": err.to_string(),
                    }));
                }
            }
        }

        concepts.push(json!({
            "concept_key": concept_key,
            "concept_type": concept_type,
            "status": status,
            "reg_version": reg_version,
            "label_ru": label_ru,
            "alias_count": alias_count,
            "aliases": aliases,
            "verified_rule_count": verified_rule_count,
            "planned_actions": [
                "graph_materialize",
                "retrieval_reindex"
            ],
            "neo4j": neo4j_result,
            "qdrant": qdrant_result,
        }));
    }

    let payload = json!({
        "status": if failures.is_empty() { "ok" } else { "partial_failure" },
        "apply_neo4j": apply_neo4j,
        "apply_qdrant": apply_qdrant,
        "concept_count": concepts.len(),
        "concepts": concepts,
        "failures": failures,
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    if let Some(report_json) = report_json.as_deref() {
        let out = write_report(report_json, &payload)?;
        eprintln!("report: {}", out.display());
    }
    Ok(if failures.is_empty() { 0 } else { 2 })
}

#[tokio::main]
async fn main() -> Result<()> {
    let Cli {
        temporal_url,
        namespace,
        task_queue,
        command,
    } = Cli::parse();
    let command = match command {
        Command::SeoPreflight {
            database_url,
            context_key,
            market,
            locale,
            country_code,
            visa_type,
            visa_subtype,
            applicant_profile,
            citizenship_code,
            bootstrap_context,
            strict_projections,
            projection_max_lag_ms,
            output_dir,
        } => {
            let code = run_seo_preflight(
                database_url,
                context_key,
                market,
                locale,
                country_code,
                visa_type,
                visa_subtype,
                applicant_profile,
                citizenship_code,
                bootstrap_context,
                strict_projections,
                projection_max_lag_ms,
                output_dir,
            )
            .await?;
            std::process::exit(code);
        }
        other => other,
    };
    let command = match command {
        Command::OntologyBackfillPlan {
            database_url,
            concept_key,
            limit,
            apply_neo4j,
            apply_qdrant,
            report_json,
        } => {
            let code = run_ontology_backfill_plan(
                database_url,
                concept_key,
                limit,
                apply_neo4j,
                apply_qdrant,
                report_json,
            )
            .await?;
            std::process::exit(code);
        }
        other => other,
    };

    let identity = format!("alegria-temporal-starter@{}", std::process::id());
    let client = connect_client(&temporal_url, identity, &namespace)
        .await
        .context("failed to connect to Temporal")?;

    match command {
        Command::Ping => {
            println!(
                "ok temporal_url={} namespace={} task_queue={}",
                temporal_url, namespace, task_queue
            );
        }
        Command::Start {
            workflow,
            workflow_id,
            database_url,
            context_key,
            market,
            locale,
            country_code,
            visa_type,
            visa_subtype,
            applicant_profile,
            citizenship_code,
            bootstrap_context,
            queries,
            query_batch_key,
            run_mode,
        } => {
            if workflow == WorkflowKind::ContentGeneration {
                anyhow::bail!(
                    "ContentGenerationWorkflow is legacy-only. Use SeoSiteBuildWorkflow for production SEO generation."
                );
            }
            let wf_id = match workflow_id {
                Some(v) => v,
                None if workflow.requires_uuid_run_id() => {
                    // For fact/content workflows workflow_id is treated as run_id in DB layer.
                    // It must be a UUID string to avoid non-deterministic retry loops.
                    Uuid::new_v4().to_string()
                }
                None => format!("{}-{}", workflow.id_prefix(), Uuid::new_v4()),
            };
            if matches!(
                workflow,
                WorkflowKind::SeoSiteBuild
                    | WorkflowKind::SeoSiteBuildCanonicalCutover
                    | WorkflowKind::ExpertDecomposedExtraction
                    | WorkflowKind::ExpertExtraction
                    | WorkflowKind::ExpertProjection
                    | WorkflowKind::ExpertSemanticSlice
            ) {
                persist_seo_site_build_input(
                    database_url,
                    &wf_id,
                    context_key,
                    market,
                    locale,
                    country_code,
                    visa_type,
                    visa_subtype,
                    applicant_profile,
                    citizenship_code,
                    bootstrap_context,
                    queries,
                    query_batch_key,
                    run_mode,
                )
                .await?;
            }
            let options = WorkflowStartOptions::new(task_queue, wf_id.clone()).build();
            let handle = client
                .start_workflow(
                    UntypedWorkflow::new(workflow.workflow_type()),
                    empty_payload(),
                    options,
                )
                .await
                .with_context(|| format!("failed to start {}", workflow.workflow_type()))?;

            println!(
                "started workflow_type={} workflow_id={} run_id={:?}",
                workflow.workflow_type(),
                handle.info().workflow_id,
                handle.info().run_id
            );
        }
        Command::SeoPreflight { .. } => {
            unreachable!("SeoPreflight is handled before Temporal connect")
        }
        Command::RebuildDispatch {
            database_url,
            limit,
            dry_run,
            workflow,
            report_json,
        } => {
            let code = run_rebuild_dispatch(
                &client,
                &task_queue,
                database_url,
                limit,
                dry_run,
                workflow,
                report_json,
            )
            .await?;
            std::process::exit(code);
        }
        Command::OntologyBackfillPlan { .. } => {
            unreachable!("OntologyBackfillPlan is handled before Temporal connect")
        }
        Command::DemoHitl {
            workflow_id,
            wait_before_resume_ms,
        } => {
            let wf_id = workflow_id.unwrap_or_else(|| format!("test-hitl-{}", Uuid::new_v4()));
            let options = WorkflowStartOptions::new(task_queue, wf_id.clone()).build();
            let handle = client
                .start_workflow(
                    UntypedWorkflow::new("TestHitlWorkflow"),
                    empty_payload(),
                    options,
                )
                .await
                .context("failed to start TestHitlWorkflow")?;

            tokio::time::sleep(Duration::from_millis(wait_before_resume_ms)).await;
            handle
                .signal(
                    UntypedSignal::<UntypedWorkflow>::new("resume"),
                    empty_payload(),
                    WorkflowSignalOptions::default(),
                )
                .await
                .context("failed to send resume signal")?;

            let result_raw = handle
                .get_result(WorkflowGetResultOptions::default())
                .await
                .context("failed to get workflow result")?;
            println!(
                "demo_hitl_ok workflow_id={} run_id={:?} result_payloads={}",
                handle.info().workflow_id,
                handle.info().run_id,
                result_raw.payloads.len()
            );
        }
        Command::WorkflowResume { workflow_id } => {
            let handle = client.get_workflow_handle::<UntypedWorkflow>(workflow_id.clone());
            handle
                .signal(
                    UntypedSignal::<UntypedWorkflow>::new("resume"),
                    empty_payload(),
                    WorkflowSignalOptions::default(),
                )
                .await
                .with_context(|| format!("failed to send resume signal to {workflow_id}"))?;
            println!("resume_sent workflow_id={workflow_id}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_applicant_profile() {
        let profile = identity::normalize_applicant_profile(" Standard ").unwrap();
        assert_eq!(profile, "standard");
    }

    #[test]
    fn rejects_unknown_applicant_profile() {
        let err = identity::normalize_applicant_profile("default-applicant")
            .expect_err("unknown profile must fail");
        let message = err.to_string();
        assert!(message.contains("unsupported applicant_profile"));
    }

    #[test]
    fn applicant_profile_does_not_change_truth_identity_without_explicit_subtype() {
        let standard = identity::derive_truth_identity("ES", "tourist", None, "BY").unwrap();
        let minor = identity::derive_truth_identity("ES", "tourist", None, "BY").unwrap();
        assert_eq!(standard.context_key, minor.context_key);
    }

    #[test]
    fn explicit_regulatory_subtype_changes_truth_identity() {
        let base = identity::derive_truth_identity("ES", "tourist", None, "BY").unwrap();
        let subtype =
            identity::derive_truth_identity("ES", "tourist", Some("priority_track"), "BY").unwrap();
        assert_ne!(base.context_key, subtype.context_key);
    }

    #[test]
    fn locale_and_profile_change_scope_not_truth_identity() {
        let truth = identity::derive_truth_identity("ES", "tourist", None, "BY").unwrap();
        let scope_ru =
            identity::derive_scope("alegria-site", "ru-RU", "ES", "tourist", "standard").unwrap();
        let scope_en =
            identity::derive_scope("alegria-site", "en", "ES", "tourist", "minor").unwrap();
        assert_eq!(
            truth.context_key,
            identity::derive_truth_identity("ES", "tourist", None, "BY")
                .unwrap()
                .context_key
        );
        assert_ne!(scope_ru.scope_signature, scope_en.scope_signature);
    }
}
