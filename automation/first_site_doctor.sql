-- Read-only first-site dashboard counters. The SQL owner lives outside Python
-- to preserve the automation Python/DB boundary. No schema modification.
SELECT (SELECT count(*) FROM pipeline.execution_runs)::text || ',' ||
       (SELECT count(*) FROM pipeline.manual_llm_jobs
        WHERE job_status = 'pending')::text || ',' ||
       (SELECT count(*) FROM site.seo_hitl_tasks
        WHERE queue_state IN ('open', 'in_review', 'reopened'))::text;
