use contracts::generated::alegria::sync::v1::Neo4jRuleUpsertPayload;
use contracts::generated::alegria::temporal::v1::{
    CmsApprovalDecision, CmsPublishInputPayload, CmsPublishOutputPayload, CmsReviewPage,
    ContentBlockPlanState, ContentContractValidateInputPayload,
    ContentContractValidateOutputPayload, CrawlSourcesInputPayload, CrawlSourcesOutputPayload,
    DraftAssembleInputPayload, DraftAssembleOutputPayload, DraftNormalizeInputPayload,
    DraftNormalizeOutputPayload, DraftQaInputPayload, DraftQaOutputPayload,
    EditorialDraftGenerateInputPayload, EditorialDraftGenerateOutputPayload, ExtractedPayloadState,
    FactExtractionInputPayload, FactValueState, FinalizePublishInputPayload,
    FinalizePublishOutputPayload, FreshnessReport, GenerationBlockState, GenerationResultState,
    GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload, HitlDecision, HitlPauseInfo,
    HitlResolutionInput, IaBuildInputPayload, IaBuildOutputPayload, LinkRecommendInputPayload,
    LinkRecommendOutputPayload, LlmDraftCandidate, LlmDraftRequest, OpportunityBuildInputPayload,
    OpportunityBuildOutputPayload, PersistReport, PublishArtifact, PublishMaterializeInputPayload,
    PublishMaterializeOutputPayload, RawKnowledgeIngestionInputPayload,
    RawKnowledgeIngestionOutputPayload, ProjectionBarrierAuditInputPayload,
    ProjectionBarrierAuditOutputPayload, RebuildDetectInputPayload, RebuildDetectOutputPayload,
    ReconcileSummaryPayload, ReconcileTargetInputPayload, ReconcileTargetReportPayload,
    RenderPreviewValidateInputPayload, RenderPreviewValidateOutputPayload,
    RuleInstanceCandidateState, RuleParamsState, RuleRoleTypeV1, RuntimeErrorPayload,
    SeoSiteBuildInputPayload, SerpIngestInputPayload, SerpIngestOutputPayload,
    SerpNormalizeInputPayload, SerpNormalizeOutputPayload, StringPayload, ValidationInputPayload,
    ValidationReport, VerifyReport,
};
use prost::Message;
use serde_json::Value;
use std::collections::BTreeMap;

use super::sqlx_outbox_adapter::OutboxEnvelope;
use primitives::errors::DomainError;
use primitives::hash::{blake3_hex, content_hash_v1};
use runtime_models::{
    ExecutionRunBlob, ExtractedPayload, FactCandidateValue, ReconcileSummary,
    ReconcileTargetReportRecord, RuleInstanceCandidate, RuleParams as RuntimeRuleParams,
    RuleRoleType, ValidationInputRecord,
};

pub(crate) fn encode_payload<T: Message>(value: &T) -> Vec<u8> {
    value.encode_to_vec()
}

