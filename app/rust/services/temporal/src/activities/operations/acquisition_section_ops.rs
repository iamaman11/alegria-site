pub(crate) async fn whole_page_semantic_pass_impl(
    acts: &AlegriaActivities,
    input: &WholePageSemanticPassInput,
) -> Result<WholePageSemanticPassOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let mut by_page: BTreeMap<i64, Vec<raw_crawl_adapter::RawSectionRecord>> = BTreeMap::new();
    for section in sections {
        by_page.entry(section.page_id).or_default().push(section);
    }
    let pages = by_page
        .into_iter()
        .map(|(_page_id, sections)| async move {
            let combined = sections
                .iter()
                .map(|section| section.content_md.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let snapshot = deterministic_whole_page_snapshot(&sections, &combined);
            let page_sketch = build_page_sketch(sections[0].page_id, &sections, &snapshot);
            let advisory_hits =
                match whole_page_advisory_adapter::search_whole_page_prototypes(
                    &page_sketch,
                    advisory_hit_limit(),
                )
                    .await
                {
                    Ok(hits) => hits,
                    Err(err) => {
                        tracing::warn!(
                            page_id = sections[0].page_id,
                            error = %err,
                            "whole-page advisory retrieval unavailable; falling back to deterministic semantics"
                        );
                        Vec::new()
                    }
                };
            fuse_with_advisory_retrieval(&sections, snapshot, &advisory_hits)
        })
        .collect::<Vec<_>>();
    let mut resolved_pages = Vec::with_capacity(pages.len());
    for page in pages {
        resolved_pages.push(page.await);
    }
    Ok(WholePageSemanticPassOutput {
        page_count: resolved_pages.len(),
        pages: resolved_pages,
    })
}

pub(crate) async fn sectioning_impl(
    acts: &AlegriaActivities,
    input: &SectioningInput,
) -> Result<SectioningOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let page_count = sections
        .iter()
        .map(|section| section.page_id)
        .collect::<BTreeSet<_>>()
        .len();
    let mapped = sections
        .iter()
        .map(|section| SectioningSectionState {
            section_id: section.id,
            page_id: section.page_id,
            source_url: section.source_url.clone(),
            heading_path: section.heading_path.clone(),
            section_type: section.section_type.clone(),
            content_hash: section.content_hash.clone(),
            text_len: section.content_md.len(),
        })
        .collect::<Vec<_>>();
    Ok(SectioningOutput {
        page_count,
        section_count: mapped.len(),
        sections: mapped,
    })
}

