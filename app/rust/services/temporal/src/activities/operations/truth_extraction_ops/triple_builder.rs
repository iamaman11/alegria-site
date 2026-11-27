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
        .map(|section| -> Result<TripleBuilderSectionState, DomainError> {
            let operational = operational_by_section
                .get(&section.section_id)
                .copied()
                .ok_or_else(|| DomainError::ContractViolation {
                    message: format!(
                        "triple_builder stage=section_join run_id={} section_id={} missing operational extraction",
                        input.run_id, section.section_id
                    ),
                })?;
            let editorial = editorial_by_section
                .get(&section.section_id)
                .copied()
                .ok_or_else(|| DomainError::ContractViolation {
                    message: format!(
                        "triple_builder stage=section_join run_id={} section_id={} missing editorial extraction",
                        input.run_id, section.section_id
                    ),
                })?;
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
            Ok(TripleBuilderSectionState {
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
            })
        })
        .collect::<Result<Vec<_>, DomainError>>()?;
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
        &runtime_composition::adapters::semantic_neighbor_search_adapter::SemanticNeighborSearchAdapter,
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

#[cfg(test)]
mod triple_builder_join_tests {
    use super::*;

    fn matched_layers() -> TripleBuilderSweepInput {
        TripleBuilderSweepInput {
            run_id: "typed-join-run".into(),
            procedural: ProceduralExtractionSweepOutput {
                section_count: 1,
                blocked_section_count: 0,
                skipped_section_count: 0,
                rule_count: 0,
                sections: vec![ProceduralExtractionSectionState {
                    section_id: 17,
                    page_id: 4,
                    rules: vec![],
                    blocked_by_gate: false,
                    skipped: false,
                    decision: "pass".into(),
                }],
            },
            operational: OperationalExtractionSweepOutput {
                section_count: 1,
                blocked_section_count: 0,
                entity_count: 0,
                sections: vec![OperationalExtractionSectionState {
                    section_id: 17,
                    page_id: 4,
                    entities: vec![],
                    blocked_by_gate: false,
                    decision: "pass".into(),
                }],
            },
            editorial: EditorialExtractionSweepOutput {
                section_count: 1,
                blocked_section_count: 0,
                skipped_section_count: 0,
                topic_count: 0,
                sections: vec![EditorialExtractionSectionState {
                    section_id: 17,
                    page_id: 4,
                    topics: vec![],
                    blocked_by_gate: false,
                    skipped: false,
                    decision: "pass".into(),
                }],
            },
        }
    }

    #[tokio::test]
    async fn complete_typed_join_can_run_with_one_independent_section() {
        let result = triple_builder_sweep_impl(&matched_layers()).await.unwrap();
        assert_eq!(result.section_count, 1);
        assert_eq!(result.triple_count, 0);
        assert_eq!(result.blocked_section_count, 0);
        assert_eq!(result.sections[0].section_id, 17);
        assert_eq!(result.sections[0].decision, "pass");
    }

    #[tokio::test]
    async fn missing_operational_section_returns_typed_error_not_panic() {
        let mut input = matched_layers();
        input.operational.sections.clear();
        let error = triple_builder_sweep_impl(&input).await.unwrap_err();
        assert!(matches!(error, DomainError::ContractViolation { .. }));
        assert!(error.to_string().contains("missing operational extraction"));
        assert!(error.to_string().contains("section_id=17"));
    }

    #[tokio::test]
    async fn missing_editorial_section_returns_typed_error_not_panic() {
        let mut input = matched_layers();
        input.editorial.sections.clear();
        let error = triple_builder_sweep_impl(&input).await.unwrap_err();
        assert!(matches!(error, DomainError::ContractViolation { .. }));
        assert!(error.to_string().contains("missing editorial extraction"));
        assert!(error.to_string().contains("run_id=typed-join-run"));
    }
}