pub trait RuntimeProtoPayload: Sized {
    fn payload_type() -> &'static str;
    fn schema_version() -> i32 {
        1
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError>;
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError>;
}

pub(crate) fn encode_runtime_payload<T: RuntimeProtoPayload>(
    value: &T,
) -> std::result::Result<(Vec<u8>, String), DomainError> {
    let payload_bytes = value.encode_payload_bytes()?;
    let payload_hash = blake3_hex(&payload_bytes);
    Ok((payload_bytes, payload_hash))
}

pub(crate) fn neo4j_rule_upserted(rule_instance_id: &str, context_key: &str) -> OutboxEnvelope {
    let payload_bytes = encode_payload(&Neo4jRuleUpsertPayload {
        rule_instance_id: rule_instance_id.to_string(),
        context_key: context_key.to_string(),
    });
    OutboxEnvelope {
        run_id: String::new(),
        aggregate_type: "rule_instance".to_string(),
        aggregate_key: rule_instance_id.to_string(),
        target_system: "neo4j".to_string(),
        event_type: "RuleInstanceUpserted".to_string(),
        payload_type: "alegria.outbox.neo4j_rule_upserted.v1".to_string(),
        schema_version: 1,
        idempotency_key: content_hash_v1(&format!(
            "{rule_instance_id}|RuleInstanceUpserted|{}",
            primitives::hash::blake3_hex(&payload_bytes)
        )),
        payload_bytes,
    }
}

pub(crate) fn contract_violation(message: impl Into<String>) -> DomainError {
    DomainError::ContractViolation {
        message: message.into(),
    }
}

pub(crate) fn validation_failure(message: impl Into<String>) -> DomainError {
    DomainError::ValidationFailure {
        message: message.into(),
    }
}

pub(crate) fn infra_unavailable(message: impl Into<String>) -> DomainError {
    DomainError::InfraUnavailable {
        message: message.into(),
    }
}

pub(crate) fn classify_sqlx(err: sqlx::Error) -> DomainError {
    match err {
        sqlx::Error::RowNotFound => validation_failure("expected row was not found"),
        other => infra_unavailable(other.to_string()),
    }
}

pub(crate) fn decode_prost<T: Message + Default>(
    payload_bytes: &[u8],
    label: &str,
) -> std::result::Result<T, DomainError> {
    T::decode(payload_bytes)
        .map_err(|e| contract_violation(format!("failed to decode {label}: {e}")))
}

fn encode_rule_params_state(params: &RuntimeRuleParams) -> RuleParamsState {
    match params {
        RuntimeRuleParams::Fee {
            amount, currency, ..
        } => RuleParamsState {
            amount: Some(*amount),
            currency: Some(currency.clone()),
            days: None,
            extra_json_utf8: Default::default(),
        },
        RuntimeRuleParams::Timeline { days, .. } => RuleParamsState {
            amount: None,
            currency: None,
            days: Some(*days),
            extra_json_utf8: Default::default(),
        },
        _ => RuleParamsState {
            amount: None,
            currency: None,
            days: None,
            extra_json_utf8: Default::default(),
        },
    }
}

fn decode_rule_params_state(params: Option<RuleParamsState>) -> RuntimeRuleParams {
    let Some(params) = params else {
        return RuntimeRuleParams::None;
    };
    if let Some(amount) = params.amount {
        return RuntimeRuleParams::Fee {
            amount,
            currency: params.currency.unwrap_or_else(|| "EUR".to_string()),
            severity: "unspecified".to_string(),
            channel: None,
            conditions_key: String::new(),
        };
    }
    if let Some(days) = params.days {
        return RuntimeRuleParams::Timeline {
            days,
            subtype: None,
            severity: "unspecified".to_string(),
            conditions_key: String::new(),
        };
    }
    RuntimeRuleParams::None
}

fn encode_rule_role_type(role: RuleRoleType) -> i32 {
    match role {
        RuleRoleType::MustProvide => RuleRoleTypeV1::MustProvide as i32,
        RuleRoleType::MustPay => RuleRoleTypeV1::MustPay as i32,
        RuleRoleType::MustSatisfy => RuleRoleTypeV1::MustSatisfy as i32,
        RuleRoleType::Allows => RuleRoleTypeV1::Allows as i32,
        RuleRoleType::Forbids => RuleRoleTypeV1::Forbids as i32,
        RuleRoleType::Timeline => RuleRoleTypeV1::Timeline as i32,
        RuleRoleType::DocumentRequired => RuleRoleTypeV1::DocumentRequired as i32,
        RuleRoleType::EligibilityRule => RuleRoleTypeV1::EligibilityRule as i32,
        RuleRoleType::FeeItem => RuleRoleTypeV1::FeeItem as i32,
        RuleRoleType::TimelineItem => RuleRoleTypeV1::TimelineItem as i32,
        RuleRoleType::WhereToApply => RuleRoleTypeV1::WhereToApply as i32,
        RuleRoleType::AppointmentRule => RuleRoleTypeV1::AppointmentRule as i32,
        RuleRoleType::FormRequired => RuleRoleTypeV1::FormRequired as i32,
        RuleRoleType::Step => RuleRoleTypeV1::Step as i32,
    }
}

fn decode_rule_role_type(value: i32) -> std::result::Result<RuleRoleType, DomainError> {
    let role = RuleRoleTypeV1::try_from(value)
        .map_err(|_| contract_violation(format!("unknown RuleRoleTypeV1 enum value: {value}")))?;
    Ok(match role {
        RuleRoleTypeV1::MustProvide => RuleRoleType::MustProvide,
        RuleRoleTypeV1::MustPay => RuleRoleType::MustPay,
        RuleRoleTypeV1::MustSatisfy => RuleRoleType::MustSatisfy,
        RuleRoleTypeV1::Allows => RuleRoleType::Allows,
        RuleRoleTypeV1::Forbids => RuleRoleType::Forbids,
        RuleRoleTypeV1::Timeline => RuleRoleType::Timeline,
        RuleRoleTypeV1::DocumentRequired => RuleRoleType::DocumentRequired,
        RuleRoleTypeV1::EligibilityRule => RuleRoleType::EligibilityRule,
        RuleRoleTypeV1::FeeItem => RuleRoleType::FeeItem,
        RuleRoleTypeV1::TimelineItem => RuleRoleType::TimelineItem,
        RuleRoleTypeV1::WhereToApply => RuleRoleType::WhereToApply,
        RuleRoleTypeV1::AppointmentRule => RuleRoleType::AppointmentRule,
        RuleRoleTypeV1::FormRequired => RuleRoleType::FormRequired,
        RuleRoleTypeV1::Step => RuleRoleType::Step,
        RuleRoleTypeV1::Unspecified => {
            return Err(contract_violation(
                "unspecified RuleRoleTypeV1 is not allowed",
            ));
        }
    })
}

fn encode_fact_value_state(value: &FactCandidateValue) -> FactValueState {
    match value {
        FactCandidateValue::Null => FactValueState {
            value: Some(
                contracts::generated::alegria::temporal::v1::fact_value_state::Value::NullValue(
                    true,
                ),
            ),
        },
        FactCandidateValue::Integer(v) => FactValueState {
            value: Some(
                contracts::generated::alegria::temporal::v1::fact_value_state::Value::IntegerValue(
                    *v,
                ),
            ),
        },
        FactCandidateValue::Decimal(v) => FactValueState {
            value: Some(
                contracts::generated::alegria::temporal::v1::fact_value_state::Value::DecimalValue(
                    *v,
                ),
            ),
        },
        FactCandidateValue::Text(v) => FactValueState {
            value: Some(
                contracts::generated::alegria::temporal::v1::fact_value_state::Value::TextValue(
                    v.clone(),
                ),
            ),
        },
        FactCandidateValue::Boolean(v) => FactValueState {
            value: Some(
                contracts::generated::alegria::temporal::v1::fact_value_state::Value::BooleanValue(
                    *v,
                ),
            ),
        },
    }
}

fn decode_fact_value_state(state: FactValueState) -> FactCandidateValue {
    use contracts::generated::alegria::temporal::v1::fact_value_state::Value;
    match state.value {
        Some(Value::IntegerValue(v)) => FactCandidateValue::Integer(v),
        Some(Value::DecimalValue(v)) => FactCandidateValue::Decimal(v),
        Some(Value::TextValue(v)) => FactCandidateValue::Text(v),
        Some(Value::BooleanValue(v)) => FactCandidateValue::Boolean(v),
        Some(Value::NullValue(_)) | None => FactCandidateValue::Null,
    }
}

impl RuntimeProtoPayload for String {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.StringPayload"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(StringPayload {
            value: self.clone(),
        }
        .encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        Ok(decode_prost::<StringPayload>(payload_bytes, "StringPayload")?.value)
    }
}

