ALTER TABLE serp.crawl_queue
    ADD COLUMN IF NOT EXISTS source_domain TEXT NOT NULL DEFAULT '';
ALTER TABLE serp.crawl_queue
    ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now();

UPDATE serp.crawl_queue
SET source_domain = lower(
        regexp_replace(
            regexp_replace(
                split_part(split_part(split_part(url, '://', 2), '/', 1), ':', 1),
                '^www\\.',
                ''
            ),
            '\\.$',
            ''
        )
    ),
    next_attempt_at = COALESCE(next_attempt_at, now())
WHERE source_domain = ''
   OR next_attempt_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_serp_crawl_queue_domain_ready
    ON serp.crawl_queue(source_domain, status, next_attempt_at, first_seen_at);
