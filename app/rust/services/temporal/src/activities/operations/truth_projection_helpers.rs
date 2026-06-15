fn section_source_tier(section: &raw_crawl_adapter::RawSectionRecord) -> String {
    let dtype = section.source_dtype.to_ascii_lowercase();
    let domain = section.source_domain.to_ascii_lowercase();
    if dtype.contains("government")
        || dtype.contains("official")
        || domain.contains(".gov")
        || domain.contains("embassy")
        || domain.contains("consulate")
    {
        "government".to_string()
    } else if dtype.contains("vfs") || domain.contains("vfsglobal") {
        "vfs".to_string()
    } else if dtype.contains("editorial") || domain.contains("news") || domain.contains("blog") {
        "editorial".to_string()
    } else {
        "low_trust".to_string()
    }
}

fn find_evidence_span(raw_text: &str, candidates: &[String]) -> (usize, usize, String) {
    for candidate in candidates {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(start) = raw_text.find(trimmed) {
            let end = start + trimmed.len();
            return (start, end, trimmed.to_string());
        }
    }
    let fallback = raw_text.trim();
    if fallback.is_empty() {
        (0, 0, String::new())
    } else {
        let quote = fallback
            .split('.')
            .next()
            .unwrap_or(fallback)
            .trim()
            .to_string();
        let start = raw_text.find(&quote).unwrap_or(0);
        (start, start + quote.len(), quote)
    }
}

fn extract_first_number(token: &str) -> Option<f64> {
    let normalized = token.replace(',', ".");
    let digits = normalized
        .chars()
        .filter(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect::<String>();
    digits.parse::<f64>().ok()
}

fn extract_first_days(token: &str) -> Option<i64> {
    let digits = token
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse::<i64>().ok()
}

fn normalize_numeric_token_fragments(tokens: &[String]) -> Vec<String> {
    tokens
        .iter()
        .flat_map(|token| {
            let normalized = token.replace(',', ".");
            let fragments = normalized
                .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
                .filter(|fragment| !fragment.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            if fragments.is_empty() {
                vec![normalized]
            } else {
                fragments
            }
        })
        .collect()
}

fn role_and_concept_for_rule(
    rule: &seo_steps::procedural_extraction_step::ProceduralRule,
) -> (&'static str, String) {
    match rule.rule_key.as_str() {
        "consular_fee" => ("FEE_ITEM", "consular_fee".to_string()),
        "processing_time" => ("TIMELINE_ITEM", "processing_time".to_string()),
        "passport_required" => ("DOCUMENT_REQUIRED", "passport".to_string()),
        "insurance_required" => ("DOCUMENT_REQUIRED", "medical_insurance".to_string()),
        _ => ("DOCUMENT_REQUIRED", rule.rule_key.clone()),
    }
}

fn params_for_rule(
    rule: &seo_steps::procedural_extraction_step::ProceduralRule,
) -> TruthParamValue {
    match rule.rule_key.as_str() {
        "consular_fee" => {
            let amount = rule
                .numeric_tokens
                .first()
                .and_then(|token| extract_first_number(token))
                .unwrap_or_default();
            TruthParamValue::object([
                ("amount", TruthParamValue::Decimal(amount)),
                ("currency", TruthParamValue::Text("EUR".to_string())),
            ])
        }
        "processing_time" => {
            let days = rule
                .numeric_tokens
                .first()
                .and_then(|token| extract_first_days(token))
                .unwrap_or_default();
            TruthParamValue::object([("days", TruthParamValue::Integer(days))])
        }
        "passport_required" => TruthParamValue::object([
            ("subtype", TruthParamValue::Text("passport".to_string())),
            ("severity", TruthParamValue::Text("mandatory".to_string())),
        ]),
        "insurance_required" => TruthParamValue::object([
            (
                "subtype",
                TruthParamValue::Text("medical_insurance".to_string()),
            ),
            ("severity", TruthParamValue::Text("mandatory".to_string())),
        ]),
        _ => TruthParamValue::object([("subtype", TruthParamValue::Text(rule.rule_key.clone()))]),
    }
}

fn truth_param_value_to_json_local(value: &TruthParamValue) -> Value {
    match value {
        TruthParamValue::Null => Value::Null,
        TruthParamValue::Bool(value) => Value::Bool(*value),
        TruthParamValue::Integer(value) => serde_json::to_value(value).unwrap_or(Value::Null),
        TruthParamValue::Decimal(value) => serde_json::to_value(value).unwrap_or(Value::Null),
        TruthParamValue::Text(value) => Value::String(value.clone()),
        TruthParamValue::List(values) => {
            Value::Array(values.iter().map(truth_param_value_to_json_local).collect())
        }
        TruthParamValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), truth_param_value_to_json_local(value)))
                .collect(),
        ),
    }
}

