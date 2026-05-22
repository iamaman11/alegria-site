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

fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("read file failed: {}", path.display()))
}

fn write_report(root: &Path, report_path: &str, payload: &Value) -> Result<PathBuf> {
    let out = root.join(report_path);
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create report dir failed: {}", parent.display()))?;
    }
    fs::write(&out, serde_json::to_vec_pretty(payload)?)
        .with_context(|| format!("write report failed: {}", out.display()))?;
    Ok(out)
}

fn truncate_command_output(text: &str) -> String {
    const LIMIT: usize = 4000;
    if text.len() <= LIMIT {
        text.to_string()
    } else {
        let mut truncated = text
            .char_indices()
            .take_while(|(idx, _)| *idx < LIMIT)
            .map(|(_, ch)| ch)
            .collect::<String>();
        truncated.push_str(&format!(
            "\n...[truncated {} bytes]",
            text.len().saturating_sub(LIMIT)
        ));
        truncated
    }
}

fn run_gate_command(
    root: &Path,
    label: &str,
    command: &str,
    args: &[&str],
    enabled: bool,
) -> Result<Value> {
    let joined = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    if !enabled {
        return Ok(json!({
            "label": label,
            "status": "skipped",
            "command": joined,
        }));
    }

    let output = ProcessCommand::new(command)
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("run gate command failed: {joined}"))?;
    let status = if output.status.success() {
        "ok"
    } else {
        "error"
    };
    Ok(json!({
        "label": label,
        "status": status,
        "command": joined,
        "exit_code": output.status.code(),
        "stdout": truncate_command_output(&String::from_utf8_lossy(&output.stdout)),
        "stderr": truncate_command_output(&String::from_utf8_lossy(&output.stderr)),
    }))
}

fn print_hex(bytes: &[u8]) {
    println!("{}", hex::encode(bytes));
}

fn warning_finding(code: &'static str, message: impl Into<String>) -> Finding {
    Finding {
        level: "warn",
        code,
        message: message.into(),
    }
}

fn error_finding(code: &'static str, message: impl Into<String>) -> Finding {
    Finding {
        level: "error",
        code,
        message: message.into(),
    }
}

fn value_as_i64(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

fn value_as_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn value_as_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn value_as_string_vec(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn value_as_i64_vec(value: &Value, key: &str) -> Vec<i64> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_i64).collect::<Vec<_>>())
        .unwrap_or_default()
}

fn status_from_findings(findings: &[Finding], strict: bool) -> &'static str {
    if findings.iter().any(|finding| finding.level == "error") {
        "blocked"
    } else if strict && findings.iter().any(|finding| finding.level == "warn") {
        "blocked"
    } else {
        "ok"
    }
}

async fn load_latest_step_blob(
    pool: &sqlx::PgPool,
    run_id: &str,
    step_name: &str,
    payload_kind: &str,
) -> Result<Option<StepPayloadBlob>> {
    let row = sqlx::query(
        r#"
        SELECT blobs.payload_type, blobs.payload_bytes
        FROM pipeline.step_payload_blobs blobs
        JOIN pipeline.step_executions exec
          ON exec.run_id = blobs.run_id
         AND exec.step_name = blobs.step_name
         AND exec.idempotency_key = blobs.idempotency_key
        WHERE blobs.run_id = $1
          AND blobs.step_name = $2
          AND blobs.payload_kind = $3
          AND exec.status = 'done'
        ORDER BY blobs.created_at DESC
        LIMIT 1
        "#,
    )
    .bind(Uuid::parse_str(run_id).with_context(|| format!("invalid run_id uuid: {run_id}"))?)
    .bind(step_name)
    .bind(payload_kind)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| StepPayloadBlob {
        payload_type: row.get("payload_type"),
        payload_bytes: row.get("payload_bytes"),
    }))
}

fn decode_step_blob_to_value(blob: &StepPayloadBlob) -> Result<Value> {
    match blob.payload_type.as_str() {
        "alegria.temporal.v1.CrawlSourcesOutputPayload" => {
            let payload = CrawlSourcesOutputPayload::decode(blob.payload_bytes.as_slice())?;
            Ok(json!({
                "claimed_count": payload.claimed_count,
                "crawled_count": payload.crawled_count,
                "failed_count": payload.failed_count,
                "raw_page_count": payload.raw_page_count,
                "raw_section_count": payload.raw_section_count,
                "qdrant_event_count": payload.qdrant_event_count,
                "status": payload.status,
                "raw_page_ids": payload.raw_page_ids,
                "failed_urls": payload.failed_urls,
            }))
        }
        "alegria.temporal.v1.RawKnowledgeIngestionInputPayload" => {
            let payload = RawKnowledgeIngestionInputPayload::decode(blob.payload_bytes.as_slice())?;
            Ok(json!({
                "run_id": payload.run_id,
                "context_key": payload.context_key,
                "query_batch_key": payload.query_batch_key,
                "raw_page_ids": payload.raw_page_ids,
                "source_policy": payload.source_policy,
            }))
        }
        "alegria.temporal.v1.RawKnowledgeIngestionOutputPayload" => {
            let payload =
                RawKnowledgeIngestionOutputPayload::decode(blob.payload_bytes.as_slice())?;
            Ok(json!({
                "raw_page_count": payload.raw_page_count,
                "raw_section_count": payload.raw_section_count,
                "extracted_rule_count": payload.extracted_rule_count,
                "verified_rule_count": payload.verified_rule_count,
                "outbox_event_count": payload.outbox_event_count,
                "changed_truth_keys": payload.changed_truth_keys,
                "status": payload.status,
            }))
        }
        "alegria.temporal.v1.ProjectionBarrierAuditOutputPayload" => {
            let payload =
                ProjectionBarrierAuditOutputPayload::decode(blob.payload_bytes.as_slice())?;
            Ok(json!({
                "run_id": payload.run_id,
                "checkpoint": payload.checkpoint,
                "blocked_events": payload.blocked_events,
                "max_open_lag_ms": payload.max_open_lag_ms,
                "status": payload.status,
            }))
        }
        payload_type if payload_type.starts_with("alegria.runtime.json.") => {
            Ok(serde_json::from_slice(&blob.payload_bytes)?)
        }
        other => anyhow::bail!("unsupported step payload type for shadow verification: {other}"),
    }
}

async fn load_latest_step_value(
    pool: &sqlx::PgPool,
    run_id: &str,
    step_name: &str,
    payload_kind: &str,
) -> Result<Option<Value>> {
    let Some(blob) = load_latest_step_blob(pool, run_id, step_name, payload_kind).await? else {
        return Ok(None);
    };
    Ok(Some(decode_step_blob_to_value(&blob)?))
}

async fn list_run_step_statuses(
    pool: &sqlx::PgPool,
    run_id: &str,
) -> Result<HashMap<String, String>> {
    let rows = sqlx::query(
        r#"
        SELECT step_name, status
        FROM pipeline.step_executions
        WHERE run_id = $1
        ORDER BY updated_at, step_name
        "#,
    )
    .bind(Uuid::parse_str(run_id).with_context(|| format!("invalid run_id uuid: {run_id}"))?)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.get::<String, _>("step_name"),
                row.get::<String, _>("status"),
            )
        })
        .collect())
}

async fn load_candidate_status_counts(
    pool: &sqlx::PgPool,
    context_key: &str,
    raw_page_ids: &[i64],
) -> Result<Value> {
    let rows = sqlx::query(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE c.epistemic_status = 'structured')::BIGINT AS structured_count,
            COUNT(*) FILTER (WHERE c.epistemic_status = 'verified')::BIGINT AS verified_count,
            COUNT(*) FILTER (WHERE c.epistemic_status = 'needs_hitl')::BIGINT AS needs_hitl_count,
            COUNT(*) FILTER (WHERE c.epistemic_status = 'rejected')::BIGINT AS rejected_count,
            COUNT(*)::BIGINT AS total_count
        FROM extracted.rule_candidates c
        JOIN raw.sections s ON s.id = c.raw_section_id
        WHERE c.context_key = $1
          AND s.page_id = ANY($2)
        "#,
    )
    .bind(context_key)
    .bind(raw_page_ids)
    .fetch_one(pool)
    .await?;
    Ok(json!({
        "structured_count": rows.get::<i64, _>("structured_count"),
        "verified_count": rows.get::<i64, _>("verified_count"),
        "needs_hitl_count": rows.get::<i64, _>("needs_hitl_count"),
        "rejected_count": rows.get::<i64, _>("rejected_count"),
        "total_count": rows.get::<i64, _>("total_count"),
    }))
}

async fn projection_status_value(pool: &sqlx::PgPool, run_id: &str) -> Result<Value> {
    let statuses = infrastructure::adapters::sqlx_seo_adapter::read_projection_sync_status_for_run(
        pool, run_id,
    )
    .await
    .map_err(|err| anyhow::anyhow!("{err}"))?;
    Ok(json!(statuses
        .into_iter()
        .map(|status| json!({
            "target_system": status.target_system,
            "pending_events": status.pending_events,
            "processing_events": status.processing_events,
            "failed_events": status.failed_events,
            "done_events": status.done_events,
            "max_open_lag_ms": status.max_open_lag_ms,
            "oldest_open_event_id": status.oldest_open_event_id,
            "oldest_open_aggregate_key": status.oldest_open_aggregate_key,
            "oldest_open_event_type": status.oldest_open_event_type,
            "latest_failed_aggregate_key": status.latest_failed_aggregate_key,
            "latest_failed_event_type": status.latest_failed_event_type,
            "latest_failed_error": status.latest_failed_error,
        }))
        .collect::<Vec<_>>()))
}

async fn cms_publish_event_summary(pool: &sqlx::PgPool, run_id: &str) -> Result<Value> {
    let rows = sqlx::query(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE event_type = 'seo_page_review_requested')::BIGINT AS review_requested_events,
            COUNT(DISTINCT page_node_key) FILTER (WHERE event_type = 'seo_page_review_requested')::BIGINT AS review_requested_pages,
            COUNT(*) FILTER (WHERE event_type = 'seo_page_approved')::BIGINT AS approved_events,
            COUNT(DISTINCT page_node_key) FILTER (WHERE event_type = 'seo_page_approved')::BIGINT AS approved_pages,
            COUNT(*) FILTER (WHERE event_type = 'seo_page_publish_blocked')::BIGINT AS blocked_events,
            COUNT(DISTINCT page_node_key) FILTER (WHERE event_type = 'seo_page_publish_blocked')::BIGINT AS blocked_pages
        FROM site.cms_publish_events
        WHERE event_payload ->> 'run_id' = $1
        "#,
    )
    .bind(run_id)
    .fetch_one(pool)
    .await?;
    Ok(json!({
        "review_requested_events": rows.get::<i64, _>("review_requested_events"),
        "review_requested_pages": rows.get::<i64, _>("review_requested_pages"),
        "approved_events": rows.get::<i64, _>("approved_events"),
        "approved_pages": rows.get::<i64, _>("approved_pages"),
        "blocked_events": rows.get::<i64, _>("blocked_events"),
        "blocked_pages": rows.get::<i64, _>("blocked_pages"),
    }))
}

