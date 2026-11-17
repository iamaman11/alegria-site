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
    use primitives::truth_candidates::TruthParamValue;
    let mut params: TruthParamValue = serde_json::from_value(candidate.params.clone())
        .map_err(|err| validation(format!("manual truth params malformed: {err}")))?;
    let scope = serde_json::from_value(candidate.scope.clone())
        .map_err(|err| validation(format!("manual truth scope malformed: {err}")))?;

    // The procedural fee extractor represents amounts as Decimal. JSON LLM
    // answers often encode whole amounts as Integer. Without this narrow
    // canonicalization, the SAME 80 EUR quote has two different stable truth
    // signatures ("int:80" versus "decimal:80.00000000"), which the strict
    // contradiction gate correctly treats as conflicting values and blocks
    // all source-section evidence. Do not alter the global signature contract
    // or normalize unrelated integer fields such as processing days.
    if candidate.role == "FEE_ITEM" {
        if let TruthParamValue::Object(fields) = &mut params {
            for name in ["amount", "min_amount", "max_amount"] {
                if let Some(value) = fields.get_mut(name) {
                    if let TruthParamValue::Integer(integer) = value {
                        let original = *integer;
                        // f64 represents integers exactly only through 2^53.
                        // Never round an oversized financial claim silently.
                        if !(-9_007_199_254_740_992..=9_007_199_254_740_992)
                            .contains(&original)
                        {
                            return Err(validation(format!(
                                "manual truth fee {name} cannot be represented exactly as decimal"
                            )));
                        }
                        *value = TruthParamValue::Decimal(original as f64);
                    }
                }
            }
        }
    }
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
    // One immutable extraction request per run and source section. A prompt
    // upgrade cannot silently enqueue a second response during replay.
    let mut tx = pool.begin().await.map_err(db_error)?;
    sqlx::query("SELECT pg_advisory_xact_lock(21919, hashtext($1))")
        .bind(format!("{run_id}:{}", input.raw_section_id))
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let existing: Vec<Value> = sqlx::query_scalar(
        "SELECT input_json FROM pipeline.manual_llm_jobs
         WHERE run_id = $1 AND operation = $2
           AND input_json->>'raw_section_id' = $3",
    )
    .bind(run_id)
    .bind(TRUTH_OPERATION)
    .bind(input.raw_section_id.to_string())
    .fetch_all(&mut *tx)
    .await
    .map_err(db_error)?;
    if existing.len() > 1 {
        return Err(validation("multiple truth requests own the same run and raw section"));
    }
    if let Some(prior) = existing.into_iter().next() {
        let mut prior: TruthExtractionInput = serde_json::from_value(prior)
            .map_err(|err| validation(format!("stored truth input invalid: {err}")))?;
        prior.revision = input.revision;
        if &prior != input {
            return Err(validation("truth source changed within existing run; start a new run"));
        }
        tx.commit().await.map_err(db_error)?;
        return Ok(());
    }
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
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
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
        let operation: String = row.get("operation");
        let payload: Value = row.get("input_json");
        let instructions = if operation == TRUTH_OPERATION {
            let input: TruthExtractionInput = serde_json::from_value(payload.clone())
                .map_err(|err| validation(format!("stored truth input invalid: {err}")))?;
            Some(truth_extraction_llm_adapter::truth_extraction_instructions(input.revision))
        } else {
            None
        };
        Ok(ManualLlmJob {
            task_id: row.get("task_id"),
            schema_version: row.get("schema_version"),
            input_sha256: row.get("input_sha256"),
            operation,
            payload,
            instructions,
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
        if output.provider_key != "chat_manual"
            || output.prompt_version != input.revision.prompt_version()
        {
            return Err(validation("manual truth result provider or prompt revision does not match"));
        }
        results.push((input, output));
    }
    Ok(results)
}

#[cfg(test)]
mod fee_parameter_canonicalization_tests {
    use super::*;
    use primitives::truth_candidates::TruthParamValue;
    use serde_json::json;

    fn proposal(role: &str, params: serde_json::Value) -> TruthRuleCandidate {
        TruthRuleCandidate {
            role: role.to_string(),
            params,
            scope: json!({}),
            ..TruthRuleCandidate::default()
        }
    }

    #[test]
    fn manual_whole_fee_matches_procedural_decimal_without_weakening_other_types() {
        let (manual, scope) = validated_candidate_values(&proposal(
            "FEE_ITEM",
            json!({"amount": 80, "currency": "EUR"}),
        ))
        .unwrap();
        let procedural = TruthParamValue::object([
            ("amount", TruthParamValue::Decimal(80.0)),
            ("currency", TruthParamValue::Text("EUR".to_string())),
        ]);
        assert_eq!(manual.stable_signature(), procedural.stable_signature());
        assert_eq!(scope.stable_signature(), TruthParamValue::default().stable_signature());
        let (different, _) = validated_candidate_values(&proposal(
            "FEE_ITEM",
            json!({"amount": 120, "currency": "EUR"}),
        ))
        .unwrap();
        assert_ne!(manual.stable_signature(), different.stable_signature());
        let (fractional, _) = validated_candidate_values(&proposal(
            "FEE_ITEM",
            json!({"amount": 80.5, "currency": "EUR"}),
        ))
        .unwrap();
        assert_ne!(manual.stable_signature(), fractional.stable_signature());
        let (timeline, _) = validated_candidate_values(&proposal(
            "TIMELINE_ITEM",
            json!({"days": 15}),
        ))
        .unwrap();
        assert_eq!(
            timeline.stable_signature(),
            TruthParamValue::object([("days", TruthParamValue::Integer(15))])
                .stable_signature()
        );
    }

    #[test]
    fn fee_ranges_canonicalize_and_out_of_range_values_fail_closed() {
        let (values, _) = validated_candidate_values(&proposal(
            "FEE_ITEM",
            json!({"min_amount": 35, "max_amount": 80, "currency": "EUR"}),
        ))
        .unwrap();
        let expected = TruthParamValue::object([
            ("min_amount", TruthParamValue::Decimal(35.0)),
            ("max_amount", TruthParamValue::Decimal(80.0)),
            ("currency", TruthParamValue::Text("EUR".to_string())),
        ]);
        assert_eq!(values.stable_signature(), expected.stable_signature());
        assert!(validated_candidate_values(&proposal(
            "FEE_ITEM",
            json!({"amount": 9007199254740993i64, "currency": "EUR"}),
        ))
        .is_err());
    }
}
