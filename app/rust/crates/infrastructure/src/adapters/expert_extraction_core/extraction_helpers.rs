fn block_role_for_section(
    section: &RawSectionRecord,
) -> acquisition::source_relevance::BlockRole {
    acquisition::source_relevance::block_role_for_section(
        &section.section_type,
        &section.heading_path,
    )
}

fn contradiction_facts(
    section: &RawSectionRecord,
    procedural: &seo_steps::procedural_extraction_step::ProceduralExtractionOutput,
) -> Vec<truth_domain::contradictions::FactAssertion> {
    let mut facts = Vec::new();
    for rule in &procedural.rules {
        if rule.numeric_tokens.is_empty() {
            facts.push(truth_domain::contradictions::FactAssertion {
                subject_key: rule.rule_key.clone(),
                predicate_key: format!("{:?}", rule.role_type).to_lowercase(),
                value_normalized: "true".to_string(),
                source_key: Some(section.source_url.clone()),
                confidence: rule.confidence,
            });
            continue;
        }
        for token in &rule.numeric_tokens {
            facts.push(truth_domain::contradictions::FactAssertion {
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

