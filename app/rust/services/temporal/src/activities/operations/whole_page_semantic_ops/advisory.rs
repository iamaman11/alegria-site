fn advisory_signal_from_hits(
    advisory_hits: &[whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit],
) -> WholePageAdvisorySignal {
    let mut page_mode_candidates = Vec::new();
    let mut layer_candidates = Vec::new();
    let mut country_hints = Vec::new();
    let mut visa_type_hints = Vec::new();
    let mut authority_hints = Vec::new();
    let mut mixed_section_pressure = false;
    let mut retrieval_confidence = 0.0_f32;

    for hit in advisory_hits {
        if !page_mode_candidates.contains(&hit.page_mode) {
            page_mode_candidates.push(hit.page_mode.clone());
        }
        for layer in &hit.dominant_layers {
            if !layer_candidates.contains(layer) {
                layer_candidates.push(layer.clone());
            }
        }
        merge_unique_strings(&mut country_hints, hit.country_hints.clone());
        merge_unique_strings(&mut visa_type_hints, hit.visa_type_hints.clone());
        merge_unique_strings(&mut authority_hints, hit.authority_hints.clone());
        mixed_section_pressure |= hit.mixed_section_pressure;
        retrieval_confidence = retrieval_confidence.max(hit.score);
    }

    WholePageAdvisorySignal {
        page_mode_candidates,
        layer_candidates,
        context_profile_candidates: WholePageContextProfile {
            country_hints,
            visa_type_hints,
            authority_hints,
        },
        mixed_section_pressure,
        retrieval_confidence,
    }
}