async fn step_execution_counts(pool: &sqlx::PgPool, run_id: &str) -> Result<HashMap<String, i64>> {
    let rows = sqlx::query(
        r#"
        SELECT step_name, COUNT(*)::BIGINT AS step_count
        FROM pipeline.step_executions
        WHERE run_id = $1
          AND status = 'done'
        GROUP BY step_name
        "#,
    )
    .bind(Uuid::parse_str(run_id).with_context(|| format!("invalid run_id uuid: {run_id}"))?)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.get::<String, _>("step_name"),
                row.get::<i64, _>("step_count"),
            )
        })
        .collect())
}

fn default_seo_run_id() -> String {
    Uuid::new_v4().to_string()
}

fn map_seo_run_kind(command: SeoRunCommand) -> SeoScenarioKind {
    match command {
        SeoRunCommand::Planning => SeoScenarioKind::PlanningOnly,
        SeoRunCommand::Drafting => SeoScenarioKind::DraftingOnly,
        SeoRunCommand::Publish => SeoScenarioKind::PublishOnly,
        SeoRunCommand::Full => SeoScenarioKind::Full,
        SeoRunCommand::Rebuild => SeoScenarioKind::RebuildOnly,
        SeoRunCommand::CrawlIngest => SeoScenarioKind::CrawlIngestOnly,
    }
}

async fn register_seo_run_input(
    database_url: Option<String>,
    scenario: SeoRunCommand,
    run_id: Option<String>,
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
    publish: bool,
    require_preapproved_decision: bool,
    warn_only_projections: bool,
) -> Result<contracts::generated::alegria::temporal::v1::SeoSiteBuildInputPayload> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    register_site_build_input(
        &repo,
        &SeoSiteBuildRegistrationRequest {
            run_id: run_id.unwrap_or_else(default_seo_run_id),
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
            run_mode: Some(
                run_mode_for_scenario(
                    map_seo_run_kind(scenario),
                    publish,
                    require_preapproved_decision,
                    warn_only_projections,
                )
                .to_string(),
            ),
        },
    )
    .await
    .map_err(|err| anyhow::anyhow!("{err}"))
}

async fn seo_run(
    scenario: SeoRunCommand,
    database_url: Option<String>,
    run_id: Option<String>,
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
    output_dir: String,
    base_url: String,
    publish: bool,
    require_preapproved_decision: bool,
    warn_only_projections: bool,
) -> Result<i32> {
    let site_input = register_seo_run_input(
        database_url.clone(),
        scenario,
        run_id,
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
        publish,
        require_preapproved_decision,
        warn_only_projections,
    )
    .await?;
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    let result = execute_site_build_scenario(
        &repo,
        &SeoScenarioRequest {
            scenario: map_seo_run_kind(scenario),
            mode: SeoExecutionMode::SemiAutoOperator,
            policy: SeoRunPolicy::for_semi_auto_operator(
                publish,
                require_preapproved_decision,
                warn_only_projections,
            ),
            output_dir,
            base_url,
            site_input,
        },
    )
    .await
    .map_err(|err| anyhow::anyhow!("{err}"))?;

    println!(
        "SEO_RUN_RESULT scenario={} mode={} status={} page_total={} published_pages={} changed_truth_keys={}",
        result.scenario,
        result.mode,
        result.status,
        result.page_total,
        result.published_pages,
        result.changed_truth_keys.len()
    );
    for report in result.phase_reports {
        println!(
            "SEO_RUN_PHASE phase={} status={} page_node_key={} detail={}",
            report.phase, report.status, report.page_node_key, report.detail
        );
    }
    Ok(if result.status.starts_with("blocked") {
        1
    } else {
        0
    })
}

fn check_rust_migration_contract(root: &Path, report_json: &str, strict: bool) -> Result<i32> {
    let mut findings: Vec<Finding> = Vec::new();

    // ---------------------------------------------------------------------
    // M001: Required Rust-first files must exist.
    // Legacy Python paths are intentionally NOT required anymore (R9 archive).
    // ---------------------------------------------------------------------
    let required_files = [
        "app/rust/crates/primitives/src/block_validator.rs",
        "app/rust/crates/primitives/src/fact_verifier.rs",
        "app/rust/crates/primitives/src/writer.rs",
        "app/rust/crates/primitives/src/pdf_parser.rs",
        "app/rust/crates/seo_application/src/seo_runtime.rs",
        "app/rust/crates/infrastructure/src/adapters/sqlx_pipeline_runtime_adapter.rs",
        "app/rust/crates/infrastructure/src/adapters/proto_runtime_payload_store.rs",
        "app/rust/crates/infrastructure/src/adapters/truth_extraction_llm_adapter.rs",
        "app/rust/services/temporal/src/activities/mod.rs",
        "app/rust/services/temporal/src/workflows/mod.rs",
        "app/rust/crates/infrastructure/src/adapters/sqlx_adapter.rs",
        "app/rust/crates/infrastructure/src/adapters/neo4rs_adapter.rs",
        "app/rust/crates/infrastructure/src/adapters/qdrant_client_adapter.rs",
        "app/rust/crates/infrastructure/src/adapters/tonic_adapter.rs",
        "app/rust/services/analytics_svc/src/main.rs",
        "app/analytics_lab/proto/analytics.proto",
    ];

    for rel in required_files {
        let p = root.join(rel);
        if !p.exists() {
            findings.push(Finding {
                level: "error",
                code: "M001",
                message: format!("missing required file: {}", p.display()),
            });
        }
    }

    // ---------------------------------------------------------------------
    // R101: Enforce Rust Temporal activity coverage (MERGED_MODE target chain).
    // ---------------------------------------------------------------------
    let temporal_activities = root.join("app/rust/services/temporal/src/activities/mod.rs");
    if temporal_activities.exists() {
        let text = read_text(&temporal_activities)?;
        for fn_name in [
            "pub async fn generate_content(",
            "pub async fn validate_blocks(",
            "pub async fn finalize_run(",
        ] {
            if !text.contains(fn_name) {
                findings.push(Finding {
                    level: "warn",
                    code: "R101",
                    message: format!("missing temporal activity in Rust worker: {fn_name}"),
                });
            }
        }
    }

    // ---------------------------------------------------------------------
    // R102: outbox dedup protection must be present in schema and use-case SQL.
    // ---------------------------------------------------------------------
    let schema_sql = root.join("app/db/schema.sql");
    if schema_sql.exists() {
        let schema = read_text(&schema_sql)?;
        if !schema.contains("idx_sync_outbox_dedup") {
            findings.push(Finding {
                level: "warn",
                code: "R102",
                message: "schema.sql missing unique dedup index idx_sync_outbox_dedup".to_string(),
            });
        }
    }

    let pipeline_storage_files = [
        root.join("app/rust/crates/infrastructure/src/adapters/proto_runtime_payload_store.rs"),
        root.join("app/rust/crates/infrastructure/src/adapters/sqlx_runtime_outbox_adapter.rs"),
        root.join("app/rust/crates/infrastructure/src/adapters/sqlx_source_projection_adapter.rs"),
        root.join("app/rust/crates/infrastructure/src/adapters/sqlx_step_ledger_adapter.rs"),
    ];
    if pipeline_storage_files.iter().any(|path| path.exists()) {
        let mut ptxt = String::new();
        for path in pipeline_storage_files {
            if path.exists() {
                ptxt.push_str(&read_text(&path)?);
                ptxt.push('\n');
            }
        }
        if !ptxt.contains("ON CONFLICT") {
            findings.push(Finding {
                level: "warn",
                code: "R102",
                message: "pipeline_storage outbox emit path has no ON CONFLICT dedup guard"
                    .to_string(),
            });
        }
    }

    // ---------------------------------------------------------------------
    // R103: Python runtime files under app/ are forbidden.
    // ---------------------------------------------------------------------
    let app_root = root.join("app");
    if app_root.exists() {
        fn scan_py(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
            for ent in
                fs::read_dir(dir).with_context(|| format!("read_dir failed: {}", dir.display()))?
            {
                let ent = ent?;
                let path = ent.path();
                if path.is_dir() {
                    let name = ent.file_name();
                    let name = name.to_string_lossy();
                    if name == "__pycache__" || name == ".venv" || name == "target" {
                        continue;
                    }
                    scan_py(&path, out)?;
                } else if path.extension().and_then(|e| e.to_str()) == Some("py") {
                    out.push(path);
                }
            }
            Ok(())
        }

        let mut py_files = Vec::new();
        scan_py(&app_root, &mut py_files)?;
        for p in py_files {
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .to_string();
            findings.push(Finding {
                level: "warn",
                code: "R103",
                message: format!("python runtime file present in app (expected 0): {rel}"),
            });
        }
    }

    let mut status = "ok";
    if findings.iter().any(|f| f.level == "error") {
        status = "failed";
    } else if strict && findings.iter().any(|f| f.code.starts_with('R')) {
        status = "failed";
    }

    let report = json!({
        "status": status,
        "strict": strict,
        "findings": findings.iter().map(|f| json!({
            "level": f.level,
            "code": f.code,
            "message": f.message,
        })).collect::<Vec<_>>()
    });
    let out = write_report(root, report_json, &report)?;

    println!("RUST_MIGRATION_CONTRACT: {}", status.to_uppercase());
    println!("report: {}", out.display());
    for f in &findings {
        println!("[{}] {}: {}", f.level, f.code, f.message);
    }

    Ok(if status == "ok" { 0 } else { 1 })
}

