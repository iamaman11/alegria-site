use std::collections::BTreeMap;

use contracts::generated::alegria::temporal::v1::{FreshnessReport, SeoScopePayload, StepContractMeta};
use infrastructure::adapters::raw_crawl_adapter;
use infrastructure::adapters::sqlx_freshness_adapter::load_freshness_snapshot;
use infrastructure::adapters::sqlx_pipeline_runtime_adapter::RuntimeProtoPayload;
use infrastructure::adapters::sqlx_reconcile_adapter;
use infrastructure::adapters::sqlx_seo_adapter;
use primitives::errors::DomainError;
use primitives::hash::content_hash_v1;
use runtime_models::ReconcileTargetReportRecord;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use super::AlegriaActivities;

macro_rules! impl_json_runtime_payload_local {
    ($ty:ty, $payload_type:expr) => {
        impl RuntimeProtoPayload for $ty {
            fn payload_type() -> &'static str {
                $payload_type
            }

            fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
                serde_json::to_vec(self).map_err(|e| DomainError::ContractViolation {
                    message: e.to_string(),
                })
            }

            fn decode_payload_bytes(
                payload_bytes: &[u8],
            ) -> std::result::Result<Self, DomainError> {
                serde_json::from_slice(payload_bytes).map_err(|e| DomainError::ContractViolation {
                    message: e.to_string(),
                })
            }
        }
    };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neo4jBackwriteInput {
    pub target_system: String,
    pub dry_run: bool,
    pub max_retry_count: Option<i32>,
    pub batch_limit: Option<i64>,
    pub requeue_base_delay_sec: Option<i64>,
    pub requeue_jitter_sec: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neo4jBackwriteOutput {
    pub target_system: String,
    pub dry_run: bool,
    pub stale_candidates: i64,
    pub failed_candidates: i64,
    pub reset_stale_processing: i64,
    pub requeued_failed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSectionSampleInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSectionSampleOutput {
    pub section_id: String,
    pub page_id: i64,
    pub source_url: String,
    pub source_domain: String,
    pub heading_path: String,
    pub section_type: String,
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeoPreflightInput {
    pub run_id: String,
    pub context_key: String,
    pub scope: SeoScopePayload,
    pub projection_max_lag_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeoPreflightOutput {
    pub context_key: String,
    pub normalized_profile: String,
    pub page_type_count: i64,
    pub page_node_count: i64,
    pub navigation_item_count: i64,
    pub verified_rule_count: i64,
    pub pending_rule_count: i64,
    pub qdrant_point_count: i64,
    pub projection_blocked: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WholePageSemanticPassInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WholePageSemanticPageState {
    pub page_id: i64,
    pub section_count: usize,
    pub page_mode_hint: String,
    pub dominant_layers: Vec<String>,
    pub page_summary: String,
    pub global_entities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WholePageSemanticPassOutput {
    pub page_count: usize,
    pub pages: Vec<WholePageSemanticPageState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub source_url: String,
    pub heading_path: String,
    pub section_type: String,
    pub content_hash: String,
    pub text_len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningOutput {
    pub page_count: usize,
    pub section_count: usize,
    pub sections: Vec<SectioningSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageUtilitySweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageUtilitySectionDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub allow_procedural_extraction: bool,
    pub allow_editorial_extraction: bool,
    pub allow_structural_extraction: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageUtilitySweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub decisions: Vec<PageUtilitySectionDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomBlockRelevanceSweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomBlockRelevanceSectionDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub block_role: String,
    pub allow_extraction: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomBlockRelevanceSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub decisions: Vec<DomBlockRelevanceSectionDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningContractGateInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningContractDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub heading_present: bool,
    pub text_present: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningContractGateOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub decisions: Vec<SectioningContractDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasGateInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasGateDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub snapshot_hash: String,
    pub is_replay_safe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasGateOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub decisions: Vec<CasGateDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvidenceRegisterInput {
    pub run_id: String,
    pub context_key: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvidenceRegisterOutput {
    pub context_key: String,
    pub page_count: usize,
    pub section_count: usize,
    pub unique_source_count: usize,
    pub evidence_refs: Vec<String>,
    pub status: String,
}

impl_json_runtime_payload_local!(
    SemanticSectionSampleInput,
    "alegria.runtime.json.SemanticSectionSampleInput"
);
impl_json_runtime_payload_local!(
    SemanticSectionSampleOutput,
    "alegria.runtime.json.SemanticSectionSampleOutput"
);
impl_json_runtime_payload_local!(SeoPreflightInput, "alegria.runtime.json.SeoPreflightInput");
impl_json_runtime_payload_local!(SeoPreflightOutput, "alegria.runtime.json.SeoPreflightOutput");
impl_json_runtime_payload_local!(
    WholePageSemanticPassInput,
    "alegria.runtime.json.WholePageSemanticPassInput"
);
impl_json_runtime_payload_local!(
    WholePageSemanticPassOutput,
    "alegria.runtime.json.WholePageSemanticPassOutput"
);
impl_json_runtime_payload_local!(SectioningInput, "alegria.runtime.json.SectioningInput");
impl_json_runtime_payload_local!(SectioningOutput, "alegria.runtime.json.SectioningOutput");
impl_json_runtime_payload_local!(
    PageUtilitySweepInput,
    "alegria.runtime.json.PageUtilitySweepInput"
);
impl_json_runtime_payload_local!(
    PageUtilitySweepOutput,
    "alegria.runtime.json.PageUtilitySweepOutput"
);
impl_json_runtime_payload_local!(
    DomBlockRelevanceSweepInput,
    "alegria.runtime.json.DomBlockRelevanceSweepInput"
);
impl_json_runtime_payload_local!(
    DomBlockRelevanceSweepOutput,
    "alegria.runtime.json.DomBlockRelevanceSweepOutput"
);
impl_json_runtime_payload_local!(
    SectioningContractGateInput,
    "alegria.runtime.json.SectioningContractGateInput"
);
impl_json_runtime_payload_local!(
    SectioningContractGateOutput,
    "alegria.runtime.json.SectioningContractGateOutput"
);
impl_json_runtime_payload_local!(CasGateInput, "alegria.runtime.json.CasGateInput");
impl_json_runtime_payload_local!(CasGateOutput, "alegria.runtime.json.CasGateOutput");
impl_json_runtime_payload_local!(
    RawEvidenceRegisterInput,
    "alegria.runtime.json.RawEvidenceRegisterInput"
);
impl_json_runtime_payload_local!(
    RawEvidenceRegisterOutput,
    "alegria.runtime.json.RawEvidenceRegisterOutput"
);

fn dominant_layers(text: &str) -> Vec<String> {
    let lowered = text.to_lowercase();
    let mut layers = Vec::new();
    if ["паспорт", "страхов", "анкет", "fee", "eur", "сбор", "visa", "виза"]
        .iter()
        .any(|token| lowered.contains(token))
    {
        layers.push("procedural".to_string());
    }
    if ["schedule", "график", "holiday", "appointment", "запись", "время работы"]
        .iter()
        .any(|token| lowered.contains(token))
    {
        layers.push("operational".to_string());
    }
    if ["faq", "что делать", "почему", "ошибк", "отказ", "проблем"]
        .iter()
        .any(|token| lowered.contains(token))
    {
        layers.push("editorial".to_string());
    }
    if layers.is_empty() {
        layers.push("procedural".to_string());
    }
    layers
}

fn page_mode_hint(text: &str) -> String {
    let lowered = text.to_lowercase();
    if lowered.contains("sitemap") || lowered.contains("directory") || lowered.contains("каталог")
    {
        "directory_page".to_string()
    } else if lowered.contains("breadcrumb")
        || lowered.contains("menu")
        || lowered.contains("навигац")
    {
        "menu_page".to_string()
    } else if lowered.contains("privacy")
        || lowered.contains("cookie")
        || lowered.contains("login")
    {
        "utility_page".to_string()
    } else if lowered.contains("consultation") || lowered.contains("book now") {
        "landing_page".to_string()
    } else {
        "content_page".to_string()
    }
}

fn summarize(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(180).collect())
        .unwrap_or_default()
}

fn global_entities(text: &str) -> Vec<String> {
    let lowered = text.to_lowercase();
    let mut entities = Vec::new();
    for token in ["passport", "паспорт", "insurance", "страхов", "vfs", "посольств"] {
        if lowered.contains(token) {
            entities.push(token.to_string());
        }
    }
    entities.sort();
    entities.dedup();
    entities
}

fn block_role_for_section(
    section: &raw_crawl_adapter::RawSectionRecord,
) -> seo_steps::dom_block_relevance_step::BlockRole {
    let section_type = section.section_type.to_lowercase();
    let heading = section.heading_path.to_lowercase();
    if section_type.contains("nav") || heading.contains("breadcrumb") {
        seo_steps::dom_block_relevance_step::BlockRole::Navigation
    } else if section_type.contains("toc") {
        seo_steps::dom_block_relevance_step::BlockRole::Toc
    } else if section_type.contains("footer") {
        seo_steps::dom_block_relevance_step::BlockRole::Footer
    } else {
        seo_steps::dom_block_relevance_step::BlockRole::ContentMain
    }
}

pub(crate) fn test_step_prepare_impl(workflow_id: &str) -> String {
    format!("prepared:{workflow_id}")
}

pub(crate) fn test_step_finalize_impl(prepared_token: &str) -> String {
    format!("completed:{prepared_token}")
}

pub(crate) async fn check_data_freshness_impl(
    acts: &AlegriaActivities,
    threshold_input: &str,
) -> Result<String, DomainError> {
    let threshold_hours: i64 = threshold_input
        .parse::<i64>()
        .ok()
        .filter(|v| *v > 0)
        .unwrap_or(24);

    let snapshot = load_freshness_snapshot(&acts.pool, threshold_hours)
        .await
        .map_err(AlegriaActivities::classify_error)?;

    let report = FreshnessReport {
        meta: Some(StepContractMeta {
            run_id: "operational:freshness".to_string(),
            step_name: "check_data_freshness".to_string(),
            schema_version: 1,
            input_hash: content_hash_v1(threshold_input),
            output_hash: String::new(),
            idempotency_key: content_hash_v1(&format!("operational:freshness|{}", threshold_input)),
            requires_hitl: false,
            prompt_version: String::new(),
            model_version: String::new(),
            registry_version: String::new(),
            error_class: String::new(),
            retry_class: "transient".to_string(),
            executor_version: AlegriaActivities::current_build_id(),
            derivation_version: "check_data_freshness@1".to_string(),
            scope_signature: String::new(),
            max_retries: 3,
        }),
        threshold_hours,
        stale_count: snapshot.stale_count,
        max_lag_hours: snapshot.max_lag_hours,
        status: if snapshot.stale_count > 0 {
            "stale"
        } else {
            "ok"
        }
        .to_string(),
    };

    serde_json::to_string(&report).map_err(AlegriaActivities::classify_error)
}

pub(crate) async fn neo4j_backwrite_impl(
    input: &Neo4jBackwriteInput,
) -> Result<Neo4jBackwriteOutput, DomainError> {
    let mut opts = sqlx_reconcile_adapter::load_default_reconcile_options();
    opts.dry_run = input.dry_run;
    if let Some(v) = input.max_retry_count {
        opts.max_retry_count = v;
    }
    if let Some(v) = input.batch_limit {
        opts.batch_limit = v;
    }
    if let Some(v) = input.requeue_base_delay_sec {
        opts.requeue_base_delay_sec = v;
    }
    if let Some(v) = input.requeue_jitter_sec {
        opts.requeue_jitter_sec = v;
    }

    let target = if input.target_system.trim().is_empty() {
        "neo4j"
    } else {
        input.target_system.as_str()
    };
    let report = sqlx_reconcile_adapter::reconcile_target_system_default(target, &opts)
        .await
        .map_err(AlegriaActivities::classify_error)?;

    Ok(Neo4jBackwriteOutput {
        target_system: report.target_system,
        dry_run: report.dry_run,
        stale_candidates: report.stale_candidates,
        failed_candidates: report.failed_candidates,
        reset_stale_processing: report.reset_stale_processing,
        requeued_failed: report.requeued_failed,
    })
}

pub(crate) async fn projection_reconcile_impl(
    target_system: &str,
    dry_run: bool,
    max_retry_count: i32,
    batch_limit: i64,
    requeue_base_delay_sec: i64,
    requeue_jitter_sec: i64,
) -> Result<ReconcileTargetReportRecord, DomainError> {
    let report = sqlx_reconcile_adapter::reconcile_target_system_default(
        target_system,
        &sqlx_reconcile_adapter::ReconcileOptionsRecord {
            max_retry_count,
            batch_limit,
            dry_run,
            requeue_base_delay_sec,
            requeue_jitter_sec,
        },
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

    Ok(ReconcileTargetReportRecord {
        target_system: report.target_system,
        dry_run: report.dry_run,
        stale_candidates: report.stale_candidates,
        failed_candidates: report.failed_candidates,
        reset_stale_processing: report.reset_stale_processing,
        requeued_failed: report.requeued_failed,
    })
}

pub(crate) async fn load_semantic_section_sample_impl(
    acts: &AlegriaActivities,
    input: &SemanticSectionSampleInput,
) -> Result<SemanticSectionSampleOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids)
            .await
            .map_err(AlegriaActivities::classify_error)?;
    let section = sections
        .into_iter()
        .find(|section| !section.content_md.trim().is_empty())
        .ok_or_else(|| DomainError::ValidationFailure {
            message: "no non-empty raw section available for semantic slice".to_string(),
        })?;

    Ok(SemanticSectionSampleOutput {
        section_id: section.id.to_string(),
        page_id: section.page_id,
        source_url: section.source_url,
        source_domain: section.source_domain,
        heading_path: section.heading_path,
        section_type: section.section_type,
        raw_text: section.content_md,
    })
}

pub(crate) async fn seo_preflight_impl(
    acts: &AlegriaActivities,
    input: &SeoPreflightInput,
) -> Result<SeoPreflightOutput, DomainError> {
    sqlx_seo_adapter::ensure_seo_runtime_registries(&acts.pool).await?;
    let scope = seo_domain::identity::derive_scope_from_payload(&input.scope)?;
    let normalized_profile =
        sqlx_seo_adapter::validate_applicant_profile_reference(&acts.pool, &scope.applicant_profile)
            .await?;

    let context_row = sqlx::query(
        "SELECT count(*)::bigint AS count FROM kb.visa_contexts WHERE context_key = $1 AND status = 'active'",
    )
    .bind(&input.context_key)
    .fetch_one(&*acts.pool)
    .await
    .map_err(AlegriaActivities::classify_error)?;
    let context_count: i64 = context_row.get("count");
    if context_count == 0 {
        return Err(DomainError::ValidationFailure {
            message: format!("active context is missing for `{}`", input.context_key),
        });
    }

    let page_type_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM site.registry_page_types WHERE status = 'active'",
    )
    .fetch_one(&*acts.pool)
    .await
    .map_err(AlegriaActivities::classify_error)?;
    let page_node_count: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM site.page_nodes")
        .fetch_one(&*acts.pool)
        .await
        .map_err(AlegriaActivities::classify_error)?;
    let navigation_item_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM site.navigation_items")
            .fetch_one(&*acts.pool)
            .await
            .map_err(AlegriaActivities::classify_error)?;
    let verified_rule_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM verified.rule_instances WHERE context_key = $1 AND status = 'verified'",
    )
    .bind(&input.context_key)
    .fetch_one(&*acts.pool)
    .await
    .map_err(AlegriaActivities::classify_error)?;
    let pending_rule_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM verified.rule_instances WHERE context_key = $1 AND status = 'pending'",
    )
    .bind(&input.context_key)
    .fetch_one(&*acts.pool)
    .await
    .map_err(AlegriaActivities::classify_error)?;
    let qdrant_point_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM kb.qdrant_points")
            .fetch_one(&*acts.pool)
            .await
            .map_err(AlegriaActivities::classify_error)?;

    let projection_statuses = sqlx_seo_adapter::read_projection_sync_status(&acts.pool).await?;
    let projection_blocked = projection_statuses.iter().any(|status| {
        status.failed_events > 0
            || (status.open_event_count() > 0
                && status.max_open_lag_ms > input.projection_max_lag_ms)
    });

    Ok(SeoPreflightOutput {
        context_key: input.context_key.clone(),
        normalized_profile,
        page_type_count,
        page_node_count,
        navigation_item_count,
        verified_rule_count,
        pending_rule_count,
        qdrant_point_count,
        projection_blocked,
        status: if projection_blocked { "warn" } else { "ok" }.to_string(),
    })
}

pub(crate) async fn whole_page_semantic_pass_impl(
    acts: &AlegriaActivities,
    input: &WholePageSemanticPassInput,
) -> Result<WholePageSemanticPassOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let mut by_page: BTreeMap<i64, Vec<raw_crawl_adapter::RawSectionRecord>> = BTreeMap::new();
    for section in sections {
        by_page.entry(section.page_id).or_default().push(section);
    }
    let pages = by_page
        .into_iter()
        .map(|(page_id, sections)| {
            let combined = sections
                .iter()
                .map(|section| section.content_md.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            WholePageSemanticPageState {
                page_id,
                section_count: sections.len(),
                page_mode_hint: page_mode_hint(&combined),
                dominant_layers: dominant_layers(&combined),
                page_summary: summarize(&combined),
                global_entities: global_entities(&combined),
            }
        })
        .collect::<Vec<_>>();
    Ok(WholePageSemanticPassOutput {
        page_count: pages.len(),
        pages,
    })
}

pub(crate) async fn sectioning_impl(
    acts: &AlegriaActivities,
    input: &SectioningInput,
) -> Result<SectioningOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let page_count = sections
        .iter()
        .map(|section| section.page_id)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let mapped = sections
        .iter()
        .map(|section| SectioningSectionState {
            section_id: section.id,
            page_id: section.page_id,
            source_url: section.source_url.clone(),
            heading_path: section.heading_path.clone(),
            section_type: section.section_type.clone(),
            content_hash: section.content_hash.clone(),
            text_len: section.content_md.len(),
        })
        .collect::<Vec<_>>();
    Ok(SectioningOutput {
        page_count,
        section_count: mapped.len(),
        sections: mapped,
    })
}

pub(crate) async fn page_utility_sweep_impl(
    acts: &AlegriaActivities,
    input: &PageUtilitySweepInput,
) -> Result<PageUtilitySweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let decisions = sections
        .iter()
        .map(|section| {
            let output = seo_steps::page_utility_classifier_step::execute(
                &seo_steps::page_utility_classifier_step::PageUtilityClassifierInput {
                    url: section.source_url.clone(),
                    title: section.heading_path.clone(),
                    raw_text: section.content_md.clone(),
                },
            );
            PageUtilitySectionDecision {
                section_id: section.id,
                page_id: section.page_id,
                allow_procedural_extraction: output.allow_procedural_extraction,
                allow_editorial_extraction: output.allow_editorial_extraction,
                allow_structural_extraction: output.allow_structural_extraction,
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| !decision.allow_structural_extraction)
        .count();
    Ok(PageUtilitySweepOutput {
        section_count: decisions.len(),
        blocked_section_count,
        decisions,
    })
}

pub(crate) async fn dom_block_relevance_sweep_impl(
    acts: &AlegriaActivities,
    input: &DomBlockRelevanceSweepInput,
) -> Result<DomBlockRelevanceSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let decisions = sections
        .iter()
        .map(|section| {
            let role = block_role_for_section(section);
            let output = seo_steps::dom_block_relevance_step::execute(&[
                seo_steps::dom_block_relevance_step::DomBlockInput {
                    dom_block_id: format!("raw-section:{}", section.id),
                    block_role: role.clone(),
                    text: section.content_md.clone(),
                },
            ]);
            let block = output.blocks.into_iter().next().expect("single block");
            DomBlockRelevanceSectionDecision {
                section_id: section.id,
                page_id: section.page_id,
                block_role: format!("{:?}", role).to_lowercase(),
                allow_extraction: block.allow_extraction,
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| !decision.allow_extraction)
        .count();
    Ok(DomBlockRelevanceSweepOutput {
        section_count: decisions.len(),
        blocked_section_count,
        decisions,
    })
}

pub(crate) async fn sectioning_contract_gate_impl(
    acts: &AlegriaActivities,
    input: &SectioningContractGateInput,
) -> Result<SectioningContractGateOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let decisions = sections
        .iter()
        .map(|section| {
            let heading_present = !section.heading_path.trim().is_empty();
            let text_present = !section.content_md.trim().is_empty();
            SectioningContractDecision {
                section_id: section.id,
                page_id: section.page_id,
                heading_present,
                text_present,
                decision: if heading_present && text_present {
                    "pass".to_string()
                } else {
                    "blocked".to_string()
                },
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| decision.decision != "pass")
        .count();
    Ok(SectioningContractGateOutput {
        section_count: decisions.len(),
        blocked_section_count,
        decisions,
    })
}

pub(crate) async fn cas_gate_impl(
    acts: &AlegriaActivities,
    input: &CasGateInput,
) -> Result<CasGateOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let decisions = sections
        .iter()
        .map(|section| CasGateDecision {
            section_id: section.id,
            page_id: section.page_id,
            snapshot_hash: if section.content_hash.trim().is_empty() {
                content_hash_v1(&section.content_md)
            } else {
                section.content_hash.clone()
            },
            is_replay_safe: !section.content_md.trim().is_empty(),
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| !decision.is_replay_safe)
        .count();
    Ok(CasGateOutput {
        section_count: decisions.len(),
        blocked_section_count,
        decisions,
    })
}

pub(crate) async fn raw_evidence_register_impl(
    acts: &AlegriaActivities,
    input: &RawEvidenceRegisterInput,
) -> Result<RawEvidenceRegisterOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let page_count = sections
        .iter()
        .map(|section| section.page_id)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let unique_source_count = sections
        .iter()
        .map(|section| section.source_url.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let evidence_refs = sections
        .iter()
        .map(|section| format!("raw.section:{}", section.id))
        .collect::<Vec<_>>();
    Ok(RawEvidenceRegisterOutput {
        context_key: input.context_key.clone(),
        page_count,
        section_count: evidence_refs.len(),
        unique_source_count,
        evidence_refs,
        status: "registered".to_string(),
    })
}
