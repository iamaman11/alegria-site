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
    let snapshot = deterministic_whole_page_snapshot(sections, &combined);
    fuse_with_advisory_retrieval(sections, snapshot, advisory_hits)
}