fn check_fact_verifier_parity(root: &Path, report_json: &str) -> Result<i32> {
    let mut findings: Vec<Finding> = Vec::new();

    let registry = json!({
        "govA": {"source_type": "government", "trust_level": 5},
        "vfsA": {"source_type": "vfs", "trust_level": 4},
        "ag1": {"source_type": "niche_agency", "trust_level": 2},
        "ag2": {"source_type": "niche_agency", "trust_level": 2},
        "ed1": {"source_type": "editorial", "trust_level": 3},
    });

    // Case 1: empty -> hitl_required
    let r1 = verify_fact_json("consular_fee", "[]", &registry.to_string());
    let r1v: Value = serde_json::from_str(&r1)?;
    if r1v.get("resolution").and_then(Value::as_str) != Some("hitl_required") {
        findings.push(Finding {
            level: "error",
            code: "FVP001",
            message: "empty candidates must return hitl_required".to_string(),
        });
    }

    // Case 2: gov/vfs authoritative wins by confidence
    let c2 = json!([
        {"source_key":"ag1","fact_value":35,"extraction_confidence":0.95},
        {"source_key":"govA","fact_value":90,"extraction_confidence":0.60},
        {"source_key":"vfsA","fact_value":80,"extraction_confidence":0.90}
    ]);
    let r2: Value = serde_json::from_str(&verify_fact_json(
        "consular_fee",
        &c2.to_string(),
        &registry.to_string(),
    ))?;
    if r2.get("resolution").and_then(Value::as_str) != Some("gov_wins") {
        findings.push(Finding {
            level: "error",
            code: "FVP002",
            message: "authoritative case must return gov_wins".to_string(),
        });
    }
    if r2.get("value").and_then(Value::as_i64) != Some(80) {
        findings.push(Finding {
            level: "error",
            code: "FVP003",
            message: "authoritative winner must pick highest confidence value=80".to_string(),
        });
    }

    // Case 3: consensus >= 60%
    let c3 = json!([
        {"source_key":"ag1","fact_value":15,"extraction_confidence":0.70},
        {"source_key":"ag2","fact_value":15,"extraction_confidence":0.72},
        {"source_key":"ed1","fact_value":20,"extraction_confidence":0.65},
        {"source_key":"ag2","fact_value":15,"extraction_confidence":0.60},
        {"source_key":"ag1","fact_value":20,"extraction_confidence":0.60}
    ]);
    let r3: Value = serde_json::from_str(&verify_fact_json(
        "processing_days",
        &c3.to_string(),
        &registry.to_string(),
    ))?;
    if r3.get("resolution").and_then(Value::as_str) != Some("consensus") {
        findings.push(Finding {
            level: "error",
            code: "FVP004",
            message: "consensus case must return consensus".to_string(),
        });
    }
    if r3.get("value").and_then(Value::as_i64) != Some(15) {
        findings.push(Finding {
            level: "error",
            code: "FVP005",
            message: "consensus winner must be value=15".to_string(),
        });
    }

    // Case 4: conflict -> hitl_required with diagnostics
    let c4 = json!([
        {"source_key":"ag1","fact_value":7,"extraction_confidence":0.80},
        {"source_key":"ag2","fact_value":9,"extraction_confidence":0.81},
        {"source_key":"ed1","fact_value":11,"extraction_confidence":0.82}
    ]);
    let r4: Value = serde_json::from_str(&verify_fact_json(
        "processing_days",
        &c4.to_string(),
        &registry.to_string(),
    ))?;
    if r4.get("resolution").and_then(Value::as_str) != Some("hitl_required") {
        findings.push(Finding {
            level: "error",
            code: "FVP006",
            message: "conflict case must return hitl_required".to_string(),
        });
    }

    // Numeric range merge
    let c5 = json!([
        {"params":{"amount":35.0}},
        {"params":{"amount":90.0}},
        {"params":{"amount":60.0}}
    ]);
    let r5: Value = serde_json::from_str(&verify_numeric_rule_json(
        "consular_fee",
        &c5.to_string(),
        "amount",
    ))?;
    if r5.get("resolution").and_then(Value::as_str) != Some("range_merged") {
        findings.push(Finding {
            level: "error",
            code: "FVP007",
            message: "numeric case must return range_merged".to_string(),
        });
    }
    let range = r5.get("range").cloned().unwrap_or(json!({}));
    if range.get("min").and_then(Value::as_f64) != Some(35.0)
        || range.get("max").and_then(Value::as_f64) != Some(90.0)
        || range.get("count").and_then(Value::as_u64) != Some(3)
    {
        findings.push(Finding {
            level: "error",
            code: "FVP008",
            message: "numeric range expected min=35 max=90 count=3".to_string(),
        });
    }

    let status = if findings.iter().any(|f| f.level == "error") {
        "failed"
    } else {
        "ok"
    };

    let report = json!({
        "status": status,
        "cases_total": 5,
        "findings": findings.iter().map(|f| json!({
            "level": f.level,
            "code": f.code,
            "message": f.message,
        })).collect::<Vec<_>>()
    });
    let out = write_report(root, report_json, &report)?;

    println!("FACT_VERIFIER_PARITY: {}", status.to_uppercase());
    println!("report: {}", out.display());
    for f in &findings {
        println!("[{}] {}: {}", f.level, f.code, f.message);
    }

    Ok(if status == "ok" { 0 } else { 1 })
}

#[derive(Debug, Clone)]
struct StaticArtifact {
    relative_path: String,
    bytes: Vec<u8>,
}

fn default_database_url() -> String {
    env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres_password@localhost:5433/alegria".to_string()
    })
}

fn default_temporal_url() -> String {
    env::var("TEMPORAL_URL").unwrap_or_else(|_| "http://localhost:7233".to_string())
}

fn default_temporal_namespace() -> String {
    env::var("TEMPORAL_NAMESPACE").unwrap_or_else(|_| "default".to_string())
}

fn evaluate_gate_status(report: &Value, blocking_reasons: &mut Vec<String>) {
    let label = report
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("unknown_gate");
    let status = report
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("error");
    if status == "error" {
        let exit_code = report
            .get("exit_code")
            .and_then(Value::as_i64)
            .map(|code| code.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        blocking_reasons.push(format!("{label} failed (exit_code={exit_code})"));
    }
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn normalize_url_path(raw: &str) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        anyhow::bail!("canonical_url_path must not be empty");
    }
    if raw.contains('\\') || raw.contains('?') || raw.contains('#') {
        anyhow::bail!("canonical_url_path contains unsupported characters: {raw}");
    }
    let mut path = format!("/{}", raw.trim_start_matches('/'));
    while path.contains("//") {
        path = path.replace("//", "/");
    }
    if path != "/" {
        path = path.trim_end_matches('/').to_string();
    }
    if path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        anyhow::bail!("canonical_url_path must not contain traversal segments: {raw}");
    }
    Ok(path)
}

fn output_path_for_url(url_path: &str) -> Result<String> {
    let normalized = normalize_url_path(url_path)?;
    if normalized == "/" {
        return Ok("index.html".to_string());
    }
    Ok(format!("{}/index.html", normalized.trim_start_matches('/')))
}

fn absolute_url(base_url: &str, url_path: &str) -> Result<String> {
    let path = normalize_url_path(url_path)?;
    Ok(format!(
        "{}{}",
        base_url.trim_end_matches('/'),
        if path == "/" { "/".to_string() } else { path }
    ))
}

fn markdown_to_html(markdown: &str) -> String {
    let parser = MarkdownParser::new_ext(markdown, MarkdownOptions::all());
    let mut rendered = String::new();
    html::push_html(&mut rendered, parser);
    rendered
}

fn page_markdown(page: &StaticCmsPageRow) -> &str {
    page.body_payload
        .get("markdown")
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn render_content_blocks(page: &StaticCmsPageRow) -> String {
    let Some(blocks) = page
        .body_payload
        .get("content_blocks")
        .and_then(Value::as_array)
    else {
        return String::new();
    };
    let mut html = String::new();
    for block in blocks {
        let block_type = block
            .get("block_type")
            .and_then(Value::as_str)
            .unwrap_or("prose");
        let section_role = block
            .get("section_role")
            .and_then(Value::as_str)
            .unwrap_or("section");
        let heading = block.get("heading").and_then(Value::as_str).unwrap_or("");
        let markdown = block.get("markdown").and_then(Value::as_str).unwrap_or("");
        if markdown.trim().is_empty() && heading.trim().is_empty() {
            continue;
        }
        html.push_str(&format!(
            r#"<section class="content-block content-block--{}" data-section-role="{}">"#,
            escape_html(block_type),
            escape_html(section_role)
        ));
        if !heading.trim().is_empty() {
            html.push_str(&format!("<h2>{}</h2>", escape_html(heading)));
        }
        html.push_str(&markdown_to_html(markdown));
        html.push_str("</section>");
    }
    html
}

fn menu_depth(url_path: &str) -> Result<usize> {
    let path = normalize_url_path(url_path)?;
    Ok(path
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .count())
}

fn render_navigation(pages: &[StaticCmsPageRow]) -> Result<String> {
    let mut items = String::new();
    for page in pages {
        let href = normalize_url_path(&page.canonical_url_path)?;
        let depth = menu_depth(&href)?;
        items.push_str(&format!(
            r#"<a href="{}" data-depth="{}">{}</a>"#,
            escape_html(&href),
            depth,
            escape_html(&page.title)
        ));
    }
    Ok(items)
}

fn breadcrumb_label(segment: &str) -> String {
    segment
        .split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn breadcrumb_entries(page: &StaticCmsPageRow, base_url: &str) -> Result<Vec<(String, String)>> {
    let canonical_path = normalize_url_path(&page.canonical_url_path)?;
    let mut entries = vec![("Home".to_string(), absolute_url(base_url, "/")?)];
    if canonical_path == "/" {
        return Ok(entries);
    }

    let segments = canonical_path
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let mut current = String::new();
    for (idx, segment) in segments.iter().enumerate() {
        current.push('/');
        current.push_str(segment);
        current.push('/');
        let label = if idx + 1 == segments.len() {
            page.title.clone()
        } else {
            breadcrumb_label(segment)
        };
        entries.push((label, absolute_url(base_url, &current)?));
    }
    Ok(entries)
}

fn render_breadcrumbs(page: &StaticCmsPageRow, base_url: &str) -> Result<String> {
    let entries = breadcrumb_entries(page, base_url)?;
    let mut items = String::new();
    for (idx, (label, href)) in entries.iter().enumerate() {
        let current = if idx + 1 == entries.len() {
            r#" aria-current="page""#
        } else {
            ""
        };
        items.push_str(&format!(
            r#"<li><a href="{}"{}>{}</a></li>"#,
            escape_html(href),
            current,
            escape_html(label)
        ));
    }
    Ok(format!(
        r#"<nav class="breadcrumbs" aria-label="Breadcrumb"><ol>{items}</ol></nav>"#
    ))
}

fn breadcrumb_list_json(page: &StaticCmsPageRow, base_url: &str) -> Result<Value> {
    let entries = breadcrumb_entries(page, base_url)?;
    Ok(json!({
        "@type": "BreadcrumbList",
        "itemListElement": entries.iter().enumerate().map(|(idx, (label, href))| json!({
            "@type": "ListItem",
            "position": idx + 1,
            "name": label,
            "item": href,
        })).collect::<Vec<_>>()
    }))
}

fn push_faq_item(items: &mut Vec<Value>, question: &Option<String>, answer_lines: &[String]) {
    let Some(question_text) = question.as_ref() else {
        return;
    };
    let answer = answer_lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if answer.is_empty() {
        return;
    }
    items.push(json!({
        "@type": "Question",
        "name": question_text,
        "acceptedAnswer": {
            "@type": "Answer",
            "text": answer,
        }
    }));
}

fn faq_entities(markdown: &str) -> Vec<Value> {
    let mut items = Vec::new();
    let mut in_faq = false;
    let mut question: Option<String> = None;
    let mut answer_lines: Vec<String> = Vec::new();

    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("## ") {
            let heading_key = heading.trim().to_ascii_lowercase();
            if in_faq {
                push_faq_item(&mut items, &question, &answer_lines);
                question = None;
                answer_lines.clear();
            }
            in_faq = heading_key == "faq"
                || heading_key == "frequently asked questions"
                || heading_key == "вопросы и ответы";
            continue;
        }
        if !in_faq {
            continue;
        }
        if let Some(next_question) = trimmed.strip_prefix("### ") {
            push_faq_item(&mut items, &question, &answer_lines);
            question = Some(next_question.trim().to_string());
            answer_lines.clear();
        } else if question.is_some() {
            answer_lines.push(trimmed.to_string());
        }
    }
    if in_faq {
        push_faq_item(&mut items, &question, &answer_lines);
    }
    items
}

fn fallback_schema_json(page: &StaticCmsPageRow, canonical: &str, base_url: &str) -> Result<Value> {
    let markdown = page_markdown(page);
    let mut graph = vec![
        json!({
            "@type": "Article",
            "headline": page.title,
            "url": canonical,
        }),
        breadcrumb_list_json(page, base_url)?,
    ];
    let faq_items = faq_entities(markdown);
    if !faq_items.is_empty() {
        graph.push(json!({
            "@type": "FAQPage",
            "mainEntity": faq_items,
        }));
    }
    Ok(json!({
        "@context": "https://schema.org",
        "@graph": graph,
    }))
}

fn page_schema_json(page: &StaticCmsPageRow, canonical: &str, base_url: &str) -> Result<Value> {
    let custom_schema = page.schema_markup_payload.is_object()
        && !page
            .schema_markup_payload
            .as_object()
            .map(|o| o.is_empty())
            .unwrap_or(true);
    if !custom_schema {
        return fallback_schema_json(page, canonical, base_url);
    }

    let mut graph = vec![
        page.schema_markup_payload.clone(),
        breadcrumb_list_json(page, base_url)?,
    ];
    let faq_items = faq_entities(page_markdown(page));
    if !faq_items.is_empty() {
        graph.push(json!({
            "@type": "FAQPage",
            "mainEntity": faq_items,
        }));
    }
    Ok(json!({
        "@context": "https://schema.org",
        "@graph": graph,
    }))
}

fn render_related_links(
    page: &StaticCmsPageRow,
    pages_by_key: &HashMap<String, StaticCmsPageRow>,
    links_by_source: &HashMap<String, Vec<StaticCmsLinkRow>>,
) -> Result<String> {
    let Some(links) = links_by_source.get(&page.page_node_key) else {
        return Ok(String::new());
    };
    let mut items = String::new();
    for link in links {
        let Some(target) = pages_by_key.get(&link.target_page_key) else {
            continue;
        };
        let href = normalize_url_path(&target.canonical_url_path)?;
        let marker = if link.required_flag {
            " data-required=\"true\""
        } else {
            ""
        };
        items.push_str(&format!(
            r#"<li{}><a href="{}">{}</a><span>{}</span></li>"#,
            marker,
            escape_html(&href),
            escape_html(&target.title),
            escape_html(&link.link_role)
        ));
    }
    if items.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!(
            r#"<section class="related"><h2>Related Pages</h2><ul>{items}</ul></section>"#
        ))
    }
}

