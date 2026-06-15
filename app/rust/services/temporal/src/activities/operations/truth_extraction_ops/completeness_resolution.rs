pub(crate) async fn completeness_judge_sweep_impl(
    acts: &AlegriaActivities,
    input: &CompletenessJudgeSweepInput,
) -> Result<CompletenessJudgeSweepOutput, DomainError> {
    ensure_graph_required_for_phase("completeness_judge").await?;
    let raw_sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let raw_by_section: BTreeMap<i64, &raw_crawl_adapter::RawSectionRecord> = raw_sections
        .iter()
        .map(|section| (section.id, section))
        .collect();
    let entity_by_section: BTreeMap<i64, &EntitySpanSectionMentions> = input
        .entity_spans
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let mut sections = Vec::with_capacity(input.procedural.sections.len());
    for section in &input.procedural.sections {
        let raw = raw_by_section.get(&section.section_id).copied().unwrap();
        let entity = entity_by_section.get(&section.section_id).copied().unwrap();
        let numeric_tokens: BTreeSet<String> = section
            .rules
            .iter()
            .flat_map(|rule| normalize_numeric_token_fragments(&rule.numeric_tokens))
            .collect();
        let source_numeric_tokens: BTreeSet<String> = entity
            .mentions
            .iter()
            .filter(|mention| mention.has_numeric)
            .flat_map(|mention| {
                normalize_numeric_token_fragments(std::slice::from_ref(&mention.raw_text))
            })
            .collect();
        let output = seo_steps::completeness_judge_step::execute(
            &seo_steps::completeness_judge_step::CompletenessJudgeInput {
                section_id: section.section_id.to_string(),
                raw_text: raw.content_md.clone(),
                source_numeric_tokens: source_numeric_tokens.into_iter().collect(),
                extracted_numeric_tokens: numeric_tokens.into_iter().collect(),
                extracted_rule_keys: section
                    .rules
                    .iter()
                    .map(|rule| rule.rule_key.clone())
                    .collect(),
            },
        );
        let mut retrieval_collection_used = None;
        let mut retrieval_evidence_refs = Vec::new();
        let mut semantic_diagnostic_reason_codes = Vec::new();
        if !section.blocked_by_gate {
            let (collection_used, neighbors, reason_codes) = retrieve_semantic_neighbors(
                &raw.content_md,
                "raw_chunks_ctx",
                semantic_search_adapter::VoyageSearchSurface::Contextualized,
                Some((
                    "raw_chunks_4",
                    semantic_search_adapter::VoyageSearchSurface::Standard,
                )),
                4,
            )
            .await?;
            retrieval_collection_used = collection_used;
            retrieval_evidence_refs = neighbors
                .into_iter()
                .map(|record| format!("{}:{:.4}", record.entity_key, record.score))
                .collect();
            semantic_diagnostic_reason_codes.extend(reason_codes);
            if retrieval_evidence_refs.is_empty() {
                semantic_diagnostic_reason_codes.push("semantic_neighbor_gap_detected".to_string());
            }
        }
        sections.push(CompletenessJudgeSectionState {
            section_id: section.section_id,
            page_id: section.page_id,
            blocked_by_gate: section.blocked_by_gate,
            status: if section.blocked_by_gate {
                "blocked"
            } else if output.needs_hitl {
                "needs_hitl"
            } else {
                "pass"
            }
            .to_string(),
            output,
            retrieval_collection_used,
            retrieval_evidence_refs,
            semantic_diagnostic_reason_codes,
        });
    }
    Ok(CompletenessJudgeSweepOutput {
        section_count: sections.len(),
        blocked_section_count: sections
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        needs_hitl_count: sections
            .iter()
            .filter(|section| !section.blocked_by_gate && section.output.needs_hitl)
            .count(),
        sections,
    })
}

pub(crate) async fn resolution_loop_impl(
    input: &ResolutionLoopInput,
) -> Result<ResolutionLoopOutput, DomainError> {
    ensure_graph_required_for_phase("resolution_loop").await?;
    let ontology_by_section: BTreeMap<i64, &OntologyIntakeGateDecision> = input
        .ontology
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let schema_by_section: BTreeMap<i64, &ExtractionSchemaSectionDecision> = input
        .schema_validate
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let mut sections = Vec::with_capacity(input.completeness.sections.len());
    for section in &input.completeness.sections {
        let ontology = ontology_by_section
            .get(&section.section_id)
            .copied()
            .unwrap();
        let schema = schema_by_section.get(&section.section_id).copied().unwrap();
        let decision = if section.blocked_by_gate {
            "drop_with_reason"
        } else if section.output.needs_hitl || ontology.needs_hitl {
            "pause_for_hitl"
        } else if schema.status == "invalid" {
            "drop_with_reason"
        } else {
            "accept"
        };
        let blockers = section
            .output
            .missing_elements
            .iter()
            .map(|missing| missing.action.clone())
            .chain(ontology.unresolved_mentions.iter().cloned())
            .collect::<Vec<_>>();
        let mut retrieval_trace_refs = section.retrieval_evidence_refs.clone();
        let mut retrieval_trace_reason_codes = section.semantic_diagnostic_reason_codes.clone();
        if !section.blocked_by_gate
            && (decision == "pause_for_hitl" || decision == "drop_with_reason")
        {
            let retrieval_query = if blockers.is_empty() {
                section
                    .retrieval_evidence_refs
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                blockers.join(" ")
            };
            if !retrieval_query.trim().is_empty() {
                let (_, verified_neighbors, verified_reason_codes) = retrieve_semantic_neighbors(
                    &retrieval_query,
                    "verified_rules_4",
                    semantic_search_adapter::VoyageSearchSurface::Standard,
                    None,
                    3,
                )
                .await?;
                for neighbor in verified_neighbors {
                    retrieval_trace_refs.push(format!(
                        "verified_rules_4:{}:{:.4}",
                        neighbor.entity_key, neighbor.score
                    ));
                }
                retrieval_trace_reason_codes.extend(verified_reason_codes);
                let (_, canonical_neighbors, canonical_reason_codes) = retrieve_semantic_neighbors(
                    &retrieval_query,
                    "kb_canonical_4",
                    semantic_search_adapter::VoyageSearchSurface::Standard,
                    None,
                    2,
                )
                .await?;
                for neighbor in canonical_neighbors {
                    retrieval_trace_refs.push(format!(
                        "kb_canonical_4:{}:{:.4}",
                        neighbor.entity_key, neighbor.score
                    ));
                }
                retrieval_trace_reason_codes.extend(canonical_reason_codes);
            }
        }
        sections.push(ResolutionLoopSectionState {
            section_id: section.section_id,
            page_id: section.page_id,
            decision: decision.to_string(),
            blockers,
            needs_hitl: !section.blocked_by_gate
                && (section.output.needs_hitl || ontology.needs_hitl),
            blocked_by_gate: section.blocked_by_gate,
            retrieval_trace_refs,
            retrieval_trace_reason_codes,
        });
    }
    Ok(ResolutionLoopOutput {
        section_count: sections.len(),
        blocked_section_count: sections
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        needs_hitl_count: sections.iter().filter(|section| section.needs_hitl).count(),
        rejected_count: sections
            .iter()
            .filter(|section| !section.blocked_by_gate && section.decision == "drop_with_reason")
            .count(),
        sections,
    })
}