fn classify_candidate_completeness_local(validation: &TruthCandidateValidationResult) -> String {
    if validation.issues.iter().any(|issue| {
        matches!(
            issue.code.as_str(),
            "fee_item_incomplete"
                | "timeline_item_incomplete"
                | "where_to_apply_incomplete"
                | "role_specific_params_missing"
                | "missing_numeric_params"
                | "missing_range_bounds"
                | "candidate_marked_incomplete"
        )
    }) {
        "incomplete".to_string()
    } else if validation.epistemic_status == "needs_hitl" {
        "partial".to_string()
    } else {
        "complete".to_string()
    }
}

fn section_uncertainty_flags(raw_text: &str) -> Vec<String> {
    let lowered = raw_text.to_lowercase();
    let mut flags = Vec::new();
    if [
        "archived",
        "archive",
        "outdated",
        "obsolete",
        "retained for record-keeping",
        "retained for record keeping",
        "устар",
        "архив",
        "может быть устар",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        flags.push("stale_source".to_string());
    }
    flags
}

fn classify_candidate_freshness_local(uncertainty_flags: &[String]) -> String {
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

fn non_structured_candidate_reason(candidate: &ValidatedTruthCandidateRecord) -> String {
    if candidate.freshness_class != "fresh" {
        format!(
            "freshness_block; freshness_class={}",
            candidate.freshness_class
        )
    } else if candidate.completeness_class == "incomplete" {
        format!(
            "completeness_block; completeness_class={}",
            candidate.completeness_class
        )
    } else if candidate.epistemic_status == "needs_hitl" {
        "candidate_validation_requires_hitl".to_string()
    } else {
        "non_structured_input".to_string()
    }
}

fn semantic_rule_instance_id_local(
    context_key: &str,
    role: &str,
    concept_canonical_key: &str,
) -> String {
    blake3_hex(format!("{context_key}|{role}|{concept_canonical_key}").as_bytes())
}

const VERIFIED_RULES_COLLECTION: &str = "verified_rules_4";

#[derive(Debug, Clone)]
struct VerifiedRuleProjectionCandidate {
    rule_instance_id: String,
    role_type: String,
    concept_key: String,
    source_key: String,
    evidence_quote: String,
}

fn verified_rule_projection_text(
    context_key: &str,
    candidate: &VerifiedRuleProjectionCandidate,
) -> String {
    format!(
        "{} {} {} {} {}",
        context_key,
        candidate.role_type,
        candidate.concept_key,
        candidate.source_key,
        candidate.evidence_quote
    )
    .trim()
    .to_string()
}

async fn delete_verified_rules_4_projection(
    acts: &AlegriaActivities,
    rule_instance_ids: &[String],
) -> Result<(), DomainError> {
    if rule_instance_ids.is_empty() {
        return Ok(());
    }

    let point_ids = rule_instance_ids
        .iter()
        .map(|rule_instance_id| {
            qdrant_point_id_v1(VERIFIED_RULES_COLLECTION, "rule_instance", rule_instance_id)
        })
        .collect::<Vec<_>>();

    if env_flag("RETRIEVAL_CAPABILITY_REQUIRED") {
        let qdrant_url =
            std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());
        let client = qdrant_client_adapter::connect_qdrant(&qdrant_url)
            .await
            .map_err(|err| DomainError::InfraUnavailable {
                message: format!("connect Qdrant for verified_rules_4 delete failed: {err}"),
            })?;
        qdrant_client_adapter::delete_point_ids(
            &client,
            VERIFIED_RULES_COLLECTION,
            point_ids.clone(),
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("delete verified_rules_4 points failed: {err}"),
        })?;
    }

    sqlx_seo_adapter::delete_qdrant_rule_points(
        &acts.pool,
        VERIFIED_RULES_COLLECTION,
        rule_instance_ids,
    )
    .await?;
    Ok(())
}

