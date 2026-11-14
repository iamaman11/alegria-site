-- Extend the existing SEO HITL queue for candidate-level truth reviews.
-- Human approval must never directly publish or verify a rule.
ALTER TABLE site.seo_hitl_tasks
    DROP CONSTRAINT IF EXISTS seo_hitl_tasks_task_type_check;
ALTER TABLE site.seo_hitl_tasks
    ADD CONSTRAINT seo_hitl_tasks_task_type_check
    CHECK (task_type IN (
        'cannibalization_conflict','unsafe_serp_pattern',
        'unsupported_factual_fragment','truth_candidate_review',
        'registry_change','rebuild_suppression','canonical_url_conflict',
        'publish_gate_blocker'
    ));
