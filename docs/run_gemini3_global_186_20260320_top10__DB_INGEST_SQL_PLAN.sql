-- DB ingest and transform plan for:
-- run_id = run_gemini3_global_186_20260320_top10
-- raw file = results.jsonl
-- enrichment file = enrichment_resolve.jsonl
--
-- Notes:
-- 1) raw is immutable source-of-truth
-- 2) run_id is injected at load time
-- 3) recorded_at comes from raw ts (record write time)

BEGIN;

CREATE SCHEMA IF NOT EXISTS serp;

-- 1) Staging raw
CREATE TABLE IF NOT EXISTS serp.raw_snapshots (
  run_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  query TEXT NOT NULL,
  recorded_at TIMESTAMPTZ,
  raw_result JSONB NOT NULL,
  PRIMARY KEY (run_id, job_id)
);

-- 2) Staging source resolutions
CREATE TABLE IF NOT EXISTS serp.source_resolutions (
  source_key TEXT PRIMARY KEY,
  source_type TEXT NOT NULL, -- source_resolved | rendered_chip
  run_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  query TEXT,
  -- For source_resolved rows: chunk_index from raw sources.
  -- For rendered_chip rows: map chip_index -> chunk_index at loader stage.
  chunk_index INT NOT NULL,
  uri_redirect TEXT NOT NULL,
  uri_resolved TEXT,
  http_status INT,
  redirect_hops INT,
  resolved_at TIMESTAMPTZ,
  latency_ms INT,
  attempts_used INT,
  resolve_error TEXT,
  resolver_version TEXT
);

CREATE INDEX IF NOT EXISTS idx_source_resolutions_run_job
  ON serp.source_resolutions (run_id, job_id);
CREATE INDEX IF NOT EXISTS idx_source_resolutions_source_type
  ON serp.source_resolutions (source_type);
CREATE INDEX IF NOT EXISTS idx_source_resolutions_uri_resolved
  ON serp.source_resolutions (uri_resolved);

-- 3) Normalized top10
CREATE TABLE IF NOT EXISTS serp.gemini_top10 (
  run_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  rank INT NOT NULL,
  title TEXT,
  url TEXT NOT NULL,
  url_norm TEXT,
  domain_norm TEXT,
  also_in_sources BOOLEAN DEFAULT FALSE,
  PRIMARY KEY (run_id, job_id, rank)
);
CREATE INDEX IF NOT EXISTS idx_gemini_top10_url_norm
  ON serp.gemini_top10 (url_norm);
CREATE INDEX IF NOT EXISTS idx_gemini_top10_domain_norm
  ON serp.gemini_top10 (domain_norm);

-- 4) Normalized sources
CREATE TABLE IF NOT EXISTS serp.gemini_sources (
  source_key TEXT PRIMARY KEY,
  source_type TEXT NOT NULL, -- source_resolved | rendered_chip
  run_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  chunk_index INT NOT NULL,
  uri_redirect TEXT NOT NULL,
  uri_resolved TEXT,
  domain_source_api TEXT,
  domain_resolved TEXT,
  title_source TEXT,
  resolve_error TEXT
);
CREATE INDEX IF NOT EXISTS idx_gemini_sources_run_job
  ON serp.gemini_sources (run_id, job_id);
CREATE INDEX IF NOT EXISTS idx_gemini_sources_domain_resolved
  ON serp.gemini_sources (domain_resolved);

-- 5) Queries and supports (avoid data loss)
CREATE TABLE IF NOT EXISTS serp.gemini_queries (
  run_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  position INT NOT NULL,
  query TEXT NOT NULL,
  PRIMARY KEY (run_id, job_id, position)
);
CREATE INDEX IF NOT EXISTS idx_gemini_queries_query
  ON serp.gemini_queries (query);

CREATE TABLE IF NOT EXISTS serp.gemini_supports (
  run_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  support_idx INT NOT NULL,
  text_fragment TEXT NOT NULL,
  confidence NUMERIC,
  is_model_json BOOLEAN DEFAULT FALSE,
  chunk_indices INT[],
  PRIMARY KEY (run_id, job_id, support_idx)
);

