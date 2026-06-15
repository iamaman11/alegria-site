pub(crate) async fn ontology_intake_gate_impl(
    input: &OntologyIntakeGateInput,
) -> Result<OntologyIntakeGateOutput, DomainError> {
    let sections = input
        .canonical_mapping
        .sections
        .iter()
        .map(|section| {
            let unresolved_mentions = section
                .mappings
                .iter()
                .filter(|mapping| mapping.needs_hitl || mapping.canonical_key.is_none())
                .map(|mapping| mapping.raw_text.clone())
                .collect::<Vec<_>>();
            let accepted_keys = section
                .mappings
                .iter()
                .filter_map(|mapping| mapping.canonical_key.clone())
                .collect::<Vec<_>>();
            let needs_hitl = !unresolved_mentions.is_empty();
            let decision = if section.blocked_by_gate {
                "blocked_by_gate"
            } else if needs_hitl {
                "needs_hitl"
            } else {
                "pass"
            };
            OntologyIntakeGateDecision {
                section_id: section.section_id,
                page_id: section.page_id,
                accepted_keys,
                unresolved_mentions,
                blocked_by_gate: section.blocked_by_gate,
                needs_hitl,
                decision: decision.to_string(),
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = sections
        .iter()
        .filter(|section| section.blocked_by_gate)
        .count();
    let needs_hitl_count = sections
        .iter()
        .filter(|section| !section.blocked_by_gate && section.needs_hitl)
        .count();
    Ok(OntologyIntakeGateOutput {
        section_count: sections.len(),
        blocked_section_count,
        needs_hitl_count,
        sections,
    })
}

pub(crate) async fn procedural_extraction_sweep_impl(
    acts: &AlegriaActivities,
    input: &ProceduralExtractionSweepInput,
) -> Result<ProceduralExtractionSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let entity_by_section: BTreeMap<i64, &EntitySpanSectionMentions> = input
        .entity_spans
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let section_states = sections
        .iter()
        .map(|section| {
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            // Procedural extraction remains allowed even when ontology intake marked
            // entity-span mentions as unresolved. Otherwise fee/timeline sections with
            // deterministic numeric patterns get dropped before the strict procedural
            // path can build candidates, which weakens expert extraction and turns
            // ontology ambiguity into a false hard skip.
            let skipped = !gates.allow_procedural_extraction(section.id);
            let rules = if blocked_by_gate || skipped {
                Vec::new()
            } else {
                let entity = entity_by_section.get(&section.id).copied().unwrap();
                seo_steps::procedural_extraction_step::execute(
                    &seo_steps::procedural_extraction_step::ProceduralExtractionInput {
                        section_id: section.id.to_string(),
                        raw_text: section.content_md.clone(),
                        mentions: entity.mentions.clone(),
                    },
                )
                .rules
            };
            let decision = if blocked_by_gate {
                "blocked_by_gate"
            } else if skipped {
                "skipped"
            } else {
                "pass"
            };
            ProceduralExtractionSectionState {
                section_id: section.id,
                page_id: section.page_id,
                rules,
                blocked_by_gate,
                skipped,
                decision: decision.to_string(),
            }
        })
        .collect::<Vec<_>>();
    Ok(ProceduralExtractionSweepOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        skipped_section_count: section_states
            .iter()
            .filter(|section| section.skipped)
            .count(),
        rule_count: section_states
            .iter()
            .map(|section| section.rules.len())
            .sum(),
        sections: section_states,
    })
}

pub(crate) async fn operational_extraction_sweep_impl(
    acts: &AlegriaActivities,
    input: &OperationalExtractionSweepInput,
) -> Result<OperationalExtractionSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let section_states = sections
        .iter()
        .map(|section| {
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            let entities = if blocked_by_gate {
                Vec::new()
            } else {
                seo_steps::operational_extraction_step::execute(
                    &seo_steps::operational_extraction_step::OperationalExtractionInput {
                        section_id: section.id.to_string(),
                        raw_text: section.content_md.clone(),
                    },
                )
                .entities
            };
            OperationalExtractionSectionState {
                section_id: section.id,
                page_id: section.page_id,
                entities,
                blocked_by_gate,
                decision: if blocked_by_gate {
                    "blocked_by_gate"
                } else {
                    "pass"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    Ok(OperationalExtractionSweepOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        entity_count: section_states
            .iter()
            .map(|section| section.entities.len())
            .sum(),
        sections: section_states,
    })
}

pub(crate) async fn editorial_extraction_sweep_impl(
    acts: &AlegriaActivities,
    input: &EditorialExtractionSweepInput,
) -> Result<EditorialExtractionSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let section_states = sections
        .iter()
        .map(|section| {
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            let skipped = !gates.allow_editorial_extraction(section.id);
            let topics = if blocked_by_gate || skipped {
                Vec::new()
            } else {
                seo_steps::editorial_extraction_step::execute(
                    &seo_steps::editorial_extraction_step::EditorialExtractionInput {
                        section_id: section.id.to_string(),
                        raw_text: section.content_md.clone(),
                    },
                )
                .topics
            };
            EditorialExtractionSectionState {
                section_id: section.id,
                page_id: section.page_id,
                topics,
                blocked_by_gate,
                skipped,
                decision: if blocked_by_gate {
                    "blocked_by_gate"
                } else if skipped {
                    "skipped"
                } else {
                    "pass"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    Ok(EditorialExtractionSweepOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        skipped_section_count: section_states
            .iter()
            .filter(|section| section.skipped)
            .count(),
        topic_count: section_states
            .iter()
            .map(|section| section.topics.len())
            .sum(),
        sections: section_states,
    })
}

pub(crate) async fn seo_signal_extraction_sweep_impl(
    acts: &AlegriaActivities,
    input: &SeoSignalExtractionSweepInput,
) -> Result<SeoSignalExtractionSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let section_states = sections
        .iter()
        .map(|section| {
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            let lowered = section.content_md.to_lowercase();
            let mut signals = Vec::new();
            if !blocked_by_gate && (lowered.contains("seo") || lowered.contains("serp")) {
                signals.push(SeoSignalRecord {
                    signal_type: "ranking_signal".to_string(),
                    value: "seo_or_serp_mentioned".to_string(),
                    confidence: 0.82,
                });
            }
            if !blocked_by_gate && (lowered.contains("keyword") || lowered.contains("ключев"))
            {
                signals.push(SeoSignalRecord {
                    signal_type: "keyword_signal".to_string(),
                    value: "keyword_language_present".to_string(),
                    confidence: 0.79,
                });
            }
            SeoSignalExtractionSectionState {
                section_id: section.id,
                page_id: section.page_id,
                signals,
                blocked_by_gate,
                decision: if blocked_by_gate {
                    "blocked_by_gate"
                } else {
                    "pass"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    Ok(SeoSignalExtractionSweepOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        signal_count: section_states
            .iter()
            .map(|section| section.signals.len())
            .sum(),
        sections: section_states,
    })
}

pub(crate) async fn commercial_signal_extraction_sweep_impl(
    acts: &AlegriaActivities,
    input: &CommercialSignalExtractionSweepInput,
) -> Result<CommercialSignalExtractionSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let section_states = sections
        .iter()
        .map(|section| {
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            let lowered = section.content_md.to_lowercase();
            let mut signals = Vec::new();
            if !blocked_by_gate
                && (lowered.contains("консультац")
                    || lowered.contains("под ключ")
                    || lowered.contains("заказать")
                    || lowered.contains("service"))
            {
                signals.push(CommercialSignalRecord {
                    signal_type: "service_offer".to_string(),
                    value: "service_offer_detected".to_string(),
                    confidence: 0.84,
                });
            }
            if !blocked_by_gate
                && (lowered.contains("стоимость услуги")
                    || lowered.contains("our fee")
                    || lowered.contains("price"))
            {
                signals.push(CommercialSignalRecord {
                    signal_type: "commercial_price".to_string(),
                    value: "commercial_price_detected".to_string(),
                    confidence: 0.81,
                });
            }
            CommercialSignalExtractionSectionState {
                section_id: section.id,
                page_id: section.page_id,
                signals,
                blocked_by_gate,
                decision: if blocked_by_gate {
                    "blocked_by_gate"
                } else {
                    "pass"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    Ok(CommercialSignalExtractionSweepOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        signal_count: section_states
            .iter()
            .map(|section| section.signals.len())
            .sum(),
        sections: section_states,
    })
}

