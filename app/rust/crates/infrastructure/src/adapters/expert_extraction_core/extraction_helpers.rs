fn block_role_for_section(
    section: &RawSectionRecord,
) -> seo_steps::dom_block_relevance_step::BlockRole {
    let section_type = section.section_type.to_lowercase();
    let heading = section.heading_path.to_lowercase();
    if section_type.contains("nav") || heading.contains("breadcrumb") {
        seo_steps::dom_block_relevance_step::BlockRole::Navigation
    } else if section_type.contains("toc") {
        seo_steps::dom_block_relevance_step::BlockRole::Toc
    } else if section_type.contains("footer") {
        seo_steps::dom_block_relevance_step::BlockRole::Footer
    } else {
        seo_steps::dom_block_relevance_step::BlockRole::ContentMain
    }
}

fn contradiction_facts(
    section: &RawSectionRecord,
    procedural: &seo_steps::procedural_extraction_step::ProceduralExtractionOutput,
) -> Vec<seo_steps::contradiction_gate_step::FactAssertion> {
    let mut facts = Vec::new();
    for rule in &procedural.rules {
        if rule.numeric_tokens.is_empty() {
            facts.push(seo_steps::contradiction_gate_step::FactAssertion {
                subject_key: rule.rule_key.clone(),
                predicate_key: format!("{:?}", rule.role_type).to_lowercase(),
                value_normalized: "true".to_string(),
                source_key: Some(section.source_url.clone()),
                confidence: rule.confidence,
            });
            continue;
        }
        for token in &rule.numeric_tokens {
            facts.push(seo_steps::contradiction_gate_step::FactAssertion {
                subject_key: rule.rule_key.clone(),
                predicate_key: format!("{:?}", rule.role_type).to_lowercase(),
                value_normalized: token.replace(',', "."),
                source_key: Some(section.source_url.clone()),
                confidence: rule.confidence,
            });
        }
    }
    facts
}

