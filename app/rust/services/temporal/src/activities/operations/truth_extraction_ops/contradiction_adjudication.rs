pub(crate) async fn contradiction_gate_sweep_impl(
    acts: &AlegriaActivities,
    input: &ContradictionGateSweepInput,
) -> Result<ContradictionGateSweepOutput, DomainError> {
    ensure_graph_required_for_phase("contradiction_gate").await?;
    let raw_sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let raw_by_section: BTreeMap<i64, &raw_crawl_adapter::RawSectionRecord> = raw_sections
        .iter()
        .map(|section| (section.id, section))
        .collect();
    let mut sections = Vec::with_capacity(input.procedural.sections.len());
    for section in &input.procedural.sections {
        let raw = raw_by_section.get(&section.section_id).copied().unwrap();
        let facts = section
            .rules
            .iter()
            .map(|rule| {
                let predicate = match rule.rule_key.as_str() {
                    "consular_fee" => "amount",
                    "processing_time" => "days",
                    _ => "required",
                };
                let value_normalized = if !rule.numeric_tokens.is_empty() {
                    rule.numeric_tokens.join("|")
                } else {
                    rule.rule_key.clone()
                };
                seo_steps::contradiction_gate_step::FactAssertion {
                    subject_key: format!("section:{}:{}", section.section_id, rule.rule_key),
                    predicate_key: predicate.to_string(),
                    value_normalized,
                    source_key: Some(raw.source_url.clone()),
                    confidence: rule.confidence,
                }
            })
            .collect::<Vec<_>>();
        let output = seo_steps::contradiction_gate_step::execute(
            &seo_steps::contradiction_gate_step::ContradictionGateInput {
                run_id: input.run_id.clone(),
                facts,
            },
        );
        let mut semantic_neighbor_refs = Vec::new();
        let mut semantic_neighbor_reason_codes = Vec::new();
        if !section.blocked_by_gate {
            let contradiction_query = format!(
                "{} {}",
                raw.heading_path,
                raw.content_md
                    .split_whitespace()
                    .take(64)
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            let (_, neighbors, reason_codes) = retrieve_semantic_neighbors(
                &contradiction_query,
                "verified_rules_4",
                semantic_search_adapter::VoyageSearchSurface::Standard,
                None,
                4,
            )
            .await?;
            semantic_neighbor_refs = neighbors
                .into_iter()
                .map(|record| format!("{}:{:.4}", record.entity_key, record.score))
                .collect();
            semantic_neighbor_reason_codes.extend(reason_codes);
            if output.needs_hitl && !semantic_neighbor_refs.is_empty() {
                semantic_neighbor_reason_codes
                    .push("semantic_neighbor_conflict_review_required".to_string());
            }
        }
        sections.push(ContradictionGateSectionState {
            section_id: section.section_id,
            page_id: section.page_id,
            status: if section.blocked_by_gate {
                "blocked"
            } else if output.is_blocked {
                "rejected"
            } else if output.needs_hitl {
                "needs_hitl"
            } else {
                "pass"
            }
            .to_string(),
            output,
            semantic_neighbor_refs,
            semantic_neighbor_reason_codes,
        });
    }
    Ok(ContradictionGateSweepOutput {
        section_count: sections.len(),
        blocked_section_count: sections
            .iter()
            .filter(|section| section.status == "blocked")
            .count(),
        needs_hitl_count: sections
            .iter()
            .filter(|section| section.status == "needs_hitl")
            .count(),
        conflict_count: sections
            .iter()
            .map(|section| section.output.conflict_count)
            .sum(),
        sections,
    })
}

pub(crate) async fn truth_adjudication_sweep_impl(
    acts: &AlegriaActivities,
    input: &TruthAdjudicationSweepInput,
) -> Result<TruthAdjudicationSweepOutput, DomainError> {
    let resolution_by_section: BTreeMap<i64, &ResolutionLoopSectionState> = input
        .resolution
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let contradiction_by_section: BTreeMap<i64, &ContradictionGateSectionState> = input
        .contradiction
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();

    let mut decisions = Vec::new();
    let mut candidate_bindings =
        BTreeMap::<String, (&ValidatedTruthCandidateRecord, i64, i64)>::new();
    let mut grouped_candidates =
        BTreeMap::<(String, String), Vec<&ValidatedTruthCandidateRecord>>::new();
    let mut fixed_states = BTreeMap::<i64, TruthAdjudicationSectionState>::new();
    let source_registry = load_source_registry_entries(&acts.pool)
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

    for section in &input.candidate_validation.sections {
        let resolution = resolution_by_section
            .get(&section.section_id)
            .copied()
            .unwrap();
        let contradiction = contradiction_by_section
            .get(&section.section_id)
            .copied()
            .unwrap();
        for candidate in &section.candidates {
            candidate_bindings.insert(
                candidate.rule_candidate_id.clone(),
                (candidate, section.section_id, section.page_id),
            );
        }

        if section.blocked_by_gate {
            fixed_states.insert(
                section.section_id,
                TruthAdjudicationSectionState {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    decision: "blocked".to_string(),
                    status: "blocked".to_string(),
                    verified_count: 0,
                    needs_hitl_count: 0,
                    rejected_count: 0,
                },
            );
            continue;
        }

        let structured_count = section
            .candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "structured")
            .count();
        let needs_hitl_candidates = section
            .candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "needs_hitl")
            .collect::<Vec<_>>();
        if structured_count == 0 && !needs_hitl_candidates.is_empty() {
            for candidate in &needs_hitl_candidates {
                decisions.push(TruthAdjudicationCandidateDecision {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    rule_candidate_id: candidate.rule_candidate_id.clone(),
                    role: candidate.role.clone(),
                    concept_canonical_key: candidate.concept_canonical_key.clone(),
                    params: candidate.params.clone(),
                    source_key: candidate.source_key.clone(),
                    source_tier: candidate.source_tier.clone(),
                    confidence: candidate.confidence,
                    freshness_class: candidate.freshness_class.clone(),
                    completeness_class: candidate.completeness_class.clone(),
                    evidence_quote: candidate.evidence_quote.clone(),
                    span_start: candidate.span_start,
                    span_end: candidate.span_end,
                    source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                    decision: "needs_hitl".to_string(),
                    publish_admissibility: "needs_hitl".to_string(),
                    verification_method: "truth_adjudication@1".to_string(),
                    adjudication_reason: non_structured_candidate_reason(candidate),
                });
            }
            fixed_states.insert(
                section.section_id,
                TruthAdjudicationSectionState {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    decision: "needs_hitl".to_string(),
                    status: "needs_hitl".to_string(),
                    verified_count: 0,
                    needs_hitl_count: needs_hitl_candidates.len(),
                    rejected_count: 0,
                },
            );
            continue;
        }

        if contradiction.output.is_blocked {
            for candidate in &section.candidates {
                decisions.push(TruthAdjudicationCandidateDecision {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    rule_candidate_id: candidate.rule_candidate_id.clone(),
                    role: candidate.role.clone(),
                    concept_canonical_key: candidate.concept_canonical_key.clone(),
                    params: candidate.params.clone(),
                    source_key: candidate.source_key.clone(),
                    source_tier: candidate.source_tier.clone(),
                    confidence: candidate.confidence,
                    freshness_class: candidate.freshness_class.clone(),
                    completeness_class: candidate.completeness_class.clone(),
                    evidence_quote: candidate.evidence_quote.clone(),
                    span_start: candidate.span_start,
                    span_end: candidate.span_end,
                    source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                    decision: "rejected".to_string(),
                    publish_admissibility: "not_admissible".to_string(),
                    verification_method: "truth_adjudication@1".to_string(),
                    adjudication_reason: "contradiction_block".to_string(),
                });
            }
            fixed_states.insert(
                section.section_id,
                TruthAdjudicationSectionState {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    decision: "rejected".to_string(),
                    status: "rejected".to_string(),
                    verified_count: 0,
                    needs_hitl_count: 0,
                    rejected_count: section.candidates.len(),
                },
            );
            continue;
        }

        if resolution.needs_hitl || resolution.decision == "pause_for_hitl" {
            for candidate in &section.candidates {
                decisions.push(TruthAdjudicationCandidateDecision {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    rule_candidate_id: candidate.rule_candidate_id.clone(),
                    role: candidate.role.clone(),
                    concept_canonical_key: candidate.concept_canonical_key.clone(),
                    params: candidate.params.clone(),
                    source_key: candidate.source_key.clone(),
                    source_tier: candidate.source_tier.clone(),
                    confidence: candidate.confidence,
                    freshness_class: candidate.freshness_class.clone(),
                    completeness_class: "partial".to_string(),
                    evidence_quote: candidate.evidence_quote.clone(),
                    span_start: candidate.span_start,
                    span_end: candidate.span_end,
                    source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                    decision: "needs_hitl".to_string(),
                    publish_admissibility: "needs_hitl".to_string(),
                    verification_method: "truth_adjudication@1".to_string(),
                    adjudication_reason: "resolution_loop_requires_hitl".to_string(),
                });
            }
            fixed_states.insert(
                section.section_id,
                TruthAdjudicationSectionState {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    decision: "needs_hitl".to_string(),
                    status: "needs_hitl".to_string(),
                    verified_count: 0,
                    needs_hitl_count: section.candidates.len(),
                    rejected_count: 0,
                },
            );
            continue;
        }

        for candidate in &section.candidates {
            grouped_candidates
                .entry((
                    candidate.role.clone(),
                    candidate.concept_canonical_key.clone(),
                ))
                .or_default()
                .push(candidate);
        }
    }

    for ((_role, _concept), group) in grouped_candidates {
        let structured = group
            .iter()
            .map(|candidate| TruthStructuredCandidate {
                rule_candidate_id: candidate.rule_candidate_id.clone(),
                context_key: input.context_key.clone(),
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
            })
            .collect::<Vec<_>>();
        let adjudication =
            adjudicate_truth_candidates_with_governance(&structured, &source_registry);
        for decision in adjudication.decisions {
            let (candidate, section_id, page_id) = candidate_bindings
                .get(&decision.rule_candidate_id)
                .copied()
                .ok_or_else(|| DomainError::ValidationFailure {
                    message: format!(
                        "candidate section binding is missing for `{}`",
                        decision.rule_candidate_id
                    ),
                })?;
            decisions.push(TruthAdjudicationCandidateDecision {
                section_id,
                page_id,
                rule_candidate_id: candidate.rule_candidate_id.clone(),
                role: candidate.role.clone(),
                concept_canonical_key: candidate.concept_canonical_key.clone(),
                params: candidate.params.clone(),
                source_key: candidate.source_key.clone(),
                source_tier: candidate.source_tier.clone(),
                confidence: candidate.confidence,
                freshness_class: candidate.freshness_class.clone(),
                completeness_class: candidate.completeness_class.clone(),
                evidence_quote: candidate.evidence_quote.clone(),
                span_start: candidate.span_start,
                span_end: candidate.span_end,
                source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                decision: decision.decision,
                publish_admissibility: decision.publish_admissibility,
                verification_method: decision.verification_method,
                adjudication_reason: decision.adjudication_reason,
            });
        }
    }

    let mut section_states = input
        .candidate_validation
        .sections
        .iter()
        .map(|section| {
            if let Some(state) = fixed_states.remove(&section.section_id) {
                return state;
            }
            let mut verified_count = 0usize;
            let mut needs_hitl_count = 0usize;
            let mut rejected_count = 0usize;
            for decision in decisions
                .iter()
                .filter(|decision| decision.section_id == section.section_id)
            {
                match decision.decision.as_str() {
                    "verified" => verified_count += 1,
                    "needs_hitl" => needs_hitl_count += 1,
                    _ => rejected_count += 1,
                }
            }
            let section_decision = if verified_count > 0 {
                "verified"
            } else if needs_hitl_count > 0 {
                "needs_hitl"
            } else {
                "rejected"
            };
            TruthAdjudicationSectionState {
                section_id: section.section_id,
                page_id: section.page_id,
                decision: section_decision.to_string(),
                status: section_decision.to_string(),
                verified_count,
                needs_hitl_count,
                rejected_count,
            }
        })
        .collect::<Vec<_>>();
    section_states.sort_by_key(|section| section.section_id);

    Ok(TruthAdjudicationSweepOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.status == "blocked")
            .count(),
        verified_count: section_states
            .iter()
            .map(|section| section.verified_count)
            .sum(),
        needs_hitl_count: section_states
            .iter()
            .map(|section| section.needs_hitl_count)
            .sum(),
        rejected_count: section_states
            .iter()
            .map(|section| section.rejected_count)
            .sum(),
        decisions,
        sections: section_states,
    })
}

