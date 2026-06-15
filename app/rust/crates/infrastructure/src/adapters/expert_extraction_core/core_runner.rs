pub fn run_expert_extraction_core(
    run_id: &str,
    _context_key: &str,
    sections: &[RawSectionRecord],
) -> ExpertExtractionCoreReport {
    let mut report = ExpertExtractionCoreReport {
        schema_version: EXPERT_CORE_SCHEMA_VERSION,
        section_count: sections.len(),
        ..ExpertExtractionCoreReport::default()
    };

    for section in sections {
        let mut stage_records = Vec::new();
        let whole_page = WholePageSemanticPassOutput {
            page_mode_hint: page_mode_hint(section),
            page_mode_confidence: 0.70,
            dominant_layers: dominant_layers(&section.content_md),
            layer_scores: layer_scores_for_text(&section.content_md),
            page_summary: summarize(&section.content_md),
            page_context_profile: page_context_profile(
                std::slice::from_ref(section),
                &section.content_md,
            ),
            mixed_section_ids: if dominant_layers(&section.content_md).len() > 1 {
                vec![section.id]
            } else {
                Vec::new()
            },
            global_entities: global_entities(&section.content_md),
            advisory_model_used: false,
            advisory_consensus: "not_used".to_string(),
            advisory_prototype_families: Vec::new(),
            uncertainty_flags: if dominant_layers(&section.content_md).len() > 1 {
                vec!["multi_layer_page".to_string()]
            } else {
                Vec::new()
            },
            reason_codes: vec![format!("page_mode:{}", page_mode_hint(section))],
        };
        stage_records.push(stage_record(
            "whole_page_semantic_pass",
            section,
            &section.content_md,
            &whole_page,
            ExpertStageStatus::Executed,
            None,
        ));

        let page_input = seo_steps::page_utility_classifier_step::PageUtilityClassifierInput {
            url: section.source_url.clone(),
            title: section.heading_path.clone(),
            raw_text: section.content_md.clone(),
        };
        let page_utility = seo_steps::page_utility_classifier_step::execute(&page_input);
        stage_records.push(stage_record(
            "page_utility_classifier",
            section,
            &page_input,
            &page_utility,
            ExpertStageStatus::Executed,
            None,
        ));

        let dom_inputs = vec![seo_steps::dom_block_relevance_step::DomBlockInput {
            dom_block_id: format!("raw-section:{}", section.id),
            block_role: block_role_for_section(section),
            text: section.content_md.clone(),
        }];
        let dom_output = seo_steps::dom_block_relevance_step::execute(&dom_inputs);
        stage_records.push(stage_record(
            "dom_block_relevance_filter",
            section,
            &dom_inputs,
            &dom_output,
            ExpertStageStatus::Executed,
            None,
        ));

        let sectioning_contract = SectioningContractGateOutput {
            heading_present: !section.heading_path.trim().is_empty(),
            text_present: !section.content_md.trim().is_empty(),
            decision: if !section.heading_path.trim().is_empty()
                && !section.content_md.trim().is_empty()
            {
                "pass".to_string()
            } else {
                "blocked".to_string()
            },
        };
        stage_records.push(stage_record(
            "sectioning_contract_gate",
            section,
            &section.heading_path,
            &sectioning_contract,
            if sectioning_contract.decision == "pass" {
                ExpertStageStatus::Executed
            } else {
                ExpertStageStatus::Blocked
            },
            None,
        ));

        let cas_gate = CasGateOutput {
            section_key: format!("raw.section:{}", section.id),
            snapshot_hash: if section.content_hash.trim().is_empty() {
                content_hash_v1(&section.content_md)
            } else {
                section.content_hash.clone()
            },
            is_replay_safe: !section.content_md.trim().is_empty(),
        };
        stage_records.push(stage_record(
            "cas_gate",
            section,
            &section.id,
            &cas_gate,
            if cas_gate.is_replay_safe {
                ExpertStageStatus::Executed
            } else {
                ExpertStageStatus::Blocked
            },
            None,
        ));

        let blocked_by_gate = !page_utility.allow_structural_extraction
            || !dom_output.blocks.iter().any(|block| block.allow_extraction)
            || sectioning_contract.decision != "pass"
            || !cas_gate.is_replay_safe;

        let layer_input = seo_steps::layer_router_step::LayerRouterInput {
            section_id: section.id.to_string(),
            heading_text: section.heading_path.clone(),
            raw_text: section.content_md.clone(),
            source_tier: section.source_dtype.clone(),
            block_type: section.section_type.clone(),
        };
        let layer_output = seo_steps::layer_router_step::execute(&layer_input);
        stage_records.push(stage_record(
            "layer_router",
            section,
            &layer_input,
            &layer_output,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else if layer_output.needs_hitl {
                ExpertStageStatus::NeedsHitl
            } else {
                ExpertStageStatus::Executed
            },
            None,
        ));

        let subspan_output = SubspanLayerRouterOutput {
            mixed_layers: layer_output
                .secondary_layers
                .iter()
                .map(|entry| entry.layer.clone())
                .collect(),
            needs_split: !layer_output.secondary_layers.is_empty(),
        };
        stage_records.push(stage_record(
            "subspan_layer_router",
            section,
            &layer_output,
            &subspan_output,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else if subspan_output.needs_split {
                ExpertStageStatus::NeedsHitl
            } else {
                ExpertStageStatus::Executed
            },
            None,
        ));

        let entity_input = seo_steps::entity_span_detection_step::EntitySpanInput {
            section_id: section.id.to_string(),
            raw_text: section.content_md.clone(),
        };
        let entity_output = seo_steps::entity_span_detection_step::execute(&entity_input);
        stage_records.push(stage_record(
            "entity_span_detection",
            section,
            &entity_input,
            &entity_output,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else {
                ExpertStageStatus::Executed
            },
            None,
        ));

        let mapping_input = seo_steps::canonical_mapping_step::CanonicalMappingInput {
            section_id: section.id.to_string(),
            mentions: entity_output
                .mentions
                .iter()
                .map(
                    |mention| seo_steps::canonical_mapping_step::MentionForMapping {
                        raw_text: mention.raw_text.clone(),
                        entity_type: mention.entity_type.clone(),
                    },
                )
                .collect(),
        };
        let mapping_output = seo_steps::canonical_mapping_step::execute(&mapping_input);
        stage_records.push(stage_record(
            "canonical_mapping",
            section,
            &mapping_input,
            &mapping_output,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else if mapping_output
                .mappings
                .iter()
                .any(|mapping| mapping.needs_hitl)
            {
                ExpertStageStatus::NeedsHitl
            } else {
                ExpertStageStatus::Executed
            },
            None,
        ));

        let ontology_output = OntologyIntakeGateOutput {
            accepted_keys: mapping_output
                .mappings
                .iter()
                .filter_map(|mapping| mapping.canonical_key.clone())
                .collect(),
            unresolved_mentions: mapping_output
                .mappings
                .iter()
                .filter(|mapping| mapping.needs_hitl || mapping.canonical_key.is_none())
                .map(|mapping| mapping.raw_text.clone())
                .collect(),
            needs_hitl: mapping_output
                .mappings
                .iter()
                .any(|mapping| mapping.needs_hitl || mapping.canonical_key.is_none()),
            decision: if mapping_output
                .mappings
                .iter()
                .any(|mapping| mapping.needs_hitl || mapping.canonical_key.is_none())
            {
                "needs_hitl".to_string()
            } else {
                "pass".to_string()
            },
        };
        stage_records.push(stage_record(
            "ontology_intake_gate",
            section,
            &mapping_output,
            &ontology_output,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else if ontology_output.needs_hitl {
                ExpertStageStatus::NeedsHitl
            } else {
                ExpertStageStatus::Executed
            },
            None,
        ));

        let procedural_input = seo_steps::procedural_extraction_step::ProceduralExtractionInput {
            section_id: section.id.to_string(),
            raw_text: section.content_md.clone(),
            mentions: entity_output.mentions.clone(),
        };
        let procedural_output = if blocked_by_gate || !page_utility.allow_procedural_extraction {
            seo_steps::procedural_extraction_step::ProceduralExtractionOutput { rules: Vec::new() }
        } else {
            seo_steps::procedural_extraction_step::execute(&procedural_input)
        };
        stage_records.push(stage_record(
            "procedural_extraction",
            section,
            &procedural_input,
            &procedural_output,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else if !page_utility.allow_procedural_extraction {
                ExpertStageStatus::Skipped
            } else {
                ExpertStageStatus::Executed
            },
            None,
        ));

        let operational_input =
            seo_steps::operational_extraction_step::OperationalExtractionInput {
                section_id: section.id.to_string(),
                raw_text: section.content_md.clone(),
            };
        let operational_output =
            seo_steps::operational_extraction_step::execute(&operational_input);
        stage_records.push(stage_record(
            "operational_extraction",
            section,
            &operational_input,
            &operational_output,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else {
                ExpertStageStatus::Executed
            },
            None,
        ));

        let editorial_input = seo_steps::editorial_extraction_step::EditorialExtractionInput {
            section_id: section.id.to_string(),
            raw_text: section.content_md.clone(),
        };
        let editorial_output = if blocked_by_gate || !page_utility.allow_editorial_extraction {
            seo_steps::editorial_extraction_step::EditorialExtractionOutput { topics: Vec::new() }
        } else {
            seo_steps::editorial_extraction_step::execute(&editorial_input)
        };
        stage_records.push(stage_record(
            "editorial_extraction",
            section,
            &editorial_input,
            &editorial_output,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else if !page_utility.allow_editorial_extraction {
                ExpertStageStatus::Skipped
            } else {
                ExpertStageStatus::Executed
            },
            None,
        ));

        let schema_validate = ExtractionSchemaValidateOutput {
            status: if blocked_by_gate {
                "blocked".to_string()
            } else if procedural_output
                .rules
                .iter()
                .any(|rule| rule.rule_key.trim().is_empty() || rule.confidence <= 0.0)
            {
                "invalid".to_string()
            } else {
                "valid".to_string()
            },
            blocking_reasons: if blocked_by_gate {
                vec!["blocked_by_early_gate".to_string()]
            } else if procedural_output
                .rules
                .iter()
                .any(|rule| rule.rule_key.trim().is_empty() || rule.confidence <= 0.0)
            {
                vec!["invalid_procedural_candidate_shape".to_string()]
            } else {
                Vec::new()
            },
        };
        stage_records.push(stage_record(
            "extraction_schema_validate",
            section,
            &procedural_output,
            &schema_validate,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else if schema_validate.status == "valid" {
                ExpertStageStatus::Executed
            } else {
                ExpertStageStatus::Rejected
            },
            None,
        ));

        let candidate_validation = CandidateValidationOutput {
            accepted_count: procedural_output
                .rules
                .iter()
                .filter(|rule| rule.confidence >= 0.80)
                .count(),
            needs_hitl_count: mapping_output
                .mappings
                .iter()
                .filter(|mapping| mapping.needs_hitl)
                .count(),
            rejected_count: usize::from(schema_validate.status == "invalid"),
            status: if blocked_by_gate {
                "blocked".to_string()
            } else if schema_validate.status == "invalid" {
                "rejected".to_string()
            } else if mapping_output
                .mappings
                .iter()
                .any(|mapping| mapping.needs_hitl)
            {
                "needs_hitl".to_string()
            } else {
                "accepted".to_string()
            },
        };
        stage_records.push(stage_record(
            "candidate_validation",
            section,
            &mapping_output,
            &candidate_validation,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else if candidate_validation.status == "accepted" {
                ExpertStageStatus::Executed
            } else if candidate_validation.status == "needs_hitl" {
                ExpertStageStatus::NeedsHitl
            } else {
                ExpertStageStatus::Rejected
            },
            None,
        ));

        let triple_input = seo_steps::triple_builder_step::TripleBuilderInput {
            section_id: section.id.to_string(),
            procedural_rules: procedural_output
                .rules
                .iter()
                .map(
                    |rule| seo_steps::triple_builder_step::ProceduralRuleForTriple {
                        rule_key: rule.rule_key.clone(),
                        role_type: rule.role_type,
                    },
                )
                .collect(),
            operational_entities: operational_output
                .entities
                .iter()
                .map(
                    |entity| seo_steps::triple_builder_step::OperationalEntityForTriple {
                        entity_kind: entity.entity_kind.clone(),
                        value: entity.value.clone(),
                    },
                )
                .collect(),
            editorial_topics: editorial_output
                .topics
                .iter()
                .map(
                    |topic| seo_steps::triple_builder_step::EditorialTopicForTriple {
                        topic_type: topic.topic_type.clone(),
                        topic_key_candidate: topic.topic_key_candidate.clone(),
                    },
                )
                .collect(),
        };
        let triple_output = seo_steps::triple_builder_step::execute(&triple_input);
        stage_records.push(stage_record(
            "triple_builder",
            section,
            &triple_input,
            &triple_output,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else {
                ExpertStageStatus::Executed
            },
            None,
        ));

        let numeric_tokens: BTreeSet<String> = procedural_output
            .rules
            .iter()
            .flat_map(|rule| rule.numeric_tokens.iter().cloned())
            .collect();
        let source_numeric_tokens: BTreeSet<String> = entity_output
            .mentions
            .iter()
            .filter(|mention| mention.has_numeric)
            .map(|mention| mention.raw_text.replace(',', "."))
            .collect();
        let completeness_input = seo_steps::completeness_judge_step::CompletenessJudgeInput {
            section_id: section.id.to_string(),
            raw_text: section.content_md.clone(),
            source_numeric_tokens: source_numeric_tokens.into_iter().collect(),
            extracted_numeric_tokens: numeric_tokens.into_iter().collect(),
            extracted_rule_keys: procedural_output
                .rules
                .iter()
                .map(|rule| rule.rule_key.clone())
                .collect(),
        };
        let completeness_output = seo_steps::completeness_judge_step::execute(&completeness_input);
        stage_records.push(stage_record(
            "completeness_judge",
            section,
            &completeness_input,
            &completeness_output,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else if completeness_output.needs_hitl {
                ExpertStageStatus::NeedsHitl
            } else {
                ExpertStageStatus::Executed
            },
            None,
        ));

        let resolution_output = ResolutionLoopOutput {
            decision: if blocked_by_gate {
                "drop_with_reason".to_string()
            } else if completeness_output.needs_hitl || ontology_output.needs_hitl {
                "pause_for_hitl".to_string()
            } else if schema_validate.status == "invalid" {
                "drop_with_reason".to_string()
            } else {
                "accept".to_string()
            },
            blockers: completeness_output
                .missing_elements
                .iter()
                .map(|missing| missing.action.clone())
                .chain(ontology_output.unresolved_mentions.iter().cloned())
                .collect(),
            needs_hitl: !blocked_by_gate
                && (completeness_output.needs_hitl || ontology_output.needs_hitl),
        };
        stage_records.push(stage_record(
            "resolution_loop",
            section,
            &completeness_output,
            &resolution_output,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else if resolution_output.needs_hitl {
                ExpertStageStatus::NeedsHitl
            } else if resolution_output.decision == "accept" {
                ExpertStageStatus::Executed
            } else {
                ExpertStageStatus::Rejected
            },
            None,
        ));

        let contradiction_input = seo_steps::contradiction_gate_step::ContradictionGateInput {
            run_id: run_id.to_string(),
            facts: contradiction_facts(section, &procedural_output),
        };
        let contradiction_output =
            seo_steps::contradiction_gate_step::execute(&contradiction_input);
        stage_records.push(stage_record(
            "contradiction_gate",
            section,
            &contradiction_input,
            &contradiction_output,
            if blocked_by_gate {
                ExpertStageStatus::Blocked
            } else if contradiction_output.is_blocked {
                ExpertStageStatus::Rejected
            } else if contradiction_output.needs_hitl {
                ExpertStageStatus::NeedsHitl
            } else {
                ExpertStageStatus::Executed
            },
            None,
        ));

        let truth_adjudication = if blocked_by_gate {
            TruthAdjudicationOutput {
                decision: "blocked".to_string(),
                status: ExpertStageStatus::Blocked,
            }
        } else if contradiction_output.is_blocked || schema_validate.status == "invalid" {
            TruthAdjudicationOutput {
                decision: "rejected".to_string(),
                status: ExpertStageStatus::Rejected,
            }
        } else if resolution_output.needs_hitl || candidate_validation.status == "needs_hitl" {
            TruthAdjudicationOutput {
                decision: "needs_hitl".to_string(),
                status: ExpertStageStatus::NeedsHitl,
            }
        } else {
            TruthAdjudicationOutput {
                decision: "verified".to_string(),
                status: ExpertStageStatus::Verified,
            }
        };
        stage_records.push(stage_record(
            "truth_adjudication",
            section,
            &candidate_validation,
            &truth_adjudication,
            truth_adjudication.status.clone(),
            None,
        ));

        let verified_write = VerifiedTruthWriteOutput {
            write_allowed: truth_adjudication.status == ExpertStageStatus::Verified,
            reason: truth_adjudication.decision.clone(),
        };
        stage_records.push(stage_record(
            "verified_truth_write",
            section,
            &truth_adjudication,
            &verified_write,
            if verified_write.write_allowed {
                ExpertStageStatus::Verified
            } else {
                truth_adjudication.status.clone()
            },
            None,
        ));

        report.procedural_rule_count += procedural_output.rules.len();
        report.operational_entity_count += operational_output.entities.len();
        report.editorial_topic_count += editorial_output.topics.len();
        report.triple_count += triple_output.triples.len();
        report.contradiction_conflict_count += contradiction_output.conflict_count;

        let final_status = if blocked_by_gate {
            report.blocked_section_count += 1;
            ExpertStageStatus::Blocked
        } else if truth_adjudication.status == ExpertStageStatus::NeedsHitl {
            report.needs_hitl_section_count += 1;
            ExpertStageStatus::NeedsHitl
        } else if truth_adjudication.status == ExpertStageStatus::Verified {
            report.verified_ready_section_count += 1;
            ExpertStageStatus::Verified
        } else {
            truth_adjudication.status.clone()
        };

        report.sections.push(ExpertExtractionSectionOutcome {
            raw_section_id: section.id,
            source_url: section.source_url.clone(),
            primary_layer: layer_output.primary_layer,
            needs_hitl: matches!(final_status, ExpertStageStatus::NeedsHitl),
            verified_ready: matches!(final_status, ExpertStageStatus::Verified),
            final_status,
            stage_records,
        });
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(id: i64, text: &str) -> RawSectionRecord {
        RawSectionRecord {
            id,
            page_id: 10,
            source_url: "https://example.com/visa".to_string(),
            source_domain: "example.com".to_string(),
            source_dtype: "government".to_string(),
            heading_path: "Visa requirements".to_string(),
            section_type: "content".to_string(),
            content_md: text.to_string(),
            content_hash: content_hash_v1(text),
        }
    }

    #[test]
    fn mixed_layer_section_produces_rich_stage_records() {
        let report = run_expert_extraction_core(
            "run-1",
            "ES|tourist||BY",
            &[section(
                1,
                "Passport required. Fee 80 EUR. Processing time 15 days. FAQ: что делать при отказе. Appointment schedule applies.",
            )],
        );

        assert_eq!(report.section_count, 1);
        assert!(report.procedural_rule_count >= 3);
        assert!(report.operational_entity_count >= 1);
        assert!(report.editorial_topic_count >= 1);
        assert!(report.triple_count >= 1);
        assert_eq!(
            report.sections[0].final_status,
            ExpertStageStatus::Verified
        );
        assert!(report.sections[0]
            .stage_records
            .iter()
            .any(|record| record.stage_name == "entity_span_detection"));
    }

    #[test]
    fn unresolved_mapping_and_loss_require_hitl() {
        let report = run_expert_extraction_core(
            "run-2",
            "ES|tourist||BY",
            &[section(
                2,
                "Кроме стандартного пакета нужен mysterious permit и 80 EUR либо письмо спонсора.",
            )],
        );

        assert_eq!(report.section_count, 1);
        assert_eq!(report.needs_hitl_section_count, 1);
        assert_eq!(
            report.sections[0].final_status,
            ExpertStageStatus::NeedsHitl
        );
        assert!(report.sections[0]
            .stage_records
            .iter()
            .any(|record| record.stage_name == "resolution_loop"
                && record.status == ExpertStageStatus::NeedsHitl));
    }

    #[test]
    fn conflicting_high_confidence_values_are_rejected() {
        let report = run_expert_extraction_core(
            "run-3",
            "ES|tourist||BY",
            &[section(
                3,
                "Consular fee is 80 EUR. Another paragraph says fee is 120 EUR. Passport required.",
            )],
        );

        assert_eq!(report.section_count, 1);
        assert!(report.contradiction_conflict_count >= 1);
        assert_eq!(report.sections[0].final_status, ExpertStageStatus::Rejected);
    }

    #[test]
    fn section_order_permutation_does_not_change_truth_ready_counts() {
        let sections = vec![
            section(1, "Passport required. Fee 80 EUR."),
            section(2, "Processing time 15 days. Appointment schedule applies."),
        ];
        let forward = run_expert_extraction_core("run-a", "ES|tourist||BY", &sections);
        let reverse = run_expert_extraction_core(
            "run-b",
            "ES|tourist||BY",
            &sections.iter().cloned().rev().collect::<Vec<_>>(),
        );
        assert_eq!(forward.section_count, reverse.section_count);
        assert_eq!(
            forward.verified_ready_section_count,
            reverse.verified_ready_section_count
        );
        assert_eq!(
            forward.needs_hitl_section_count,
            reverse.needs_hitl_section_count
        );
    }

    #[test]
    fn irrelevant_footer_injection_does_not_remove_procedural_detection() {
        let base = run_expert_extraction_core(
            "run-footer-base",
            "ES|tourist||BY",
            &[section(1, "Passport required. Fee 80 EUR.")],
        );
        let with_footer = run_expert_extraction_core(
            "run-footer-injected",
            "ES|tourist||BY",
            &[section(
                1,
                "Passport required. Fee 80 EUR.\nFooter: contact us for updates and newsletter.",
            )],
        );
        assert!(with_footer.procedural_rule_count >= base.procedural_rule_count);
        assert!(with_footer.sections[0]
            .stage_records
            .iter()
            .any(|record| record.stage_name == "procedural_extraction"));
    }

    #[test]
    fn whole_page_semantic_pass_emits_context_profile_and_mixed_section_hint() {
        let raw = "Spain tourist visa. Passport required. Appointment schedule applies through the consulate and VFS.";
        let stage_output = WholePageSemanticPassOutput {
            page_mode_hint: page_mode_hint(&section(7, raw)),
            page_mode_confidence: 0.70,
            dominant_layers: dominant_layers(raw),
            layer_scores: layer_scores_for_text(raw),
            page_summary: summarize(raw),
            page_context_profile: page_context_profile(&[section(7, raw)], raw),
            mixed_section_ids: if dominant_layers(raw).len() > 1 {
                vec![7]
            } else {
                Vec::new()
            },
            global_entities: global_entities(raw),
            advisory_model_used: false,
            advisory_consensus: "not_used".to_string(),
            advisory_prototype_families: Vec::new(),
            uncertainty_flags: vec!["multi_layer_page".to_string()],
            reason_codes: vec![format!("page_mode:{}", page_mode_hint(&section(7, raw)))],
        };

        let report = run_expert_extraction_core(
            "run-semantic-page-context",
            "ES|tourist||BY",
            &[section(7, raw)],
        );
        let whole_page_record = report.sections[0]
            .stage_records
            .iter()
            .find(|record| record.stage_name == "whole_page_semantic_pass")
            .expect("whole_page_semantic_pass record");
        assert_eq!(whole_page_record.status, ExpertStageStatus::Executed);

        assert!(stage_output
            .page_context_profile
            .country_hints
            .contains(&"ES".to_string()));
        assert!(stage_output
            .page_context_profile
            .visa_type_hints
            .contains(&"tourist".to_string()));
        assert!(stage_output
            .page_context_profile
            .authority_hints
            .contains(&"consulate".to_string()));
        assert!(stage_output
            .page_context_profile
            .authority_hints
            .contains(&"visa_center".to_string()));
        assert_eq!(stage_output.mixed_section_ids, vec![7]);

        assert!(report.sections[0]
            .stage_records
            .iter()
            .any(|record| record.stage_name == "whole_page_semantic_pass"));
    }

    #[test]
    fn whole_page_semantic_pass_detects_utility_page_shape() {
        let raw =
            "Privacy policy. Cookie settings. Login and account access. Terms and legal notice.";
        let stage_output = WholePageSemanticPassOutput {
            page_mode_hint: page_mode_hint(&section(8, raw)),
            page_mode_confidence: 0.70,
            dominant_layers: dominant_layers(raw),
            layer_scores: layer_scores_for_text(raw),
            page_summary: summarize(raw),
            page_context_profile: page_context_profile(&[section(8, raw)], raw),
            mixed_section_ids: if dominant_layers(raw).len() > 1 {
                vec![8]
            } else {
                Vec::new()
            },
            global_entities: global_entities(raw),
            advisory_model_used: false,
            advisory_consensus: "not_used".to_string(),
            advisory_prototype_families: Vec::new(),
            uncertainty_flags: Vec::new(),
            reason_codes: vec![format!("page_mode:{}", page_mode_hint(&section(8, raw)))],
        };
        assert_eq!(stage_output.page_mode_hint, "utility_page");
    }

    #[test]
    fn whole_page_semantic_pass_marks_mixed_procedural_editorial_section() {
        let raw =
            "Tourist visa requirements. Passport and fee 80 EUR. FAQ: why refusals happen and what mistakes to avoid.";
        let stage_output = WholePageSemanticPassOutput {
            page_mode_hint: page_mode_hint(&section(9, raw)),
            page_mode_confidence: 0.70,
            dominant_layers: dominant_layers(raw),
            layer_scores: layer_scores_for_text(raw),
            page_summary: summarize(raw),
            page_context_profile: page_context_profile(&[section(9, raw)], raw),
            mixed_section_ids: if dominant_layers(raw).len() > 1 {
                vec![9]
            } else {
                Vec::new()
            },
            global_entities: global_entities(raw),
            advisory_model_used: false,
            advisory_consensus: "not_used".to_string(),
            advisory_prototype_families: Vec::new(),
            uncertainty_flags: vec!["multi_layer_page".to_string()],
            reason_codes: vec![format!("page_mode:{}", page_mode_hint(&section(9, raw)))],
        };
        assert!(stage_output
            .dominant_layers
            .contains(&"procedural".to_string()));
        assert!(stage_output
            .dominant_layers
            .contains(&"editorial".to_string()));
        assert_eq!(stage_output.mixed_section_ids, vec![9]);
    }
}
