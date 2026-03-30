-- ==============================================================================
-- Alegria SEO Engine — Unified Schema (V2)
-- ==============================================================================
-- Применяется автоматически при первом старте контейнера (initdb.d/init.sql).
-- Все DDL идемпотентны (CREATE IF NOT EXISTS, OR REPLACE).
-- Порядок секций: расширения → схемы → kb.* → verified.* → system.* →
--                 site.* → raw.* → serp.* → compat-layer
-- ==============================================================================

BEGIN;

-- ==============================================================================
-- 0. РАСШИРЕНИЯ
-- ==============================================================================
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- ==============================================================================
-- 1. СХЕМЫ
-- ==============================================================================
CREATE SCHEMA IF NOT EXISTS serp;
CREATE SCHEMA IF NOT EXISTS raw;
CREATE SCHEMA IF NOT EXISTS kb;
CREATE SCHEMA IF NOT EXISTS extracted;
CREATE SCHEMA IF NOT EXISTS verified;
CREATE SCHEMA IF NOT EXISTS site;
CREATE SCHEMA IF NOT EXISTS monitoring;
CREATE SCHEMA IF NOT EXISTS pipeline;
CREATE SCHEMA IF NOT EXISTS system;

-- ==============================================================================
-- 2. KB — Domain Registry & Ontology
-- ==============================================================================

CREATE TABLE IF NOT EXISTS kb.visa_families (
    key         TEXT PRIMARY KEY,
    label_ru    TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active','deprecated')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Seeds (добавляются один раз, безопасно повторять)
INSERT INTO kb.visa_families (key, label_ru) VALUES
  ('tourist',      'Туристическая'),
  ('business',     'Деловая'),
  ('transit',      'Транзитная'),
  ('national',     'Национальная'),
  ('humanitarian', 'Гуманитарная')
ON CONFLICT DO NOTHING;

-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS kb.concepts (
    concept_key  TEXT PRIMARY KEY,
    concept_type TEXT NOT NULL
        CHECK (concept_type IN ('document','fee','timeline','location','person','process','rule','other')),
    label_ru     TEXT,
    status       TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('pending','approved','active','deprecated','rejected')),
    reg_version  INTEGER NOT NULL DEFAULT 1,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS kb.concept_aliases (
    alias_id    BIGSERIAL PRIMARY KEY,
    alias_text  TEXT NOT NULL,
    concept_key TEXT NOT NULL REFERENCES kb.concepts(concept_key) ON UPDATE CASCADE,
    status      TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('pending','active','rejected','deprecated')),
    confidence  NUMERIC(5,4),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (alias_text, concept_key)
);

CREATE INDEX IF NOT EXISTS idx_kb_concept_aliases_trgm
    ON kb.concept_aliases USING GIN (alias_text gin_trgm_ops);

-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS kb.concept_hierarchy (
    parent_concept_key TEXT NOT NULL REFERENCES kb.concepts(concept_key) ON UPDATE CASCADE,
    child_concept_key  TEXT NOT NULL REFERENCES kb.concepts(concept_key) ON UPDATE CASCADE,
    relation_kind      TEXT NOT NULL DEFAULT 'is_a'
        CHECK (relation_kind IN ('is_a')),
    status             TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active','deprecated')),
    source_tier        TEXT NOT NULL DEFAULT 'internal'
        CHECK (source_tier IN ('internal','government','niche_agency','editorial')),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (parent_concept_key, child_concept_key, relation_kind),
    CHECK (parent_concept_key <> child_concept_key)
);

CREATE INDEX IF NOT EXISTS idx_kb_concept_hierarchy_child
    ON kb.concept_hierarchy(child_concept_key);

-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS kb.applicant_profiles (
    profile_key  TEXT PRIMARY KEY,
    profile_type TEXT NOT NULL
        CHECK (profile_type IN ('age','status','family','employment','other')),
    label_ru     TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active','deprecated')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS kb.sources (
    source_key   TEXT PRIMARY KEY,
    source_type  TEXT NOT NULL
        CHECK (source_type IN ('government','vfs','niche_agency','editorial','internal')),
    source_label TEXT NOT NULL,
    base_url     TEXT,
    -- 1=forum/aggregator  2=niche_agency  3=editorial  4=vfs  5=government
    trust_level  INTEGER NOT NULL CHECK (trust_level BETWEEN 1 AND 5),
    status       TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active','deprecated')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS kb.visa_contexts (
    context_key      TEXT PRIMARY KEY,   -- normalize_context_key() из Rust
    country_code     TEXT NOT NULL,
    visa_family      TEXT NOT NULL REFERENCES kb.visa_families(key) ON UPDATE CASCADE,
    visa_subtype     TEXT,
    citizenship_code TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active','deprecated')),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (country_code, visa_family, visa_subtype, citizenship_code)
);

