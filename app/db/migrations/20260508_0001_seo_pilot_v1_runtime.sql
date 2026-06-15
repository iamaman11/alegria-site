-- SEO Pilot v1 runtime hardening
-- Source of truth for live evolution; schema.sql is snapshot only.

ALTER TABLE pipeline.execution_run_blobs
    DROP CONSTRAINT IF EXISTS execution_run_blobs_field_name_check;
ALTER TABLE pipeline.execution_run_blobs
    ADD CONSTRAINT execution_run_blobs_field_name_check
        CHECK (field_name IN (
            'input_payload',
            'verified_support_bundle',
            'extracted_payload',
            'generation_result',
            'content_block_plan',
            'draft_normalize_output',
            'content_contract_validation',
            'publish_materialize_output',
            'render_preview_validation',
            'finalize_publish_output',
            'verify_report',
            'persist_report',
            'errors'
        ));

CREATE TABLE IF NOT EXISTS site.page_support_bindings (
    page_support_binding_key TEXT PRIMARY KEY,
    page_node_key            TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    page_draft_key           TEXT REFERENCES site.page_drafts(page_draft_key) ON DELETE CASCADE,
    support_ref              TEXT NOT NULL,
    source_section_key       TEXT NOT NULL DEFAULT '',
    fragment_kind            TEXT NOT NULL DEFAULT 'factual',
    traceability_label       TEXT NOT NULL DEFAULT '',
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (page_node_key, page_draft_key, support_ref, source_section_key, fragment_kind)
);

CREATE INDEX IF NOT EXISTS idx_site_page_support_bindings_support_ref
    ON site.page_support_bindings(support_ref, page_node_key);
CREATE INDEX IF NOT EXISTS idx_site_page_support_bindings_page
    ON site.page_support_bindings(page_node_key, updated_at DESC);

CREATE TABLE IF NOT EXISTS site.publish_artifact_entries (
    artifact_entry_key TEXT PRIMARY KEY,
    artifact_key       TEXT NOT NULL REFERENCES site.publish_artifacts(artifact_key) ON DELETE CASCADE,
    relative_path      TEXT NOT NULL,
    entry_type         TEXT NOT NULL DEFAULT 'page'
                       CHECK (entry_type IN ('page','sitemap','robots','manifest','asset')),
    status             TEXT NOT NULL DEFAULT 'built'
                       CHECK (status IN ('built','ready','blocked','superseded')),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (artifact_key, relative_path)
);

CREATE INDEX IF NOT EXISTS idx_site_publish_artifact_entries_artifact
    ON site.publish_artifact_entries(artifact_key, entry_type, updated_at DESC);

CREATE TABLE IF NOT EXISTS monitoring.seo_rebuild_dependencies (
    rebuild_dependency_key TEXT PRIMARY KEY,
    page_node_key          TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    dependency_type        TEXT NOT NULL
                           CHECK (dependency_type IN (
                               'truth_support','section_template','blueprint',
                               'keyword_cluster','serp_query','required_link'
                           )),
    dependency_ref         TEXT NOT NULL,
    reason_package         JSONB NOT NULL DEFAULT '{}'::jsonb,
    status                 TEXT NOT NULL DEFAULT 'active'
                           CHECK (status IN ('active','resolved','deprecated')),
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (page_node_key, dependency_type, dependency_ref)
);

CREATE INDEX IF NOT EXISTS idx_monitoring_seo_rebuild_dependencies_page
    ON monitoring.seo_rebuild_dependencies(page_node_key, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_monitoring_seo_rebuild_dependencies_ref
    ON monitoring.seo_rebuild_dependencies(dependency_type, dependency_ref, status);
