ALTER TABLE site.keyword_clusters
    ADD COLUMN IF NOT EXISTS reason_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS reason_version TEXT NOT NULL DEFAULT 'graph_planning@1';

ALTER TABLE site.content_gaps
    ADD COLUMN IF NOT EXISTS reason_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS reason_version TEXT NOT NULL DEFAULT 'graph_planning@1';

ALTER TABLE site.link_recommendations
    ADD COLUMN IF NOT EXISTS reason_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS reason_version TEXT NOT NULL DEFAULT 'graph_planning@1';
