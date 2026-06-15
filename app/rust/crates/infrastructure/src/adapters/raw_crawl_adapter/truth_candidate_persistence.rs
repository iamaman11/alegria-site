async fn persist_extracted_rule_candidates(
    pool: &PgPool,
    context_key: &str,
    section: &RawSectionRecord,
    extraction: &truth_extraction_llm_adapter::TruthExtractionResponse,
) -> std::result::Result<usize, primitives::errors::DomainError> {
    for (ordinal, rule) in extraction.candidates.iter().enumerate() {
        let candidate_id = blake3_hex(
            format!(
                "{}|{}|{}|{}|{}|{}",
                context_key,
                section.id,
                ordinal,
                rule.role,
                rule.concept_canonical_key,
                rule.evidence_quote
            )
            .as_bytes(),
        );
        let source_snapshot_hash = if section.content_hash.trim().is_empty() {
            content_hash_v1(&section.content_md)
        } else {
            section.content_hash.clone()
        };
        let validation = validate_truth_candidate(
            &TruthCandidateRuntime {
                rule_candidate_id: candidate_id.clone(),
                context_key: context_key.to_string(),
                role: rule.role.clone(),
                concept_canonical_key: rule.concept_canonical_key.clone(),
                raw_mention: rule.raw_mention.clone(),
                params: truth_param_value_from_json(&rule.params),
                scope: truth_param_value_from_json(&rule.scope),
                severity: rule.severity.clone(),
                derivation_type: rule.derivation_type.clone(),
                confidence: rule.confidence,
                evidence_section_id: rule.evidence_section_id.unwrap_or(section.id),
                evidence_quote: rule.evidence_quote.clone(),
                span_start: rule.span_start,
                span_end: rule.span_end,
                source_key: section.source_url.clone(),
                source_tier: source_type_for_section(section).to_string(),
                source_snapshot_hash: source_snapshot_hash.clone(),
                is_numeric: rule.is_numeric,
                is_range: rule.is_range,
                is_incomplete: rule.is_incomplete,
                uncertainty_flags: rule.uncertainty_flags.clone(),
            },
            &section.content_md,
        );
        let mut uncertainty_flags = rule.uncertainty_flags.clone();
        for issue in &validation.issues {
            let flag = format!("validator:{}", issue.code);
            if !uncertainty_flags.iter().any(|existing| existing == &flag) {
                uncertainty_flags.push(flag);
            }
        }
        sqlx::query(
            "INSERT INTO extracted.rule_candidates (
                 rule_candidate_id,
                 context_key,
                 raw_section_id,
                 role,
                 concept_canonical_key,
                 raw_mention,
                 params,
                 scope,
                 severity,
                 applies_to_profiles,
                 exceptions_raw,
                 conditions_raw,
                 alternatives,
                 modality_raw,
                 derivation_type,
                 is_numeric,
                 is_range,
                 is_incomplete,
                 confidence,
                 evidence_section_id,
                 evidence_quote,
                 span_start,
                 span_end,
                 source_key,
                 source_snapshot_hash,
                 llm_provider,
                 llm_model,
                 prompt_version,
                 epistemic_status,
                 uncertainty_flags
             )
             VALUES (
                 $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
                 $21, $22, $23, $24, $25, $26, $27, $28, $29, $30
             )
             ON CONFLICT (rule_candidate_id) DO UPDATE
             SET concept_canonical_key = EXCLUDED.concept_canonical_key,
                 raw_mention = EXCLUDED.raw_mention,
                 params = EXCLUDED.params,
                 scope = EXCLUDED.scope,
                 severity = EXCLUDED.severity,
                 applies_to_profiles = EXCLUDED.applies_to_profiles,
                 exceptions_raw = EXCLUDED.exceptions_raw,
                 conditions_raw = EXCLUDED.conditions_raw,
                 alternatives = EXCLUDED.alternatives,
                 modality_raw = EXCLUDED.modality_raw,
                 derivation_type = EXCLUDED.derivation_type,
                 is_numeric = EXCLUDED.is_numeric,
                 is_range = EXCLUDED.is_range,
                 is_incomplete = EXCLUDED.is_incomplete,
                 confidence = EXCLUDED.confidence,
                 evidence_quote = EXCLUDED.evidence_quote,
                 span_start = EXCLUDED.span_start,
                 span_end = EXCLUDED.span_end,
                 source_key = EXCLUDED.source_key,
                 source_snapshot_hash = EXCLUDED.source_snapshot_hash,
                 llm_provider = EXCLUDED.llm_provider,
                 llm_model = EXCLUDED.llm_model,
                 prompt_version = EXCLUDED.prompt_version,
                 epistemic_status = EXCLUDED.epistemic_status,
                 uncertainty_flags = EXCLUDED.uncertainty_flags,
                 updated_at = now()",
        )
        .bind(candidate_id)
        .bind(context_key)
        .bind(section.id)
        .bind(&rule.role)
        .bind(&rule.concept_canonical_key)
        .bind(&rule.raw_mention)
        .bind(Json::<Value>(rule.params.clone()))
        .bind(Json::<Value>(rule.scope.clone()))
        .bind(&rule.severity)
        .bind(Json::<Value>(json!(rule.applies_to_profiles)))
        .bind(rule.exceptions_raw.clone().unwrap_or_default())
        .bind(rule.conditions_raw.clone().unwrap_or_default())
        .bind(Json::<Value>(rule.alternatives.clone()))
        .bind(rule.modality_raw.clone().unwrap_or_default())
        .bind(&rule.derivation_type)
        .bind(rule.is_numeric)
        .bind(rule.is_range)
        .bind(rule.is_incomplete)
        .bind(rule.confidence)
        .bind(rule.evidence_section_id.unwrap_or(section.id))
        .bind(&rule.evidence_quote)
        .bind(rule.span_start as i32)
        .bind(rule.span_end as i32)
        .bind(&section.source_url)
        .bind(&source_snapshot_hash)
        .bind(&extraction.provider_key)
        .bind(&extraction.model_key)
        .bind(&extraction.prompt_version)
        .bind(&validation.epistemic_status)
        .bind(Json::<Value>(json!(uncertainty_flags)))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }
    Ok(extraction.candidates.len())
}

