ALTER TABLE raw.pages
    ADD COLUMN IF NOT EXISTS final_url TEXT;
ALTER TABLE raw.pages
    ADD COLUMN IF NOT EXISTS redirect_chain JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE raw.pages
    ADD COLUMN IF NOT EXISTS robots_trace JSONB NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE raw.pages
    ADD COLUMN IF NOT EXISTS source_observation JSONB NOT NULL DEFAULT '{}'::jsonb;

UPDATE raw.pages
SET final_url = COALESCE(final_url, url),
    redirect_chain = COALESCE(redirect_chain, '[]'::jsonb),
    robots_trace = COALESCE(robots_trace, '{}'::jsonb),
    source_observation = COALESCE(source_observation, '{}'::jsonb)
WHERE final_url IS NULL
   OR redirect_chain IS NULL
   OR robots_trace IS NULL
   OR source_observation IS NULL;
