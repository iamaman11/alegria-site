async fn load_persisted_candidates_for_adjudication(
    pool: &PgPool,
    context_key: &str,
    section_ids: &[i64],
) -> std::result::Result<Vec<PersistedCandidateForAdjudication>, primitives::errors::DomainError> {
    let rows = sqlx::query(
        "SELECT c.rule_candidate_id,
                c.context_key,
                c.role,
                c.concept_canonical_key,
                c.params,
                c.source_key,
                coalesce(src.source_type, '') AS source_tier,
                c.confidence::float8 AS confidence,
                c.evidence_section_id,
                c.evidence_quote,
                c.span_start,
                c.span_end,
                c.source_snapshot_hash,
                c.prompt_version,
                c.llm_model,
                c.epistemic_status,
                c.uncertainty_flags,
                EXISTS (
                    SELECT 1
                    FROM kb.concepts concept
                    WHERE concept.concept_key = c.concept_canonical_key
                      AND concept.status = 'active'
                ) AS concept_exists
         FROM extracted.rule_candidates c
         LEFT JOIN kb.sources src ON src.source_key = c.source_key
         WHERE c.context_key = $1
           AND c.raw_section_id = ANY($2)
         ORDER BY c.created_at, c.rule_candidate_id",
    )
    .bind(context_key)
    .bind(section_ids)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut candidates = Vec::with_capacity(rows.len());
    for row in rows {
        let uncertainty_flags: Vec<String> = row
            .try_get::<Json<Vec<String>>, _>("uncertainty_flags")
            .map(|json| json.0)
            .unwrap_or_default();
        let mut epistemic_status: String = row.get("epistemic_status");
        let concept_exists: bool = row.get("concept_exists");
        if epistemic_status != "rejected" && !concept_exists {
            epistemic_status = "needs_hitl".to_string();
            if let Ok(rule_candidate_id) = row.try_get::<String, _>("rule_candidate_id") {
                update_candidate_epistemic_status(pool, &rule_candidate_id, &epistemic_status)
                    .await?;
            }
        }
        let freshness_class = classify_candidate_freshness(&uncertainty_flags);
        let completeness_class =
            classify_candidate_completeness(&epistemic_status, &uncertainty_flags);
        if epistemic_status == "structured"
            && (freshness_class != "fresh" || completeness_class != "complete")
        {
            epistemic_status = "needs_hitl".to_string();
            if let Ok(rule_candidate_id) = row.try_get::<String, _>("rule_candidate_id") {
                update_candidate_epistemic_status(pool, &rule_candidate_id, &epistemic_status)
                    .await?;
            }
        }
        candidates.push(PersistedCandidateForAdjudication {
            rule_candidate_id: row.get("rule_candidate_id"),
            context_key: row.get("context_key"),
            role: row.get("role"),
            concept_canonical_key: row.get("concept_canonical_key"),
            params: row
                .try_get::<Json<Value>, _>("params")
                .map(|json| truth_param_value_from_json(&json.0))
                .unwrap_or_default(),
            source_key: row.get("source_key"),
            source_tier: row.get("source_tier"),
            confidence: row.get::<f64, _>("confidence"),
            freshness_class,
            completeness_class,
            evidence_section_id: row.get("evidence_section_id"),
            evidence_quote: row.get("evidence_quote"),
            span_start: row.get("span_start"),
            span_end: row.get("span_end"),
            source_snapshot_hash: row.get("source_snapshot_hash"),
            prompt_version: row.get("prompt_version"),
            model_version: row.get("llm_model"),
            epistemic_status,
        });
    }
    Ok(candidates)
}

fn to_truth_structured_candidate(
    candidate: &PersistedCandidateForAdjudication,
) -> TruthStructuredCandidate {
    TruthStructuredCandidate {
        rule_candidate_id: candidate.rule_candidate_id.clone(),
        context_key: candidate.context_key.clone(),
        role: candidate.role.clone(),
        concept_canonical_key: candidate.concept_canonical_key.clone(),
        params: candidate.params.clone(),
        source_key: candidate.source_key.clone(),
        source_tier: candidate.source_tier.clone(),
        confidence: candidate.confidence,
        freshness_class: candidate.freshness_class.clone(),
        completeness_class: candidate.completeness_class.clone(),
        evidence_quote: candidate.evidence_quote.clone(),
        epistemic_status: candidate.epistemic_status.clone(),
    }
}