impl RuntimeProtoPayload for &str {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.StringPayload"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(StringPayload {
            value: (*self).to_string(),
        }
        .encode_to_vec())
    }

    fn decode_payload_bytes(_payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        Err(contract_violation(
            "cannot decode borrowed &str runtime payload",
        ))
    }
}

impl RuntimeProtoPayload for VerifyReport {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.VerifyReport"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "VerifyReport")
    }
}

impl RuntimeProtoPayload for PersistReport {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.PersistReport"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "PersistReport")
    }
}

impl RuntimeProtoPayload for HitlPauseInfo {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.HitlPauseInfo"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "HitlPauseInfo")
    }
}

impl RuntimeProtoPayload for HitlResolutionInput {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.HitlResolutionInput"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "HitlResolutionInput")
    }
}

impl RuntimeProtoPayload for HitlDecision {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.HitlDecision"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "HitlDecision")
    }
}

impl RuntimeProtoPayload for ValidationInputPayload {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ValidationInputPayload"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "ValidationInputPayload")
    }
}

impl RuntimeProtoPayload for ValidationReport {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ValidationReport"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "ValidationReport")
    }
}

impl RuntimeProtoPayload for FreshnessReport {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.FreshnessReport"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "FreshnessReport")
    }
}

impl RuntimeProtoPayload for ReconcileTargetInputPayload {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ReconcileTargetInputPayload"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "ReconcileTargetInputPayload")
    }
}

