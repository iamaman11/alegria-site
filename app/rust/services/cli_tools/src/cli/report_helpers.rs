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