pub(crate) async fn page_utility_sweep_impl(
    acts: &AlegriaActivities,
    input: &PageUtilitySweepInput,
) -> Result<PageUtilitySweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let decisions = sections
        .iter()
        .map(|section| {
            let output = seo_steps::page_utility_classifier_step::execute(
                &seo_steps::page_utility_classifier_step::PageUtilityClassifierInput {
                    url: section.source_url.clone(),
                    title: section.heading_path.clone(),
                    raw_text: section.content_md.clone(),
                },
            );
            PageUtilitySectionDecision {
                section_id: section.id,
                page_id: section.page_id,
                allow_procedural_extraction: output.allow_procedural_extraction,
                allow_editorial_extraction: output.allow_editorial_extraction,
                allow_structural_extraction: output.allow_structural_extraction,
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| !decision.allow_structural_extraction)
        .count();
    Ok(PageUtilitySweepOutput {
        section_count: decisions.len(),
        blocked_section_count,
        decisions,
    })
}

pub(crate) async fn dom_block_relevance_sweep_impl(
    acts: &AlegriaActivities,
    input: &DomBlockRelevanceSweepInput,
) -> Result<DomBlockRelevanceSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let decisions = sections
        .iter()
        .map(|section| {
            let role = block_role_for_section(section);
            let output = seo_steps::dom_block_relevance_step::execute(&[
                seo_steps::dom_block_relevance_step::DomBlockInput {
                    dom_block_id: format!("raw-section:{}", section.id),
                    block_role: role.clone(),
                    text: section.content_md.clone(),
                },
            ]);
            let block = output.blocks.into_iter().next().expect("single block");
            DomBlockRelevanceSectionDecision {
                section_id: section.id,
                page_id: section.page_id,
                block_role: format!("{:?}", role).to_lowercase(),
                allow_extraction: block.allow_extraction,
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| !decision.allow_extraction)
        .count();
    Ok(DomBlockRelevanceSweepOutput {
        section_count: decisions.len(),
        blocked_section_count,
        decisions,
    })
}

pub(crate) async fn sectioning_contract_gate_impl(
    acts: &AlegriaActivities,
    input: &SectioningContractGateInput,
) -> Result<SectioningContractGateOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let decisions = sections
        .iter()
        .map(|section| {
            let heading_present = !section.heading_path.trim().is_empty();
            let text_present = !section.content_md.trim().is_empty();
            SectioningContractDecision {
                section_id: section.id,
                page_id: section.page_id,
                heading_present,
                text_present,
                decision: if heading_present && text_present {
                    "pass".to_string()
                } else {
                    "blocked".to_string()
                },
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| decision.decision != "pass")
        .count();
    Ok(SectioningContractGateOutput {
        section_count: decisions.len(),
        blocked_section_count,
        decisions,
    })
}

pub(crate) async fn cas_gate_impl(
    acts: &AlegriaActivities,
    input: &CasGateInput,
) -> Result<CasGateOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let decisions = sections
        .iter()
        .map(|section| CasGateDecision {
            section_id: section.id,
            page_id: section.page_id,
            snapshot_hash: if section.content_hash.trim().is_empty() {
                content_hash_v1(&section.content_md)
            } else {
                section.content_hash.clone()
            },
            is_replay_safe: !section.content_md.trim().is_empty(),
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| !decision.is_replay_safe)
        .count();
    Ok(CasGateOutput {
        section_count: decisions.len(),
        blocked_section_count,
        decisions,
    })
}

pub(crate) async fn raw_evidence_register_impl(
    acts: &AlegriaActivities,
    input: &RawEvidenceRegisterInput,
) -> Result<RawEvidenceRegisterOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let page_count = sections
        .iter()
        .map(|section| section.page_id)
        .collect::<BTreeSet<_>>()
        .len();
    let unique_source_count = sections
        .iter()
        .map(|section| section.source_url.clone())
        .collect::<BTreeSet<_>>()
        .len();
    let evidence_refs = sections
        .iter()
        .map(|section| format!("raw.section:{}", section.id))
        .collect::<Vec<_>>();
    Ok(RawEvidenceRegisterOutput {
        context_key: input.context_key.clone(),
        page_count,
        section_count: evidence_refs.len(),
        unique_source_count,
        evidence_refs,
        status: "registered".to_string(),
    })
}

pub(crate) async fn layer_router_sweep_impl(
    acts: &AlegriaActivities,
    input: &LayerRouterSweepInput,
) -> Result<LayerRouterSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let decisions = sections
        .iter()
        .map(|section| {
            let output = seo_steps::layer_router_step::execute(
                &seo_steps::layer_router_step::LayerRouterInput {
                    section_id: section.id.to_string(),
                    heading_text: section.heading_path.clone(),
                    raw_text: section.content_md.clone(),
                    source_tier: section.source_dtype.clone(),
                    block_type: section.section_type.clone(),
                },
            );
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            let decision = if blocked_by_gate {
                "blocked_by_gate"
            } else if output.needs_hitl {
                "needs_hitl"
            } else {
                "pass"
            };
            LayerRouterSectionDecision {
                section_id: section.id,
                page_id: section.page_id,
                primary_layer: output.primary_layer,
                secondary_layers: output.secondary_layers,
                confidence: output.confidence,
                needs_hitl: output.needs_hitl,
                blocked_by_gate,
                decision: decision.to_string(),
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| decision.blocked_by_gate)
        .count();
    let needs_hitl_count = decisions
        .iter()
        .filter(|decision| !decision.blocked_by_gate && decision.needs_hitl)
        .count();
    Ok(LayerRouterSweepOutput {
        section_count: decisions.len(),
        blocked_section_count,
        needs_hitl_count,
        decisions,
    })
}

pub(crate) async fn subspan_layer_router_impl(
    input: &SubspanLayerRouterInput,
) -> Result<SubspanLayerRouterOutput, DomainError> {
    let decisions = input
        .layer_router
        .decisions
        .iter()
        .map(|decision| {
            let mixed_layers = decision
                .secondary_layers
                .iter()
                .map(|layer| layer.layer.clone())
                .collect::<Vec<_>>();
            let needs_split = !mixed_layers.is_empty();
            let stage_decision = if decision.blocked_by_gate {
                "blocked_by_gate"
            } else if needs_split {
                "needs_hitl"
            } else {
                "pass"
            };
            SubspanLayerRouterDecision {
                section_id: decision.section_id,
                page_id: decision.page_id,
                mixed_layers,
                needs_split,
                blocked_by_gate: decision.blocked_by_gate,
                decision: stage_decision.to_string(),
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| decision.blocked_by_gate)
        .count();
    let needs_split_count = decisions
        .iter()
        .filter(|decision| !decision.blocked_by_gate && decision.needs_split)
        .count();
    Ok(SubspanLayerRouterOutput {
        section_count: decisions.len(),
        blocked_section_count,
        needs_split_count,
        decisions,
    })
}

pub(crate) async fn entity_span_sweep_impl(
    acts: &AlegriaActivities,
    input: &EntitySpanSweepInput,
) -> Result<EntitySpanSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let section_outputs = sections
        .iter()
        .map(|section| {
            let output = seo_steps::entity_span_detection_step::execute(
                &seo_steps::entity_span_detection_step::EntitySpanInput {
                    section_id: section.id.to_string(),
                    raw_text: section.content_md.clone(),
                },
            );
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            EntitySpanSectionMentions {
                section_id: section.id,
                page_id: section.page_id,
                mentions: output.mentions,
                blocked_by_gate,
                decision: if blocked_by_gate {
                    "blocked_by_gate".to_string()
                } else {
                    "pass".to_string()
                },
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = section_outputs
        .iter()
        .filter(|section| section.blocked_by_gate)
        .count();
    let mention_count = section_outputs
        .iter()
        .map(|section| section.mentions.len())
        .sum();
    Ok(EntitySpanSweepOutput {
        section_count: section_outputs.len(),
        blocked_section_count,
        mention_count,
        sections: section_outputs,
    })
}

pub(crate) async fn canonical_mapping_sweep_impl(
    input: &CanonicalMappingSweepInput,
) -> Result<CanonicalMappingSweepOutput, DomainError> {
    let mut sections = Vec::with_capacity(input.entity_spans.sections.len());
    for section in &input.entity_spans.sections {
        let mut output = seo_steps::canonical_mapping_step::execute(
            &seo_steps::canonical_mapping_step::CanonicalMappingInput {
                section_id: section.section_id.to_string(),
                mentions: section
                    .mentions
                    .iter()
                    .map(
                        |mention| seo_steps::canonical_mapping_step::MentionForMapping {
                            raw_text: mention.raw_text.clone(),
                            entity_type: mention.entity_type.clone(),
                        },
                    )
                    .collect(),
            },
        );
        output.mappings = resolve_canonical_vector_mappings(output.mappings).await?;
        let needs_hitl = output.mappings.iter().any(|mapping| mapping.needs_hitl);
        let decision = if section.blocked_by_gate {
            "blocked_by_gate"
        } else if needs_hitl {
            "needs_hitl"
        } else {
            "pass"
        };
        sections.push(CanonicalMappingSectionState {
            section_id: section.section_id,
            page_id: section.page_id,
            mappings: output.mappings,
            blocked_by_gate: section.blocked_by_gate,
            needs_hitl,
            decision: decision.to_string(),
        });
    }
    let blocked_section_count = sections
        .iter()
        .filter(|section| section.blocked_by_gate)
        .count();
    let needs_hitl_count = sections
        .iter()
        .filter(|section| !section.blocked_by_gate && section.needs_hitl)
        .count();
    Ok(CanonicalMappingSweepOutput {
        section_count: sections.len(),
        blocked_section_count,
        needs_hitl_count,
        sections,
    })
}