fn truth_param_value_from_json(value: &Value) -> TruthParamValue {
    match value {
        Value::Null => TruthParamValue::Null,
        Value::Bool(value) => TruthParamValue::Bool(*value),
        Value::Number(value) => {
            if let Some(integer) = value.as_i64() {
                TruthParamValue::Integer(integer)
            } else if let Some(decimal) = value.as_f64() {
                TruthParamValue::Decimal(decimal)
            } else {
                TruthParamValue::Null
            }
        }
        Value::String(value) => TruthParamValue::Text(value.clone()),
        Value::Array(values) => TruthParamValue::List(
            values
                .iter()
                .map(truth_param_value_from_json)
                .collect::<Vec<_>>(),
        ),
        Value::Object(values) => TruthParamValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), truth_param_value_from_json(value)))
                .collect(),
        ),
    }
}

fn truth_param_value_to_json(value: &TruthParamValue) -> Value {
    match value {
        TruthParamValue::Null => Value::Null,
        TruthParamValue::Bool(value) => Value::Bool(*value),
        TruthParamValue::Integer(value) => json!(value),
        TruthParamValue::Decimal(value) => json!(value),
        TruthParamValue::Text(value) => Value::String(value.clone()),
        TruthParamValue::List(values) => {
            Value::Array(values.iter().map(truth_param_value_to_json).collect())
        }
        TruthParamValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), truth_param_value_to_json(value)))
                .collect(),
        ),
    }
}

fn classify_candidate_freshness(uncertainty_flags: &[String]) -> String {
    if uncertainty_flags.iter().any(|flag| {
        flag == "stale_source" || flag == "validator:freshness_or_temporality_ambiguous"
    }) {
        "stale".to_string()
    } else if uncertainty_flags
        .iter()
        .any(|flag| flag == "freshness_ambiguous" || flag == "temporal_ambiguous")
    {
        "watch".to_string()
    } else {
        "fresh".to_string()
    }
}

fn classify_candidate_completeness(epistemic_status: &str, uncertainty_flags: &[String]) -> String {
    if uncertainty_flags.iter().any(|flag| {
        matches!(
            flag.as_str(),
            "validator:fee_item_incomplete"
                | "validator:timeline_item_incomplete"
                | "validator:where_to_apply_incomplete"
                | "validator:role_specific_params_missing"
                | "validator:missing_numeric_params"
                | "validator:missing_range_bounds"
                | "validator:candidate_marked_incomplete"
        )
    }) {
        "incomplete".to_string()
    } else if epistemic_status == "needs_hitl" {
        "partial".to_string()
    } else {
        "complete".to_string()
    }
}

