ALTER TABLE monitoring.seo_rebuild_dependencies
    DROP CONSTRAINT IF EXISTS seo_rebuild_dependencies_dependency_type_check;

ALTER TABLE monitoring.seo_rebuild_dependencies
    ADD CONSTRAINT seo_rebuild_dependencies_dependency_type_check
    CHECK (dependency_type IN (
        'truth_support',
        'blueprint',
        'section_template',
        'keyword_cluster',
        'serp_query',
        'required_link',
        'source_provenance',
        'navigation_state'
    ));