fn render_page(
    page: &StaticCmsPageRow,
    pages: &[StaticCmsPageRow],
    pages_by_key: &HashMap<String, StaticCmsPageRow>,
    links_by_source: &HashMap<String, Vec<StaticCmsLinkRow>>,
    base_url: &str,
) -> Result<String> {
    let canonical_path = normalize_url_path(&page.canonical_url_path)?;
    let canonical = absolute_url(base_url, &canonical_path)?;
    let block_body = render_content_blocks(page);
    let body = if block_body.trim().is_empty() {
        markdown_to_html(page_markdown(page))
    } else {
        block_body
    };
    let nav = render_navigation(pages)?;
    let breadcrumbs = render_breadcrumbs(page, base_url)?;
    let related = render_related_links(page, pages_by_key, links_by_source)?;
    let schema_json = serde_json::to_string_pretty(&page_schema_json(page, &canonical, base_url)?)?;

    Ok(format!(
        r#"<!doctype html>
<html lang="{locale}">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <meta name="description" content="{description}">
  <link rel="canonical" href="{canonical}">
  <script type="application/ld+json">{schema_json}</script>
  <style>
    :root {{ color-scheme: light; --ink: #182026; --muted: #5a6872; --line: #d9e0e5; --accent: #176b5d; --bg: #fbfcfc; }}
    body {{ margin: 0; font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; color: var(--ink); background: var(--bg); line-height: 1.62; }}
    header {{ border-bottom: 1px solid var(--line); background: #ffffff; }}
    nav {{ max-width: 1120px; margin: 0 auto; padding: 14px 24px; display: flex; gap: 16px; overflow-x: auto; }}
    nav a {{ color: var(--ink); text-decoration: none; white-space: nowrap; font-size: 14px; }}
    nav a[data-depth="0"] {{ font-weight: 700; }}
    nav a[data-depth="1"] {{ font-weight: 600; }}
    nav a[data-depth="2"] {{ color: var(--muted); }}
    main {{ max-width: 820px; margin: 0 auto; padding: 48px 24px 72px; }}
    .breadcrumbs {{ max-width: 820px; padding: 0; margin: 0 0 28px; display: block; overflow: visible; }}
    .breadcrumbs ol {{ display: flex; flex-wrap: wrap; gap: 8px; list-style: none; padding: 0; margin: 0; color: var(--muted); font-size: 13px; }}
    .breadcrumbs li:not(:last-child)::after {{ content: "/"; margin-left: 8px; color: var(--muted); }}
    .breadcrumbs a {{ color: var(--muted); font-size: 13px; }}
    article h1 {{ font-size: clamp(32px, 5vw, 48px); line-height: 1.08; margin: 0 0 14px; }}
    .meta {{ color: var(--muted); font-size: 14px; margin-bottom: 34px; }}
    article h2 {{ margin-top: 38px; font-size: 26px; line-height: 1.2; }}
    article a {{ color: var(--accent); }}
    article code {{ background: #eef3f2; padding: 2px 5px; border-radius: 4px; }}
    .related {{ margin-top: 52px; border-top: 1px solid var(--line); padding-top: 26px; }}
    .related ul {{ padding: 0; margin: 0; list-style: none; display: grid; gap: 10px; }}
    .related li {{ display: flex; justify-content: space-between; gap: 16px; border-bottom: 1px solid var(--line); padding-bottom: 10px; }}
    .related span {{ color: var(--muted); font-size: 13px; }}
    footer {{ border-top: 1px solid var(--line); color: var(--muted); font-size: 13px; padding: 22px 24px; text-align: center; }}
  </style>
</head>
<body>
  <header><nav>{nav}</nav></header>
  <main>
    {breadcrumbs}
    <article>
      <h1>{h1}</h1>
      <div class="meta">{page_type} / {intent} / {updated_at}</div>
      {body}
    </article>
    {related}
  </main>
  <footer>Generated by Alegria Static Site Builder</footer>
</body>
</html>
"#,
        locale = escape_html(&page.locale_code),
        title = escape_html(&page.title),
        description = escape_html(&page.meta_description),
        canonical = escape_html(&canonical),
        h1 = escape_html(&page.h1),
        page_type = escape_html(&page.page_type_key),
        intent = escape_html(&page.dominant_intent_key),
        updated_at = escape_html(&page.updated_at),
        breadcrumbs = breadcrumbs,
    ))
}

fn build_static_artifacts(
    pages: &[StaticCmsPageRow],
    links: &[StaticCmsLinkRow],
    base_url: &str,
) -> Result<Vec<StaticArtifact>> {
    if pages.is_empty() {
        anyhow::bail!("no approved or published CMS pages are available for static build");
    }

    let pages_by_key: HashMap<String, StaticCmsPageRow> = pages
        .iter()
        .map(|page| (page.page_node_key.clone(), page.clone()))
        .collect();
    let mut links_by_source: HashMap<String, Vec<StaticCmsLinkRow>> = HashMap::new();
    for link in links {
        links_by_source
            .entry(link.source_page_key.clone())
            .or_default()
            .push(link.clone());
    }

    let mut artifacts = Vec::new();
    let mut sitemap_urls = Vec::new();
    let mut manifest_pages = Vec::new();
    for page in pages {
        let html = render_page(page, pages, &pages_by_key, &links_by_source, base_url)?;
        let relative_path = output_path_for_url(&page.canonical_url_path)?;
        artifacts.push(StaticArtifact {
            relative_path,
            bytes: html.into_bytes(),
        });
        let url = absolute_url(base_url, &page.canonical_url_path)?;
        sitemap_urls.push(format!(
            "<url><loc>{}</loc><lastmod>{}</lastmod></url>",
            escape_html(&url),
            escape_html(&page.updated_at)
        ));
        manifest_pages.push(json!({
            "page_node_key": page.page_node_key,
            "revision_id": page.revision_id,
            "cms_document_id": page.cms_document_id,
            "canonical_url_path": normalize_url_path(&page.canonical_url_path)?,
            "status": page.current_status,
            "title": page.title,
        }));
    }

    artifacts.push(StaticArtifact {
        relative_path: "sitemap.xml".to_string(),
        bytes: format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">{}</urlset>"#,
            sitemap_urls.join("")
        )
        .into_bytes(),
    });
    artifacts.push(StaticArtifact {
        relative_path: "robots.txt".to_string(),
        bytes: b"User-agent: *\nAllow: /\nSitemap: /sitemap.xml\n".to_vec(),
    });
    artifacts.push(StaticArtifact {
        relative_path: "alegria-static-manifest.json".to_string(),
        bytes: serde_json::to_vec_pretty(&json!({
            "builder": "alegria_static_site_builder@1",
            "base_url": base_url,
            "page_count": pages.len(),
            "pages": manifest_pages,
        }))?,
    });

    Ok(artifacts)
}

fn write_static_artifacts(output_dir: &Path, artifacts: &[StaticArtifact]) -> Result<()> {
    let marker = output_dir.join(".alegria_static_site");
    if output_dir.exists() {
        let mut entries = fs::read_dir(output_dir)
            .with_context(|| format!("read output dir failed: {}", output_dir.display()))?;
        if marker.exists() {
            fs::remove_dir_all(output_dir)
                .with_context(|| format!("clean output dir failed: {}", output_dir.display()))?;
        } else if entries.next().is_some() {
            anyhow::bail!(
                "output dir is not empty and was not created by Alegria: {}",
                output_dir.display()
            );
        }
    }
    fs::create_dir_all(output_dir)
        .with_context(|| format!("create output dir failed: {}", output_dir.display()))?;
    fs::write(&marker, b"alegria_static_site_builder@1\n")
        .with_context(|| format!("write marker failed: {}", marker.display()))?;

    for artifact in artifacts {
        let path = output_dir.join(&artifact.relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create artifact dir failed: {}", parent.display()))?;
        }
        fs::write(&path, &artifact.bytes)
            .with_context(|| format!("write artifact failed: {}", path.display()))?;
    }
    Ok(())
}

async fn build_static_site(
    database_url: Option<String>,
    output_dir: &str,
    base_url: &str,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let snapshot = load_static_site_snapshot(&pool).await?;
    let artifacts = build_static_artifacts(&snapshot.pages, &snapshot.links, base_url)?;
    write_static_artifacts(Path::new(output_dir), &artifacts)?;
    println!(
        "STATIC_SITE_BUILD: OK pages={} artifacts={} output={}",
        snapshot.pages.len(),
        artifacts.len(),
        output_dir
    );
    Ok(0)
}

async fn crawl_pending_sources(
    database_url: Option<String>,
    run_id: String,
    query_batch_key: String,
    limit: i64,
    emit_qdrant: bool,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    let output = run_crawl_sources(
        &repo,
        &contracts::generated::alegria::temporal::v1::CrawlSourcesInputPayload {
            run_id,
            query_batch_key,
            limit: limit as u32,
            emit_qdrant,
        },
    )
    .await?;
    println!(
        "CRAWL_SUMMARY claimed={} crawled={} failed={} raw_pages={} status={}",
        output.claimed_count,
        output.crawled_count,
        output.failed_count,
        output.raw_page_count,
        output.status
    );
    for url in &output.failed_urls {
        eprintln!("CRAWL_FAILED url={url}");
    }
    Ok(if output.failed_count == 0 { 0 } else { 1 })
}

async fn cms_review_list(database_url: Option<String>, limit: i64) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let rows = sqlx::query(
        r#"
        SELECT p.page_node_key, p.canonical_url_path, p.current_status,
               r.revision_id, r.title, r.updated_at::text AS updated_at
        FROM site.cms_pages p
        JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
        WHERE p.current_status IN ('review_required','blocked','approved')
        ORDER BY r.updated_at DESC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(&pool)
    .await?;
    let payload = rows
        .into_iter()
        .map(|row| {
            json!({
                "page_node_key": row.get::<String, _>("page_node_key"),
                "canonical_url_path": row.get::<String, _>("canonical_url_path"),
                "current_status": row.get::<String, _>("current_status"),
                "revision_id": row.get::<String, _>("revision_id"),
                "title": row.get::<String, _>("title"),
                "updated_at": row.get::<String, _>("updated_at"),
            })
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(0)
}

async fn cms_review_show(database_url: Option<String>, page_node_key: &str) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let row = sqlx::query(
        r#"
        SELECT p.page_node_key, p.scope_signature, p.canonical_url_path, p.current_status,
               r.revision_id, r.title, r.meta_description, r.h1, r.body_payload,
               r.schema_markup_payload, r.required_link_payload, r.traceability_manifest,
               r.revision_status, r.updated_at::text AS updated_at
        FROM site.cms_pages p
        JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
        WHERE p.page_node_key = $1
        "#,
    )
    .bind(page_node_key)
    .fetch_one(&pool)
    .await?;
    let body: Json<Value> = row.get("body_payload");
    let schema: Json<Value> = row.get("schema_markup_payload");
    let links: Json<Value> = row.get("required_link_payload");
    let traceability: Json<Value> = row.get("traceability_manifest");
    let payload = json!({
        "page_node_key": row.get::<String, _>("page_node_key"),
        "scope_signature": row.get::<String, _>("scope_signature"),
        "canonical_url_path": row.get::<String, _>("canonical_url_path"),
        "current_status": row.get::<String, _>("current_status"),
        "revision_id": row.get::<String, _>("revision_id"),
        "revision_status": row.get::<String, _>("revision_status"),
        "title": row.get::<String, _>("title"),
        "meta_description": row.get::<String, _>("meta_description"),
        "h1": row.get::<String, _>("h1"),
        "body_payload": body.0,
        "schema_markup_payload": schema.0,
        "required_link_payload": links.0,
        "traceability_manifest": traceability.0,
        "updated_at": row.get::<String, _>("updated_at"),
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(0)
}

async fn cms_publish_status(database_url: Option<String>, page_node_key: &str) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let row = sqlx::query(
        r#"
        SELECT p.page_node_key, p.current_status, p.published_at::text AS published_at,
               p.current_revision_id, r.revision_status,
               a.status AS artifact_status, a.artifact_uri, a.updated_at::text AS artifact_updated_at
        FROM site.cms_pages p
        LEFT JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
        LEFT JOIN site.publish_artifacts a
          ON a.revision_id = p.current_revision_id
         AND a.page_node_key = p.page_node_key
        WHERE p.page_node_key = $1
        ORDER BY a.updated_at DESC NULLS LAST
        LIMIT 1
        "#,
    )
    .bind(page_node_key)
    .fetch_one(&pool)
    .await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "page_node_key": row.get::<String, _>("page_node_key"),
            "current_status": row.get::<String, _>("current_status"),
            "published_at": row.get::<Option<String>, _>("published_at"),
            "current_revision_id": row.get::<Option<String>, _>("current_revision_id"),
            "revision_status": row.get::<Option<String>, _>("revision_status"),
            "artifact_status": row.get::<Option<String>, _>("artifact_status"),
            "artifact_uri": row.get::<Option<String>, _>("artifact_uri"),
            "artifact_updated_at": row.get::<Option<String>, _>("artifact_updated_at"),
        }))?
    );
    Ok(0)
}

async fn cms_traceability_inspect(
    database_url: Option<String>,
    page_node_key: &str,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let row = sqlx::query(
        r#"
        SELECT r.revision_id, r.traceability_manifest
        FROM site.cms_pages p
        JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
        WHERE p.page_node_key = $1
        "#,
    )
    .bind(page_node_key)
    .fetch_one(&pool)
    .await?;
    let traceability: Json<Value> = row.get("traceability_manifest");
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "page_node_key": page_node_key,
            "revision_id": row.get::<String, _>("revision_id"),
            "traceability_manifest": traceability.0,
        }))?
    );
    Ok(0)
}