impl RuntimeProtoPayload for ProjectionBarrierAuditInputPayload {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ProjectionBarrierAuditInputPayload"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "ProjectionBarrierAuditInputPayload")
    }
}

impl RuntimeProtoPayload for ProjectionBarrierAuditOutputPayload {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ProjectionBarrierAuditOutputPayload"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "ProjectionBarrierAuditOutputPayload")
    }
}

impl RuntimeProtoPayload for ExtractedPayload {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ExtractedPayloadState"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        let payload = ExtractedPayloadState {
            rule_instances: self
                .rule_instances
                .iter()
                .map(|rule| RuleInstanceCandidateState {
                    rule_type_key: rule.rule_type_key.clone(),
                    concept_key: rule.concept_key.clone(),
                    role_type: encode_rule_role_type(rule.role_type),
                    params: Some(encode_rule_params_state(&rule.params)),
                    status: rule.status.clone(),
                    source_key: rule.source_key.clone(),
                    condition_expr: rule.condition_expr.clone(),
                })
                .collect(),
            facts: self.facts.iter().map(encode_fact_value_state).collect(),
        };
        Ok(payload.encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        let payload =
            decode_prost::<ExtractedPayloadState>(payload_bytes, "ExtractedPayloadState")?;
        let mut rules = Vec::with_capacity(payload.rule_instances.len());
        for rule in payload.rule_instances {
            let role_type = decode_rule_role_type(rule.role_type)?;
            rules.push(RuleInstanceCandidate {
                rule_type_key: rule.rule_type_key,
                concept_key: rule.concept_key,
                role_type,
                params: decode_rule_params_state(rule.params),
                status: rule.status,
                source_key: rule.source_key,
                condition_expr: rule.condition_expr,
            });
        }
        Ok(ExtractedPayload {
            rule_instances: rules,
            facts: payload
                .facts
                .into_iter()
                .map(decode_fact_value_state)
                .collect(),
        })
    }
}

impl RuntimeProtoPayload for BTreeMap<String, String> {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.GenerationResultState"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(GenerationResultState {
            blocks: self
                .iter()
                .map(|(block_key, html)| GenerationBlockState {
                    block_key: block_key.clone(),
                    html: html.clone(),
                })
                .collect(),
        }
        .encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        let payload =
            decode_prost::<GenerationResultState>(payload_bytes, "GenerationResultState")?;
        Ok(payload
            .blocks
            .into_iter()
            .map(|block| (block.block_key, block.html))
            .collect())
    }
}

impl RuntimeProtoPayload for ReconcileSummary {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ReconcileSummaryPayload"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(ReconcileSummaryPayload {
            open_dlq: self.open_dlq,
            stale_runs: self.stale_runs,
            pending_hitl_runs: self.pending_hitl_runs,
            stuck_steps: self.stuck_steps,
        }
        .encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        let payload =
            decode_prost::<ReconcileSummaryPayload>(payload_bytes, "ReconcileSummaryPayload")?;
        Ok(Self {
            open_dlq: payload.open_dlq,
            stale_runs: payload.stale_runs,
            pending_hitl_runs: payload.pending_hitl_runs,
            stuck_steps: payload.stuck_steps,
        })
    }
}

