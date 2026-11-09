//! One durable typed inbox for manual ChatGPT responses. No LLM HTTP server.
//! Only Rust constructs the final editorial candidate; the manual response is
//! untrusted source text and is never a verification/approval decision.
use contracts::generated::alegria::temporal::v1::{
    EditorialDraftGenerateInputPayload, EditorialDraftGenerateOutputPayload,
};
use primitives::errors::DomainError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{types::Json, PgPool, Row};

const MANUAL_SCHEMA_VERSION: i32 = 1;
const MANUAL_OPERATION: &str = "editorial_draft";

#[derive(Debug, Clone, Serialize)]
pub struct ManualLlmJob<T> {
    pub task_id: String,
    pub schema_version: i32,
    pub input_sha256: String,
    pub operation: String,
    pub payload: T,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualLlmSubmission {
    pub task_id: String,
    pub schema_version: i32,
    pub input_sha256: String,
    pub response: Value,
}

fn db_error(err: sqlx::Error) -> DomainError {
    DomainError::InfraUnavailable {
        message: format!("manual LLM inbox PostgreSQL: {err}"),
    }
}

fn validation(message: impl Into<String>) -> DomainError {
    DomainError::ValidationFailure {
        message: message.into(),
    }
}

fn pending_output() -> EditorialDraftGenerateOutputPayload {
    EditorialDraftGenerateOutputPayload {
        candidate: None,
        provider_key: "chat_manual".to_string(),
        model_key: "pending".to_string(),
        status: "awaiting_manual".to_string(),
    }
}

async fn job_fingerprint(
    pool: &PgPool,
    input: &EditorialDraftGenerateInputPayload,
) -> Result<(String, String, Value), DomainError> {
    if input.run_id.trim().is_empty() || input.request.is_none() {
        return Err(validation("manual LLM job requires run_id and LlmDraftRequest"));
    }
    let value = serde_json::to_value(input)
        .map_err(|err| validation(format!("encode manual editorial input: {err}")))?;
    // Hash PostgreSQL's canonical JSONB encoding, not a caller-provided hash.
    let row = sqlx::query(
        "SELECT encode(digest(($1::jsonb)::text, 'sha256'), 'hex') AS input_sha256,
                encode(digest('editorial_draft:' || $2 || ':' || ($1::jsonb)::text,
                              'sha256'), 'hex') AS task_id",
    )
    .bind(Json(value.clone()))
    .bind(&input.run_id)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;
    Ok((row.get("task_id"), row.get("input_sha256"), value))
}

pub async fn enqueue_editorial(
    pool: &PgPool,
    input: &EditorialDraftGenerateInputPayload,
) -> Result<EditorialDraftGenerateOutputPayload, DomainError> {
    let (task_id, sha256, value) = job_fingerprint(pool, input).await?;
    sqlx::query(
        "INSERT INTO pipeline.manual_llm_jobs
         (task_id, schema_version, run_id, operation, input_sha256, input_json)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (task_id) DO NOTHING",
    )
    .bind(&task_id)
    .bind(MANUAL_SCHEMA_VERSION)
    .bind(&input.run_id)
    .bind(MANUAL_OPERATION)
    .bind(&sha256)
    .bind(Json(value))
    .execute(pool)
    .await
    .map_err(db_error)?;
    // If the response was already submitted, return the finished typed result.
    read_editorial_result(pool, input).await
}

pub async fn next_editorial(
    pool: &PgPool,
) -> Result<Option<ManualLlmJob<EditorialDraftGenerateInputPayload>>, DomainError> {
    let row = sqlx::query(
        "SELECT task_id, schema_version, input_sha256, operation, input_json
         FROM pipeline.manual_llm_jobs
         WHERE job_status = 'pending' AND operation = $1
         ORDER BY created_at, task_id LIMIT 1",
    )
    .bind(MANUAL_OPERATION)
    .fetch_optional(pool)
    .await
    .map_err(db_error)?;
    row.map(|row| {
        let payload: Value = row.get("input_json");
        let payload = serde_json::from_value(payload)
            .map_err(|err| validation(format!("stored manual job input invalid: {err}")))?;
        Ok(ManualLlmJob {
            task_id: row.get("task_id"),
            schema_version: row.get("schema_version"),
            input_sha256: row.get("input_sha256"),
            operation: row.get("operation"),
            payload,
        })
    })
    .transpose()
}

pub async fn read_editorial_result(
    pool: &PgPool,
    input: &EditorialDraftGenerateInputPayload,
) -> Result<EditorialDraftGenerateOutputPayload, DomainError> {
    let (task_id, sha256, _) = job_fingerprint(pool, input).await?;
    let row = sqlx::query(
        "SELECT input_sha256, job_status, output_json
         FROM pipeline.manual_llm_jobs
         WHERE task_id = $1 AND run_id = $2 AND operation = $3",
    )
    .bind(&task_id)
    .bind(&input.run_id)
    .bind(MANUAL_OPERATION)
    .fetch_optional(pool)
    .await
    .map_err(db_error)?
    .ok_or_else(|| validation("manual LLM task missing; refusing to skip generation"))?;
    let actual_hash: String = row.get("input_sha256");
    if actual_hash != sha256 {
        return Err(validation("manual LLM task input hash changed"));
    }
    let state: String = row.get("job_status");
    if state == "pending" {
        return Ok(pending_output());
    }
    if state != "completed" {
        return Err(validation("unknown manual LLM task state"));
    }
    let output: Option<Value> = row.get("output_json");
    let output = output.ok_or_else(|| validation("completed manual LLM job has no output"))?;
    let output: EditorialDraftGenerateOutputPayload =
        serde_json::from_value(output).map_err(|err| {
            validation(format!("manual LLM stored output failed to decode: {err}"))
        })?;
    if output.provider_key != "chat_manual" || output.candidate.is_none()
        || output.status != "done"
    {
        return Err(validation("manual LLM result has invalid completion envelope"));
    }
    Ok(output)
}

/// Atomic validation and submission. Returns the owning workflow ID.
pub async fn submit_editorial(
    pool: &PgPool,
    submission: &ManualLlmSubmission,
    model_key: &str,
) -> Result<String, DomainError> {
    if submission.schema_version != MANUAL_SCHEMA_VERSION {
        return Err(validation("unsupported manual LLM response schema version"));
    }
    if model_key.trim().is_empty() {
        return Err(validation("model_key must identify the responding model"));
    }
    let mut tx = pool.begin().await.map_err(db_error)?;
    let row = sqlx::query(
        "SELECT run_id, input_sha256, input_json, job_status, response_json, model_key
         FROM pipeline.manual_llm_jobs
         WHERE task_id = $1 AND operation = $2 AND schema_version = $3 FOR UPDATE",
    )
    .bind(&submission.task_id)
    .bind(MANUAL_OPERATION)
    .bind(submission.schema_version)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db_error)?
    .ok_or_else(|| validation("unknown manual LLM task_id"))?;
    let sha256: String = row.get("input_sha256");
    if sha256 != submission.input_sha256 {
        return Err(validation("manual LLM response input_sha256 mismatch"));
    }
    let run_id: String = row.get("run_id");
    let state: String = row.get("job_status");
    if state == "completed" {
        let previous: Option<Value> = row.get("response_json");
        if previous.as_ref() != Some(&submission.response) {
            return Err(validation("duplicate manual LLM submission changed the answer"));
        }
        let previous_model: Option<String> = row.get("model_key");
        if previous_model.as_deref() != Some(model_key) {
            return Err(validation("duplicate manual LLM submission changed model_key"));
        }
        tx.commit().await.map_err(db_error)?;
        return Ok(run_id); // Caller re-sends the Temporal signal after a crash.
    }
    if state != "pending" {
        return Err(validation("manual LLM task is not pending"));
    }
    let input: EditorialDraftGenerateInputPayload =
        serde_json::from_value(row.get("input_json")).map_err(|err| {
            validation(format!("stored editorial input is invalid: {err}"))
        })?;
    if input.run_id != run_id {
        return Err(validation("manual LLM task run_id mismatch"));
    }
    let raw = serde_json::to_string(&submission.response).map_err(|err| {
        validation(format!("serialize manual LLM response: {err}"))
    })?;
    let candidate = super::editorial_llm_adapter::manual_candidate_from_generated_text(
        &input, model_key, &raw,
    )?;
    let output = EditorialDraftGenerateOutputPayload {
        candidate: Some(candidate),
        provider_key: "chat_manual".to_string(),
        model_key: model_key.to_string(),
        status: "done".to_string(),
    };
    let output_json = serde_json::to_value(output).map_err(|err| {
        validation(format!("encode validated manual LLM output: {err}"))
    })?;
    sqlx::query(
        "UPDATE pipeline.manual_llm_jobs
         SET response_json = $2, output_json = $3, model_key = $4,
             job_status = 'completed', updated_at = now()
         WHERE task_id = $1 AND job_status = 'pending'",
    )
    .bind(&submission.task_id)
    .bind(Json(&submission.response))
    .bind(Json(output_json))
    .bind(model_key)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(run_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_type_rejects_unknown_fields_and_missing_hash() {
        let good = serde_json::json!({
            "task_id":"a", "schema_version":1, "input_sha256":"abc",
            "response":{"sections":{"intro":"safe text"}}
        });
        assert!(serde_json::from_value::<ManualLlmSubmission>(good.clone()).is_ok());
        let mut extra = good.clone();
        extra["verified"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ManualLlmSubmission>(extra).is_err());
        let mut missing = good;
        missing.as_object_mut().unwrap().remove("input_sha256");
        assert!(serde_json::from_value::<ManualLlmSubmission>(missing).is_err());
    }
}
