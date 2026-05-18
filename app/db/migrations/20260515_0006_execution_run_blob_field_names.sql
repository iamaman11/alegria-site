ALTER TABLE pipeline.execution_run_blobs
    DROP CONSTRAINT IF EXISTS execution_run_blobs_field_name_check;
ALTER TABLE pipeline.execution_run_blobs
    ADD CONSTRAINT execution_run_blobs_field_name_check
        CHECK (field_name IN (
            'input_payload',
            'verified_support_bundle',
            'extracted_payload',
            'generation_result',
            'content_block_plan',
            'draft_normalize_output',
            'content_contract_validation',
            'publish_materialize_output',
            'render_preview_validation',
            'finalize_publish_output',
            'verify_report',
            'persist_report',
            'errors'
        ));