fn advisory_hit_limit() -> u64 {
    std::env::var("WHOLE_PAGE_ADVISORY_LIMIT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(3)
}

fn fuse_with_advisory_retrieval(
    sections: &[raw_crawl_adapter::RawSectionRecord],
    snapshot: WholePageDeterministicSnapshot,
    advisory_hits: &[whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit],
) -> WholePageSemanticPageState {
    let mut dominant_layers = snapshot.dominant_layers.clone();
    let mut layer_scores = snapshot.layer_scores.clone();
    let mut page_context_profile = snapshot.page_context_profile.clone();
    let mut mixed_section_ids = snapshot.mixed_section_ids.clone();
    let global_entities = snapshot.global_entities.clone();
    let mut uncertainty_flags = snapshot.uncertainty_flags.clone();
    let mut reason_codes = snapshot.reason_codes.clone();
    let mut page_mode_confidence = snapshot.page_mode_confidence;
    let mut advisory_prototype_families = advisory_hits
        .iter()
        .map(|hit| hit.prototype_family.clone())
        .collect::<Vec<_>>();
    advisory_prototype_families.dedup();

    let advisory_model_used = !advisory_hits.is_empty();
    let advisory_consensus;
    if !advisory_model_used {
        advisory_consensus = "not_used".to_string();
    } else {
        let advisory_signal = advisory_signal_from_hits(advisory_hits);
        let mut mode_votes = BTreeMap::<String, f32>::new();
        let mut layer_votes = BTreeMap::<String, f32>::new();
        let mut country_votes = BTreeMap::<String, f32>::new();
        let mut visa_votes = BTreeMap::<String, f32>::new();
        let mut authority_votes = BTreeMap::<String, f32>::new();

        for hit in advisory_hits {
            *mode_votes.entry(hit.page_mode.clone()).or_insert(0.0) += hit.score;
            for layer in &hit.dominant_layers {
                *layer_votes.entry(layer.clone()).or_insert(0.0) += hit.score;
            }
            for hint in &hit.country_hints {
                *country_votes.entry(hint.clone()).or_insert(0.0) += hit.score;
            }
            for hint in &hit.visa_type_hints {
                *visa_votes.entry(hint.clone()).or_insert(0.0) += hit.score;
            }
            for hint in &hit.authority_hints {
                *authority_votes.entry(hint.clone()).or_insert(0.0) += hit.score;
            }
        }

        let top_mode = mode_votes
            .iter()
            .max_by(|lhs, rhs| lhs.1.total_cmp(rhs.1))
            .map(|(mode, score)| (mode.clone(), *score));
        advisory_consensus = if let Some((mode, score)) = top_mode {
            if mode == snapshot.page_mode_hint {
                page_mode_confidence = page_mode_confidence.max((0.70 + score / 4.0).min(0.92));
                reason_codes.push(format!("advisory_page_mode_confirmed:{mode}"));
                "agree".to_string()
            } else {
                page_mode_confidence = (page_mode_confidence * 0.85).max(0.45);
                uncertainty_flags.push(format!("advisory_page_mode_conflict:{mode}"));
                reason_codes.push(format!("advisory_page_mode_candidate:{mode}"));
                "conflict".to_string()
            }
        } else {
            "no_signal".to_string()
        };

        for (layer, vote) in layer_votes {
            let supported_sections = supportive_section_count_for_layer(sections, &layer);
            if !dominant_layers.contains(&layer) && vote >= 1.2 && supported_sections > 0 {
                dominant_layers.push(layer.clone());
                reason_codes.push(format!("advisory_layer_expansion:{layer}"));
            }
            let base = layer_scores.get(&layer).copied().unwrap_or(0.0);
            let fused = if supported_sections > 0 {
                base.max((vote / 3.0).min(0.85))
            } else {
                base
            };
            if fused > 0.0 {
                layer_scores.insert(layer, fused);
            }
        }

        if advisory_signal.mixed_section_pressure && mixed_section_ids.is_empty() {
            let advisory_mixed = sections
                .iter()
                .filter_map(|section| {
                    if section_mixed_layer_score(&section.content_md) >= 2 {
                        Some(section.id)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            if !advisory_mixed.is_empty() {
                for section_id in advisory_mixed {
                    if !mixed_section_ids.contains(&section_id) {
                        mixed_section_ids.push(section_id);
                    }
                }
                reason_codes.push("advisory_mixed_section_support".to_string());
            }
        }
        if advisory_signal.retrieval_confidence >= 0.85 {
            reason_codes.push("high_confidence_advisory_retrieval".to_string());
        }

        for (hint, _) in country_votes {
            if page_context_profile.country_hints.contains(&hint) {
                reason_codes.push(format!("advisory_country_confirmed:{hint}"));
            } else {
                uncertainty_flags.push(format!("advisory_only_country_hint:{hint}"));
            }
        }
        for (hint, _) in visa_votes {
            if page_context_profile.visa_type_hints.contains(&hint) {
                reason_codes.push(format!("advisory_visa_confirmed:{hint}"));
            } else {
                uncertainty_flags.push(format!("advisory_only_visa_hint:{hint}"));
            }
        }
        for (hint, _) in authority_votes {
            if page_context_profile.authority_hints.contains(&hint) {
                reason_codes.push(format!("advisory_authority_confirmed:{hint}"));
            } else {
                uncertainty_flags.push(format!("advisory_only_authority_hint:{hint}"));
            }
        }
    }

    dominant_layers.sort();
    dominant_layers.dedup();
    page_context_profile.country_hints.sort();
    page_context_profile.country_hints.dedup();
    page_context_profile.visa_type_hints.sort();
    page_context_profile.visa_type_hints.dedup();
    page_context_profile.authority_hints.sort();
    page_context_profile.authority_hints.dedup();
    uncertainty_flags.sort();
    uncertainty_flags.dedup();
    reason_codes.sort();
    reason_codes.dedup();

    WholePageSemanticPageState {
        page_id: sections
            .first()
            .map(|section| section.page_id)
            .unwrap_or_default(),
        section_count: sections.len(),
        page_mode_hint: snapshot.page_mode_hint,
        page_mode_confidence,
        dominant_layers,
        layer_scores,
        page_summary: snapshot.page_summary,
        page_context_profile,
        mixed_section_ids,
        global_entities,
        advisory_model_used,
        advisory_consensus,
        advisory_prototype_families,
        uncertainty_flags,
        reason_codes,
    }
}