impl RuntimeProtoPayload for ReconcileTargetReportRecord {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.ReconcileTargetReportPayload"
    }

    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(ReconcileTargetReportPayload {
            target_system: self.target_system.clone(),
            dry_run: self.dry_run,
            stale_candidates: self.stale_candidates,
            failed_candidates: self.failed_candidates,
            reset_stale_processing: self.reset_stale_processing,
            requeued_failed: self.requeued_failed,
        }
        .encode_to_vec())
    }

    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        let payload = decode_prost::<ReconcileTargetReportPayload>(
            payload_bytes,
            "ReconcileTargetReportPayload",
        )?;
        Ok(Self {
            target_system: payload.target_system,
            dry_run: payload.dry_run,
            stale_candidates: payload.stale_candidates,
            failed_candidates: payload.failed_candidates,
            reset_stale_processing: payload.reset_stale_processing,
            requeued_failed: payload.requeued_failed,
        })
    }
}

impl RuntimeProtoPayload for RuntimeErrorPayload {
    fn payload_type() -> &'static str {
        "alegria.temporal.v1.RuntimeErrorPayload"
    }
    fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
        Ok(self.encode_to_vec())
    }
    fn decode_payload_bytes(payload_bytes: &[u8]) -> std::result::Result<Self, DomainError> {
        decode_prost(payload_bytes, "RuntimeErrorPayload")
    }
}

macro_rules! impl_prost_runtime_payload {
    ($ty:ty, $name:literal) => {
        impl RuntimeProtoPayload for $ty {
            fn payload_type() -> &'static str {
                $name
            }
            fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
                Ok(self.encode_to_vec())
            }
            fn decode_payload_bytes(
                payload_bytes: &[u8],
            ) -> std::result::Result<Self, DomainError> {
                decode_prost(payload_bytes, stringify!($ty))
            }
        }
    };
}

