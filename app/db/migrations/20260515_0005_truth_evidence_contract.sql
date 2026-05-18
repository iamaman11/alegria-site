ALTER TABLE kb.sources
    DROP CONSTRAINT IF EXISTS sources_source_type_check;
ALTER TABLE kb.sources
    ADD CONSTRAINT sources_source_type_check
        CHECK (source_type IN ('government','vfs','niche_agency','editorial','internal','forum','low_trust'));

ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS rule_candidate_id TEXT;
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS evidence_section_id BIGINT;
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS evidence_quote TEXT NOT NULL DEFAULT '';
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS span_start INTEGER;
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS span_end INTEGER;
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS source_snapshot_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS verification_method TEXT NOT NULL DEFAULT '';
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS adjudication_reason TEXT NOT NULL DEFAULT '';
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS publish_admissibility TEXT NOT NULL DEFAULT 'not_admissible';
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS freshness_class TEXT NOT NULL DEFAULT 'unknown';
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS completeness_class TEXT NOT NULL DEFAULT 'unknown';
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS registry_version TEXT NOT NULL DEFAULT '';
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS prompt_version TEXT NOT NULL DEFAULT '';
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS model_version TEXT NOT NULL DEFAULT '';
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS pipeline_version TEXT NOT NULL DEFAULT '';
ALTER TABLE verified.rule_instances
    ADD COLUMN IF NOT EXISTS review_decision_id TEXT;

ALTER TABLE verified.rule_instances
    DROP CONSTRAINT IF EXISTS verified_rule_instances_publish_admissibility_check;
ALTER TABLE verified.rule_instances
    ADD CONSTRAINT verified_rule_instances_publish_admissibility_check
        CHECK (publish_admissibility IN ('admissible','not_admissible','needs_hitl','admissible_with_warning'));

ALTER TABLE verified.rule_instances
    DROP CONSTRAINT IF EXISTS verified_rule_instances_freshness_class_check;
ALTER TABLE verified.rule_instances
    ADD CONSTRAINT verified_rule_instances_freshness_class_check
        CHECK (freshness_class IN ('fresh','watch','stale','unknown'));

ALTER TABLE verified.rule_instances
    DROP CONSTRAINT IF EXISTS verified_rule_instances_completeness_class_check;
ALTER TABLE verified.rule_instances
    ADD CONSTRAINT verified_rule_instances_completeness_class_check
        CHECK (completeness_class IN ('complete','partial','incomplete','unknown'));

CREATE INDEX IF NOT EXISTS idx_verified_rule_instances_admissibility
    ON verified.rule_instances(publish_admissibility, status);

CREATE TABLE IF NOT EXISTS extracted.rule_candidates (
    rule_candidate_id      TEXT PRIMARY KEY,
    context_key            TEXT NOT NULL REFERENCES kb.visa_contexts(context_key) ON UPDATE CASCADE,
    raw_section_id         BIGINT NOT NULL REFERENCES raw.sections(id) ON DELETE CASCADE,
    role                   TEXT NOT NULL
                            CHECK (role IN (
                                'DOCUMENT_REQUIRED','ELIGIBILITY_RULE','FEE_ITEM','TIMELINE_ITEM',
                                'WHERE_TO_APPLY','APPOINTMENT_RULE','FORM_REQUIRED','STEP'
                            )),
    concept_canonical_key  TEXT NOT NULL,
    raw_mention            TEXT NOT NULL,
    params                 JSONB NOT NULL DEFAULT '{}'::jsonb,
    scope                  JSONB NOT NULL DEFAULT '{}'::jsonb,
    severity               TEXT NOT NULL DEFAULT 'unknown'
                            CHECK (severity IN ('mandatory','recommended','optional','unknown')),
    applies_to_profiles    JSONB NOT NULL DEFAULT '[]'::jsonb,
    exceptions_raw         TEXT NOT NULL DEFAULT '',
    conditions_raw         TEXT NOT NULL DEFAULT '',
    alternatives           JSONB NOT NULL DEFAULT '[]'::jsonb,
    modality_raw           TEXT NOT NULL DEFAULT '',
    derivation_type        TEXT NOT NULL DEFAULT 'direct'
                            CHECK (derivation_type IN ('direct','inferred','aggregated')),
    is_numeric             BOOLEAN NOT NULL DEFAULT false,
    is_range               BOOLEAN NOT NULL DEFAULT false,
    is_incomplete          BOOLEAN NOT NULL DEFAULT false,
    confidence             NUMERIC(5,4) NOT NULL CHECK (confidence > 0 AND confidence <= 1),
    evidence_section_id    BIGINT NOT NULL REFERENCES raw.sections(id) ON DELETE CASCADE,
    evidence_quote         TEXT NOT NULL,
    span_start             INTEGER NOT NULL CHECK (span_start >= 0),
    span_end               INTEGER NOT NULL CHECK (span_end > span_start),
    source_key             TEXT NOT NULL REFERENCES kb.sources(source_key) ON UPDATE CASCADE,
    source_snapshot_hash   TEXT NOT NULL,
    llm_provider           TEXT NOT NULL,
    llm_model              TEXT NOT NULL,
    prompt_version         TEXT NOT NULL,
    epistemic_status       TEXT NOT NULL DEFAULT 'candidate'
                            CHECK (epistemic_status IN ('candidate','structured','needs_hitl','rejected','verified')),
    uncertainty_flags      JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (raw_section_id, role, concept_canonical_key, evidence_quote, span_start, span_end)
);

CREATE INDEX IF NOT EXISTS idx_extracted_rule_candidates_context
    ON extracted.rule_candidates(context_key, epistemic_status);
CREATE INDEX IF NOT EXISTS idx_extracted_rule_candidates_section
    ON extracted.rule_candidates(raw_section_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_extracted_rule_candidates_source
    ON extracted.rule_candidates(source_key, created_at DESC);

ALTER TABLE verified.rule_instances
    DROP CONSTRAINT IF EXISTS verified_rule_instances_rule_candidate_fk;
ALTER TABLE verified.rule_instances
    ADD CONSTRAINT verified_rule_instances_rule_candidate_fk
        FOREIGN KEY (rule_candidate_id)
        REFERENCES extracted.rule_candidates(rule_candidate_id)
        ON DELETE SET NULL;

ALTER TABLE verified.rule_instances
    DROP CONSTRAINT IF EXISTS verified_rule_instances_evidence_section_fk;
ALTER TABLE verified.rule_instances
    ADD CONSTRAINT verified_rule_instances_evidence_section_fk
        FOREIGN KEY (evidence_section_id)
        REFERENCES raw.sections(id)
        ON DELETE RESTRICT;

UPDATE verified.rule_instances
SET publish_admissibility = 'not_admissible'
WHERE publish_admissibility IS NULL
   OR publish_admissibility = '';