CREATE INDEX IF NOT EXISTS idx_kb_visa_contexts_lookup
    ON kb.visa_contexts(country_code, visa_family, visa_subtype, citizenship_code);

-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS kb.qdrant_points (
    point_id          TEXT PRIMARY KEY,
    entity_type       TEXT NOT NULL,
    entity_key        TEXT NOT NULL,
    collection_name   TEXT NOT NULL
        CHECK (collection_name IN ('content_chunks','kb_canonical','ontology')),
    embedding_model   TEXT NOT NULL,
    embedding_version TEXT NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (entity_type, entity_key, collection_name)
);

CREATE INDEX IF NOT EXISTS idx_kb_qdrant_points_entity
    ON kb.qdrant_points(entity_type, entity_key);

-- ==============================================================================
-- 3. VERIFIED — Canonical Truth
-- ==============================================================================

CREATE TABLE IF NOT EXISTS verified.rule_instances (
    rule_instance_id   TEXT PRIMARY KEY,   -- BLAKE3 hex, вычисляет Rust
    context_key        TEXT NOT NULL REFERENCES kb.visa_contexts(context_key) ON UPDATE CASCADE,
    rule_type_key      TEXT NOT NULL,
    concept_key        TEXT NOT NULL REFERENCES kb.concepts(concept_key) ON UPDATE CASCADE,
    role_type          TEXT NOT NULL
        CHECK (role_type IN (
            'must_provide',
            'must_pay',
            'must_satisfy',
            'allows',
            'forbids',
            'timeline',
            'document_required',
            'eligibility_rule',
            'fee_item',
            'timeline_item',
            'where_to_apply',
            'appointment_rule',
            'form_required',
            'step'
        )),
    params             JSONB NOT NULL DEFAULT '{}'::jsonb,
    status             TEXT NOT NULL
        CHECK (status IN ('pending','verified','disputed','deprecated')),
    source_key         TEXT REFERENCES kb.sources(source_key) ON UPDATE CASCADE,
    confidence         NUMERIC(5,4),
    effective_from     DATE,
    effective_to       DATE,
    evidence_bundle_id UUID,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Keep role_type constraint in sync on existing DBs created before new V5 roles.
ALTER TABLE verified.rule_instances
    DROP CONSTRAINT IF EXISTS rule_instances_role_type_check;
ALTER TABLE verified.rule_instances
    ADD CONSTRAINT rule_instances_role_type_check
        CHECK (role_type IN (
            'must_provide',
            'must_pay',
            'must_satisfy',
            'allows',
            'forbids',
            'timeline',
            'document_required',
            'eligibility_rule',
            'fee_item',
            'timeline_item',
            'where_to_apply',
            'appointment_rule',
            'form_required',
            'step'
        ));

CREATE INDEX IF NOT EXISTS idx_verified_rule_instances_context
    ON verified.rule_instances(context_key);
CREATE INDEX IF NOT EXISTS idx_verified_rule_instances_rule_type
    ON verified.rule_instances(rule_type_key);
CREATE INDEX IF NOT EXISTS idx_verified_rule_instances_status
    ON verified.rule_instances(status);
CREATE INDEX IF NOT EXISTS idx_verified_rule_instances_source
    ON verified.rule_instances(source_key);
CREATE INDEX IF NOT EXISTS idx_verified_rule_instances_params_gin
    ON verified.rule_instances USING GIN (params);

-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS verified.rule_instance_profiles (
    rule_instance_id TEXT NOT NULL
        REFERENCES verified.rule_instances(rule_instance_id) ON DELETE CASCADE,
    profile_key      TEXT NOT NULL
        REFERENCES kb.applicant_profiles(profile_key) ON UPDATE CASCADE,
    applicability    TEXT NOT NULL DEFAULT 'applies'
        CHECK (applicability IN ('applies','excludes','conditional')),
    condition_json   JSONB,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (rule_instance_id, profile_key)
);

CREATE INDEX IF NOT EXISTS idx_verified_rule_instance_profiles_profile
    ON verified.rule_instance_profiles(profile_key);
CREATE INDEX IF NOT EXISTS idx_verified_rule_instance_profiles_condition_gin
    ON verified.rule_instance_profiles USING GIN (condition_json);

-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS verified.rule_exceptions (
    exception_id     TEXT PRIMARY KEY,   -- BLAKE3 hex, вычисляет Rust
    rule_instance_id TEXT NOT NULL
        REFERENCES verified.rule_instances(rule_instance_id) ON DELETE CASCADE,
    exception_key    TEXT NOT NULL,
    profile_key      TEXT REFERENCES kb.applicant_profiles(profile_key) ON UPDATE CASCADE,
    condition_json   JSONB NOT NULL,
    override_kind    TEXT NOT NULL
        CHECK (override_kind IN ('replace_value','waive','add_requirement','remove_requirement')),
    override_payload JSONB NOT NULL,
    status           TEXT NOT NULL DEFAULT 'verified'
        CHECK (status IN ('pending','verified','disputed','deprecated')),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_verified_rule_exceptions_rule_instance
    ON verified.rule_exceptions(rule_instance_id);
CREATE INDEX IF NOT EXISTS idx_verified_rule_exceptions_condition_gin
    ON verified.rule_exceptions USING GIN (condition_json);

-- ==============================================================================
-- 4. SYSTEM — Outbox с Lease-блокировками
-- ==============================================================================

CREATE TABLE IF NOT EXISTS system.sync_outbox (
    event_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    aggregate_type TEXT NOT NULL,           -- 'rule_instance' | 'concept' | 'page_context'
    aggregate_key  TEXT NOT NULL,
    target_system  TEXT NOT NULL
        CHECK (target_system IN ('neo4j','qdrant')),
    event_type     TEXT NOT NULL,           -- 'RuleInstanceUpserted' | 'ConceptUpserted' | ...
    payload_type   TEXT NOT NULL DEFAULT 'alegria.sync.v1.opaque_payload',
    schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    idempotency_key TEXT NOT NULL DEFAULT '',
    payload_bytes  BYTEA NOT NULL DEFAULT '\x',
    payload_hash   TEXT NOT NULL,           -- BLAKE3 через Rust primitives::hash::content_hash_v1
    status         TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','processing','done','failed')),
    retry_count    INTEGER NOT NULL DEFAULT 0,
    next_retry_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error     TEXT,
    worker_id      TEXT,
    locked_at      TIMESTAMPTZ,
    locked_until   TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE system.sync_outbox
    ADD COLUMN IF NOT EXISTS payload_type TEXT NOT NULL DEFAULT 'alegria.sync.v1.opaque_payload';
ALTER TABLE system.sync_outbox
    ADD COLUMN IF NOT EXISTS schema_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE system.sync_outbox
    ADD COLUMN IF NOT EXISTS idempotency_key TEXT NOT NULL DEFAULT '';
ALTER TABLE system.sync_outbox
    ADD COLUMN IF NOT EXISTS payload_bytes BYTEA NOT NULL DEFAULT '\x';
ALTER TABLE system.sync_outbox
    DROP COLUMN IF EXISTS payload_json;

-- Индекс для SELECT ... FOR UPDATE SKIP LOCKED поллинга
CREATE INDEX IF NOT EXISTS idx_system_sync_outbox_pending
    ON system.sync_outbox(status, next_retry_at, created_at)
    WHERE status IN ('pending','processing');

CREATE INDEX IF NOT EXISTS idx_system_sync_outbox_locked_until
    ON system.sync_outbox(status, locked_until);

CREATE INDEX IF NOT EXISTS idx_system_sync_outbox_aggregate
    ON system.sync_outbox(aggregate_type, aggregate_key);

-- Idempotency guard: prevents duplicate logical events on activity retries.
CREATE UNIQUE INDEX IF NOT EXISTS idx_sync_outbox_dedup
    ON system.sync_outbox(aggregate_key, event_type, payload_hash);

CREATE UNIQUE INDEX IF NOT EXISTS idx_sync_outbox_idempotency
    ON system.sync_outbox(aggregate_key, event_type, idempotency_key)
    WHERE idempotency_key <> '';

CREATE OR REPLACE FUNCTION system.notify_sync_outbox() RETURNS trigger AS $$
BEGIN
    -- channel: 'sync_outbox_channel', payload: '<target_system>:<aggregate_key>'
    PERFORM pg_notify('sync_outbox_channel', NEW.target_system || ':' || NEW.aggregate_key);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_sync_outbox_notify ON system.sync_outbox;
CREATE TRIGGER trg_sync_outbox_notify
    AFTER INSERT ON system.sync_outbox
    FOR EACH ROW EXECUTE FUNCTION system.notify_sync_outbox();

-- ==============================================================================
-- 5. SITE — Mapping Layer
-- ==============================================================================

CREATE TABLE IF NOT EXISTS site.page_context_map (
    url_path       TEXT PRIMARY KEY,
    context_key    TEXT NOT NULL
        REFERENCES kb.visa_contexts(context_key) ON UPDATE CASCADE,
    mapping_status TEXT NOT NULL DEFAULT 'active'
        CHECK (mapping_status IN ('active','deprecated')),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ==============================================================================
-- 6. RAW — Source of Truth для контента
-- ==============================================================================

CREATE TABLE IF NOT EXISTS raw.pages (
    id              BIGSERIAL PRIMARY KEY,
    url             TEXT NOT NULL,
    domain          TEXT NOT NULL,
    dtype           TEXT NOT NULL,
    status_code     SMALLINT,
    content_type    TEXT DEFAULT 'html',    -- 'html' | 'pdf'
    snapshot_at     TIMESTAMPTZ DEFAULT now(),
    crawled_at      TIMESTAMPTZ,
    title           TEXT,
    meta_desc       TEXT,
    canonical       TEXT,
    word_count      INTEGER,
    content         JSONB,                  -- структурированный: markdown, headings, links
    raw_html        TEXT,                   -- полный HTML (TOASTed)
    raw_html_bytes  INTEGER,
    content_hash    TEXT,                   -- blake3(raw_html), hex64
    processed       BOOLEAN DEFAULT false
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_raw_pages_url_date ON raw.pages (url, CAST(snapshot_at AT TIME ZONE 'UTC' AS DATE));

COMMENT ON COLUMN raw.pages.content_hash IS
    'blake3(raw_html), hex64. Source: primitives::hash::content_hash_v1';

CREATE TABLE IF NOT EXISTS raw.sections (
    id              BIGSERIAL PRIMARY KEY,
    page_id         BIGINT NOT NULL REFERENCES raw.pages(id) ON DELETE CASCADE,
    heading_path    TEXT,
    heading_level   SMALLINT,
    section_order   INTEGER,
    section_type    TEXT,
    content_md      TEXT NOT NULL,
    content_hash    TEXT,
    char_count      INTEGER GENERATED ALWAYS AS (length(content_md)) STORED
);

CREATE INDEX IF NOT EXISTS idx_raw_pages_domain     ON raw.pages(domain);
CREATE INDEX IF NOT EXISTS idx_raw_sections_page    ON raw.sections(page_id);
CREATE INDEX IF NOT EXISTS idx_raw_sections_type    ON raw.sections(section_type);

-- ==============================================================================
-- 7. SERP — SERP Runs & Crawl Queue
-- ==============================================================================

CREATE TABLE IF NOT EXISTS serp.raw_snapshots (
    run_id      TEXT NOT NULL,
    job_id      TEXT NOT NULL,
    query       TEXT NOT NULL,
    recorded_at TIMESTAMPTZ,
    raw_result  JSONB NOT NULL,
    PRIMARY KEY (run_id, job_id)
);

CREATE TABLE IF NOT EXISTS serp.source_resolutions (
    source_key       TEXT PRIMARY KEY,
    source_type      TEXT NOT NULL,   -- 'source_resolved' | 'rendered_chip'
    run_id           TEXT NOT NULL,
    job_id           TEXT NOT NULL,
    query            TEXT,
    chunk_index      INT NOT NULL,
    uri_redirect     TEXT NOT NULL,
    uri_resolved     TEXT,
    http_status      INT,
    redirect_hops    INT,
    resolved_at      TIMESTAMPTZ,
    latency_ms       INT,
    attempts_used    INT,
    resolve_error    TEXT,
    resolver_version TEXT
);

CREATE INDEX IF NOT EXISTS idx_serp_source_resolutions_run_job
    ON serp.source_resolutions(run_id, job_id);
CREATE INDEX IF NOT EXISTS idx_serp_source_resolutions_source_type
    ON serp.source_resolutions(source_type);
CREATE INDEX IF NOT EXISTS idx_serp_source_resolutions_uri_resolved
    ON serp.source_resolutions(uri_resolved);

CREATE TABLE IF NOT EXISTS serp.gemini_top10 (
    run_id      TEXT NOT NULL,
    job_id      TEXT NOT NULL,
    rank        INT NOT NULL,
    title       TEXT,
    url         TEXT NOT NULL,
    url_norm    TEXT,
    domain_norm TEXT,
    also_in_sources BOOLEAN DEFAULT false,
    PRIMARY KEY (run_id, job_id, rank)
);

CREATE INDEX IF NOT EXISTS idx_serp_gemini_top10_url_norm
    ON serp.gemini_top10(url_norm);
CREATE INDEX IF NOT EXISTS idx_serp_gemini_top10_domain_norm
    ON serp.gemini_top10(domain_norm);

CREATE TABLE IF NOT EXISTS serp.gemini_sources (
    source_key        TEXT PRIMARY KEY,
    source_type       TEXT NOT NULL,
    run_id            TEXT NOT NULL,
    job_id            TEXT NOT NULL,
    chunk_index       INT NOT NULL,
    uri_redirect      TEXT NOT NULL,
    uri_resolved      TEXT,
    domain_source_api TEXT,
    domain_resolved   TEXT,
    title_source      TEXT,
    resolve_error     TEXT
);

CREATE INDEX IF NOT EXISTS idx_serp_gemini_sources_run_job
    ON serp.gemini_sources(run_id, job_id);
CREATE INDEX IF NOT EXISTS idx_serp_gemini_sources_domain_resolved
    ON serp.gemini_sources(domain_resolved);

CREATE TABLE IF NOT EXISTS serp.gemini_queries (
    run_id   TEXT NOT NULL,
    job_id   TEXT NOT NULL,
    position INT NOT NULL,
    query    TEXT NOT NULL,
    PRIMARY KEY (run_id, job_id, position)
);

CREATE INDEX IF NOT EXISTS idx_serp_gemini_queries_query
    ON serp.gemini_queries(query);

CREATE TABLE IF NOT EXISTS serp.gemini_supports (
    run_id        TEXT NOT NULL,
    job_id        TEXT NOT NULL,
    support_idx   INT NOT NULL,
    text_fragment TEXT NOT NULL,
    confidence    NUMERIC,
    is_model_json BOOLEAN DEFAULT false,
    chunk_indices INT[],
    PRIMARY KEY (run_id, job_id, support_idx)
);

CREATE TABLE IF NOT EXISTS serp.crawl_queue (
    url               TEXT NOT NULL,
    url_norm          TEXT NOT NULL UNIQUE,
    source_type       TEXT NOT NULL,   -- 'top10' | 'source_resolved'
    dtype             TEXT,            -- 'government' | 'niche_agency' | 'forum' | ...
    first_seen_run_id TEXT NOT NULL,
    first_seen_job_id TEXT NOT NULL,
    first_seen_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    status            TEXT NOT NULL DEFAULT 'pending',
    http_status       INT,
    notes             TEXT
);

CREATE INDEX IF NOT EXISTS idx_serp_crawl_queue_status
    ON serp.crawl_queue(status);

-- ==============================================================================
-- 8. PIPELINE — Operational tables
-- ==============================================================================

CREATE TABLE IF NOT EXISTS pipeline.hitl_tasks (
    id          BIGSERIAL PRIMARY KEY,
    task_type   TEXT NOT NULL,
    priority    INTEGER NOT NULL DEFAULT 2
        CHECK (priority BETWEEN 1 AND 3),   -- 1=blocking(4h) 2=important(8h) 3=background(24h)
    context     JSONB NOT NULL DEFAULT '{}'::jsonb,
    status      TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','in_review','resolved','dismissed')),
    deadline    TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pipeline_hitl_tasks_pending
    ON pipeline.hitl_tasks(status, priority, deadline)
    WHERE status IN ('pending','in_review');

-- ----------------------------------------------------------------------------
-- Temporal durable execution: large payload store (keeps Temporal history lean)
-- Activities store large blobs here; pass only run_id through Temporal.
-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS pipeline.execution_runs (
    run_id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_run_id     TEXT        NOT NULL,   -- Temporal workflow run ID
    workflow_type       TEXT        NOT NULL    -- 'extract_facts' | 'generate_content'
                        CHECK (workflow_type IN ('extract_facts','generate_content')),
    context_key         TEXT        NOT NULL,
    status              TEXT        NOT NULL    DEFAULT 'created'
                        CHECK (status IN (
                            'created','normalizing','extracting','verifying',
                            'persisting','generating','validating','pending_hitl','done','failed'
                        )),
    created_at          TIMESTAMPTZ NOT NULL    DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL    DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_execution_runs_workflow
    ON pipeline.execution_runs(workflow_run_id);
CREATE INDEX IF NOT EXISTS idx_execution_runs_context_status
    ON pipeline.execution_runs(context_key, status);
CREATE INDEX IF NOT EXISTS idx_execution_runs_created
    ON pipeline.execution_runs(created_at DESC)
    WHERE status NOT IN ('done','failed');

CREATE TABLE IF NOT EXISTS pipeline.execution_run_blobs (
    execution_run_blob_id BIGSERIAL PRIMARY KEY,
    run_id                UUID        NOT NULL REFERENCES pipeline.execution_runs(run_id) ON DELETE CASCADE,
    field_name            TEXT        NOT NULL CHECK (field_name IN (
                           'input_payload',
                           'extracted_payload',
                           'generation_result',
                           'verify_report',
                           'persist_report',
                           'errors'
                       )),
    payload_type          TEXT        NOT NULL,
    schema_version        INTEGER     NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    payload_bytes         BYTEA       NOT NULL DEFAULT '\x',
    payload_hash          TEXT        NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, field_name)
);

CREATE INDEX IF NOT EXISTS idx_execution_run_blobs_run
    ON pipeline.execution_run_blobs(run_id, field_name);

-- ----------------------------------------------------------------------------
-- Step journal for runtime idempotency and replay-safe execution accounting.
-- idempotency_key must be deterministic (BLAKE3 over canonical step input).
-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS pipeline.step_executions (
    step_execution_id BIGSERIAL PRIMARY KEY,
    run_id            UUID        NOT NULL REFERENCES pipeline.execution_runs(run_id) ON DELETE CASCADE,
    step_name         TEXT        NOT NULL,
    schema_version    INTEGER     NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    input_hash        TEXT        NOT NULL,
    output_hash       TEXT,
    idempotency_key   TEXT        NOT NULL,
    status            TEXT        NOT NULL DEFAULT 'running'
                    CHECK (status IN ('running','done','failed','pending_hitl','poisoned','dead_letter')),
    error_class       TEXT,
    error_message     TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (step_name, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_pipeline_step_executions_run
    ON pipeline.step_executions(run_id, status, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_pipeline_step_executions_idempotency
    ON pipeline.step_executions(idempotency_key);

CREATE TABLE IF NOT EXISTS pipeline.step_attempts (
    step_attempt_id    BIGSERIAL PRIMARY KEY,
    step_execution_id  BIGINT      NOT NULL REFERENCES pipeline.step_executions(step_execution_id) ON DELETE CASCADE,
    attempt_no         INTEGER     NOT NULL CHECK (attempt_no > 0),
    status             TEXT        NOT NULL DEFAULT 'running'
                     CHECK (status IN ('running','done','failed','pending_hitl','poisoned','dead_letter')),
    error_class        TEXT,
    error_message      TEXT,
    started_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at        TIMESTAMPTZ,
    UNIQUE(step_execution_id, attempt_no)
);

CREATE INDEX IF NOT EXISTS idx_pipeline_step_attempts_status
    ON pipeline.step_attempts(step_execution_id, status, started_at DESC);

CREATE TABLE IF NOT EXISTS pipeline.step_payload_blobs (
    payload_blob_id    BIGSERIAL PRIMARY KEY,
    run_id             UUID        NOT NULL REFERENCES pipeline.execution_runs(run_id) ON DELETE CASCADE,
    step_name          TEXT        NOT NULL,
    idempotency_key    TEXT        NOT NULL,
    payload_kind       TEXT        NOT NULL CHECK (payload_kind IN ('input','output','error','diagnostic')),
    payload_type       TEXT        NOT NULL,
    schema_version     INTEGER     NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    payload_bytes      BYTEA       NOT NULL DEFAULT '\x',
    payload_hash       TEXT        NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(idempotency_key, payload_kind, payload_hash)
);

CREATE INDEX IF NOT EXISTS idx_pipeline_step_payload_blobs_run
    ON pipeline.step_payload_blobs(run_id, step_name, created_at DESC);

CREATE TABLE IF NOT EXISTS pipeline.hitl_decisions (
    decision_id        BIGSERIAL PRIMARY KEY,
    run_id             UUID        NOT NULL REFERENCES pipeline.execution_runs(run_id) ON DELETE CASCADE,
    step_name          TEXT        NOT NULL,
    task_id            BIGINT      REFERENCES pipeline.hitl_tasks(id) ON DELETE SET NULL,
    decision_status    TEXT        NOT NULL,
    decision_type      TEXT        NOT NULL,
    schema_version     INTEGER     NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    decision_bytes     BYTEA       NOT NULL DEFAULT '\x',
    decision_hash      TEXT        NOT NULL,
    resolved_by        TEXT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pipeline_hitl_decisions_run
    ON pipeline.hitl_decisions(run_id, step_name, created_at DESC);

-- ----------------------------------------------------------------------------
-- Dead-letter queue for non-retryable or exhausted pipeline failures.
-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS system.dead_letter_queue (
    dlq_id            BIGSERIAL PRIMARY KEY,
    run_id            UUID        REFERENCES pipeline.execution_runs(run_id) ON DELETE SET NULL,
    step_name         TEXT        NOT NULL,
    workflow_id       TEXT        NOT NULL,
    error_class       TEXT        NOT NULL,
    retryable         BOOLEAN     NOT NULL DEFAULT false,
    payload_type      TEXT        NOT NULL,
    schema_version    INTEGER     NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    payload_bytes     BYTEA       NOT NULL DEFAULT '\x',
    payload_hash      TEXT        NOT NULL DEFAULT '',
    input_hash        TEXT        NOT NULL,
    idempotency_key   TEXT        NOT NULL,
    build_id          TEXT        NOT NULL,
    error_message     TEXT        NOT NULL,
    resolution_status TEXT        NOT NULL DEFAULT 'open' CHECK (resolution_status IN ('open','acknowledged','resolved')),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at       TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_dead_letter_queue_open
    ON system.dead_letter_queue(resolution_status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_dead_letter_queue_run
    ON system.dead_letter_queue(run_id, step_name, created_at DESC);

CREATE TABLE IF NOT EXISTS pipeline.reconcile_runs (
    reconcile_run_id   BIGSERIAL PRIMARY KEY,
    started_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at        TIMESTAMPTZ,
    status             TEXT        NOT NULL DEFAULT 'running'
                     CHECK (status IN ('running','done','failed')),
    summary_type       TEXT        NOT NULL,
    schema_version     INTEGER     NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    summary_bytes      BYTEA       NOT NULL DEFAULT '\x',
    summary_hash       TEXT        NOT NULL DEFAULT '',
    error_message      TEXT
);

CREATE TABLE IF NOT EXISTS pipeline.reconcile_actions (
    reconcile_action_id BIGSERIAL PRIMARY KEY,
    reconcile_run_id    BIGINT      NOT NULL REFERENCES pipeline.reconcile_runs(reconcile_run_id) ON DELETE CASCADE,
    action_type         TEXT        NOT NULL,
    target_system       TEXT,
    target_key          TEXT,
    details_type        TEXT        NOT NULL,
    schema_version      INTEGER     NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    details_bytes       BYTEA       NOT NULL DEFAULT '\x',
    details_hash        TEXT        NOT NULL DEFAULT '',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pipeline_reconcile_actions_run
    ON pipeline.reconcile_actions(reconcile_run_id, created_at DESC);

ALTER TABLE pipeline.step_payload_blobs
    ADD COLUMN IF NOT EXISTS payload_type TEXT NOT NULL DEFAULT '';
ALTER TABLE pipeline.step_payload_blobs
    ADD COLUMN IF NOT EXISTS payload_bytes BYTEA NOT NULL DEFAULT '\x';

ALTER TABLE pipeline.hitl_decisions
    ADD COLUMN IF NOT EXISTS decision_type TEXT NOT NULL DEFAULT '';
ALTER TABLE pipeline.hitl_decisions
    ADD COLUMN IF NOT EXISTS schema_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE pipeline.hitl_decisions
    ADD COLUMN IF NOT EXISTS decision_bytes BYTEA NOT NULL DEFAULT '\x';

ALTER TABLE system.dead_letter_queue
    ADD COLUMN IF NOT EXISTS payload_type TEXT NOT NULL DEFAULT '';
ALTER TABLE system.dead_letter_queue
    ADD COLUMN IF NOT EXISTS schema_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE system.dead_letter_queue
    ADD COLUMN IF NOT EXISTS payload_bytes BYTEA NOT NULL DEFAULT '\x';
ALTER TABLE system.dead_letter_queue
    ADD COLUMN IF NOT EXISTS payload_hash TEXT NOT NULL DEFAULT '';

ALTER TABLE pipeline.reconcile_runs
    ADD COLUMN IF NOT EXISTS summary_type TEXT NOT NULL DEFAULT '';
ALTER TABLE pipeline.reconcile_runs
    ADD COLUMN IF NOT EXISTS schema_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE pipeline.reconcile_runs
    ADD COLUMN IF NOT EXISTS summary_bytes BYTEA NOT NULL DEFAULT '\x';
ALTER TABLE pipeline.reconcile_runs
    ADD COLUMN IF NOT EXISTS summary_hash TEXT NOT NULL DEFAULT '';

ALTER TABLE pipeline.reconcile_actions
    ADD COLUMN IF NOT EXISTS details_type TEXT NOT NULL DEFAULT '';
ALTER TABLE pipeline.reconcile_actions
    ADD COLUMN IF NOT EXISTS schema_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE pipeline.reconcile_actions
    ADD COLUMN IF NOT EXISTS details_bytes BYTEA NOT NULL DEFAULT '\x';
ALTER TABLE pipeline.reconcile_actions
    ADD COLUMN IF NOT EXISTS details_hash TEXT NOT NULL DEFAULT '';

ALTER TABLE pipeline.execution_runs DROP COLUMN IF EXISTS input_payload;
ALTER TABLE pipeline.execution_runs DROP COLUMN IF EXISTS extracted_payload;
ALTER TABLE pipeline.execution_runs DROP COLUMN IF EXISTS generation_result;
ALTER TABLE pipeline.execution_runs DROP COLUMN IF EXISTS verify_report;
ALTER TABLE pipeline.execution_runs DROP COLUMN IF EXISTS persist_report;
ALTER TABLE pipeline.execution_runs DROP COLUMN IF EXISTS errors;
ALTER TABLE pipeline.step_executions DROP COLUMN IF EXISTS result_payload;
ALTER TABLE pipeline.execution_run_blobs ALTER COLUMN payload_type DROP DEFAULT;
ALTER TABLE pipeline.step_payload_blobs ALTER COLUMN payload_type DROP DEFAULT;
ALTER TABLE pipeline.hitl_decisions ALTER COLUMN decision_type DROP DEFAULT;
ALTER TABLE system.dead_letter_queue ALTER COLUMN payload_type DROP DEFAULT;
ALTER TABLE pipeline.reconcile_runs ALTER COLUMN summary_type DROP DEFAULT;
ALTER TABLE pipeline.reconcile_actions ALTER COLUMN details_type DROP DEFAULT;
ALTER TABLE pipeline.step_payload_blobs DROP COLUMN IF EXISTS payload_json;
ALTER TABLE pipeline.hitl_decisions DROP COLUMN IF EXISTS decision_payload;
ALTER TABLE system.dead_letter_queue DROP COLUMN IF EXISTS payload_envelope;
ALTER TABLE pipeline.reconcile_runs DROP COLUMN IF EXISTS summary_json;
ALTER TABLE pipeline.reconcile_actions DROP COLUMN IF EXISTS details_json;

COMMIT;