impl_prost_runtime_payload!(
    SeoSiteBuildInputPayload,
    "alegria.temporal.v1.SeoSiteBuildInputPayload"
);
impl_prost_runtime_payload!(
    SerpIngestInputPayload,
    "alegria.temporal.v1.SerpIngestInputPayload"
);
impl_prost_runtime_payload!(
    SerpIngestOutputPayload,
    "alegria.temporal.v1.SerpIngestOutputPayload"
);
impl_prost_runtime_payload!(
    CrawlSourcesInputPayload,
    "alegria.temporal.v1.CrawlSourcesInputPayload"
);
impl_prost_runtime_payload!(
    CrawlSourcesOutputPayload,
    "alegria.temporal.v1.CrawlSourcesOutputPayload"
);
impl_prost_runtime_payload!(
    RawKnowledgeIngestionInputPayload,
    "alegria.temporal.v1.RawKnowledgeIngestionInputPayload"
);
impl_prost_runtime_payload!(
    RawKnowledgeIngestionOutputPayload,
    "alegria.temporal.v1.RawKnowledgeIngestionOutputPayload"
);
impl_prost_runtime_payload!(
    GlobalSiteReconcileInputPayload,
    "alegria.temporal.v1.GlobalSiteReconcileInputPayload"
);
impl_prost_runtime_payload!(
    GlobalSiteReconcileOutputPayload,
    "alegria.temporal.v1.GlobalSiteReconcileOutputPayload"
);
impl_prost_runtime_payload!(
    SerpNormalizeInputPayload,
    "alegria.temporal.v1.SerpNormalizeInputPayload"
);
impl_prost_runtime_payload!(
    SerpNormalizeOutputPayload,
    "alegria.temporal.v1.SerpNormalizeOutputPayload"
);
impl_prost_runtime_payload!(
    OpportunityBuildInputPayload,
    "alegria.temporal.v1.OpportunityBuildInputPayload"
);
impl_prost_runtime_payload!(
    OpportunityBuildOutputPayload,
    "alegria.temporal.v1.OpportunityBuildOutputPayload"
);
impl_prost_runtime_payload!(
    IaBuildInputPayload,
    "alegria.temporal.v1.IaBuildInputPayload"
);
impl_prost_runtime_payload!(
    IaBuildOutputPayload,
    "alegria.temporal.v1.IaBuildOutputPayload"
);
impl_prost_runtime_payload!(
    LinkRecommendInputPayload,
    "alegria.temporal.v1.LinkRecommendInputPayload"
);
impl_prost_runtime_payload!(
    LinkRecommendOutputPayload,
    "alegria.temporal.v1.LinkRecommendOutputPayload"
);
impl_prost_runtime_payload!(
    DraftAssembleInputPayload,
    "alegria.temporal.v1.DraftAssembleInputPayload"
);
impl_prost_runtime_payload!(
    DraftAssembleOutputPayload,
    "alegria.temporal.v1.DraftAssembleOutputPayload"
);
impl_prost_runtime_payload!(
    DraftNormalizeInputPayload,
    "alegria.temporal.v1.DraftNormalizeInputPayload"
);
impl_prost_runtime_payload!(
    DraftNormalizeOutputPayload,
    "alegria.temporal.v1.DraftNormalizeOutputPayload"
);
impl_prost_runtime_payload!(
    ContentBlockPlanState,
    "alegria.temporal.v1.ContentBlockPlanState"
);
impl_prost_runtime_payload!(LlmDraftRequest, "alegria.temporal.v1.LlmDraftRequest");
impl_prost_runtime_payload!(LlmDraftCandidate, "alegria.temporal.v1.LlmDraftCandidate");
impl_prost_runtime_payload!(
    EditorialDraftGenerateInputPayload,
    "alegria.temporal.v1.EditorialDraftGenerateInputPayload"
);
impl_prost_runtime_payload!(
    EditorialDraftGenerateOutputPayload,
    "alegria.temporal.v1.EditorialDraftGenerateOutputPayload"
);
impl_prost_runtime_payload!(
    DraftQaInputPayload,
    "alegria.temporal.v1.DraftQaInputPayload"
);
impl_prost_runtime_payload!(
    ContentContractValidateInputPayload,
    "alegria.temporal.v1.ContentContractValidateInputPayload"
);
impl_prost_runtime_payload!(
    ContentContractValidateOutputPayload,
    "alegria.temporal.v1.ContentContractValidateOutputPayload"
);
impl_prost_runtime_payload!(
    DraftQaOutputPayload,
    "alegria.temporal.v1.DraftQaOutputPayload"
);
impl_prost_runtime_payload!(
    CmsPublishInputPayload,
    "alegria.temporal.v1.CmsPublishInputPayload"
);
impl_prost_runtime_payload!(
    CmsPublishOutputPayload,
    "alegria.temporal.v1.CmsPublishOutputPayload"
);
impl_prost_runtime_payload!(CmsReviewPage, "alegria.temporal.v1.CmsReviewPage");
impl_prost_runtime_payload!(
    CmsApprovalDecision,
    "alegria.temporal.v1.CmsApprovalDecision"
);
impl_prost_runtime_payload!(PublishArtifact, "alegria.temporal.v1.PublishArtifact");
impl_prost_runtime_payload!(
    PublishMaterializeInputPayload,
    "alegria.temporal.v1.PublishMaterializeInputPayload"
);
impl_prost_runtime_payload!(
    PublishMaterializeOutputPayload,
    "alegria.temporal.v1.PublishMaterializeOutputPayload"
);
impl_prost_runtime_payload!(
    RenderPreviewValidateInputPayload,
    "alegria.temporal.v1.RenderPreviewValidateInputPayload"
);
impl_prost_runtime_payload!(
    RenderPreviewValidateOutputPayload,
    "alegria.temporal.v1.RenderPreviewValidateOutputPayload"
);
impl_prost_runtime_payload!(
    FinalizePublishInputPayload,
    "alegria.temporal.v1.FinalizePublishInputPayload"
);
impl_prost_runtime_payload!(
    FinalizePublishOutputPayload,
    "alegria.temporal.v1.FinalizePublishOutputPayload"
);
impl_prost_runtime_payload!(
    RebuildDetectInputPayload,
    "alegria.temporal.v1.RebuildDetectInputPayload"
);
impl_prost_runtime_payload!(
    RebuildDetectOutputPayload,
    "alegria.temporal.v1.RebuildDetectOutputPayload"
);