#[derive(Debug, Clone, Default)]
struct TruthAdjudicationPersistReport {
    verified_rule_count: usize,
    needs_hitl_candidate_count: usize,
    changed_truth_keys: Vec<String>,
}

async fn adjudicate_persisted_rule_candidates(
    pool: &PgPool,
    context_key: &str,
    section_ids: &[i64],
) -> std::result::Result<TruthAdjudicationPersistReport, primitives::errors::DomainError> {
    if section_ids.is_empty() {
        return Ok(TruthAdjudicationPersistReport::default());
    }

    let candidates =
        load_persisted_candidates_for_adjudication(pool, context_key, section_ids).await?;
    if candidates.is_empty() {
        return Ok(TruthAdjudicationPersistReport::default());
    }
    let source_registry = super::sqlx_source_projection_adapter::load_source_registry_entries(pool)
        .await?
        .into_iter()
        .map(|(source_key, record)| {
            (
                source_key,
                SourceGovernanceRecord {
                    source_type: record.source_type,
                    trust_level: record.trust_level,
                    authority_class: record.authority_class,
                    independence_group_key: record.independence_group_key,
                    freshness_ttl_days: record.freshness_ttl_days,
                    override_eligible: record.override_eligible,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut report = TruthAdjudicationPersistReport::default();
    let mut groups: BTreeMap<(String, String, String), Vec<PersistedCandidateForAdjudication>> =
        BTreeMap::new();
    for candidate in candidates {
        groups
            .entry((
                candidate.context_key.clone(),
                candidate.role.clone(),
                candidate.concept_canonical_key.clone(),
            ))
            .or_default()
            .push(candidate);
    }

    for ((group_context_key, group_role, group_concept), group_candidates) in groups {
        let semantic_slot_id =
            semantic_rule_instance_id(&group_context_key, &group_role, &group_concept);
        let structured: Vec<TruthStructuredCandidate> = group_candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "structured")
            .map(to_truth_structured_candidate)
            .collect();
        let needs_hitl_ids: Vec<String> = group_candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "needs_hitl")
            .map(|candidate| candidate.rule_candidate_id.clone())
            .collect();
        let rejected_ids: Vec<String> = group_candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "rejected")
            .map(|candidate| candidate.rule_candidate_id.clone())
            .collect();

        if structured.is_empty() {
            if !needs_hitl_ids.is_empty() {
                for candidate_id in &needs_hitl_ids {
                    update_candidate_epistemic_status(pool, candidate_id, "needs_hitl").await?;
                }
                report.needs_hitl_candidate_count += needs_hitl_ids.len();
                demote_verified_semantic_slot(
                    pool,
                    &semantic_slot_id,
                    "disputed",
                    "needs_hitl",
                    "truth_adjudication@1",
                    "no_structured_candidates_for_semantic_identity",
                )
                .await?;
                report
                    .changed_truth_keys
                    .push(format!("verified.rule_instance:{semantic_slot_id}"));
            } else if !rejected_ids.is_empty() {
                for candidate_id in &rejected_ids {
                    update_candidate_epistemic_status(pool, candidate_id, "rejected").await?;
                }
                demote_verified_semantic_slot(
                    pool,
                    &semantic_slot_id,
                    "deprecated",
                    "not_admissible",
                    "truth_adjudication@1",
                    "all_candidates_rejected_for_semantic_identity",
                )
                .await?;
                report
                    .changed_truth_keys
                    .push(format!("verified.rule_instance:{semantic_slot_id}"));
            }
            continue;
        }

        let adjudication =
            adjudicate_truth_candidates_with_governance(&structured, &source_registry);
        match adjudication.overall_status.as_str() {
            "verified" => {
                for decision in &adjudication.decisions {
                    if decision.decision == "verified" {
                        update_candidate_epistemic_status(
                            pool,
                            &decision.rule_candidate_id,
                            "verified",
                        )
                        .await?;
                    }
                }
                let canonical = pick_canonical_candidate(&group_candidates, &adjudication)?;
                upsert_verified_rule_instance_from_candidate(
                    pool,
                    &semantic_slot_id,
                    canonical,
                    "admissible",
                    &canonical_verification_method(&adjudication),
                    &canonical_adjudication_reason(&adjudication),
                )
                .await?;
                report.verified_rule_count += 1;
                report
                    .changed_truth_keys
                    .push(format!("verified.rule_instance:{semantic_slot_id}"));
            }
            "needs_hitl" => {
                for decision in &adjudication.decisions {
                    if decision.decision == "needs_hitl" {
                        update_candidate_epistemic_status(
                            pool,
                            &decision.rule_candidate_id,
                            "needs_hitl",
                        )
                        .await?;
                    }
                }
                report.needs_hitl_candidate_count += adjudication
                    .decisions
                    .iter()
                    .filter(|decision| decision.decision == "needs_hitl")
                    .count();
                demote_verified_semantic_slot(
                    pool,
                    &semantic_slot_id,
                    "disputed",
                    "needs_hitl",
                    "truth_adjudication@1",
                    &canonical_adjudication_reason(&adjudication),
                )
                .await?;
                report
                    .changed_truth_keys
                    .push(format!("verified.rule_instance:{semantic_slot_id}"));
            }
            _ => {
                for decision in &adjudication.decisions {
                    update_candidate_epistemic_status(
                        pool,
                        &decision.rule_candidate_id,
                        "rejected",
                    )
                    .await?;
                }
                demote_verified_semantic_slot(
                    pool,
                    &semantic_slot_id,
                    "deprecated",
                    "not_admissible",
                    "truth_adjudication@1",
                    &canonical_adjudication_reason(&adjudication),
                )
                .await?;
                report
                    .changed_truth_keys
                    .push(format!("verified.rule_instance:{semantic_slot_id}"));
            }
        }
    }

    report.changed_truth_keys.sort();
    report.changed_truth_keys.dedup();
    Ok(report)
}

