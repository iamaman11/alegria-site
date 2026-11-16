BEGIN;
SET LOCAL lock_timeout = '5s';

-- Previously only one snapshot per URL per UTC day was allowed. Do not
-- rewrite or delete the existing rows: VERIFIED.rule_instances keeps
-- evidence_section_id foreign keys to those immutable raw.sections.
DROP INDEX IF EXISTS raw.uq_raw_pages_url_date;
CREATE INDEX IF NOT EXISTS idx_raw_pages_url_snapshot
    ON raw.pages (url, snapshot_at DESC);

COMMIT;
