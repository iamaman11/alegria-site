use infrastructure::adapters::manual_llm_queue::{
    self, ManualLlmSubmission, ManualTruthSubmission,
};

#[derive(Debug)]
enum ManualSubmission {
    Editorial(ManualLlmSubmission),
    Truth(ManualTruthSubmission),
}

fn decode_manual_submission(envelope: serde_json::Value) -> Result<ManualSubmission> {
    // Let the existing strict, mutually exclusive response types own the
    // transport contract. Do not duplicate their field names in CLI routing.
    match serde_json::from_value::<ManualLlmSubmission>(envelope.clone()) {
        Ok(submission) => Ok(ManualSubmission::Editorial(submission)),
        Err(editorial_error) => serde_json::from_value::<ManualTruthSubmission>(envelope)
            .map(ManualSubmission::Truth)
            .with_context(|| {
                format!("invalid typed manual LLM submission (editorial: {editorial_error})")
            }),
    }
}

async fn llm_pool(database_url: Option<String>) -> Result<sqlx::PgPool> {
    let url = database_url
        .or_else(|| env::var("DATABASE_URL").ok())
        .context("DATABASE_URL or --database-url is required")?;
    connect_pg(&url).await
}

async fn llm_next(database_url: Option<String>) -> Result<i32> {
    let pool = llm_pool(database_url).await?;
    match manual_llm_queue::next_job(&pool).await? {
        Some(job) => println!("{}", serde_json::to_string_pretty(&job)?),
        None => println!("NO_PENDING_LLM_JOBS"),
    }
    Ok(0)
}

async fn llm_submit(
    database_url: Option<String>,
    response_file: &str,
    model_key: &str,
    temporal_url: Option<String>,
    namespace: &str,
) -> Result<i32> {
    let raw = fs::read_to_string(response_file)
        .with_context(|| format!("read LLM submission from {response_file}"))?;
    let envelope: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("decode LLM submission from {response_file}"))?;
    let submission = decode_manual_submission(envelope)?;
    let pool = llm_pool(database_url).await?;
    let (run_id, task_id) = match submission {
        ManualSubmission::Editorial(submission) => {
            let run_id = manual_llm_queue::submit_editorial(&pool, &submission, model_key).await?;
            (run_id, submission.task_id)
        }
        ManualSubmission::Truth(submission) => {
            let run_id = manual_llm_queue::submit_truth(&pool, &submission, model_key).await?;
            (run_id, submission.task_id)
        }
    };
    // Write-first: if Temporal is unreachable, the completed job remains in
    // PostgreSQL. Retrying this exact submission is idempotent and re-signals.
    let temporal = TemporalSeoWorkflowControlAdapter::new(
        temporal_url
            .or_else(|| env::var("TEMPORAL_URL").ok())
            .unwrap_or_else(|| "http://localhost:7233".to_string()),
        "alegria-cli-manual-llm".to_string(),
        namespace.to_string(),
    );
    temporal
        .signal_llm_result_ready(&run_id)
        .await
        .context("manual LLM response saved but Temporal signal failed; retry llm-submit")?;
    println!(
        "LLM_SUBMIT: OK task_id={} run_id={} state=completed signaled=true",
        task_id, run_id
    );
    Ok(0)
}

#[cfg(test)]
mod manual_submission_tests {
    use super::*;

    fn envelope(response: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "task_id": "test-task",
            "schema_version": 1,
            "input_sha256": "test-hash",
            "response": response
        })
    }

    #[test]
    fn cli_routes_typed_editorial_and_legacy_and_truth() {
        let typed = serde_json::json!({
            "claims": {"documents": [{"support_ref": "ref", "text": "Passport required"}]},
            "metadata": {"title": "A useful title", "description": "An evidence-bound description"}
        });
        assert!(matches!(
            decode_manual_submission(envelope(typed)),
            Ok(ManualSubmission::Editorial(_))
        ));
        assert!(matches!(
            decode_manual_submission(envelope(serde_json::json!({"sections": {}}))),
            Ok(ManualSubmission::Editorial(_))
        ));
        assert!(matches!(
            decode_manual_submission(envelope(serde_json::json!({"candidates": []}))),
            Ok(ManualSubmission::Truth(_))
        ));
    }

    #[test]
    fn cli_rejects_mixed_and_unknown_responses() {
        for response in [
            serde_json::json!({"claims": {}, "metadata": {"title": "A useful title", "description": "An evidence-bound description"}, "candidates": []}),
            serde_json::json!({"sections": {}, "candidates": []}),
            serde_json::json!({"claims": {}}),
            serde_json::json!({"unknown": {}}),
        ] {
            assert!(decode_manual_submission(envelope(response)).is_err());
        }
    }
}
