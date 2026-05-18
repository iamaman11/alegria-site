CREATE TABLE IF NOT EXISTS raw.page_content_aliases (
    alias_page_id     BIGINT PRIMARY KEY REFERENCES raw.pages(id) ON DELETE CASCADE,
    canonical_page_id BIGINT NOT NULL REFERENCES raw.pages(id) ON DELETE CASCADE,
    source_url        TEXT NOT NULL,
    final_url         TEXT NOT NULL,
    content_hash      TEXT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_raw_page_content_aliases_canonical
    ON raw.page_content_aliases(canonical_page_id);
CREATE INDEX IF NOT EXISTS idx_raw_page_content_aliases_hash
    ON raw.page_content_aliases(content_hash);