-- 6) Crawl queue with source_type + dtype
CREATE TABLE IF NOT EXISTS serp.crawl_queue (
  url TEXT NOT NULL,
  url_norm TEXT NOT NULL UNIQUE,
  source_type TEXT NOT NULL, -- top10 | source_resolved
  dtype TEXT,                -- government | niche_agency | forum | news | ...
  first_seen_run_id TEXT NOT NULL,
  first_seen_job_id TEXT NOT NULL,
  first_seen_at TIMESTAMPTZ DEFAULT now(),
  status TEXT DEFAULT 'pending',
  http_status INT,
  notes TEXT
);
CREATE INDEX IF NOT EXISTS idx_crawl_queue_status
  ON serp.crawl_queue (status);

COMMIT;

-- =====================
-- LOAD RAW SNAPSHOTS
-- =====================
-- Example temp load approach:
-- 1) parse results.jsonl externally and insert run_id/job_id/query/recorded_at/raw_result
-- 2) keep raw immutable on conflicts
--
-- INSERT INTO serp.raw_snapshots(run_id,job_id,query,recorded_at,raw_result)
-- VALUES (...)
-- ON CONFLICT (run_id, job_id) DO NOTHING;

-- =====================
-- LOAD SOURCE RESOLUTION
-- =====================
-- Load explicit sources enrichment file:
-- enrichment_resolve.jsonl
-- INSERT INTO serp.source_resolutions(...)
-- VALUES (...)
-- ON CONFLICT (source_key) DO UPDATE SET
--   uri_resolved = EXCLUDED.uri_resolved,
--   http_status = EXCLUDED.http_status,
--   redirect_hops = EXCLUDED.redirect_hops,
--   resolved_at = EXCLUDED.resolved_at,
--   latency_ms = EXCLUDED.latency_ms,
--   attempts_used = EXCLUDED.attempts_used,
--   resolve_error = EXCLUDED.resolve_error,
--   resolver_version = EXCLUDED.resolver_version,
--   source_type = EXCLUDED.source_type;
--
-- Load rendered-chip enrichment file:
-- enrichment_resolve_rendered.jsonl
-- INSERT INTO serp.source_resolutions(...)
-- VALUES (...)
-- ON CONFLICT (source_key) DO UPDATE SET
--   uri_resolved = EXCLUDED.uri_resolved,
--   http_status = EXCLUDED.http_status,
--   redirect_hops = EXCLUDED.redirect_hops,
--   resolved_at = EXCLUDED.resolved_at,
--   latency_ms = EXCLUDED.latency_ms,
--   attempts_used = EXCLUDED.attempts_used,
--   resolve_error = EXCLUDED.resolve_error,
--   resolver_version = EXCLUDED.resolver_version,
--   source_type = EXCLUDED.source_type;

-- =====================
-- TRANSFORMS
-- =====================
-- IMPORTANT:
-- For jsonb array extraction use WITH ORDINALITY where needed.
-- The sample snippets below assume run_id='run_gemini3_global_186_20260320_top10'.

-- Top10 extraction (sample shape)
-- INSERT INTO serp.gemini_top10(run_id,job_id,rank,title,url,url_norm,domain_norm,also_in_sources)
-- SELECT
--   r.run_id,
--   r.job_id,
--   (x->>'rank')::int AS rank,
--   x->>'title' AS title,
--   x->>'url' AS url,
--   lower(regexp_replace(split_part(x->>'url', '?', 1), '/+$', '')) AS url_norm,
--   lower(regexp_replace(split_part(split_part(x->>'url','//',2), '/', 1), '^www\\.', '')) AS domain_norm,
--   COALESCE((x->>'also_in_sources')::boolean, false) AS also_in_sources
-- FROM serp.raw_snapshots r
-- CROSS JOIN LATERAL jsonb_array_elements(r.raw_result->'top10') x
-- WHERE r.run_id = 'run_gemini3_global_186_20260320_top10'
-- ON CONFLICT (run_id,job_id,rank) DO UPDATE SET
--   title = EXCLUDED.title,
--   url = EXCLUDED.url,
--   url_norm = EXCLUDED.url_norm,
--   domain_norm = EXCLUDED.domain_norm,
--   also_in_sources = EXCLUDED.also_in_sources;

-- Sources extraction + join to resolutions by source_key
-- source_key expression must be produced by loader or SQL function.
-- Prefer precomputing source_key in loader and loading to temp table.
-- Populate both explicit sources and rendered_chip links into serp.gemini_sources.

