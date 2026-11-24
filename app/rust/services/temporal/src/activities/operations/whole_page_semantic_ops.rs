use seo_domain::whole_page;

// Only adapter-to-domain projections and process configuration stay in the
// Temporal activity. All semantic decisions live in seo_domain::whole_page.
fn to_whole_page_sections(
    sections: &[raw_crawl_adapter::RawSectionRecord],
) -> Vec<whole_page::WholePageSection> {
    sections
        .iter()
        .map(|section| whole_page::WholePageSection {
            id: section.id,
            page_id: section.page_id,
            source_url: section.source_url.clone(),
            heading_path: section.heading_path.clone(),
            section_type: section.section_type.clone(),
            content_md: section.content_md.clone(),
        })
        .collect()
}

fn to_whole_page_advisory_hits(
    hits: &[whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit],
) -> Vec<whole_page::WholePageAdvisoryHit> {
    hits
        .iter()
        .map(|hit| whole_page::WholePageAdvisoryHit {
            prototype_family: hit.prototype_family.clone(),
            score: hit.score,
            page_mode: hit.page_mode.clone(),
            dominant_layers: hit.dominant_layers.clone(),
            country_hints: hit.country_hints.clone(),
            visa_type_hints: hit.visa_type_hints.clone(),
            authority_hints: hit.authority_hints.clone(),
            mixed_section_pressure: hit.mixed_section_pressure,
        })
        .collect()
}

fn advisory_hit_limit() -> u64 {
    std::env::var("WHOLE_PAGE_ADVISORY_LIMIT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(3)
}

// Fixture tests exercise the actual adapter projection, not a second
// interpretation of page semantics or a deprecated algorithm facade.
#[cfg(test)]
fn deterministic_whole_page_snapshot(
    sections: &[raw_crawl_adapter::RawSectionRecord],
    combined: &str,
) -> whole_page::WholePageDeterministicSnapshot {
    whole_page::deterministic_whole_page_snapshot(&to_whole_page_sections(sections), combined)
}

#[cfg(test)]
fn fuse_with_advisory_retrieval(
    sections: &[raw_crawl_adapter::RawSectionRecord],
    snapshot: whole_page::WholePageDeterministicSnapshot,
    hits: &[whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit],
) -> whole_page::WholePageSemanticPageState {
    whole_page::fuse_with_advisory_retrieval(
        &to_whole_page_sections(sections),
        snapshot,
        &to_whole_page_advisory_hits(hits),
    )
}

include!("whole_page_semantic_ops/fixture_eval.rs");
include!("whole_page_semantic_ops/section_gate_indexes.rs");
