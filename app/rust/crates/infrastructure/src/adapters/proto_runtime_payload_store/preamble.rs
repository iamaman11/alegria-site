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
    OpportunityBuildOutputPayload, PersistReport, ProjectionBarrierAuditInputPayload,
    ProjectionBarrierAuditOutputPayload, PublishArtifact, PublishMaterializeInputPayload,
    PublishMaterializeOutputPayload, RawKnowledgeIngestionInputPayload,
    RawKnowledgeIngestionOutputPayload, RebuildDetectInputPayload, RebuildDetectOutputPayload,
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

