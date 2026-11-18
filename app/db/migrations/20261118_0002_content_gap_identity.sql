-- The same factual gap may exist on distinct pages, and different detectors
-- may observe the same topic. Preserve the existing stable row/projection key.
ALTER TABLE site.content_gaps
    DROP CONSTRAINT IF EXISTS content_gaps_scope_signature_missing_topic_detector_version_key;

UPDATE site.content_gaps
SET detector_version = CASE reason_payload->>'reason_code'
    WHEN 'low_reliability_serp' THEN 'serp_reliability@1'
    WHEN 'graph_missing_topic_coverage' THEN 'graph_coverage@1'
    WHEN 'global_reconcile_missing_topic' THEN 'reconcile_coverage@1'
    WHEN 'page_missing_verified_support' THEN 'page_missing_support@1'
    WHEN 'page_duplicate_verified_evidence' THEN 'page_duplicate_evidence@1'
    ELSE detector_version
END
WHERE detector_version = 'seo_content_gap@1';

-- NULLS NOT DISTINCT keeps scope-wide findings unique too.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'site.content_gaps'::regclass
          AND conname = 'content_gaps_owner_topic_detector_key'
    ) THEN
        ALTER TABLE site.content_gaps
            ADD CONSTRAINT content_gaps_owner_topic_detector_key
            UNIQUE NULLS NOT DISTINCT (scope_signature, page_node_key, missing_topic, detector_version);
    END IF;
END $$;