-- Queries extraction
-- INSERT INTO serp.gemini_queries(run_id,job_id,position,query)
-- SELECT
--   r.run_id,
--   r.job_id,
--   ord::int - 1 AS position,
--   q::text AS query
-- FROM serp.raw_snapshots r
-- CROSS JOIN LATERAL jsonb_array_elements_text(r.raw_result->'web_search_queries') WITH ORDINALITY t(q, ord)
-- WHERE r.run_id = 'run_gemini3_global_186_20260320_top10'
-- ON CONFLICT (run_id,job_id,position) DO UPDATE SET query = EXCLUDED.query;

-- Supports extraction
-- INSERT INTO serp.gemini_supports(run_id,job_id,support_idx,text_fragment,confidence,is_model_json,chunk_indices)
-- SELECT
--   r.run_id,
--   r.job_id,
--   ord::int - 1 AS support_idx,
--   s->>'text_fragment' AS text_fragment,
--   NULLIF(s->>'confidence','')::numeric AS confidence,
--   COALESCE((s->>'is_model_json')::boolean, false) AS is_model_json,
--   ARRAY(SELECT (z)::int FROM jsonb_array_elements_text(COALESCE(s->'chunk_indices','[]'::jsonb)) z) AS chunk_indices
-- FROM serp.raw_snapshots r
-- CROSS JOIN LATERAL jsonb_array_elements(r.raw_result->'supports') WITH ORDINALITY t(s, ord)
-- WHERE r.run_id = 'run_gemini3_global_186_20260320_top10'
-- ON CONFLICT (run_id,job_id,support_idx) DO UPDATE SET
--   text_fragment = EXCLUDED.text_fragment,
--   confidence = EXCLUDED.confidence,
--   is_model_json = EXCLUDED.is_model_json,
--   chunk_indices = EXCLUDED.chunk_indices;

-- also_in_sources refinement by resolved domain (ONLY non-null resolved domain)
UPDATE serp.gemini_top10 t
SET also_in_sources = true
FROM serp.gemini_sources s
WHERE s.domain_resolved IS NOT NULL
  AND t.domain_norm = s.domain_resolved
  AND t.run_id = s.run_id;

-- Crawl queue build
-- top10 URLs
-- INSERT INTO serp.crawl_queue(url,url_norm,source_type,dtype,first_seen_run_id,first_seen_job_id,status,http_status)
-- SELECT url, url_norm, 'top10', NULL, run_id, job_id, 'pending', 200
-- FROM serp.gemini_top10
-- WHERE run_id='run_gemini3_global_186_20260320_top10'
-- ON CONFLICT DO NOTHING;
--
-- resolved source URLs
-- INSERT INTO serp.crawl_queue(url,url_norm,source_type,dtype,first_seen_run_id,first_seen_job_id,status,http_status)
-- SELECT uri_resolved, lower(regexp_replace(split_part(uri_resolved, '?', 1), '/+$', '')),
--        source_type, NULL, run_id, job_id, 'pending', http_status
-- FROM serp.gemini_sources
-- WHERE run_id='run_gemini3_global_186_20260320_top10'
--   AND uri_resolved IS NOT NULL
--   AND http_status BETWEEN 200 AND 399
-- ON CONFLICT DO NOTHING;

-- Optional pre-crawl filters (recommended):
-- 1) mark known noise domains (e.g. wikipedia.org) for manual review/exclusion
-- UPDATE serp.crawl_queue
-- SET status='excluded', notes='noise_domain'
-- WHERE url_norm LIKE '%wikipedia.org%';
--
-- 2) mark likely geo-mismatch pages (example heuristics)
-- UPDATE serp.crawl_queue
-- SET status='review', notes='possible_geo_mismatch'
-- WHERE url_norm LIKE '%/russia/%' OR url_norm LIKE '%/kgz/%';

-- =====================
-- QUALITY CHECKS
-- =====================
-- SELECT count(*) FROM serp.raw_snapshots WHERE run_id='run_gemini3_global_186_20260320_top10';    -- 186
-- SELECT count(*) FROM serp.gemini_top10 WHERE run_id='run_gemini3_global_186_20260320_top10';      -- 1860
-- SELECT count(*) FROM serp.source_resolutions WHERE run_id='run_gemini3_global_186_20260320_top10';-- 120
-- SELECT count(*) FROM serp.source_resolutions WHERE run_id='run_gemini3_global_186_20260320_top10' AND resolve_error IS NULL;
