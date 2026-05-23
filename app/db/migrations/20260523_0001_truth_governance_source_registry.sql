ALTER TABLE kb.sources
    ADD COLUMN IF NOT EXISTS authority_class TEXT NOT NULL DEFAULT 'unknown',
    ADD COLUMN IF NOT EXISTS independence_group_key TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS freshness_ttl_days INTEGER NOT NULL DEFAULT 30,
    ADD COLUMN IF NOT EXISTS override_eligible BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE kb.sources
    DROP CONSTRAINT IF EXISTS kb_sources_authority_class_check;

ALTER TABLE kb.sources
    ADD CONSTRAINT kb_sources_authority_class_check
    CHECK (
        authority_class IN (
            'primary_authority',
            'delegated_authority',
            'official_publisher',
            'editorial',
            'agency',
            'forum',
            'unknown'
        )
    );

ALTER TABLE kb.sources
    DROP CONSTRAINT IF EXISTS kb_sources_freshness_ttl_days_check;

ALTER TABLE kb.sources
    ADD CONSTRAINT kb_sources_freshness_ttl_days_check
    CHECK (freshness_ttl_days >= 0);

UPDATE kb.sources
SET authority_class = CASE
        WHEN source_type = 'government' THEN 'primary_authority'
        WHEN source_type = 'vfs' THEN 'delegated_authority'
        WHEN source_type = 'editorial' THEN 'editorial'
        WHEN source_type = 'niche_agency' THEN 'agency'
        WHEN source_type = 'forum' THEN 'forum'
        WHEN source_type = 'low_trust' THEN 'forum'
        ELSE 'unknown'
    END,
    independence_group_key = CASE
        WHEN coalesce(base_url, '') <> '' THEN regexp_replace(base_url, '^https?://', '')
        ELSE source_key
    END,
    freshness_ttl_days = CASE
        WHEN source_type = 'government' THEN 14
        WHEN source_type = 'vfs' THEN 14
        WHEN source_type = 'editorial' THEN 7
        WHEN source_type = 'niche_agency' THEN 7
        WHEN source_type = 'forum' THEN 3
        WHEN source_type = 'low_trust' THEN 1
        ELSE 30
    END,
    override_eligible = false
WHERE authority_class = 'unknown'
   OR independence_group_key = ''
   OR freshness_ttl_days = 30
   OR override_eligible = false;
