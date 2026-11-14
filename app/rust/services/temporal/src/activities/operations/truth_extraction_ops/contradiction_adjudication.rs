// The canonical contradiction gate must examine proposals from both
// deterministic extraction and ChatGPT. The typed params use the existing
// canonical stable signature, never Rust Debug formatting.
// No evidence/source attributes are accepted from the LLM here.
fn validated_candidate_assertions(
    state: &CandidateValidationSectionState,
) -> Vec<seo_steps::contradiction_gate_step::FactAssertion> {
    state
        .candidates
        .iter()
        .filter(|candidate| candidate.epistemic_status != "rejected")
        .map(
            |candidate| seo_steps::contradiction_gate_step::FactAssertion {
                subject_key: format!(
                    "section:{}:{}:{}",
                    state.section_id, candidate.role, candidate.concept_canonical_key
                ),
                predicate_key: "structured_params".to_string(),
                value_normalized: candidate.params.stable_signature(),
                source_key: Some(candidate.source_key.clone()),
                confidence: candidate.confidence as f32,
            },
        )
        .collect()
}

// A blocked source section does not produce adjudication decisions. Still,
// its non-rejected claims are evidence about the same semantic identity:
// a sibling section must not globally verify that fact behind the block.
// Critical contradiction decisions are similarly a source-level veto.
fn held_semantic_identities(
    sections: &[CandidateValidationSectionState],
    decisions: &[TruthAdjudicationCandidateDecision],
) -> BTreeSet<(String, String)> {
    let mut held = decisions
        .iter()
        .filter(|decision| {
            decision.decision == "needs_hitl"
                || decision.adjudication_reason == "contradiction_block"
        })
        .map(|decision| {
            (
                decision.role.clone(),
                decision.concept_canonical_key.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    for section in sections.iter().filter(|section| section.blocked_by_gate) {
        for candidate in &section.candidates {
            if candidate.epistemic_status != "rejected" {
                held.insert((
                    candidate.role.clone(),
                    candidate.concept_canonical_key.clone(),
                ));
            }
        }
    }
    held
}

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
    let candidates_by_section: BTreeMap<i64, &CandidateValidationSectionState> = input
        .candidate_validation
        .sections
        .iter()
        .map(|state| (state.section_id, state))
        .collect();
    let mut sections = Vec::with_capacity(input.procedural.sections.len());
    for section in &input.procedural.sections {
        let raw = raw_by_section.get(&section.section_id).copied().unwrap();
        let candidate_state = candidates_by_section
            .get(&section.section_id)
            .ok_or_else(|| DomainError::ValidationFailure {
                message: format!(
                    "missing validated candidates at contradiction gate: {}",
                    section.section_id
                ),
            })?;
        let mut facts = section
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
        // Preserve procedural conflict checks and add every validated
        // structured proposition before the independent adjudication step.
        facts.extend(validated_candidate_assertions(candidate_state));
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

/// Rejection removes evidence and conservatively holds the *same scoped*
/// semantic identity. It never manufactures independent corroboration.
fn apply_exact_review_rejection_veto(
    decisions: &mut [TruthAdjudicationCandidateDecision],
    rejected_ids: &BTreeSet<String>,
) -> BTreeSet<i64> {
    let held_slots = decisions.iter()
        .filter(|d| rejected_ids.contains(&d.rule_candidate_id))
        .map(|d| (d.role.clone(), d.concept_canonical_key.clone(), d.scope.stable_signature()))
        .collect::<BTreeSet<_>>();
    let mut affected = BTreeSet::new();
    for decision in decisions {
        let slot = (
            decision.role.clone(),
            decision.concept_canonical_key.clone(),
            decision.scope.stable_signature(),
        );
        if rejected_ids.contains(&decision.rule_candidate_id) {
            decision.decision = "rejected".to_string();
            decision.publish_admissibility = "not_admissible".to_string();
            decision.verification_method = "truth_review_veto@1".to_string();
            decision.adjudication_reason = "human_rejected_exact_source_evidence".to_string();
            affected.insert(decision.section_id);
        } else if held_slots.contains(&slot) && decision.decision == "verified" {
            decision.decision = "needs_hitl".to_string();
            decision.publish_admissibility = "needs_hitl".to_string();
            decision.adjudication_reason = "reviewed_rejection_blocks_semantic_identity".to_string();
            affected.insert(decision.section_id);
        }
    }
    affected
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
    // Scope is part of the identity: student and minor corroboration may not
    // verify each other's assertions.
    let mut grouped_candidates =
        BTreeMap::<(String, String, String), Vec<&ValidatedTruthCandidateRecord>>::new();
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
                    scope: candidate.scope.clone(),
                    source_key: candidate.source_key.clone(),
                    source_tier: candidate.source_tier.clone(),
                    confidence: candidate.confidence,
                    freshness_class: candidate.freshness_class.clone(),
                    completeness_class: candidate.completeness_class.clone(),
                    evidence_quote: candidate.evidence_quote.clone(),
                    span_start: candidate.span_start,
                    span_end: candidate.span_end,
                    source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                    provider_key: candidate.provider_key.clone(),
                    model_key: candidate.model_key.clone(),
                    prompt_version: candidate.prompt_version.clone(),
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
                    scope: candidate.scope.clone(),
                    source_key: candidate.source_key.clone(),
                    source_tier: candidate.source_tier.clone(),
                    confidence: candidate.confidence,
                    freshness_class: candidate.freshness_class.clone(),
                    completeness_class: candidate.completeness_class.clone(),
                    evidence_quote: candidate.evidence_quote.clone(),
                    span_start: candidate.span_start,
                    span_end: candidate.span_end,
                    source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                    provider_key: candidate.provider_key.clone(),
                    model_key: candidate.model_key.clone(),
                    prompt_version: candidate.prompt_version.clone(),
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

        if resolution.needs_hitl
            || resolution.decision == "pause_for_hitl"
            || contradiction.output.needs_hitl
        {
            for candidate in &section.candidates {
                decisions.push(TruthAdjudicationCandidateDecision {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    rule_candidate_id: candidate.rule_candidate_id.clone(),
                    role: candidate.role.clone(),
                    concept_canonical_key: candidate.concept_canonical_key.clone(),
                    params: candidate.params.clone(),
                    scope: candidate.scope.clone(),
                    source_key: candidate.source_key.clone(),
                    source_tier: candidate.source_tier.clone(),
                    confidence: candidate.confidence,
                    freshness_class: candidate.freshness_class.clone(),
                    completeness_class: "partial".to_string(),
                    evidence_quote: candidate.evidence_quote.clone(),
                    span_start: candidate.span_start,
                    span_end: candidate.span_end,
                    source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                    provider_key: candidate.provider_key.clone(),
                    model_key: candidate.model_key.clone(),
                    prompt_version: candidate.prompt_version.clone(),
                    decision: "needs_hitl".to_string(),
                    publish_admissibility: "needs_hitl".to_string(),
                    verification_method: "truth_adjudication@1".to_string(),
                    adjudication_reason: if contradiction.output.needs_hitl {
                        "contradiction_gate_requires_hitl"
                    } else {
                        "resolution_loop_requires_hitl"
                    }
                    .to_string(),
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
                    candidate.scope.stable_signature(),
                ))
                .or_default()
                .push(candidate);
        }
    }

    // Hold the semantic slot when ANY section is unresolved, critically
    // contradictory or schema-blocked. Blocked sections have no decisions,
    // which is why the source sections must be consulted here as well.
    let held_identities =
        held_semantic_identities(&input.candidate_validation.sections, &decisions);
    for ((role, concept, _scope), group) in grouped_candidates {
        let mut structured = group
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
        if held_identities.contains(&(role, concept)) {
            for candidate in &mut structured {
                if candidate.epistemic_status == "structured" {
                    candidate.epistemic_status = "needs_hitl".to_string();
                }
            }
        }
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
                scope: candidate.scope.clone(),
                source_key: candidate.source_key.clone(),
                source_tier: candidate.source_tier.clone(),
                confidence: candidate.confidence,
                freshness_class: candidate.freshness_class.clone(),
                completeness_class: candidate.completeness_class.clone(),
                evidence_quote: candidate.evidence_quote.clone(),
                span_start: candidate.span_start,
                span_end: candidate.span_end,
                source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                provider_key: candidate.provider_key.clone(),
                model_key: candidate.model_key.clone(),
                prompt_version: candidate.prompt_version.clone(),
                decision: decision.decision,
                publish_admissibility: decision.publish_admissibility,
                verification_method: decision.verification_method,
                adjudication_reason: decision.adjudication_reason,
            });
        }
    }

    // No arbitrary JSON is allowed in Temporal. Infrastructure checks the
    // complete reviewed candidate fingerprint, original raw section and
    // source snapshot; this activity sees only typed rejected identities.
    let review_candidates = input.candidate_validation.sections.iter()
        .flat_map(|section| section.candidates.iter())
        .map(|candidate| {
            infrastructure::adapters::sqlx_hitl_adapter::TruthReviewCandidateSnapshot {
                candidate_id: candidate.rule_candidate_id.clone(),
                context_key: input.context_key.clone(),
                source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                source_url: candidate.source_key.clone(),
                role: candidate.role.clone(),
                concept: candidate.concept_canonical_key.clone(),
                params: truth_param_value_to_json_local(&candidate.params),
                scope: truth_param_value_to_json_local(&candidate.scope),
                evidence_quote: candidate.evidence_quote.clone(),
                span_start: candidate.span_start,
                span_end: candidate.span_end,
                llm_provider: candidate.provider_key.clone(),
                llm_model: candidate.model_key.clone(),
                prompt_version: candidate.prompt_version.clone(),
            }
        })
        .collect::<Vec<_>>();
    let rejected_ids =
        infrastructure::adapters::sqlx_hitl_adapter::load_exact_rejected_truth_candidate_ids(
            &acts.pool,
            &input.context_key,
            &review_candidates,
        )
        .await?;
    let affected_review_sections =
        apply_exact_review_rejection_veto(&mut decisions, &rejected_ids);

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
    // Recalculate counters after a human veto even for fixed resolution
    // states. No section may report VERIFIED from a held semantic identity.
    for state in &mut section_states {
        if !affected_review_sections.contains(&state.section_id) || state.status == "blocked" {
            continue;
        }
        let decisions_for_section = decisions.iter()
            .filter(|decision| decision.section_id == state.section_id)
            .collect::<Vec<_>>();
        state.verified_count = decisions_for_section.iter()
            .filter(|decision| decision.decision == "verified").count();
        state.needs_hitl_count = decisions_for_section.iter()
            .filter(|decision| decision.decision == "needs_hitl").count();
        state.rejected_count = decisions_for_section.iter()
            .filter(|decision| decision.decision == "rejected").count();
        let label = if state.verified_count > 0 {
            "verified"
        } else if state.needs_hitl_count > 0 {
            "needs_hitl"
        } else {
            "rejected"
        };
        state.status = label.to_string();
        state.decision = label.to_string();
    }
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

#[cfg(test)]
mod manual_candidate_contradiction_tests {
    use super::*;

    fn manual_candidate(
        amount: i64,
        confidence: f64,
        status: &str,
    ) -> ValidatedTruthCandidateRecord {
        ValidatedTruthCandidateRecord {
            section_id: 41,
            page_id: 11,
            rule_candidate_id: format!("manual-proposal-{amount}"),
            role: "FEE_ITEM".to_string(),
            concept_canonical_key: "consular_fee".to_string(),
            raw_mention: "consular fee".to_string(),
            params: TruthParamValue::object([("amount", TruthParamValue::Integer(amount))]),
            scope: TruthParamValue::object(Vec::<(String, TruthParamValue)>::new()),
            source_key: "https://example.gov/fees".to_string(),
            source_tier: "government".to_string(),
            confidence,
            evidence_quote: "consular fee".to_string(),
            span_start: 0,
            span_end: 12,
            source_snapshot_hash: "source-sha".to_string(),
            provider_key: "chat_manual".to_string(),
            model_key: "gpt-6".to_string(),
            prompt_version: "truth_extraction_prompt@1".to_string(),
            freshness_class: "fresh".to_string(),
            completeness_class: "complete".to_string(),
            epistemic_status: status.to_string(),
            issues: Vec::new(),
        }
    }

    fn section(candidates: Vec<ValidatedTruthCandidateRecord>) -> CandidateValidationSectionState {
        CandidateValidationSectionState {
            section_id: 41,
            page_id: 11,
            candidates,
            accepted_count: 0,
            needs_hitl_count: 0,
            rejected_count: 0,
            blocked_by_gate: false,
            status: "accepted".to_string(),
        }
    }

    fn gate(
        state: &CandidateValidationSectionState,
    ) -> seo_steps::contradiction_gate_step::ContradictionGateOutput {
        seo_steps::contradiction_gate_step::execute(
            &seo_steps::contradiction_gate_step::ContradictionGateInput {
                run_id: "manual-contradiction-test".to_string(),
                facts: validated_candidate_assertions(state),
            },
        )
    }

    #[test]
    fn rejected_source_holds_same_scope_but_not_other_profile() {
        let base = manual_candidate(35, 0.95, "structured");
        let make = |id: &str, scope: TruthParamValue| TruthAdjudicationCandidateDecision {
            section_id: 41,
            page_id: 11,
            rule_candidate_id: id.to_string(),
            role: base.role.clone(),
            concept_canonical_key: base.concept_canonical_key.clone(),
            params: base.params.clone(),
            scope,
            source_key: base.source_key.clone(),
            source_tier: base.source_tier.clone(),
            confidence: base.confidence,
            freshness_class: base.freshness_class.clone(),
            completeness_class: base.completeness_class.clone(),
            evidence_quote: base.evidence_quote.clone(),
            span_start: base.span_start,
            span_end: base.span_end,
            source_snapshot_hash: base.source_snapshot_hash.clone(),
            provider_key: base.provider_key.clone(),
            model_key: base.model_key.clone(),
            prompt_version: base.prompt_version.clone(),
            decision: "verified".to_string(),
            publish_admissibility: "admissible".to_string(),
            verification_method: "truth_governance@1".to_string(),
            adjudication_reason: "independent".to_string(),
        };
        let global = TruthParamValue::object(Vec::<(String, TruthParamValue)>::new());
        let student = TruthParamValue::object([
            ("applicant_profile", TruthParamValue::Text("student".to_string())),
        ]);
        let mut decisions = vec![
            make("review-rejected", global.clone()),
            make("other-source", global),
            make("other-profile", student),
        ];
        let sections = apply_exact_review_rejection_veto(
            &mut decisions,
            &BTreeSet::from(["review-rejected".to_string()]),
        );
        assert!(sections.contains(&41));
        assert_eq!(decisions[0].decision, "rejected");
        assert_eq!(decisions[1].decision, "needs_hitl");
        assert_eq!(decisions[2].decision, "verified");
        assert_ne!(decisions[0].publish_admissibility, "admissible");
        assert_ne!(decisions[1].publish_admissibility, "admissible");
    }

    #[test]
    fn conflicting_manual_candidates_cannot_bypass_critical_gate() {
        let result = gate(&section(vec![
            manual_candidate(80, 0.95, "structured"),
            manual_candidate(120, 0.92, "structured"),
        ]));
        assert!(result.is_blocked);
        assert!(result.needs_hitl);
        assert_eq!(result.conflict_count, 1);
        assert_eq!(result.conflicts[0].severity, "critical");
    }

    #[test]
    fn noncritical_manual_conflicts_still_require_hitl() {
        let result = gate(&section(vec![
            manual_candidate(80, 0.78, "structured"),
            manual_candidate(120, 0.70, "structured"),
        ]));
        assert!(!result.is_blocked);
        assert!(result.needs_hitl);
        assert_eq!(result.conflict_count, 1);
    }

    #[test]
    fn matching_manual_claims_do_not_create_conflict() {
        let result = gate(&section(vec![
            manual_candidate(80, 0.95, "structured"),
            manual_candidate(80, 0.82, "structured"),
        ]));
        assert!(!result.is_blocked);
        assert!(!result.needs_hitl);
        assert_eq!(result.conflict_count, 0);
    }

    #[test]
    fn blocked_section_holds_sibling_semantic_identity_without_a_decision() {
        let mut blocked = section(vec![manual_candidate(80, 0.95, "structured")]);
        blocked.blocked_by_gate = true;
        let held = held_semantic_identities(&[blocked], &[]);
        assert!(held.contains(&("FEE_ITEM".to_string(), "consular_fee".to_string())));
        assert!(!held.contains(&("FEE_ITEM".to_string(), "other_fee".to_string())));
    }

    #[test]
    fn rejected_candidate_in_blocked_section_does_not_poison_sibling() {
        let mut blocked = section(vec![manual_candidate(80, 0.95, "rejected")]);
        blocked.blocked_by_gate = true;
        assert!(held_semantic_identities(&[blocked], &[]).is_empty());
    }

    #[test]
    fn held_semantic_identity_stops_independent_global_corroboration() {
        let mut blocked = section(vec![manual_candidate(120, 0.96, "structured")]);
        blocked.blocked_by_gate = true;
        let held = held_semantic_identities(&[blocked], &[]);
        let mut siblings = vec![
            TruthStructuredCandidate {
                rule_candidate_id: "official-1".to_string(),
                role: "FEE_ITEM".to_string(),
                concept_canonical_key: "consular_fee".to_string(),
                params: TruthParamValue::object([("amount", TruthParamValue::Integer(80))]),
                epistemic_status: "structured".to_string(),
                ..TruthStructuredCandidate::default()
            },
            TruthStructuredCandidate {
                rule_candidate_id: "official-2".to_string(),
                role: "FEE_ITEM".to_string(),
                concept_canonical_key: "consular_fee".to_string(),
                params: TruthParamValue::object([("amount", TruthParamValue::Integer(80))]),
                epistemic_status: "structured".to_string(),
                ..TruthStructuredCandidate::default()
            },
        ];
        if held.contains(&("FEE_ITEM".to_string(), "consular_fee".to_string())) {
            for candidate in &mut siblings {
                if candidate.epistemic_status == "structured" {
                    candidate.epistemic_status = "needs_hitl".to_string();
                }
            }
        }
        let result = adjudicate_truth_candidates_with_governance(&siblings, &BTreeMap::new());
        assert_eq!(result.verified_count, 0);
        assert_eq!(result.needs_hitl_count, 2);
        assert!(result
            .decisions
            .iter()
            .all(|decision| { decision.publish_admissibility != "admissible" }));
    }

    #[test]
    fn rejected_candidate_is_not_allowed_to_manufacture_a_conflict() {
        let result = gate(&section(vec![
            manual_candidate(80, 0.95, "structured"),
            manual_candidate(120, 0.99, "rejected"),
        ]));
        assert_eq!(result.conflict_count, 0);
    }
}
