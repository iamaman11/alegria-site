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
    primary_collection: &str,
    primary_surface: semantic_search_adapter::VoyageSearchSurface,
    fallback_collection: Option<(&str, semantic_search_adapter::VoyageSearchSurface)>,
    limit: u64,
) -> Result<
    (
        Option<String>,
        Vec<semantic_search_adapter::SearchResultRecord>,
        Vec<String>,
    ),
    DomainError,
> {
    let retrieval_required = env_flag("RETRIEVAL_CAPABILITY_REQUIRED");
    let contextual_required = env_flag("CONTEXTUAL_RAW_CHUNK_RETRIEVAL_REQUIRED");
    let fallback_allowed = !(contextual_required && primary_collection == "raw_chunks_ctx");
    if std::env::var("VOYAGE_API_KEY").is_err() {
        if retrieval_required {
            return Err(DomainError::InfraUnavailable {
                message: "retrieval diagnostics require VOYAGE_API_KEY under hard-required retrieval contract"
                    .to_string(),
            });
        }
        return Ok((None, Vec::new(), vec!["provider_unavailable".to_string()]));
    }
    let mut reason_codes = Vec::new();
    let primary = semantic_search_adapter::search_by_text_with_surface(
        query,
        primary_collection,
        limit,
        primary_surface,
    )
    .await;
    let mut collection_used = Some(primary_collection.to_string());
    let mut records = match primary {
        Ok(found) if !found.is_empty() => found,
        Ok(_) => {
            reason_codes.push(format!("{primary_collection}:empty"));
            if fallback_allowed {
                if let Some((fallback_name, fallback_surface)) = fallback_collection {
                    collection_used = Some(fallback_name.to_string());
                    semantic_search_adapter::search_by_text_with_surface(
                        query,
                        fallback_name,
                        limit,
                        fallback_surface,
                    )
                    .await
                    .map_err(|err| DomainError::InfraUnavailable {
                        message: format!(
                            "semantic retrieval failed for `{fallback_name}` after `{primary_collection}` empty: {err}"
                        ),
                    })?
                } else {
                    Vec::new()
                }
            } else if retrieval_required {
                return Err(DomainError::InfraUnavailable {
                    message: format!(
                        "contextual retrieval is required and `{primary_collection}` returned no neighbors"
                    ),
                });
            } else {
                reason_codes.push("contextual_primary_required_no_fallback".to_string());
                Vec::new()
            }
        }
        Err(err) => {
            if retrieval_required {
                return Err(DomainError::InfraUnavailable {
                    message: format!("semantic retrieval failed for `{primary_collection}`: {err}"),
                });
            }
            reason_codes.push(format!("{primary_collection}:search_failed"));
            if fallback_allowed {
                if let Some((fallback_name, fallback_surface)) = fallback_collection {
                    collection_used = Some(fallback_name.to_string());
                    semantic_search_adapter::search_by_text_with_surface(
                        query,
                        fallback_name,
                        limit,
                        fallback_surface,
                    )
                    .await
                    .unwrap_or_default()
                } else {
                    Vec::new()
                }
            } else {
                reason_codes.push("contextual_primary_required_no_fallback".to_string());
                Vec::new()
            }
        }
    };
    if records.is_empty() {
        reason_codes.push("semantic_neighbors_empty".to_string());
        return Ok((collection_used, records, reason_codes));
    }
    records = semantic_search_adapter::rerank_records(query, records, Some(limit as usize))
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!(
                "semantic rerank failed for `{}`: {err}",
                collection_used.clone().unwrap_or_default()
            ),
        })?;
    reason_codes.push("semantic_neighbors_reranked".to_string());
    Ok((collection_used, records, reason_codes))
}

