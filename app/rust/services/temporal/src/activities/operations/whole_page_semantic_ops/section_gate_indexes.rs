fn block_role_for_section(
    section: &raw_crawl_adapter::RawSectionRecord,
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

struct SectionSemanticGateIndexes {
    procedural_allowed: BTreeMap<i64, bool>,
    editorial_allowed: BTreeMap<i64, bool>,
    structural_allowed: BTreeMap<i64, bool>,
    dom_allowed: BTreeMap<i64, bool>,
    contract_pass: BTreeMap<i64, bool>,
    replay_safe: BTreeMap<i64, bool>,
}

impl SectionSemanticGateIndexes {
    fn from_bundle(bundle: &SectionSemanticGateBundle) -> Self {
        Self {
            procedural_allowed: bundle
                .page_utility
                .decisions
                .iter()
                .map(|decision| (decision.section_id, decision.allow_procedural_extraction))
                .collect(),
            editorial_allowed: bundle
                .page_utility
                .decisions
                .iter()
                .map(|decision| (decision.section_id, decision.allow_editorial_extraction))
                .collect(),
            structural_allowed: bundle
                .page_utility
                .decisions
                .iter()
                .map(|decision| (decision.section_id, decision.allow_structural_extraction))
                .collect(),
            dom_allowed: bundle
                .dom_relevance
                .decisions
                .iter()
                .map(|decision| (decision.section_id, decision.allow_extraction))
                .collect(),
            contract_pass: bundle
                .sectioning_contract
                .decisions
                .iter()
                .map(|decision| (decision.section_id, decision.decision == "pass"))
                .collect(),
            replay_safe: bundle
                .cas_gate
                .decisions
                .iter()
                .map(|decision| (decision.section_id, decision.is_replay_safe))
                .collect(),
        }
    }

    fn blocked_by_gate(&self, section_id: i64) -> bool {
        !self
            .structural_allowed
            .get(&section_id)
            .copied()
            .unwrap_or(false)
            || !self.dom_allowed.get(&section_id).copied().unwrap_or(false)
            || !self
                .contract_pass
                .get(&section_id)
                .copied()
                .unwrap_or(false)
            || !self.replay_safe.get(&section_id).copied().unwrap_or(false)
    }

    fn allow_procedural_extraction(&self, section_id: i64) -> bool {
        self.procedural_allowed
            .get(&section_id)
            .copied()
            .unwrap_or(false)
    }

    fn allow_editorial_extraction(&self, section_id: i64) -> bool {
        self.editorial_allowed
            .get(&section_id)
            .copied()
            .unwrap_or(false)
    }
}
