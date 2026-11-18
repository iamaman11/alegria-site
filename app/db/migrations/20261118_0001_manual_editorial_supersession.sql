-- A pending editorial request has exactly one owner per run/page.
-- A superseded request is not a completed response or a human decision.
ALTER TABLE pipeline.manual_llm_jobs
    DROP CONSTRAINT manual_llm_jobs_check,
    DROP CONSTRAINT manual_llm_jobs_job_status_check;
ALTER TABLE pipeline.manual_llm_jobs
    ADD CONSTRAINT manual_llm_jobs_job_status_check
    CHECK (job_status IN ('pending', 'completed', 'superseded')),
    ADD CONSTRAINT manual_llm_jobs_check
    CHECK (
        (job_status IN ('pending', 'superseded')
         AND response_json IS NULL AND output_json IS NULL)
        OR
        (job_status = 'completed' AND response_json IS NOT NULL
         AND output_json IS NOT NULL AND model_key IS NOT NULL)
    );

-- Keep completed historical decisions and exact input receipts. Only pending
-- older requests for the same page in the same run can be superseded.
UPDATE pipeline.manual_llm_jobs old
SET job_status = 'superseded', updated_at = now()
WHERE old.operation = 'editorial_draft' AND old.job_status = 'pending'
  AND nullif(old.input_json #>> '{request,editorial_brief,page_node_key}', '') IS NOT NULL
  AND EXISTS (
      SELECT 1 FROM pipeline.manual_llm_jobs newer
      WHERE newer.run_id = old.run_id AND newer.operation = old.operation
        AND newer.job_status IN ('pending','completed')
        AND newer.input_json #>> '{request,editorial_brief,page_node_key}'
          = old.input_json #>> '{request,editorial_brief,page_node_key}'
        AND (newer.created_at, newer.task_id) > (old.created_at, old.task_id)
  );

CREATE UNIQUE INDEX IF NOT EXISTS manual_llm_one_pending_editorial_owner
    ON pipeline.manual_llm_jobs
    (run_id, (input_json #>> '{request,editorial_brief,page_node_key}'))
    WHERE operation = 'editorial_draft' AND job_status = 'pending'
      AND nullif(input_json #>> '{request,editorial_brief,page_node_key}', '') IS NOT NULL;
