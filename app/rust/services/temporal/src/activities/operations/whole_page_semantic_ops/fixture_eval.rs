#[allow(dead_code)]
pub fn evaluate_whole_page_semantic_fixture(
    sections: &[raw_crawl_adapter::RawSectionRecord],
    advisory_hits: &[whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit],
) -> WholePageSemanticPageState {
    let combined = sections
        .iter()
        .map(|section| section.content_md.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let domain_sections = to_whole_page_sections(sections);
    let snapshot = whole_page::deterministic_whole_page_snapshot(&domain_sections, &combined);
    whole_page::fuse_with_advisory_retrieval(
        &domain_sections,
        snapshot,
        &to_whole_page_advisory_hits(advisory_hits),
    )
}
