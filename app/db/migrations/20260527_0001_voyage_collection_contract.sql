ALTER TABLE kb.qdrant_points
    DROP CONSTRAINT IF EXISTS qdrant_points_collection_name_check;

ALTER TABLE kb.qdrant_points
    ADD CONSTRAINT qdrant_points_collection_name_check
    CHECK (collection_name IN (
        'content_chunks',
        'kb_canonical',
        'ontology',
        'raw_chunks_4',
        'raw_chunks_ctx',
        'kb_canonical_4',
        'verified_rules_voyage4',
        'editorial_topics_voyage4',
        'seo_keyword_clusters_voyage4',
        'whole_page_advisory_prototypes',
        'seo_page_blueprints',
        'seo_serp_patterns'
    ));
