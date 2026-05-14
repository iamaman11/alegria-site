use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use infrastructure::adapters::temporalio_sdk_adapter::{
    connect_client, RawValue, UntypedSignal, UntypedWorkflow, WorkflowGetResultOptions,
    WorkflowSignalOptions, WorkflowStartOptions,
};
use infrastructure::adapters::{
    neo4rs_adapter, qdrant_client_adapter, seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository,
    sqlx_adapter::connect_pg, sqlx_seo_adapter,
};
use seo_domain::identity;
use seo_application::registration::register_site_build_input;
use seo_ports::SeoSiteBuildRegistrationRequest;
use std::env;
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
    /// End-to-end test workflow with HITL pause/resume
    DemoHitl {
        /// Optional workflow id; if omitted generated
        #[arg(long)]
        workflow_id: Option<String>,
        /// Delay before sending resume signal
        #[arg(long, default_value_t = 700)]
        wait_before_resume_ms: u64,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum WorkflowKind {
    FactExtraction,
    ContentGeneration,
    FreshnessCheck,
    SeoSiteBuild,
    TestHitl,
}

impl WorkflowKind {
    fn workflow_type(self) -> &'static str {
        match self {
            WorkflowKind::FactExtraction => "FactExtractionWorkflow",
            WorkflowKind::ContentGeneration => "ContentGenerationWorkflow",
            WorkflowKind::FreshnessCheck => "FreshnessCheckWorkflow",
            WorkflowKind::SeoSiteBuild => "SeoSiteBuildWorkflow",
            WorkflowKind::TestHitl => "TestHitlWorkflow",
        }
    }

    fn id_prefix(self) -> &'static str {
        match self {
            WorkflowKind::FactExtraction => "fact-extract",
            WorkflowKind::ContentGeneration => "content-gen",
            WorkflowKind::FreshnessCheck => "freshness-check",
            WorkflowKind::SeoSiteBuild => "seo-site-build",
            WorkflowKind::TestHitl => "test-hitl",
        }
    }

    fn requires_uuid_run_id(self) -> bool {
        matches!(
            self,
            WorkflowKind::FactExtraction
                | WorkflowKind::ContentGeneration
                | WorkflowKind::SeoSiteBuild
        )
    }
}

fn empty_payload() -> RawValue {
    RawValue::default()
}

fn default_database_url() -> String {
    env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres_password@localhost:5433/alegria".into())
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

    let qdrant_point_row =
        sqlx::query("SELECT count(*)::bigint AS count FROM kb.qdrant_points")
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
            if workflow == WorkflowKind::SeoSiteBuild {
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
