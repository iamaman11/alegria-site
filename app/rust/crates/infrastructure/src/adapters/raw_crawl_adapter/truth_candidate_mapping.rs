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
                c.scope,
                -- Older persisted LLM proposals may carry limiting conditions
                -- outside scope. Do not let them become universal facts.
                (c.applies_to_profiles <> '[]'::jsonb
                 OR btrim(c.conditions_raw) <> ''
                 OR btrim(c.exceptions_raw) <> ''
                 OR c.alternatives <> '[]'::jsonb
                 OR COALESCE(c.params->>'conditions_key', '') <> '')
                AS unresolved_applicability,
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
        let original_status: String = row.get("epistemic_status");
        // Legacy candidates may predate the scoped applicability validator.
        // A nonempty/malformed scope must never be promoted by historical
        // extraction or by a previously persisted "structured" verdict.
        let scope = row
            .try_get::<Json<Value>, _>("scope")
            .map(|value| value.0)
            .unwrap_or(Value::Null);
        let unresolved_applicability: bool = row.get("unresolved_applicability");
        let concept_exists: bool = row.get("concept_exists");
        let classification = classify_persisted_candidate(
            &original_status,
            &truth_param_value_from_json(&scope),
            unresolved_applicability,
            concept_exists,
            &uncertainty_flags,
        );
        if classification.epistemic_status != original_status {
            let candidate_id: String = row.get("rule_candidate_id");
            update_candidate_epistemic_status(pool, &candidate_id, &classification.epistemic_status)
                .await?;
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
            freshness_class: classification.freshness_class,
            completeness_class: classification.completeness_class,
            evidence_section_id: row.get("evidence_section_id"),
            evidence_quote: row.get("evidence_quote"),
            span_start: row.get("span_start"),
            span_end: row.get("span_end"),
            source_snapshot_hash: row.get("source_snapshot_hash"),
            prompt_version: row.get("prompt_version"),
            model_version: row.get("llm_model"),
            epistemic_status: classification.epistemic_status,
        });
    }
    Ok(candidates)
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