pub fn decode_extracted_payload(payload: Option<&ExecutionRunBlob>) -> ExtractedPayload {
    let Some(payload) = payload else {
        return ExtractedPayload::default();
    };
    if payload.payload_type != <ExtractedPayload as RuntimeProtoPayload>::payload_type() {
        return ExtractedPayload::default();
    }
    ExtractedPayload::decode_payload_bytes(&payload.payload_bytes).unwrap_or_default()
}

pub fn decode_verify_report(payload: Option<&ExecutionRunBlob>) -> Option<VerifyReport> {
    let payload = payload?;
    if payload.payload_type != <VerifyReport as RuntimeProtoPayload>::payload_type() {
        return None;
    }
    VerifyReport::decode_payload_bytes(&payload.payload_bytes).ok()
}

pub fn extracted_payload_rules_json(payload: &ExtractedPayload) -> String {
    let rules = payload
        .rule_instances
        .iter()
        .map(|r| {
            serde_json::json!({
                "rule_type_key": r.rule_type_key,
                "concept_key": r.concept_key,
                "role_type": r.role_type.as_str(),
                "params": r.params.as_json_value(),
                "status": r.status,
                "source_key": r.source_key,
                "condition_expr": r.condition_expr,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&rules).unwrap_or_else(|_| "[]".to_string())
}

pub fn extracted_payload_facts_json(payload: &ExtractedPayload) -> String {
    let facts = payload
        .facts
        .iter()
        .map(FactCandidateValue::as_json_value)
        .collect::<Vec<_>>();
    serde_json::to_string(&facts).unwrap_or_else(|_| "[]".to_string())
}

pub fn extract_sections_json(input_payload: Option<&ExecutionRunBlob>) -> String {
    let Some(payload) = input_payload else {
        return "[]".to_string();
    };
    if payload.payload_type != "alegria.temporal.v1.FactExtractionInputPayload" {
        return "[]".to_string();
    }
    decode_prost::<FactExtractionInputPayload>(&payload.payload_bytes, "FactExtractionInputPayload")
        .map(|v| v.sections_json)
        .unwrap_or_else(|_| "[]".to_string())
}

pub fn decode_validation_input(input_payload: Option<&ExecutionRunBlob>) -> ValidationInputRecord {
    let Some(payload) = input_payload else {
        return ValidationInputRecord::default();
    };
    if payload.payload_type != <ValidationInputPayload as RuntimeProtoPayload>::payload_type() {
        return ValidationInputRecord::default();
    }
    let payload = match ValidationInputPayload::decode_payload_bytes(&payload.payload_bytes) {
        Ok(v) => v,
        Err(_) => return ValidationInputRecord::default(),
    };
    ValidationInputRecord {
        required_links_json: String::from_utf8(payload.required_links_json_utf8)
            .unwrap_or_else(|_| "[]".to_string()),
        required_keys_json: String::from_utf8(payload.required_keys_json_utf8)
            .unwrap_or_else(|_| "[]".to_string()),
        used_rule_keys_json: String::from_utf8(payload.used_rule_keys_json_utf8)
            .unwrap_or_else(|_| "[]".to_string()),
        used_fact_keys_json: String::from_utf8(payload.used_fact_keys_json_utf8)
            .unwrap_or_else(|_| "[]".to_string()),
        url_norm: payload.url_norm,
    }
}

pub fn json_string(v: Option<&Value>) -> String {
    serde_json::to_string(v.unwrap_or(&Value::Null)).unwrap_or_else(|_| "null".to_string())
}

pub fn decode_generation_result(
    generation_result: Option<&ExecutionRunBlob>,
) -> BTreeMap<String, String> {
    let Some(payload) = generation_result else {
        return BTreeMap::new();
    };
    if payload.payload_type != <BTreeMap<String, String> as RuntimeProtoPayload>::payload_type() {
        return BTreeMap::new();
    }
    <BTreeMap<String, String> as RuntimeProtoPayload>::decode_payload_bytes(&payload.payload_bytes)
        .unwrap_or_default()
}
