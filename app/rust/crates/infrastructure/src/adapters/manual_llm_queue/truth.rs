// The same durable inbox serves editorial and untrusted truth-candidate work.
// Truth results are proposals only; this module never writes verified truth.
use super::truth_extraction_llm_adapter::{
    self, TruthExtractionInput, TruthExtractionResponse, TruthRuleCandidate,
};

const TRUTH_OPERATION: &str = "truth_candidates";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualTruthResponse {
    pub candidates: Vec<TruthRuleCandidate>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualTruthSubmission {
    pub task_id: String,
    pub schema_version: i32,
    pub input_sha256: String,
    pub response: ManualTruthResponse,
}

/// JSON-to-typed-value conversion belongs to the infrastructure boundary,
/// not to Temporal orchestration or SEO application logic.
pub fn validated_candidate_values(
    candidate: &TruthRuleCandidate,
) -> Result<
    (
        primitives::truth_candidates::TruthParamValue,
        primitives::truth_candidates::TruthParamValue,
    ),
    DomainError,
> {
    let params = serde_json::from_value(candidate.params.clone())
        .map_err(|err| validation(format!("manual truth params malformed: {err}")))?;
    let scope = serde_json::from_value(candidate.scope.clone())
        .map_err(|err| validation(format!("manual truth scope malformed: {err}")))?;
    Ok((params, scope))
}

pub fn truth_manual_selected() -> bool {
    std::env::var("SEO_TRUTH_LLM_PROVIDER")
        .map(|provider| provider.trim().eq_ignore_ascii_case("chat_manual"))
        .unwrap_or(false)
}

async fn truth_fingerprint(
    pool: &PgPool,
    run_id: &str,
    input: &TruthExtractionInput,
) -> Result<(String, String, Value), DomainError> {
    if run_id.trim().is_empty() || input.raw_section_id <= 0 || input.raw_text.is_empty() {
        return Err(validation(
            "manual truth job requires run_id and persisted source section",
        ));
    }
    let value = serde_json::to_value(input)
        .map_err(|err| validation(format!("encode manual truth input: {err}")))?;
    let row = sqlx::query(
        "SELECT encode(digest(($1::jsonb)::text, 'sha256'), 'hex') AS input_sha256,
                encode(digest('truth_candidates:' || $2 || ':' || ($1::jsonb)::text,
                              'sha256'), 'hex') AS task_id",
    )
    .bind(Json(value.clone()))
    .bind(run_id)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;
    Ok((row.get("task_id"), row.get("input_sha256"), value))
}

pub async fn enqueue_truth(
    pool: &PgPool,
    run_id: &str,
    input: &TruthExtractionInput,
) -> Result<(), DomainError> {
    let (task_id, sha256, value) = truth_fingerprint(pool, run_id, input).await?;
    sqlx::query(
        "INSERT INTO pipeline.manual_llm_jobs
          (task_id, schema_version, run_id, operation, input_sha256, input_json)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (task_id) DO NOTHING",
    )
    .bind(task_id)
    .bind(MANUAL_SCHEMA_VERSION)
    .bind(run_id)
    .bind(TRUTH_OPERATION)
    .bind(sha256)
    .bind(Json(value))
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(())
}

pub async fn next_job(pool: &PgPool) -> Result<Option<ManualLlmJob<Value>>, DomainError> {
    let row = sqlx::query(
        "SELECT task_id, schema_version, input_sha256, operation, input_json
         FROM pipeline.manual_llm_jobs
         WHERE job_status = 'pending'
         ORDER BY created_at, task_id LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(db_error)?;
    row.map(|row| {
        Ok(ManualLlmJob {
            task_id: row.get("task_id"),
            schema_version: row.get("schema_version"),
            input_sha256: row.get("input_sha256"),
            operation: row.get("operation"),
            payload: row.get("input_json"),
        })
    })
    .transpose()
}

pub async fn truth_run_status(pool: &PgPool, run_id: &str) -> Result<String, DomainError> {
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM pipeline.manual_llm_jobs
         WHERE run_id = $1 AND operation = $2 AND job_status = 'pending'",
    )
    .bind(run_id)
    .bind(TRUTH_OPERATION)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;
    Ok(if pending == 0 {
        "done"
    } else {
        "awaiting_manual"
    }
    .to_string())
}

/// Atomic validation/commit. Rejects changed answers, cross-operation IDs,
/// unknown fields and fabricated evidence before anything reaches adjudication.
pub async fn submit_truth(
    pool: &PgPool,
    submission: &ManualTruthSubmission,
    model_key: &str,
) -> Result<String, DomainError> {
    if submission.schema_version != MANUAL_SCHEMA_VERSION {
        return Err(validation("unsupported manual truth response schema"));
    }
    if model_key.trim().is_empty() {
        return Err(validation("manual truth model_key must be non-empty"));
    }
    let mut tx = pool.begin().await.map_err(db_error)?;
    let row = sqlx::query(
        "SELECT run_id, input_sha256, input_json, job_status, response_json, model_key
         FROM pipeline.manual_llm_jobs
         WHERE task_id = $1 AND operation = $2 AND schema_version = $3 FOR UPDATE",
    )
    .bind(&submission.task_id)
    .bind(TRUTH_OPERATION)
    .bind(submission.schema_version)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db_error)?
    .ok_or_else(|| validation("unknown manual truth task_id"))?;
    let sha256: String = row.get("input_sha256");
    if sha256 != submission.input_sha256 {
        return Err(validation("manual truth input_sha256 mismatch"));
    }
    let run_id: String = row.get("run_id");
    let response_value = serde_json::to_value(&submission.response)
        .map_err(|err| validation(format!("encode manual truth response: {err}")))?;
    let status: String = row.get("job_status");
    if status == "completed" {
        let previous: Option<Value> = row.get("response_json");
        let previous_model: Option<String> = row.get("model_key");
        if previous.as_ref() != Some(&response_value)
            || previous_model.as_deref() != Some(model_key)
        {
            return Err(validation("manual truth duplicate changed answer or model"));
        }
        tx.commit().await.map_err(db_error)?;
        return Ok(run_id);
    }
    if status != "pending" {
        return Err(validation("manual truth task is not pending"));
    }
    let input: TruthExtractionInput = serde_json::from_value(row.get("input_json"))
        .map_err(|err| validation(format!("manual truth source input corrupted: {err}")))?;
    let raw = serde_json::to_string(&submission.response)
        .map_err(|err| validation(format!("serialize manual truth response: {err}")))?;
    let output =
        truth_extraction_llm_adapter::validate_manual_truth_response(&input, model_key, &raw)?;
    let output_value = serde_json::to_value(output)
        .map_err(|err| validation(format!("encode manual truth output: {err}")))?;
    sqlx::query(
        "UPDATE pipeline.manual_llm_jobs
         SET response_json = $2, output_json = $3, model_key = $4,
             job_status = 'completed', updated_at = now()
         WHERE task_id = $1 AND job_status = 'pending'",
    )
    .bind(&submission.task_id)
    .bind(Json(response_value))
    .bind(Json(output_value))
    .bind(model_key)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(run_id)
}

