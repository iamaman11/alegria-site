use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub struct StepExecution {
    pub step_execution_id: i64,
    pub run_id: Uuid,
    pub step_name: String,
    pub schema_version: i32,
    pub input_hash: String,
    pub output_hash: Option<String>,
    pub idempotency_key: String,
    pub status: String,
    pub error_class: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
pub struct StepAttempt {
    pub step_attempt_id: i64,
    pub step_execution_id: i64,
    pub attempt_no: i32,
    pub status: String,
    pub error_class: Option<String>,
    pub error_message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}
