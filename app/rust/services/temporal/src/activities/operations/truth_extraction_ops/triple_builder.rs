pub(crate) async fn triple_builder_sweep_impl(
    input: &TripleBuilderSweepInput,
) -> Result<TripleBuilderSweepOutput, DomainError> {
    let operational_by_section: BTreeMap<i64, &OperationalExtractionSectionState> = input
        .operational
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let editorial_by_section: BTreeMap<i64, &EditorialExtractionSectionState> = input
        .editorial
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let sections = input
        .procedural
        .sections
        .iter()
        .map(|section| {
            let operational = operational_by_section
                .get(&section.section_id)
                .copied()
                .unwrap();
            let editorial = editorial_by_section
                .get(&section.section_id)
                .copied()
                .unwrap();
            let output = seo_steps::triple_builder_step::execute(
                &seo_steps::triple_builder_step::TripleBuilderInput {
                    section_id: section.section_id.to_string(),
                    procedural_rules: section
                        .rules
                        .iter()
                        .map(
                            |rule| seo_steps::triple_builder_step::ProceduralRuleForTriple {
                                rule_key: rule.rule_key.clone(),
                                role_type: rule.role_type,
                            },
                        )
                        .collect(),
                    operational_entities: operational
                        .entities
                        .iter()
                        .map(
                            |entity| seo_steps::triple_builder_step::OperationalEntityForTriple {
                                entity_kind: entity.entity_kind.clone(),
                                value: entity.value.clone(),
                            },
                        )
                        .collect(),
                    editorial_topics: editorial
                        .topics
                        .iter()
                        .map(
                            |topic| seo_steps::triple_builder_step::EditorialTopicForTriple {
                                topic_type: topic.topic_type.clone(),
                                topic_key_candidate: topic.topic_key_candidate.clone(),
                            },
                        )
                        .collect(),
                },
            );
            TripleBuilderSectionState {
                section_id: section.section_id,
                page_id: section.page_id,
                triples: output.triples,
                blocked_by_gate: section.blocked_by_gate,
                decision: if section.blocked_by_gate {
                    "blocked_by_gate"
                } else {
                    "pass"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    Ok(TripleBuilderSweepOutput {
        section_count: sections.len(),
        blocked_section_count: sections
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        triple_count: sections.iter().map(|section| section.triples.len()).sum(),
        sections,
    })
}

async fn retrieve_semantic_neighbors(
    query: &str,
    primary: SemanticNeighborCollection,
    fallback: Option<SemanticNeighborCollection>,
    limit: usize,
) -> Result<SemanticNeighborLookup, DomainError> {
    seo_application::knowledge::retrieve_semantic_neighbors(
        &infrastructure::adapters::semantic_neighbor_search_adapter::SemanticNeighborSearchAdapter,
        &SemanticNeighborQuery {
            text: query.to_string(),
            primary,
            fallback,
            limit,
            retrieval_required: env_flag("RETRIEVAL_CAPABILITY_REQUIRED"),
            contextual_required: env_flag("CONTEXTUAL_RAW_CHUNK_RETRIEVAL_REQUIRED"),
        },
    )
    .await
}