async fn cms_blockers_inspect(database_url: Option<String>, page_node_key: &str) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let rows = sqlx::query(
        r#"
        SELECT task_key, task_type, queue_state, blocking_step_name,
               severity, decision_payload, audit_log_payload,
               updated_at::text AS updated_at
        FROM site.seo_hitl_tasks
        WHERE page_node_key = $1
          AND queue_state IN ('open','reopened','in_review')
        ORDER BY updated_at DESC
        "#,
    )
    .bind(page_node_key)
    .fetch_all(&pool)
    .await?;
    let payload = rows
        .into_iter()
        .map(|row| {
            json!({
                "task_key": row.get::<String, _>("task_key"),
                "task_type": row.get::<String, _>("task_type"),
                "queue_state": row.get::<String, _>("queue_state"),
                "blocking_step_name": row.get::<String, _>("blocking_step_name"),
                "severity": row.get::<String, _>("severity"),
                "decision_payload": row.get::<Json<Value>, _>("decision_payload").0,
                "audit_log_payload": row.get::<Json<Value>, _>("audit_log_payload").0,
                "updated_at": row.get::<String, _>("updated_at"),
            })
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(0)
}

async fn seo_rebuild_backlog_inspect(
    database_url: Option<String>,
    page_node_key: Option<String>,
    limit: i64,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let rows = if let Some(page_node_key) = page_node_key.as_ref() {
        sqlx::query(
            r#"
            SELECT rebuild_request_key, page_node_key, trigger_type, priority, status, reason,
                   updated_at::text AS updated_at
            FROM monitoring.seo_rebuild_backlog
            WHERE page_node_key = $1
            ORDER BY updated_at DESC
            LIMIT $2
            "#,
        )
        .bind(page_node_key)
        .bind(limit)
        .fetch_all(&pool)
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT rebuild_request_key, page_node_key, trigger_type, priority, status, reason,
                   updated_at::text AS updated_at
            FROM monitoring.seo_rebuild_backlog
            ORDER BY updated_at DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&pool)
        .await?
    };
    let payload = rows
        .into_iter()
        .map(|row| {
            json!({
                "rebuild_request_key": row.get::<String, _>("rebuild_request_key"),
                "page_node_key": row.get::<Option<String>, _>("page_node_key"),
                "trigger_type": row.get::<String, _>("trigger_type"),
                "priority": row.get::<i32, _>("priority"),
                "status": row.get::<String, _>("status"),
                "reason": row.get::<String, _>("reason"),
                "updated_at": row.get::<String, _>("updated_at"),
            })
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(0)
}

async fn seo_support_bundle_inspect(
    database_url: Option<String>,
    context_key: &str,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let rows = sqlx::query(
        r#"
        SELECT
            r.rule_instance_id,
            r.rule_type_key,
            r.role_type,
            r.status,
            r.effective_from::text AS effective_from,
            r.effective_to::text AS effective_to,
            COALESCE(c.label_ru, c.concept_key, r.concept_key) AS concept_label,
            COALESCE(s.source_label, '') AS source_label,
            COALESCE(s.source_type, 'editorial') AS source_type,
            COALESCE(s.trust_level, 3) AS trust_level
        FROM verified.rule_instances r
        LEFT JOIN kb.concepts c ON c.concept_key = r.concept_key
        LEFT JOIN kb.sources s ON s.source_key = r.source_key
        WHERE r.context_key = $1
          AND r.status = 'verified'
        ORDER BY r.role_type, r.rule_instance_id
        "#,
    )
    .bind(context_key)
    .fetch_all(&pool)
    .await?;
    let payload = rows
        .into_iter()
        .map(|row| {
            json!({
                "rule_instance_id": row.get::<String, _>("rule_instance_id"),
                "rule_type_key": row.get::<String, _>("rule_type_key"),
                "role_type": row.get::<String, _>("role_type"),
                "concept_label": row.get::<String, _>("concept_label"),
                "source_label": row.get::<String, _>("source_label"),
                "source_type": row.get::<String, _>("source_type"),
                "trust_level": row.get::<i32, _>("trust_level"),
                "effective_from": row.get::<Option<String>, _>("effective_from"),
                "effective_to": row.get::<Option<String>, _>("effective_to"),
                "status": row.get::<String, _>("status"),
            })
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(0)
}

async fn seo_post_publish_feedback_probe(
    analytics_addr: &str,
    require_gsc: bool,
    report_json: Option<String>,
) -> Result<i32> {
    let analytics = match AnalyticsClient::connect(analytics_addr).await {
        Ok(mut client) => {
            let ok = client.ping().await;
            json!({
                "status": if ok { "ok" } else { "error" },
                "addr": analytics_addr,
            })
        }
        Err(err) => json!({
            "status": "error",
            "addr": analytics_addr,
            "error": err.to_string(),
        }),
    };

    let gsc_access_token = env::var("GSC_ACCESS_TOKEN").ok();
    let gsc_site_url = env::var("GSC_SITE_URL").ok();
    let gsc = match (gsc_access_token, gsc_site_url) {
        (Some(token), Some(site_url))
            if !token.trim().is_empty() && !site_url.trim().is_empty() =>
        {
            let http = new_default_client(30)?;
            match http
                .get("https://www.googleapis.com/webmasters/v3/sites")
                .bearer_auth(token)
                .send()
                .await
            {
                Ok(resp) => json!({
                    "status": if resp.status().is_success() { "ok" } else { "error" },
                    "http_status": resp.status().as_u16(),
                    "site_url": site_url,
                }),
                Err(err) => json!({
                    "status": "error",
                    "site_url": site_url,
                    "error": err.to_string(),
                }),
            }
        }
        _ if require_gsc => json!({
            "status": "error",
            "error": "GSC_ACCESS_TOKEN and GSC_SITE_URL are required",
        }),
        _ => json!({
            "status": "not_configured",
        }),
    };

    let analytics_ok = analytics.get("status").and_then(Value::as_str) == Some("ok");
    let gsc_status = gsc.get("status").and_then(Value::as_str).unwrap_or("error");
    let gsc_ok = gsc_status == "ok" || (!require_gsc && gsc_status == "not_configured");
    let overall_status = if analytics_ok && gsc_ok {
        "ok"
    } else {
        "error"
    };

    let payload = json!({
        "status": overall_status,
        "analytics": analytics,
        "gsc": gsc,
        "truth_mutation": "forbidden",
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    if let Some(report_json) = report_json.as_deref() {
        let out = write_report(Path::new("."), report_json, &payload)?;
        eprintln!("report: {}", out.display());
    }
    Ok(if overall_status == "ok" { 0 } else { 2 })
}

fn seo_release_restore_gate(
    root: &Path,
    run_ci_verify: bool,
    run_temporal_gate: bool,
    run_restore_drill: bool,
    report_json: Option<String>,
) -> Result<i32> {
    let required_paths = [
        "automation/ci_verify.sh",
        "automation/temporal_production_gate.sh",
        "infra/backups/restore_drill.sh",
        "automation/check_temporal_build_id_policy.py",
        "automation/check_seo_rollout_compat_contract.py",
        "automation/check_backup_restore_layout.py",
    ];
    let mut blocking_reasons = Vec::new();
    let mut missing_paths = Vec::new();
    for rel in required_paths {
        if !root.join(rel).exists() {
            missing_paths.push(rel.to_string());
            blocking_reasons.push(format!("missing required gate path: {rel}"));
        }
    }

    let build_id_policy = run_gate_command(
        root,
        "check_temporal_build_id_policy",
        "python3",
        &["automation/check_temporal_build_id_policy.py"],
        true,
    )?;
    evaluate_gate_status(&build_id_policy, &mut blocking_reasons);

    let rollout_compat = run_gate_command(
        root,
        "check_seo_rollout_compat_contract",
        "python3",
        &["automation/check_seo_rollout_compat_contract.py"],
        true,
    )?;
    evaluate_gate_status(&rollout_compat, &mut blocking_reasons);

    let backup_restore_layout = run_gate_command(
        root,
        "check_backup_restore_layout",
        "python3",
        &["automation/check_backup_restore_layout.py"],
        true,
    )?;
    evaluate_gate_status(&backup_restore_layout, &mut blocking_reasons);

    let ci_verify = run_gate_command(
        root,
        "ci_verify",
        "bash",
        &["automation/ci_verify.sh"],
        run_ci_verify,
    )?;
    evaluate_gate_status(&ci_verify, &mut blocking_reasons);

    let temporal_gate = run_gate_command(
        root,
        "temporal_production_gate",
        "bash",
        &["automation/temporal_production_gate.sh"],
        run_temporal_gate,
    )?;
    evaluate_gate_status(&temporal_gate, &mut blocking_reasons);

    let restore_drill = run_gate_command(
        root,
        "restore_drill",
        "bash",
        &["infra/backups/restore_drill.sh"],
        run_restore_drill,
    )?;
    evaluate_gate_status(&restore_drill, &mut blocking_reasons);

    let status = if blocking_reasons.is_empty() {
        "ok"
    } else {
        "blocked"
    };
    let payload = json!({
        "status": status,
        "release_gate": "release_and_restore_gate",
        "root": root.display().to_string(),
        "executed": {
            "run_ci_verify": run_ci_verify,
            "run_temporal_gate": run_temporal_gate,
            "run_restore_drill": run_restore_drill,
        },
        "required_paths": required_paths,
        "missing_paths": missing_paths,
        "blocking_reasons": blocking_reasons,
        "checks": {
            "build_id_policy": build_id_policy,
            "rollout_compat": rollout_compat,
            "backup_restore_layout": backup_restore_layout,
            "ci_verify": ci_verify,
            "temporal_production_gate": temporal_gate,
            "restore_drill": restore_drill,
        }
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    if let Some(report_json) = report_json.as_deref() {
        let out = write_report(root, report_json, &payload)?;
        eprintln!("report: {}", out.display());
    }
    Ok(if status == "ok" { 0 } else { 2 })
}

async fn seo_cutover_shadow_verify(
    database_url: Option<String>,
    legacy_run_id: &str,
    cutover_run_id: &str,
    strict: bool,
    report_json: Option<String>,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;

    let legacy_crawl = load_latest_step_value(&pool, legacy_run_id, "crawl_sources", "output")
        .await?
        .context("legacy run missing crawl_sources output")?;
    let legacy_raw_ingestion =
        load_latest_step_value(&pool, legacy_run_id, "raw_knowledge_ingestion", "output")
            .await?
            .context("legacy run missing raw_knowledge_ingestion output")?;
    let legacy_raw_ingestion_input =
        load_latest_step_value(&pool, legacy_run_id, "raw_knowledge_ingestion", "input")
            .await?
            .context("legacy run missing raw_knowledge_ingestion input")?;

    let cutover_raw_evidence =
        load_latest_step_value(&pool, cutover_run_id, "raw_evidence_register", "output")
            .await?
            .context("cutover run missing raw_evidence_register output")?;
    let cutover_candidate_validation =
        load_latest_step_value(&pool, cutover_run_id, "candidate_validation", "output")
            .await?
            .context("cutover run missing candidate_validation output")?;
    let cutover_truth_adjudication =
        load_latest_step_value(&pool, cutover_run_id, "truth_adjudication", "output")
            .await?
            .context("cutover run missing truth_adjudication output")?;
    let cutover_verified_write =
        load_latest_step_value(&pool, cutover_run_id, "verified_truth_write", "output")
            .await?
            .context("cutover run missing verified_truth_write output")?;
    let cutover_contradiction_gate =
        load_latest_step_value(&pool, cutover_run_id, "contradiction_gate", "output")
            .await?
            .context("cutover run missing contradiction_gate output")?;
    let cutover_projection_barrier = load_latest_step_value(
        &pool,
        cutover_run_id,
        "projection_barrier(semantic_projection)",
        "output",
    )
    .await?
    .context("cutover run missing projection_barrier(semantic_projection) output")?;
    let cutover_candidate_validation_input =
        load_latest_step_value(&pool, cutover_run_id, "candidate_validation", "input")
            .await?
            .context("cutover run missing candidate_validation input")?;

    let legacy_context_key = value_as_str(&legacy_raw_ingestion_input, "context_key")
        .map(ToOwned::to_owned)
        .context("legacy raw_knowledge_ingestion input missing context_key")?;
    let cutover_context_key = value_as_str(&cutover_candidate_validation_input, "context_key")
        .map(ToOwned::to_owned)
        .context("cutover candidate_validation input missing context_key")?;
    let legacy_raw_page_ids = value_as_i64_vec(&legacy_crawl, "raw_page_ids");
    let cutover_raw_page_ids =
        value_as_i64_vec(&cutover_candidate_validation_input, "raw_page_ids");

    let legacy_candidate_counts =
        load_candidate_status_counts(&pool, &legacy_context_key, &legacy_raw_page_ids).await?;
    let cutover_projection_status = projection_status_value(&pool, cutover_run_id).await?;
    let legacy_projection_status = projection_status_value(&pool, legacy_run_id).await?;
    let legacy_publish_events = cms_publish_event_summary(&pool, legacy_run_id).await?;
    let cutover_publish_events = cms_publish_event_summary(&pool, cutover_run_id).await?;
    let cutover_step_statuses = list_run_step_statuses(&pool, cutover_run_id).await?;
    let legacy_step_statuses = list_run_step_statuses(&pool, legacy_run_id).await?;
    let cutover_step_counts = step_execution_counts(&pool, cutover_run_id).await?;
    let legacy_step_counts = step_execution_counts(&pool, legacy_run_id).await?;

    let mut findings = Vec::new();
    if legacy_context_key != cutover_context_key {
        findings.push(error_finding(
            "SHADOW_CONTEXT_MISMATCH",
            format!(
                "legacy context_key `{legacy_context_key}` does not match cutover context_key `{cutover_context_key}`"
            ),
        ));
    }
    if legacy_raw_page_ids != cutover_raw_page_ids {
        findings.push(warning_finding(
            "SHADOW_RAW_PAGE_SCOPE_DIFF",
            format!(
                "legacy raw_page_ids ({}) and cutover raw_page_ids ({}) differ",
                legacy_raw_page_ids.len(),
                cutover_raw_page_ids.len()
            ),
        ));
    }

    let missing_cutover_steps = CUTOVER_PHASE_M1_LEDGER_STEPS
        .iter()
        .filter(|step_name| cutover_step_statuses.get(**step_name) != Some(&"done".to_string()))
        .map(|step_name| (*step_name).to_string())
        .collect::<Vec<_>>();
    if !missing_cutover_steps.is_empty() {
        findings.push(error_finding(
            "SHADOW_CUTOVER_STEP_COVERAGE",
            format!(
                "cutover run is missing completed ledger steps: {}",
                missing_cutover_steps.join(", ")
            ),
        ));
    }

    let legacy_verified = value_as_u64(&legacy_raw_ingestion, "verified_rule_count").unwrap_or(0);
    let legacy_changed_truth_keys =
        value_as_string_vec(&legacy_raw_ingestion, "changed_truth_keys").len() as u64;
    let legacy_needs_hitl = value_as_i64(&legacy_candidate_counts, "needs_hitl_count")
        .unwrap_or_default()
        .max(0) as u64;

    let cutover_verified =
        value_as_u64(&cutover_verified_write, "verified_rule_count").unwrap_or(0);
    let cutover_changed_truth_keys =
        value_as_string_vec(&cutover_verified_write, "changed_truth_keys").len() as u64;
    let cutover_needs_hitl =
        value_as_u64(&cutover_candidate_validation, "needs_hitl_count").unwrap_or(0);
    let cutover_contradictions =
        value_as_u64(&cutover_contradiction_gate, "conflict_count").unwrap_or(0);

    if cutover_verified > legacy_verified {
        findings.push(error_finding(
            "SHADOW_VERIFIED_INCREASE_UNJUSTIFIED",
            format!(
                "cutover verified_rule_count {} exceeds legacy verified_rule_count {}",
                cutover_verified, legacy_verified
            ),
        ));
    } else if cutover_verified < legacy_verified {
        findings.push(warning_finding(
            "SHADOW_VERIFIED_COUNT_LOWER",
            format!(
                "cutover verified_rule_count {} is lower than legacy verified_rule_count {}",
                cutover_verified, legacy_verified
            ),
        ));
    }

    if cutover_needs_hitl < legacy_needs_hitl {
        findings.push(warning_finding(
            "SHADOW_NEEDS_HITL_LOWER",
            format!(
                "cutover needs_hitl_count {} is lower than legacy needs_hitl_count {}; verify that no ambiguity was silently accepted",
                cutover_needs_hitl, legacy_needs_hitl
            ),
        ));
    }

    let barrier_blocked_events =
        value_as_i64(&cutover_projection_barrier, "blocked_events").unwrap_or_default();
    let barrier_status = value_as_str(&cutover_projection_barrier, "status")
        .unwrap_or("unknown")
        .to_string();
    if barrier_status != "clear" || barrier_blocked_events > 0 {
        findings.push(error_finding(
            "SHADOW_PROJECTION_BARRIER_BLOCKED",
            format!(
                "cutover semantic projection barrier status={} blocked_events={}",
                barrier_status, barrier_blocked_events
            ),
        ));
    }

    let cutover_truth_verified =
        value_as_u64(&cutover_truth_adjudication, "verified_count").unwrap_or(0);
    if cutover_truth_verified != cutover_verified {
        findings.push(warning_finding(
            "SHADOW_TRUTH_WRITE_DELTA",
            format!(
                "truth_adjudication verified_count {} differs from verified_truth_write verified_rule_count {}",
                cutover_truth_verified, cutover_verified
            ),
        ));
    }

    let cutover_m2_observed =
        value_as_i64(&cutover_publish_events, "review_requested_events").unwrap_or_default() > 0
            || CUTOVER_PHASE_M2_LEDGER_STEPS.iter().any(|step_name| {
                cutover_step_statuses.get(*step_name) == Some(&"done".to_string())
            });
    let legacy_m2_observed =
        value_as_i64(&legacy_publish_events, "review_requested_events").unwrap_or_default() > 0
            || LEGACY_PHASE_M2_LEDGER_STEPS.iter().any(|step_name| {
                legacy_step_statuses.get(*step_name) == Some(&"done".to_string())
            });

    let m1_raw_scope_required = !(legacy_m2_observed
        && cutover_m2_observed
        && legacy_raw_page_ids.is_empty()
        && cutover_raw_page_ids.is_empty());
    if legacy_raw_page_ids.is_empty() && m1_raw_scope_required {
        findings.push(error_finding(
            "SHADOW_LEGACY_RAW_PAGES_MISSING",
            "legacy crawl_sources output has no raw_page_ids",
        ));
    }
    if cutover_raw_page_ids.is_empty() && m1_raw_scope_required {
        findings.push(error_finding(
            "SHADOW_CUTOVER_RAW_PAGES_MISSING",
            "cutover candidate_validation input has no raw_page_ids",
        ));
    }

    let cutover_missing_m2_steps = if cutover_m2_observed {
        CUTOVER_PHASE_M2_LEDGER_STEPS
            .iter()
            .filter(|step_name| cutover_step_statuses.get(**step_name) != Some(&"done".to_string()))
            .map(|step_name| (*step_name).to_string())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let legacy_missing_m2_steps = if legacy_m2_observed {
        LEGACY_PHASE_M2_LEDGER_STEPS
            .iter()
            .filter(|step_name| legacy_step_statuses.get(**step_name) != Some(&"done".to_string()))
            .map(|step_name| (*step_name).to_string())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let legacy_review_requested_pages = *legacy_step_counts.get("load_cms_approval_decision").unwrap_or(&0);
    let legacy_approved_pages = *legacy_step_counts.get("finalize_publish").unwrap_or(&0);
    let cutover_review_requested_pages = *cutover_step_counts.get("human_approval_wait").unwrap_or(&0);
    let cutover_approved_pages = *cutover_step_counts.get("finalize_publish").unwrap_or(&0);
    let legacy_blocked_pages =
        value_as_i64(&legacy_publish_events, "blocked_pages").unwrap_or_default();
    let cutover_blocked_pages =
        value_as_i64(&cutover_publish_events, "blocked_pages").unwrap_or_default();

    if cutover_m2_observed {
        if !cutover_missing_m2_steps.is_empty() {
            findings.push(error_finding(
                "SHADOW_CUTOVER_PHASE_M2_STEP_COVERAGE",
                format!(
                    "cutover run is missing completed Phase M2 steps: {}",
                    cutover_missing_m2_steps.join(", ")
                ),
            ));
        }
        if cutover_review_requested_pages == 0 {
            findings.push(error_finding(
                "SHADOW_CUTOVER_PHASE_M2_NO_REVIEW_REQUESTS",
                "cutover run emitted no seo_page_review_requested events",
            ));
        }
        if cutover_blocked_pages > 0 {
            findings.push(error_finding(
                "SHADOW_CUTOVER_PHASE_M2_BLOCKED_PAGES",
                format!(
                    "cutover run still has {} blocked publish pages",
                    cutover_blocked_pages
                ),
            ));
        }
        if cutover_approved_pages != cutover_review_requested_pages {
            findings.push(error_finding(
                "SHADOW_CUTOVER_PHASE_M2_APPROVAL_COUNT_MISMATCH",
                format!(
                    "cutover approved_pages {} differs from review_requested_pages {}",
                    cutover_approved_pages, cutover_review_requested_pages
                ),
            ));
        }
    }

    if legacy_m2_observed {
        if !legacy_missing_m2_steps.is_empty() {
            findings.push(error_finding(
                "SHADOW_LEGACY_PHASE_M2_STEP_COVERAGE",
                format!(
                    "legacy run is missing completed Phase M2 steps: {}",
                    legacy_missing_m2_steps.join(", ")
                ),
            ));
        }
        if legacy_review_requested_pages == 0 {
            findings.push(error_finding(
                "SHADOW_LEGACY_PHASE_M2_NO_REVIEW_REQUESTS",
                "legacy run emitted no seo_page_review_requested events",
            ));
        }
        if legacy_blocked_pages > 0 {
            findings.push(error_finding(
                "SHADOW_LEGACY_PHASE_M2_BLOCKED_PAGES",
                format!(
                    "legacy run still has {} blocked publish pages",
                    legacy_blocked_pages
                ),
            ));
        }
        if legacy_approved_pages != legacy_review_requested_pages {
            findings.push(error_finding(
                "SHADOW_LEGACY_PHASE_M2_APPROVAL_COUNT_MISMATCH",
                format!(
                    "legacy approved_pages {} differs from review_requested_pages {}",
                    legacy_approved_pages, legacy_review_requested_pages
                ),
            ));
        }
    }

    if legacy_m2_observed && cutover_m2_observed && legacy_approved_pages != cutover_approved_pages {
        findings.push(error_finding(
            "SHADOW_PHASE_M2_PUBLISHED_PAGE_COUNT_DIFF",
            format!(
                "legacy approved_pages {} differs from cutover approved_pages {}",
                legacy_approved_pages, cutover_approved_pages
            ),
        ));
    }

    let status = status_from_findings(&findings, strict);
    let payload = json!({
        "status": status,
        "strict": strict,
        "legacy_run_id": legacy_run_id,
        "cutover_run_id": cutover_run_id,
        "context_key": {
            "legacy": legacy_context_key,
            "cutover": cutover_context_key,
        },
        "phase_m1": {
            "raw_scope_required_for_verdict": m1_raw_scope_required,
            "expected_ledger_steps": CUTOVER_PHASE_M1_LEDGER_STEPS,
            "non_ledgered_activity_steps": CUTOVER_PHASE_M1_NON_LEDGERED_ACTIVITY_STEPS,
            "completed_ledger_steps": cutover_step_statuses
                .iter()
                .filter(|(_, status)| status.as_str() == "done")
                .map(|(step_name, _)| step_name.clone())
                .collect::<Vec<_>>(),
            "missing_ledger_steps": missing_cutover_steps,
        },
        "phase_m2": {
            "legacy": {
                "observed": legacy_m2_observed,
                "expected_ledger_steps": LEGACY_PHASE_M2_LEDGER_STEPS,
                "missing_ledger_steps": legacy_missing_m2_steps,
                "publish_events": legacy_publish_events,
                "step_counts": legacy_step_counts,
            },
            "cutover": {
                "observed": cutover_m2_observed,
                "expected_ledger_steps": CUTOVER_PHASE_M2_LEDGER_STEPS,
                "missing_ledger_steps": cutover_missing_m2_steps,
                "publish_events": cutover_publish_events,
                "step_counts": cutover_step_counts,
            }
        },
        "legacy_runtime": {
            "crawl_sources": legacy_crawl,
            "raw_knowledge_ingestion": legacy_raw_ingestion,
            "candidate_status_counts": legacy_candidate_counts,
            "projection_status": legacy_projection_status,
            "summary": {
                "verified_rule_count": legacy_verified,
                "needs_hitl_count": legacy_needs_hitl,
                "changed_truth_key_count": legacy_changed_truth_keys,
            }
        },
        "cutover_runtime": {
            "raw_evidence_register": cutover_raw_evidence,
            "candidate_validation": cutover_candidate_validation,
            "truth_adjudication": cutover_truth_adjudication,
            "verified_truth_write": cutover_verified_write,
            "contradiction_gate": cutover_contradiction_gate,
            "projection_barrier_semantic_projection": cutover_projection_barrier,
            "projection_status": cutover_projection_status,
            "support_refresh": {
                "expected": cutover_changed_truth_keys > 0,
                "observed_via_step_ledger": false,
                "note": "load_verified_support_bundle.* remains a non-ledgered activity surface"
            },
            "summary": {
                "verified_rule_count": cutover_verified,
                "needs_hitl_count": cutover_needs_hitl,
                "contradiction_conflict_count": cutover_contradictions,
                "changed_truth_key_count": cutover_changed_truth_keys,
            }
        },
        "findings": findings.iter().map(|finding| json!({
            "level": finding.level,
            "code": finding.code,
            "message": finding.message,
        })).collect::<Vec<_>>()
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    if let Some(report_json) = report_json.as_deref() {
        let out = write_report(Path::new("."), report_json, &payload)?;
        eprintln!("report: {}", out.display());
    }

    Ok(if status == "ok" || !strict { 0 } else { 2 })
}

async fn cms_review_decision(
    database_url: Option<String>,
    page_node_key: &str,
    actor_role: &str,
    decision: &str,
    reason: &str,
) -> Result<(String, String, String)> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    let workflow = TemporalSeoWorkflowControlAdapter::new(
        default_temporal_url(),
        format!("alegria-cli-tools@{}", std::process::id()),
        default_temporal_namespace(),
    );
    let result = apply_human_review_decision(
        &repo,
        &workflow,
        &ApplyHumanReviewDecisionInput {
            page_node_key: page_node_key.to_string(),
            actor_role: actor_role.to_string(),
            decision: decision.to_string(),
            reason: reason.to_string(),
            source: "cli_tools_headless_cms@1".to_string(),
        },
    )
    .await
    .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    Ok((result.decision_key, result.revision_id, result.workflow_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_path_uses_directory_indexes() {
        assert_eq!(output_path_for_url("/").unwrap(), "index.html");
        assert_eq!(
            output_path_for_url("/guides/poland-visa/").unwrap(),
            "guides/poland-visa/index.html"
        );
    }

    #[test]
    fn output_path_rejects_traversal() {
        assert!(output_path_for_url("/../secret").is_err());
        assert!(output_path_for_url("/safe?x=1").is_err());
    }

    #[test]
    fn markdown_renderer_outputs_html() {
        let html = markdown_to_html("# Title\n\nBody with **strong** text.");
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<strong>strong</strong>"));
    }

    #[test]
    fn static_artifacts_include_sitemap_and_manifest() {
        let pages = vec![StaticCmsPageRow {
            page_node_key: "page_home".to_string(),
            canonical_url_path: "/".to_string(),
            locale_code: "en".to_string(),
            page_type_key: "home".to_string(),
            dominant_intent_key: "overview".to_string(),
            current_status: "approved".to_string(),
            cms_document_id: "doc_home".to_string(),
            revision_id: "rev_home".to_string(),
            title: "Home".to_string(),
            meta_description: "Home page".to_string(),
            h1: "Home".to_string(),
            body_payload: json!({
                "markdown": "Welcome.\n\n## FAQ\n\n### Is this reviewed?\n\nYes, before publish."
            }),
            schema_markup_payload: json!({}),
            updated_at: "2026-05-06T00:00:00Z".to_string(),
        }];

        let artifacts = build_static_artifacts(&pages, &[], "https://alegria.test").unwrap();
        let paths: Vec<&str> = artifacts
            .iter()
            .map(|artifact| artifact.relative_path.as_str())
            .collect();
        assert!(paths.contains(&"index.html"));
        assert!(paths.contains(&"sitemap.xml"));
        assert!(paths.contains(&"robots.txt"));
        assert!(paths.contains(&"alegria-static-manifest.json"));
        let home = artifacts
            .iter()
            .find(|artifact| artifact.relative_path == "index.html")
            .unwrap();
        let html = String::from_utf8(home.bytes.clone()).unwrap();
        assert!(html.contains("BreadcrumbList"));
        assert!(html.contains("FAQPage"));
        assert!(html.contains("class=\"breadcrumbs\""));
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let code = match cli.command {
        Command::CheckRustMigrationContract {
            root,
            report_json,
            strict,
        } => check_rust_migration_contract(Path::new(&root), &report_json, strict)?,
        Command::CheckFactVerifierParity { root, report_json } => {
            check_fact_verifier_parity(Path::new(&root), &report_json)?
        }
        Command::ComputeContentHash { text } => {
            println!("{}", primitives::hash::content_hash_v1(&text));
            0
        }
        Command::ComputeBytesHash { hex } => {
            let bytes = hex::decode(&hex).context("invalid hex for ComputeBytesHash")?;
            println!("{}", primitives::hash::blake3_hex(&bytes));
            0
        }
        Command::ComputeStableRuleId { parts } => {
            let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
            println!("{}", primitives::stable_id::stable_rule_instance_id(&refs));
            0
        }
        Command::EncodeFactInput { sections_json } => {
            let bytes = FactExtractionInputPayload { sections_json }.encode_to_vec();
            print_hex(&bytes);
            0
        }
        Command::EncodeValidationInput {
            required_links_json,
            required_keys_json,
            used_rule_keys_json,
            used_fact_keys_json,
            url_norm,
        } => {
            let bytes = ValidationInputPayload {
                meta: None,
                required_links_json_utf8: required_links_json.into_bytes(),
                required_keys_json_utf8: required_keys_json.into_bytes(),
                used_rule_keys_json_utf8: used_rule_keys_json.into_bytes(),
                used_fact_keys_json_utf8: used_fact_keys_json.into_bytes(),
                url_norm,
            }
            .encode_to_vec();
            print_hex(&bytes);
            0
        }
        Command::ReconcileTargetSystem {
            target_system,
            dry_run,
            max_retry_count,
            batch_limit,
            requeue_base_delay_sec,
            requeue_jitter_sec,
        } => {
            let report = reconcile_target_system_default(
                &target_system,
                &ReconcileOptionsRecord {
                    max_retry_count,
                    batch_limit,
                    dry_run,
                    requeue_base_delay_sec,
                    requeue_jitter_sec,
                },
            )
            .await?;
            println!(
                "{}|{}|{}|{}|{}|{}",
                report.target_system,
                report.dry_run,
                report.stale_candidates,
                report.failed_candidates,
                report.reset_stale_processing,
                report.requeued_failed
            );
            0
        }
        Command::BuildStaticSite {
            database_url,
            output_dir,
            base_url,
        } => build_static_site(database_url, &output_dir, &base_url).await?,
        Command::CrawlPendingSources {
            database_url,
            run_id,
            query_batch_key,
            limit,
            emit_qdrant,
        } => {
            crawl_pending_sources(database_url, run_id, query_batch_key, limit, emit_qdrant).await?
        }
        Command::CmsReviewList {
            database_url,
            limit,
        } => cms_review_list(database_url, limit).await?,
        Command::CmsReviewShow {
            database_url,
            page_node_key,
        } => cms_review_show(database_url, &page_node_key).await?,
        Command::CmsPublishStatus {
            database_url,
            page_node_key,
        } => cms_publish_status(database_url, &page_node_key).await?,
        Command::CmsTraceabilityInspect {
            database_url,
            page_node_key,
        } => cms_traceability_inspect(database_url, &page_node_key).await?,
        Command::CmsBlockersInspect {
            database_url,
            page_node_key,
        } => cms_blockers_inspect(database_url, &page_node_key).await?,
        Command::SeoRebuildBacklogInspect {
            database_url,
            page_node_key,
            limit,
        } => seo_rebuild_backlog_inspect(database_url, page_node_key, limit).await?,
        Command::SeoSupportBundleInspect {
            database_url,
            context_key,
        } => seo_support_bundle_inspect(database_url, &context_key).await?,
        Command::SeoPostPublishFeedbackProbe {
            analytics_addr,
            require_gsc,
            report_json,
        } => seo_post_publish_feedback_probe(&analytics_addr, require_gsc, report_json).await?,
        Command::SeoReleaseRestoreGate {
            root,
            run_ci_verify,
            run_temporal_gate,
            run_restore_drill,
            report_json,
        } => seo_release_restore_gate(
            Path::new(&root),
            run_ci_verify,
            run_temporal_gate,
            run_restore_drill,
            report_json,
        )?,
        Command::SeoCutoverShadowVerify {
            database_url,
            legacy_run_id,
            cutover_run_id,
            strict,
            report_json,
        } => {
            seo_cutover_shadow_verify(
                database_url,
                &legacy_run_id,
                &cutover_run_id,
                strict,
                report_json,
            )
            .await?
        }
        Command::CmsApprovePublish {
            database_url,
            page_node_key,
            actor_role,
            reason,
            output_dir,
            base_url,
        } => {
            let _keep_smoke_happy = (&output_dir, &base_url);
            let (decision_key, revision_id, workflow_id) = cms_review_decision(
                database_url.clone(),
                &page_node_key,
                &actor_role,
                "approved",
                &reason,
            )
            .await?;
            println!(
                "CMS_APPROVE_PUBLISH: OK decision={} revision={} workflow={} mode=workflow_owned_publish",
                decision_key, revision_id, workflow_id
            );
            0
        }
        Command::CmsBlock {
            database_url,
            page_node_key,
            actor_role,
            reason,
        } => {
            let (decision_key, revision_id, workflow_id) = cms_review_decision(
                database_url,
                &page_node_key,
                &actor_role,
                "blocked",
                &reason,
            )
            .await?;
            println!(
                "CMS_BLOCK: OK decision={} revision={} workflow={}",
                decision_key, revision_id, workflow_id
            );
            0
        }
        Command::CmsReopen {
            database_url,
            page_node_key,
            actor_role,
            reason,
        } => {
            let (decision_key, revision_id, workflow_id) = cms_review_decision(
                database_url,
                &page_node_key,
                &actor_role,
                "reopened",
                &reason,
            )
            .await?;
            println!(
                "CMS_REOPEN: OK decision={} revision={} workflow={}",
                decision_key, revision_id, workflow_id
            );
            0
        }
        Command::SeoRun {
            scenario,
            database_url,
            run_id,
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
            output_dir,
            base_url,
            publish,
            require_preapproved_decision,
            warn_only_projections,
        } => {
            seo_run(
                scenario,
                database_url,
                run_id,
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
                output_dir,
                base_url,
                publish,
                require_preapproved_decision,
                warn_only_projections,
            )
            .await?
        }
    };

    if code == 0 {
        Ok(())
    } else {
        std::process::exit(code)
    }
}
