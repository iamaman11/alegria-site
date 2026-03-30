# SERP Ingest Playbook (run_gemini3_global_186_20260320_top10)

> Status: reference runbook for the immutable SERP run artifacts in this directory. It is not the source of truth for current Temporal/runtime architecture.

Этот документ объединяет runbook, protocol, DB guide и audit.

## 1) Input artifacts

- `run_gemini3_global_186_20260320_top10__results.jsonl`
- `run_gemini3_global_186_20260320_top10__enrichment_resolve.jsonl`
- `run_gemini3_global_186_20260320_top10__enrichment_resolve_rendered.jsonl`
- `run_gemini3_global_186_20260320_top10__DB_INGEST_SQL_PLAN.sql`
- `run_gemini3_global_186_20260320_top10__domain_audit.csv`
- `run_gemini3_global_186_20260320_top10__summary.json`
- `run_gemini3_global_186_20260320_top10__resolve_summary.json`

## 2) Confirmed baseline metrics

- jobs: `186`
- top10 rows: `1860`
- source_resolved rows: `120`
- rendered_chip rows: `748`
- vertex redirect URLs in top10: `0`
- explicit enrichment status: `200=104, 400=5, 403=5, 404=2, 500=1, null=3`

## 3) Layer model

- `source_resolved`: content-source candidates
- `rendered_chip`: intent/query layer (not direct content crawl source)

## 4) Ingest invariants

- raw immutable ingest: `ON CONFLICT (run_id, job_id) DO NOTHING`
- `source_key` uniqueness
- `url_norm UNIQUE` in crawl queue
- resolved source enqueue only for `http_status BETWEEN 200 AND 399`
- rendered loader maps `chip_index -> chunk_index`
- `also_in_sources` update only for `domain_resolved IS NOT NULL`

## 5) Execution order

1. Load raw snapshots
2. Load explicit source resolutions (`source_resolved`)
3. Load rendered-chip resolutions (`rendered_chip`)
4. Build normalized tables:
   - `serp.gemini_top10`
   - `serp.gemini_sources`
   - `serp.gemini_queries`
   - `serp.gemini_supports`
5. Build crawl queue (dedup + domain audit mapping)

## 6) Verification queries

```sql
SELECT count(*) FROM serp.raw_snapshots
WHERE run_id='run_gemini3_global_186_20260320_top10';

SELECT count(*) FROM serp.gemini_top10
WHERE run_id='run_gemini3_global_186_20260320_top10';

SELECT source_type, count(*) FROM serp.source_resolutions
WHERE run_id='run_gemini3_global_186_20260320_top10'
GROUP BY source_type ORDER BY source_type;

SELECT url_norm, count(*) FROM serp.crawl_queue
GROUP BY url_norm HAVING count(*) > 1;
```

## 7) Known non-blocking risks

- geo-mismatch resolved URLs
- anti-bot challenge pages
- dead links (404)

Handling:
- `review` lane for geo/challenge,
- `exclude` for dead links/noise,
- do not enqueue invalid resolved status.

## 8) Decision

Current state: `GO` for ingest with the filters above.
