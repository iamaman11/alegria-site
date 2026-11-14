pub(crate) async fn extraction_schema_validate_impl(
    input: &ExtractionSchemaValidateInput,
) -> Result<ExtractionSchemaValidateOutput, DomainError> {
    let sections = input
        .procedural
        .sections
        .iter()
        .map(|section| {
            let mut reasons = Vec::new();
            if !section.blocked_by_gate && !section.skipped {
                if section
                    .rules
                    .iter()
                    .any(|rule| rule.rule_key.trim().is_empty() || rule.confidence <= 0.0)
                {
                    reasons.push("invalid_procedural_candidate_shape".to_string());
                }
            }
            let status = if section.blocked_by_gate {
                "blocked"
            } else if !reasons.is_empty() {
                "invalid"
            } else {
                "valid"
            };
            ExtractionSchemaSectionDecision {
                section_id: section.section_id,
                page_id: section.page_id,
                status: status.to_string(),
                blocking_reasons: reasons,
                blocked_by_gate: section.blocked_by_gate,
            }
        })
        .collect::<Vec<_>>();
    Ok(ExtractionSchemaValidateOutput {
        section_count: sections.len(),
        blocked_section_count: sections
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        invalid_section_count: sections
            .iter()
            .filter(|section| section.status == "invalid")
            .count(),
        sections,
    })
}

