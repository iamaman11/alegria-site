ALTER TABLE serp.gemini_top10
    ADD COLUMN IF NOT EXISTS source_tier TEXT NOT NULL DEFAULT 'low_trust';

UPDATE serp.gemini_top10
SET source_tier = COALESCE(NULLIF(source_tier, ''), 'low_trust')
WHERE source_tier IS NULL
   OR source_tier = '';
