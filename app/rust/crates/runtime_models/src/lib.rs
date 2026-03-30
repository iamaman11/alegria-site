use chrono::{DateTime, Utc};
use contracts::wire::condition::ConditionExprV1;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleRoleType {
    #[default]
    MustProvide,
    MustPay,
    MustSatisfy,
    Allows,
    Forbids,
    Timeline,
    DocumentRequired,
    EligibilityRule,
    FeeItem,
    TimelineItem,
    WhereToApply,
    AppointmentRule,
    FormRequired,
    Step,
}

impl RuleRoleType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MustProvide => "must_provide",
            Self::MustPay => "must_pay",
            Self::MustSatisfy => "must_satisfy",
            Self::Allows => "allows",
            Self::Forbids => "forbids",
            Self::Timeline => "timeline",
            Self::DocumentRequired => "document_required",
            Self::EligibilityRule => "eligibility_rule",
            Self::FeeItem => "fee_item",
            Self::TimelineItem => "timeline_item",
            Self::WhereToApply => "where_to_apply",
            Self::AppointmentRule => "appointment_rule",
            Self::FormRequired => "form_required",
            Self::Step => "step",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "must_provide" => Some(Self::MustProvide),
            "must_pay" => Some(Self::MustPay),
            "must_satisfy" => Some(Self::MustSatisfy),
            "allows" => Some(Self::Allows),
            "forbids" => Some(Self::Forbids),
            "timeline" => Some(Self::Timeline),
            "document_required" => Some(Self::DocumentRequired),
            "eligibility_rule" => Some(Self::EligibilityRule),
            "fee_item" => Some(Self::FeeItem),
            "timeline_item" => Some(Self::TimelineItem),
            "where_to_apply" => Some(Self::WhereToApply),
            "appointment_rule" => Some(Self::AppointmentRule),
            "form_required" => Some(Self::FormRequired),
            "step" => Some(Self::Step),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct ExecutionRun {
    pub run_id: Uuid,
    pub context_key: String,
    pub status: String,
    pub input_payload: Option<ExecutionRunBlob>,
    pub extracted_payload: Option<ExecutionRunBlob>,
    pub verify_report: Option<ExecutionRunBlob>,
    pub generation_result: Option<ExecutionRunBlob>,
    pub persist_report: Option<ExecutionRunBlob>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ExecutionRunBlob {
    pub payload_type: String,
    pub schema_version: i32,
    pub payload_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistPipelineState {
    pub context_key: String,
    #[serde(default)]
    pub extracted_payload: ExtractedPayload,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractedPayload {
    #[serde(default)]
    pub rule_instances: Vec<RuleInstanceCandidate>,
    #[serde(default)]
    pub facts: Vec<FactCandidateValue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleInstanceCandidate {
    #[serde(default)]
    pub rule_type_key: String,
    #[serde(default)]
    pub concept_key: String,
    #[serde(default = "default_role_type")]
    pub role_type: RuleRoleType,
    #[serde(default)]
    pub params: RuleParams,
    #[serde(default = "default_status_pending")]
    pub status: String,
    #[serde(default)]
    pub source_key: Option<String>,
    #[serde(default)]
    pub condition_expr: Option<ConditionExprV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleParams {
    #[default]
    None,
    Document {
        severity: String,
        subtype: Option<String>,
        notarization_required: bool,
        translation_required: bool,
        accepts_alternatives: bool,
        conditions_key: String,
    },
    Fee {
        amount: f64,
        currency: String,
        severity: String,
        channel: Option<String>,
        conditions_key: String,
    },
    Timeline {
        days: i64,
        subtype: Option<String>,
        severity: String,
        conditions_key: String,
    },
    WhereToApply {
        location_key: String,
        channel: String,
        conditions_key: String,
    },
    EligibilityRule {
        subtype: String,
        severity: String,
        conditions_key: String,
    },
    AppointmentRule {
        subtype: String,
        advance_days: i32,
        conditions_key: String,
    },
    FormRequired {
        form_id: String,
        severity: String,
        conditions_key: String,
    },
    Step {
        step_index: i32,
        subtype: String,
        conditions_key: String,
    },
}

impl RuleParams {
    pub fn primary_value_text(&self) -> Option<String> {
        match self {
            Self::Fee { amount, .. } => Some(amount.to_string()),
            Self::Timeline { days, .. } => Some(days.to_string()),
            _ => None,
        }
    }

    pub fn as_json_value(&self) -> serde_json::Value {
        serde_json::to_value(self)
            .unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(untagged)]
pub enum FactCandidateValue {
    #[default]
    Null,
    Integer(i64),
    Decimal(f64),
    Text(String),
    Boolean(bool),
}

impl FactCandidateValue {
    pub fn as_json_value(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Integer(v) => serde_json::json!(v),
            Self::Decimal(v) => serde_json::json!(v),
            Self::Text(v) => serde_json::json!(v),
            Self::Boolean(v) => serde_json::json!(v),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationInputRecord {
    pub required_links_json: String,
    pub required_keys_json: String,
    pub used_rule_keys_json: String,
    pub used_fact_keys_json: String,
    pub url_norm: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceRegistryRecord {
    pub source_type: String,
    pub trust_level: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxEventStatus {
    pub event_id: String,
    pub status: String,
    pub retry_count: i32,
    pub next_retry_at: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconcileSummary {
    #[serde(default)]
    pub open_dlq: i64,
    #[serde(default)]
    pub stale_runs: i64,
    #[serde(default)]
    pub pending_hitl_runs: i64,
    #[serde(default)]
    pub stuck_steps: i64,
}

#[derive(Debug, Clone)]
pub struct ReconcileTargetReportRecord {
    pub target_system: String,
    pub dry_run: bool,
    pub stale_candidates: i64,
    pub failed_candidates: i64,
    pub reset_stale_processing: i64,
    pub requeued_failed: i64,
}

fn default_role_type() -> RuleRoleType {
    RuleRoleType::MustProvide
}

fn default_status_pending() -> String {
    "pending".to_string()
}