pub(crate) async fn candidate_validation_impl(
    acts: &AlegriaActivities,
    input: &CandidateValidationInput,
) -> Result<CandidateValidationOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let raw_by_section: BTreeMap<i64, &raw_crawl_adapter::RawSectionRecord> = sections
        .iter()
        .map(|section| (section.id, section))
        .collect();
    let schema_by_section: BTreeMap<i64, &ExtractionSchemaSectionDecision> = input
        .schema_validate
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let ontology_by_section: BTreeMap<i64, &OntologyIntakeGateDecision> = input
        .ontology
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    // Manual ChatGPT text is never trusted as a truth decision. Load it only
    // after every durable job is committed and bind it to current raw evidence.
    let manual_enabled = infrastructure::adapters::manual_llm_queue::truth_manual_selected();
    let mut manual_by_section = BTreeMap::new();
    if manual_enabled {
        let results = infrastructure::adapters::manual_llm_queue::read_truth_results(
            &acts.pool,
            &input.run_id,
        )
        .await?;
        let allowed = input
            .procedural
            .sections
            .iter()
            .filter(|section| !section.blocked_by_gate && !section.skipped)
            .map(|section| section.section_id)
            .collect::<BTreeSet<_>>();
        if results.len() != allowed.len() {
            return Err(DomainError::ValidationFailure {
                message: "manual truth response count does not match eligible raw sections"
                    .to_string(),
            });
        }
        for (source, output) in results {
            if !allowed.contains(&source.raw_section_id)
                || manual_by_section
                    .insert(source.raw_section_id, (source, output))
                    .is_some()
            {
                return Err(DomainError::ValidationFailure {
                    message: "manual truth source was not eligible or was duplicated".to_string(),
                });
            }
        }
    }
    let mut section_states = Vec::new();
    for section in &input.procedural.sections {
        let raw = raw_by_section.get(&section.section_id).ok_or_else(|| {
            DomainError::ValidationFailure {
                message: format!(
                    "missing raw section for candidate validation: {}",
                    section.section_id
                ),
            }
        })?;
        let schema = schema_by_section.get(&section.section_id).ok_or_else(|| {
            DomainError::ValidationFailure {
                message: format!(
                    "missing schema validation section for {}",
                    section.section_id
                ),
            }
        })?;
        let ontology = ontology_by_section
            .get(&section.section_id)
            .ok_or_else(|| DomainError::ValidationFailure {
                message: format!("missing ontology section for {}", section.section_id),
            })?;
        let mut candidates = Vec::new();
        for (ordinal, rule) in section.rules.iter().enumerate() {
            let (role, concept_key) = role_and_concept_for_rule(rule);
            // The FIRST numeric token determines rule parameters; never
            // cite a different amount or duration merely because it appears
            // elsewhere in this raw section.
            let evidence_targets: Vec<String> = if let Some(first) = rule.numeric_tokens.first() {
                vec![first.clone()]
            } else {
                match rule.rule_key.as_str() {
                    "passport_required" => vec!["passport".to_string(), "паспорт".to_string()],
                    "insurance_required" => vec!["insurance".to_string(), "страхов".to_string()],
                    _ => vec![rule.rule_key.clone()],
                }
            };
            let (span_start, span_end, evidence_quote) =
                find_evidence_span(&raw.content_md, &evidence_targets, &rule.rule_key);
            let uncertainty_flags = section_uncertainty_flags(&raw.content_md);
            let runtime = TruthCandidateRuntime {
                rule_candidate_id: blake3_hex(
                    format!(
                        "{}|{}|{}|{}|{}",
                        input.context_key, raw.id, ordinal, role, concept_key
                    )
                    .as_bytes(),
                ),
                context_key: input.context_key.clone(),
                role: role.to_string(),
                concept_canonical_key: concept_key.clone(),
                raw_mention: evidence_quote.clone(),
                params: params_for_rule(rule),
                scope: TruthParamValue::object(Vec::<(String, TruthParamValue)>::new()),
                unresolved_applicability: false,
                severity: "mandatory".to_string(),
                derivation_type: "deterministic_procedural_extraction".to_string(),
                confidence: rule.confidence as f64,
                evidence_section_id: raw.id,
                evidence_quote: evidence_quote.clone(),
                span_start,
                span_end,
                source_key: raw.source_url.clone(),
                source_tier: section_source_tier(raw),
                source_snapshot_hash: if raw.content_hash.trim().is_empty() {
                    content_hash_v1(&raw.content_md)
                } else {
                    raw.content_hash.clone()
                },
                is_numeric: !rule.numeric_tokens.is_empty(),
                is_range: false,
                is_incomplete: false,
                uncertainty_flags: uncertainty_flags.clone(),
            };
            let validation = validate_truth_candidate(&runtime, &raw.content_md);
            let rule_context_supported =
                deterministic_evidence_has_role_context(&rule.rule_key, &evidence_quote);
            let mut epistemic_status = validation.epistemic_status.clone();
            if !rule_context_supported && epistemic_status != "rejected" {
                epistemic_status = "needs_hitl".to_string();
            }
            if ontology.needs_hitl && epistemic_status != "rejected" {
                epistemic_status = "needs_hitl".to_string();
            }
            if schema.status == "invalid" && epistemic_status != "rejected" {
                epistemic_status = "rejected".to_string();
            }
            candidates.push(ValidatedTruthCandidateRecord {
                section_id: section.section_id,
                page_id: section.page_id,
                rule_candidate_id: runtime.rule_candidate_id,
                role: runtime.role,
                concept_canonical_key: runtime.concept_canonical_key,
                raw_mention: runtime.raw_mention,
                params: runtime.params,
                scope: runtime.scope,
                source_key: runtime.source_key,
                source_tier: runtime.source_tier,
                confidence: runtime.confidence,
                evidence_quote: runtime.evidence_quote,
                span_start: runtime.span_start,
                span_end: runtime.span_end,
                source_snapshot_hash: runtime.source_snapshot_hash,
                provider_key: "deterministic".to_string(),
                model_key: "cutover-runtime".to_string(),
                prompt_version: "cutover@1".to_string(),
                freshness_class: classify_candidate_freshness_local(&uncertainty_flags),
                completeness_class: classify_candidate_completeness_local(&validation),
                epistemic_status,
                issues: validation
                    .issues
                    .iter()
                    .map(|issue| format!("{}:{}", issue.code, issue.message))
                    .chain(
                        (!rule_context_supported)
                            .then_some("evidence_role_context_unsubstantiated:quoted sentence does not establish the extracted rule".to_string()),
                    )
                    .collect(),
            });
        }
        if manual_enabled && !section.blocked_by_gate && !section.skipped {
            let (source, response) = manual_by_section.remove(&raw.id).ok_or_else(|| {
                DomainError::ValidationFailure {
                    message: format!("manual truth source missing: {}", raw.id),
                }
            })?;
            let current_snapshot_hash = if raw.content_hash.trim().is_empty() {
                content_hash_v1(&raw.content_md)
            } else {
                raw.content_hash.clone()
            };
            if source.context_key != input.context_key
                || source.raw_section_id != raw.id
                || source.raw_text != raw.content_md
                || source.source_url != raw.source_url
                || source.source_domain != raw.source_domain
                || source.source_snapshot_hash != current_snapshot_hash
            {
                return Err(DomainError::ValidationFailure {
                    message: format!("manual truth evidence snapshot changed: {}", raw.id),
                });
            }
            let provider_key = response.provider_key.clone();
            let model_key = response.model_key.clone();
            let prompt_version = response.prompt_version.clone();
            for (ordinal, rule) in response.candidates.into_iter().enumerate() {
                let (params, scope) =
                    infrastructure::adapters::manual_llm_queue::validated_candidate_values(&rule)?;
                let unresolved_applicability = rule.has_unmodeled_applicability();
                let uncertainty_flags = section_uncertainty_flags(&raw.content_md)
                    .into_iter()
                    .chain(rule.uncertainty_flags)
                    .collect::<Vec<_>>();
                let runtime = TruthCandidateRuntime {
                    rule_candidate_id: blake3_hex(
                        format!(
                            "chat_manual|{}|{}|{}|{}|{}|{}",
                            input.run_id,
                            raw.id,
                            ordinal,
                            rule.role,
                            rule.concept_canonical_key,
                            rule.evidence_quote,
                        )
                        .as_bytes(),
                    ),
                    context_key: input.context_key.clone(),
                    role: rule.role,
                    concept_canonical_key: rule.concept_canonical_key,
                    raw_mention: rule.raw_mention,
                    params,
                    scope,
                    unresolved_applicability,
                    severity: rule.severity,
                    derivation_type: "manual_llm_candidate".to_string(),
                    confidence: rule.confidence,
                    evidence_section_id: raw.id,
                    evidence_quote: rule.evidence_quote,
                    span_start: rule.span_start,
                    span_end: rule.span_end,
                    source_key: raw.source_url.clone(),
                    source_tier: section_source_tier(raw),
                    source_snapshot_hash: current_snapshot_hash.clone(),
                    is_numeric: rule.is_numeric,
                    is_range: rule.is_range,
                    is_incomplete: rule.is_incomplete,
                    uncertainty_flags: uncertainty_flags.clone(),
                };
                let validation = validate_truth_candidate(&runtime, &raw.content_md);
                let mut epistemic_status = validation.epistemic_status.clone();
                if ontology.needs_hitl && epistemic_status != "rejected" {
                    epistemic_status = "needs_hitl".to_string();
                }
                if schema.status == "invalid" && epistemic_status != "rejected" {
                    epistemic_status = "rejected".to_string();
                }
                candidates.push(ValidatedTruthCandidateRecord {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    rule_candidate_id: runtime.rule_candidate_id,
                    role: runtime.role,
                    concept_canonical_key: runtime.concept_canonical_key,
                    raw_mention: runtime.raw_mention,
                    params: runtime.params,
                    scope: runtime.scope,
                    source_key: runtime.source_key,
                    source_tier: runtime.source_tier,
                    confidence: runtime.confidence,
                    evidence_quote: runtime.evidence_quote,
                    span_start: runtime.span_start,
                    span_end: runtime.span_end,
                    source_snapshot_hash: runtime.source_snapshot_hash,
                    provider_key: provider_key.clone(),
                    model_key: model_key.clone(),
                    prompt_version: prompt_version.clone(),
                    freshness_class: classify_candidate_freshness_local(&uncertainty_flags),
                    completeness_class: classify_candidate_completeness_local(&validation),
                    epistemic_status,
                    issues: validation
                        .issues
                        .iter()
                        .map(|issue| format!("{}:{}", issue.code, issue.message))
                        .collect(),
                });
            }
        }
        let accepted_count = candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "structured")
            .count();
        let needs_hitl_count = candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "needs_hitl")
            .count();
        let rejected_count = candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "rejected")
            .count();
        let status = if section.blocked_by_gate {
            "blocked"
        } else if needs_hitl_count > 0 {
            "needs_hitl"
        } else if rejected_count > 0 && accepted_count == 0 {
            "rejected"
        } else {
            "accepted"
        };
        section_states.push(CandidateValidationSectionState {
            section_id: section.section_id,
            page_id: section.page_id,
            candidates,
            accepted_count,
            needs_hitl_count,
            rejected_count,
            blocked_by_gate: section.blocked_by_gate,
            status: status.to_string(),
        });
    }
    Ok(CandidateValidationOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        accepted_count: section_states
            .iter()
            .map(|section| section.accepted_count)
            .sum(),
        needs_hitl_count: section_states
            .iter()
            .map(|section| section.needs_hitl_count)
            .sum(),
        rejected_count: section_states
            .iter()
            .map(|section| section.rejected_count)
            .sum(),
        sections: section_states,
    })
}
