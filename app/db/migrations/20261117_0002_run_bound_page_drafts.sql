BEGIN;
SET LOCAL lock_timeout = '5s';

-- Draft revision numbering is local to a business run. The existing truth
-- snapshot reference is that run identity; do not create a second owner.
ALTER TABLE site.page_drafts
    DROP CONSTRAINT IF EXISTS page_drafts_page_node_key_page_brief_key_draft_revision_key;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'site.page_drafts'::regclass
          AND conname = 'page_drafts_run_revision_uq'
    ) THEN
        ALTER TABLE site.page_drafts ADD CONSTRAINT page_drafts_run_revision_uq
            UNIQUE (page_node_key, page_brief_key, truth_snapshot_ref, draft_revision);
    END IF;
END $$;
COMMIT;
