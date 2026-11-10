-- Durable, provider-independent typed manual LLM inbox.
-- Manual LLM responses never imply verified truth or publication approval.
CREATE TABLE IF NOT EXISTS pipeline.manual_llm_jobs (
    task_id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    run_id TEXT NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('editorial_draft', 'truth_candidates')),
    input_sha256 TEXT NOT NULL CHECK (input_sha256 ~ '^[a-f0-9]{64}$'),
    input_json JSONB NOT NULL,
    job_status TEXT NOT NULL DEFAULT 'pending' CHECK (job_status IN ('pending', 'completed')),
    response_json JSONB,
    output_json JSONB,
    model_key TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, operation, input_sha256),
    CHECK (
        (job_status = 'pending' AND response_json IS NULL AND output_json IS NULL)
        OR
        (job_status = 'completed' AND response_json IS NOT NULL AND output_json IS NOT NULL
         AND model_key IS NOT NULL)
    )
);
CREATE INDEX IF NOT EXISTS idx_manual_llm_jobs_pending
    ON pipeline.manual_llm_jobs (created_at, task_id) WHERE job_status = 'pending';

-- Upgrade an already-created editorial-only inbox without duplicating tables.
ALTER TABLE pipeline.manual_llm_jobs
    DROP CONSTRAINT IF EXISTS manual_llm_jobs_operation_check;
ALTER TABLE pipeline.manual_llm_jobs
    ADD CONSTRAINT manual_llm_jobs_operation_check
    CHECK (operation IN ('editorial_draft', 'truth_candidates'));