async fn update_candidate_epistemic_status(
    pool: &PgPool,
    rule_candidate_id: &str,
    epistemic_status: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    sqlx::query(
        "UPDATE extracted.rule_candidates
         SET epistemic_status = $2,
             updated_at = now()
         WHERE rule_candidate_id = $1",
    )
    .bind(rule_candidate_id)
    .bind(epistemic_status)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

fn semantic_rule_instance_id(context_key: &str, role: &str, concept_canonical_key: &str) -> String {
    blake3_hex(format!("{context_key}|{role}|{concept_canonical_key}").as_bytes())
}

fn canonical_role_key(role: &str) -> String {
    role.trim().to_ascii_lowercase()
}

fn pick_canonical_candidate<'a>(
    group_candidates: &'a [PersistedCandidateForAdjudication],
    adjudication: &primitives::truth_candidates::TruthAdjudicationResult,
) -> std::result::Result<&'a PersistedCandidateForAdjudication, primitives::errors::DomainError> {
    let verified_ids: Vec<&str> = adjudication
        .decisions
        .iter()
        .filter(|decision| decision.decision == "verified")
        .map(|decision| decision.rule_candidate_id.as_str())
        .collect();
    group_candidates
        .iter()
        .filter(|candidate| {
            verified_ids
                .iter()
                .any(|id| *id == candidate.rule_candidate_id)
        })
        .max_by(|left, right| {
            left.confidence
                .partial_cmp(&right.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .ok_or_else(|| primitives::errors::DomainError::UnexpectedBug {
            message: "truth adjudication produced verified verdict without canonical candidate"
                .to_string(),
        })
}

fn canonical_verification_method(
    adjudication: &primitives::truth_candidates::TruthAdjudicationResult,
) -> String {
    adjudication
        .decisions
        .iter()
        .find(|decision| decision.decision == "verified")
        .map(|decision| decision.verification_method.clone())
        .unwrap_or_else(|| "truth_adjudication@1".to_string())
}

fn canonical_adjudication_reason(
    adjudication: &primitives::truth_candidates::TruthAdjudicationResult,
) -> String {
    adjudication
        .decisions
        .first()
        .map(|decision| decision.adjudication_reason.clone())
        .unwrap_or_else(|| "truth_adjudication_result_missing".to_string())
}

async fn upsert_verified_rule_instance_from_candidate(
    pool: &PgPool,
    rule_instance_id: &str,
    candidate: &PersistedCandidateForAdjudication,
    publish_admissibility: &str,
    verification_method: &str,
    adjudication_reason: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    let role_key = canonical_role_key(&candidate.role);
    sqlx::query(
        "INSERT INTO verified.rule_instances (
             rule_instance_id,
             context_key,
             rule_type_key,
             concept_key,
             role_type,
             params,
             status,
             source_key,
             confidence,
             effective_from,
             rule_candidate_id,
             evidence_section_id,
             evidence_quote,
             span_start,
             span_end,
             source_snapshot_hash,
             verification_method,
             adjudication_reason,
             publish_admissibility,
             freshness_class,
             completeness_class,
             registry_version,
             prompt_version,
             model_version,
             pipeline_version
         )
         VALUES (
             $1, $2, $3, $4, $5, $6, 'verified', $7, $8, current_date,
             $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23
         )
         ON CONFLICT (rule_instance_id) DO UPDATE
         SET rule_type_key = EXCLUDED.rule_type_key,
             concept_key = EXCLUDED.concept_key,
             role_type = EXCLUDED.role_type,
             params = EXCLUDED.params,
             status = EXCLUDED.status,
             source_key = EXCLUDED.source_key,
             confidence = EXCLUDED.confidence,
             effective_from = EXCLUDED.effective_from,
             rule_candidate_id = EXCLUDED.rule_candidate_id,
             evidence_section_id = EXCLUDED.evidence_section_id,
             evidence_quote = EXCLUDED.evidence_quote,
             span_start = EXCLUDED.span_start,
             span_end = EXCLUDED.span_end,
             source_snapshot_hash = EXCLUDED.source_snapshot_hash,
             verification_method = EXCLUDED.verification_method,
             adjudication_reason = EXCLUDED.adjudication_reason,
             publish_admissibility = EXCLUDED.publish_admissibility,
             freshness_class = EXCLUDED.freshness_class,
             completeness_class = EXCLUDED.completeness_class,
             registry_version = EXCLUDED.registry_version,
             prompt_version = EXCLUDED.prompt_version,
             model_version = EXCLUDED.model_version,
             pipeline_version = EXCLUDED.pipeline_version,
             updated_at = now()",
    )
    .bind(rule_instance_id)
    .bind(&candidate.context_key)
    .bind(&role_key)
    .bind(&candidate.concept_canonical_key)
    .bind(&role_key)
    .bind(Json::<Value>(truth_param_value_to_json(&candidate.params)))
    .bind(&candidate.source_key)
    .bind(candidate.confidence)
    .bind(&candidate.rule_candidate_id)
    .bind(candidate.evidence_section_id)
    .bind(&candidate.evidence_quote)
    .bind(candidate.span_start)
    .bind(candidate.span_end)
    .bind(&candidate.source_snapshot_hash)
    .bind(verification_method)
    .bind(adjudication_reason)
    .bind(publish_admissibility)
    .bind(&candidate.freshness_class)
    .bind(&candidate.completeness_class)
    .bind("registry@1")
    .bind(&candidate.prompt_version)
    .bind(&candidate.model_version)
    .bind("truth_adjudication_runtime@1")
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

async fn demote_verified_semantic_slot(
    pool: &PgPool,
    rule_instance_id: &str,
    status: &str,
    publish_admissibility: &str,
    verification_method: &str,
    adjudication_reason: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    sqlx::query(
        "UPDATE verified.rule_instances
         SET status = $2,
             publish_admissibility = $3,
             verification_method = $4,
             adjudication_reason = $5,
             updated_at = now()
         WHERE rule_instance_id = $1",
    )
    .bind(rule_instance_id)
    .bind(status)
    .bind(publish_admissibility)
    .bind(verification_method)
    .bind(adjudication_reason)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