async fn emit_verified_rules_4_projection(
    acts: &AlegriaActivities,
    run_id: &str,
    context_key: &str,
    candidates: &[VerifiedRuleProjectionCandidate],
) -> Result<(), DomainError> {
    if candidates.is_empty() {
        return Ok(());
    }

    let voyage_api_key = match std::env::var("VOYAGE_API_KEY") {
        Ok(value) => value,
        Err(_) if env_flag("RETRIEVAL_CAPABILITY_REQUIRED") => {
            return Err(DomainError::InfraUnavailable {
                message: "verified_rules_4 projection requires VOYAGE_API_KEY".to_string(),
            });
        }
        Err(_) => return Ok(()),
    };
    let voyage_model =
        std::env::var("VOYAGE_MODEL").unwrap_or_else(|_| "voyage-4-large".to_string());
    let voyage = VoyageClient::new(voyage_api_key, voyage_model.clone());
    let texts = candidates
        .iter()
        .map(|candidate| verified_rule_projection_text(context_key, candidate))
        .collect::<Vec<_>>();
    let embeddings = voyage
        .embed_all_with_settings(
            &texts,
            &VoyageEmbeddingOptions {
                input_type: Some(VoyageInputType::Document),
                output_dimension: Some(1024),
                output_dtype: Some(VoyageOutputDtype::Float),
                truncation: Some(false),
            },
        )
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("embed verified_rules_4 projection with Voyage failed: {err}"),
        })?;
    if embeddings.len() != candidates.len() {
        return Err(DomainError::InfraUnavailable {
            message: format!(
                "verified_rules_4 embedding size mismatch: expected {}, got {}",
                candidates.len(),
                embeddings.len()
            ),
        });
    }

    let mut events = Vec::with_capacity(candidates.len());
    for (candidate, vector) in candidates.iter().zip(embeddings.into_iter()) {
        let typed_event = seo_steps::outbox_builder::qdrant_rule_upsert(
            &candidate.rule_instance_id,
            context_key,
            &candidate.role_type,
            &candidate.concept_key,
            &candidate.role_type,
            &candidate.source_key,
            VERIFIED_RULES_COLLECTION,
            vector.iter().map(|value| *value as f64).collect(),
        );
        events.push(sqlx_outbox_adapter::OutboxEnvelope {
            run_id: run_id.to_string(),
            aggregate_type: typed_event.aggregate_type,
            aggregate_key: typed_event.aggregate_key,
            target_system: typed_event.target_system,
            event_type: typed_event.event_type,
            payload_type: typed_event.payload_type,
            schema_version: typed_event.schema_version,
            idempotency_key: typed_event.idempotency_key,
            payload_bytes: typed_event.payload_bytes,
        });

        let point_id = qdrant_point_id_v1(
            VERIFIED_RULES_COLLECTION,
            "rule_instance",
            &candidate.rule_instance_id,
        );
        sqlx_seo_adapter::upsert_qdrant_rule_point(
            &acts.pool,
            &point_id,
            &candidate.rule_instance_id,
            VERIFIED_RULES_COLLECTION,
            &voyage_model,
            "verified_rules_4@1",
        )
        .await?;
    }

    sqlx_runtime_outbox_adapter::outbox_emit_many(&acts.pool, &events).await?;
    Ok(())
}