pub async fn read_truth_results(
    pool: &PgPool,
    run_id: &str,
) -> Result<Vec<(TruthExtractionInput, TruthExtractionResponse)>, DomainError> {
    let rows = sqlx::query(
        "SELECT input_json, output_json, job_status
         FROM pipeline.manual_llm_jobs
         WHERE run_id = $1 AND operation = $2
         ORDER BY created_at, task_id",
    )
    .bind(run_id)
    .bind(TRUTH_OPERATION)
    .fetch_all(pool)
    .await
    .map_err(db_error)?;
    let mut results = Vec::with_capacity(rows.len());
    for row in rows {
        let status: String = row.get("job_status");
        if status != "completed" {
            return Err(validation(
                "manual truth results requested before every job completed",
            ));
        }
        let input: TruthExtractionInput = serde_json::from_value(row.get("input_json"))
            .map_err(|err| validation(format!("decode stored truth source: {err}")))?;
        let output: Option<Value> = row.get("output_json");
        let output: TruthExtractionResponse = serde_json::from_value(
            output.ok_or_else(|| validation("completed manual truth result missing"))?,
        )
        .map_err(|err| validation(format!("decode stored truth candidates: {err}")))?;
        if output.provider_key != "chat_manual" {
            return Err(validation("manual truth result provider does not match"));
        }
        results.push((input, output));
    }
    Ok(results)
}
