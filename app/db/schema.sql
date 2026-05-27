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
  ('work',         'Рабочая'),
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
        CHECK (source_type IN ('government','vfs','niche_agency','editorial','internal','forum','low_trust')),
    authority_class TEXT NOT NULL DEFAULT 'unknown'
        CHECK (authority_class IN ('primary_authority','delegated_authority','official_publisher','editorial','agency','forum','unknown')),
    independence_group_key TEXT NOT NULL DEFAULT '',
    source_label TEXT NOT NULL,
    base_url     TEXT,
    -- 1=forum/aggregator  2=niche_agency  3=editorial  4=vfs  5=government
    trust_level  INTEGER NOT NULL CHECK (trust_level BETWEEN 1 AND 5),
    freshness_ttl_days INTEGER NOT NULL DEFAULT 30 CHECK (freshness_ttl_days >= 0),
    override_eligible BOOLEAN NOT NULL DEFAULT false,
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
        CHECK (collection_name IN (
            'content_chunks',
            'kb_canonical',
            'ontology',
            'raw_chunks_4',
            'raw_chunks_ctx',
            'kb_canonical_4',
            'verified_rules_4',
            'editorial_topics_4',
            'seo_keyword_clusters_4',
            'whole_page_advisory_prototypes',
            'seo_keyword_clusters',
            'seo_page_blueprints',
            'seo_serp_patterns',
            'seo_link_targets',
            'seo_draft_support_sections'
        )),
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
CREATE INDEX IF NOT EXISTS idx_verified_rule_instances_admissibility
    ON verified.rule_instances(publish_admissibility, status);

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
    run_id         TEXT NOT NULL DEFAULT '',
    aggregate_type TEXT NOT NULL,           -- 'rule_instance' | 'concept' | 'page_context'
    aggregate_key  TEXT NOT NULL,
    target_system  TEXT NOT NULL
        CHECK (target_system IN ('neo4j','qdrant','cms')),
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
    DROP CONSTRAINT IF EXISTS sync_outbox_target_system_check;
ALTER TABLE system.sync_outbox
    ADD CONSTRAINT sync_outbox_target_system_check
        CHECK (target_system IN ('neo4j','qdrant','cms'));

ALTER TABLE system.sync_outbox
    ADD COLUMN IF NOT EXISTS run_id TEXT NOT NULL DEFAULT '';
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

CREATE INDEX IF NOT EXISTS idx_system_sync_outbox_target_run_status
    ON system.sync_outbox(target_system, run_id, status);

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

-- ----------------------------------------------------------------------------
-- SEO operating model: canonical site-planning and draft artifacts.
-- These tables are source-of-record; Neo4j, Qdrant, and CMS records are projections.
-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS site.registry_page_types (
    page_type_key TEXT PRIMARY KEY,
    label         TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'active'
                  CHECK (status IN ('active','deprecated')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS site.registry_intent_types (
    intent_type_key TEXT PRIMARY KEY,
    label           TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'active'
                    CHECK (status IN ('active','deprecated')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS site.registry_anchor_strategies (
    anchor_strategy_key TEXT PRIMARY KEY,
    label               TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active','deprecated')),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS site.registry_section_templates (
    section_template_key TEXT PRIMARY KEY,
    label                TEXT NOT NULL,
    status               TEXT NOT NULL DEFAULT 'active'
                         CHECK (status IN ('active','deprecated')),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS site.registry_cta_patterns (
    cta_pattern_key TEXT PRIMARY KEY,
    label           TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'active'
                    CHECK (status IN ('active','deprecated')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO site.registry_page_types (page_type_key, label) VALUES
  ('country_hub_page', 'Country hub'),
  ('hub_page', 'Topic hub'),
  ('detail_page', 'Detail guide'),
  ('requirement_page', 'Requirements guide'),
  ('fee_page', 'Fee guide'),
  ('timeline_page', 'Timeline guide'),
  ('faq_page', 'FAQ guide'),
  ('checklist_page', 'Checklist guide'),
  ('troubleshooting_page', 'Troubleshooting guide'),
  ('comparison_page', 'Comparison guide'),
  ('supporting_editorial', 'Supporting editorial guide')
ON CONFLICT (page_type_key) DO NOTHING;

INSERT INTO site.registry_intent_types (intent_type_key, label) VALUES
  ('informational', 'Informational'),
  ('transactional', 'Transactional'),
  ('navigational', 'Navigational'),
  ('investigative', 'Investigative'),
  ('comparison', 'Comparison'),
  ('troubleshooting', 'Troubleshooting'),
  ('commercial', 'Commercial')
ON CONFLICT (intent_type_key) DO NOTHING;

INSERT INTO site.registry_anchor_strategies (anchor_strategy_key, label) VALUES
  ('descriptive', 'Descriptive'),
  ('hub_contextual', 'Hub contextual'),
  ('child_contextual', 'Child contextual'),
  ('journey_next_step', 'Journey next step'),
  ('faq_source_owner', 'FAQ source owner')
ON CONFLICT (anchor_strategy_key) DO NOTHING;

INSERT INTO site.registry_section_templates (section_template_key, label) VALUES
  ('overview', 'Overview'),
  ('who_fits', 'Who fits'),
  ('documents', 'Documents'),
  ('process', 'Process'),
  ('fees', 'Fees'),
  ('timing', 'Timing'),
  ('where_to_apply', 'Where to apply'),
  ('faq', 'FAQ'),
  ('related_pages', 'Related pages'),
  ('cta_disclaimer', 'CTA and disclaimer')
ON CONFLICT (section_template_key) DO NOTHING;

INSERT INTO site.registry_cta_patterns (cta_pattern_key, label) VALUES
  ('human_review_required', 'Human review required'),
  ('consultation_request', 'Consultation request'),
  ('document_checklist_request', 'Document checklist request')
ON CONFLICT (cta_pattern_key) DO NOTHING;

CREATE TABLE IF NOT EXISTS site.keyword_clusters (
    cluster_key       TEXT PRIMARY KEY,
    scope_signature   TEXT NOT NULL,
    market            TEXT NOT NULL,
    locale            TEXT NOT NULL,
    country_code      TEXT,
    visa_type         TEXT,
    applicant_profile TEXT,
    seed_keyword      TEXT NOT NULL,
    dominant_intent   TEXT NOT NULL,
    cluster_version   INTEGER NOT NULL DEFAULT 1 CHECK (cluster_version > 0),
    reason_payload    JSONB NOT NULL DEFAULT '{}'::jsonb,
    reason_version    TEXT NOT NULL DEFAULT 'graph_planning@1',
    derivation_version TEXT NOT NULL DEFAULT 'seo_cluster@1',
    status            TEXT NOT NULL DEFAULT 'candidate'
                      CHECK (status IN ('candidate','accepted','rejected','deprecated')),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (market, locale, country_code, visa_type, applicant_profile, seed_keyword, cluster_version)
);

CREATE INDEX IF NOT EXISTS idx_site_keyword_clusters_scope
    ON site.keyword_clusters(scope_signature);
CREATE INDEX IF NOT EXISTS idx_site_keyword_clusters_status
    ON site.keyword_clusters(status, dominant_intent);

CREATE TABLE IF NOT EXISTS site.search_features (
    search_feature_key TEXT PRIMARY KEY,
    scope_signature    TEXT NOT NULL,
    feature_type       TEXT NOT NULL,
    label              TEXT NOT NULL,
    evidence_ref       TEXT,
    derivation_version TEXT NOT NULL DEFAULT 'seo_search_feature@1',
    status             TEXT NOT NULL DEFAULT 'active'
                       CHECK (status IN ('active','deprecated')),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_site_search_features_scope
    ON site.search_features(scope_signature);

CREATE TABLE IF NOT EXISTS site.section_templates (
    section_template_key TEXT PRIMARY KEY,
    page_type_key        TEXT NOT NULL,
    dominant_intent      TEXT NOT NULL,
    section_role         TEXT NOT NULL,
    template_version     INTEGER NOT NULL DEFAULT 1 CHECK (template_version > 0),
    template_body        TEXT NOT NULL DEFAULT '',
    derivation_version   TEXT NOT NULL DEFAULT 'seo_section_template@1',
    status               TEXT NOT NULL DEFAULT 'active'
                         CHECK (status IN ('active','deprecated')),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_site_section_templates_lookup
    ON site.section_templates(page_type_key, dominant_intent, status);

CREATE TABLE IF NOT EXISTS site.page_blueprints (
    blueprint_key      TEXT PRIMARY KEY,
    page_type_key      TEXT NOT NULL,
    dominant_intent    TEXT NOT NULL,
    scope_class        TEXT NOT NULL DEFAULT 'visa_scope',
    blueprint_version  INTEGER NOT NULL DEFAULT 1 CHECK (blueprint_version > 0),
    title_pattern      TEXT NOT NULL DEFAULT '',
    section_plan       JSONB NOT NULL DEFAULT '[]'::jsonb,
    derivation_version TEXT NOT NULL DEFAULT 'seo_blueprint@1',
    status             TEXT NOT NULL DEFAULT 'active'
                       CHECK (status IN ('active','deprecated')),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (page_type_key, dominant_intent, scope_class, blueprint_version)
);

CREATE INDEX IF NOT EXISTS idx_site_page_blueprints_lookup
    ON site.page_blueprints(page_type_key, dominant_intent, status);

CREATE TABLE IF NOT EXISTS site.page_nodes (
    page_node_key       TEXT PRIMARY KEY,
    scope_signature     TEXT NOT NULL,
    keyword_cluster_key TEXT REFERENCES site.keyword_clusters(cluster_key) ON UPDATE CASCADE,
    blueprint_key       TEXT REFERENCES site.page_blueprints(blueprint_key) ON UPDATE CASCADE,
    page_type_key       TEXT NOT NULL,
    dominant_intent     TEXT NOT NULL,
    canonical_slug      TEXT NOT NULL,
    canonical_url_path  TEXT NOT NULL,
    parent_page_node_key TEXT REFERENCES site.page_nodes(page_node_key) ON DELETE SET NULL,
    hierarchy_depth     INTEGER NOT NULL DEFAULT 0 CHECK (hierarchy_depth >= 0),
    menu_group          TEXT NOT NULL DEFAULT '',
    breadcrumb_policy   TEXT NOT NULL DEFAULT 'path_segments',
    canonical_url_family TEXT NOT NULL DEFAULT 'visa_scope',
    lifecycle_state     TEXT NOT NULL DEFAULT 'candidate'
                        CHECK (lifecycle_state IN (
                            'candidate','planned','draft_ready','review_required',
                            'approved','published','stale','needs_rebuild',
                            'deprecated','blocked'
                        )),
    derivation_version  TEXT NOT NULL DEFAULT 'seo_page_node@1',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (scope_signature, dominant_intent, canonical_url_path)
);

CREATE INDEX IF NOT EXISTS idx_site_page_nodes_scope
    ON site.page_nodes(scope_signature);
CREATE INDEX IF NOT EXISTS idx_site_page_nodes_lifecycle
    ON site.page_nodes(lifecycle_state, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_site_page_nodes_parent
    ON site.page_nodes(parent_page_node_key, hierarchy_depth);

CREATE TABLE IF NOT EXISTS site.site_scopes (
    scope_signature    TEXT PRIMARY KEY,
    market             TEXT NOT NULL DEFAULT '',
    locale             TEXT NOT NULL DEFAULT '',
    country_code       TEXT,
    visa_type          TEXT,
    applicant_profile  TEXT,
    context_key        TEXT REFERENCES kb.visa_contexts(context_key) ON UPDATE CASCADE,
    page_count         INTEGER NOT NULL DEFAULT 0 CHECK (page_count >= 0),
    status             TEXT NOT NULL DEFAULT 'active'
                       CHECK (status IN ('active','stale','deprecated')),
    derivation_version TEXT NOT NULL DEFAULT 'site_scope_from_page_nodes@1',
    last_reconciled_at TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_site_site_scopes_lookup
    ON site.site_scopes(market, locale, country_code, visa_type, status);

CREATE TABLE IF NOT EXISTS site.silo_groups (
    silo_group_key      TEXT PRIMARY KEY,
    parent_silo_group_key TEXT REFERENCES site.silo_groups(silo_group_key) ON DELETE SET NULL,
    scope_signature    TEXT REFERENCES site.site_scopes(scope_signature) ON DELETE SET NULL,
    group_type         TEXT NOT NULL
                       CHECK (group_type IN ('global','locale','country','visa_type','directory','custom')),
    label              TEXT NOT NULL,
    canonical_url_path TEXT NOT NULL DEFAULT '',
    sort_order         INTEGER NOT NULL DEFAULT 1000,
    status             TEXT NOT NULL DEFAULT 'active'
                       CHECK (status IN ('active','stale','deprecated')),
    derivation_version TEXT NOT NULL DEFAULT 'silo_group_reconcile@1',
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (group_type, label, canonical_url_path)
);

CREATE INDEX IF NOT EXISTS idx_site_silo_groups_scope
    ON site.silo_groups(scope_signature, status, sort_order);

CREATE TABLE IF NOT EXISTS site.navigation_trees (
    navigation_tree_key TEXT PRIMARY KEY,
    market              TEXT NOT NULL DEFAULT '',
    locale              TEXT NOT NULL DEFAULT '',
    tree_version        INTEGER NOT NULL DEFAULT 1 CHECK (tree_version > 0),
    policy_version      TEXT NOT NULL DEFAULT 'navigation_policy@1',
    status              TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active','superseded','deprecated')),
    generated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (market, locale, tree_version, policy_version)
);

CREATE INDEX IF NOT EXISTS idx_site_navigation_trees_lookup
    ON site.navigation_trees(market, locale, status, generated_at DESC);

CREATE TABLE IF NOT EXISTS site.navigation_items (
    navigation_item_key  TEXT PRIMARY KEY,
    navigation_tree_key  TEXT NOT NULL REFERENCES site.navigation_trees(navigation_tree_key) ON DELETE CASCADE,
    parent_item_key      TEXT,
    silo_group_key       TEXT REFERENCES site.silo_groups(silo_group_key) ON DELETE SET NULL,
    page_node_key        TEXT REFERENCES site.page_nodes(page_node_key) ON DELETE SET NULL,
    scope_signature      TEXT,
    item_type            TEXT NOT NULL
                         CHECK (item_type IN ('page','silo','directory','external')),
    label                TEXT NOT NULL,
    url_path             TEXT NOT NULL DEFAULT '',
    hierarchy_depth      INTEGER NOT NULL DEFAULT 0 CHECK (hierarchy_depth >= 0),
    sort_order           INTEGER NOT NULL DEFAULT 1000,
    status               TEXT NOT NULL DEFAULT 'active'
                         CHECK (status IN ('active','hidden','stale','deprecated')),
    derivation_version   TEXT NOT NULL DEFAULT 'navigation_item_from_page_nodes@1',
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (navigation_tree_key, item_type, page_node_key),
    UNIQUE (navigation_tree_key, url_path, label)
);

CREATE INDEX IF NOT EXISTS idx_site_navigation_items_tree
    ON site.navigation_items(navigation_tree_key, parent_item_key, sort_order);
CREATE INDEX IF NOT EXISTS idx_site_navigation_items_page
    ON site.navigation_items(page_node_key, status);

CREATE TABLE IF NOT EXISTS site.global_rebuild_plan (
    rebuild_plan_key    TEXT PRIMARY KEY,
    scope_signature     TEXT REFERENCES site.site_scopes(scope_signature) ON DELETE SET NULL,
    affected_page_node_key TEXT REFERENCES site.page_nodes(page_node_key) ON DELETE SET NULL,
    trigger_type        TEXT NOT NULL,
    priority            INTEGER NOT NULL DEFAULT 2 CHECK (priority BETWEEN 1 AND 3),
    reason_payload      JSONB NOT NULL DEFAULT '{}'::jsonb,
    status              TEXT NOT NULL DEFAULT 'queued'
                        CHECK (status IN ('queued','running','blocked','done','cancelled','failed')),
    derivation_version  TEXT NOT NULL DEFAULT 'global_rebuild_plan@1',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_site_global_rebuild_plan_status
    ON site.global_rebuild_plan(status, priority, created_at);
CREATE INDEX IF NOT EXISTS idx_site_global_rebuild_plan_page
    ON site.global_rebuild_plan(affected_page_node_key, status);

CREATE TABLE IF NOT EXISTS site.content_gaps (
    content_gap_key    TEXT PRIMARY KEY,
    scope_signature    TEXT NOT NULL,
    page_node_key      TEXT REFERENCES site.page_nodes(page_node_key) ON UPDATE CASCADE,
    missing_topic      TEXT NOT NULL,
    severity           TEXT NOT NULL DEFAULT 'medium'
                       CHECK (severity IN ('low','medium','high','blocking')),
    evidence_ref       TEXT,
    reason_payload     JSONB NOT NULL DEFAULT '{}'::jsonb,
    reason_version     TEXT NOT NULL DEFAULT 'graph_planning@1',
    detector_version   TEXT NOT NULL DEFAULT 'seo_content_gap@1',
    status             TEXT NOT NULL DEFAULT 'open'
                       CHECK (status IN ('open','accepted','resolved','rejected','deprecated')),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (scope_signature, missing_topic, detector_version)
);

CREATE INDEX IF NOT EXISTS idx_site_content_gaps_scope
    ON site.content_gaps(scope_signature, status);

CREATE TABLE IF NOT EXISTS site.link_recommendations (
    link_recommendation_key TEXT PRIMARY KEY,
    scope_signature         TEXT NOT NULL,
    source_page_key         TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    target_page_key         TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    link_role               TEXT NOT NULL DEFAULT 'contextual',
    anchor_strategy         TEXT NOT NULL DEFAULT 'descriptive',
    required_flag           BOOLEAN NOT NULL DEFAULT false,
    score                   NUMERIC(6,5) NOT NULL DEFAULT 0 CHECK (score BETWEEN 0 AND 1),
    reason_payload          JSONB NOT NULL DEFAULT '{}'::jsonb,
    reason_version          TEXT NOT NULL DEFAULT 'graph_planning@1',
    scoring_version         TEXT NOT NULL DEFAULT 'seo_link_score@1',
    status                  TEXT NOT NULL DEFAULT 'candidate'
                            CHECK (status IN ('candidate','accepted','applied','rejected','expired','deprecated')),
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source_page_key, target_page_key, link_role, anchor_strategy, scoring_version),
    CHECK (source_page_key <> target_page_key)
);

CREATE INDEX IF NOT EXISTS idx_site_link_recommendations_source
    ON site.link_recommendations(source_page_key, status);
CREATE INDEX IF NOT EXISTS idx_site_link_recommendations_target
    ON site.link_recommendations(target_page_key, status);

CREATE TABLE IF NOT EXISTS site.cannibalization_conflicts (
    conflict_key       TEXT PRIMARY KEY,
    scope_signature    TEXT NOT NULL,
    page_key_a         TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    page_key_b         TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    conflict_reason    TEXT NOT NULL,
    severity           TEXT NOT NULL DEFAULT 'medium'
                       CHECK (severity IN ('low','medium','high','blocking')),
    detector_version   TEXT NOT NULL DEFAULT 'seo_cannibalization@1',
    status             TEXT NOT NULL DEFAULT 'open'
                       CHECK (status IN ('open','resolved','accepted_risk','rejected','deprecated')),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (page_key_a <> page_key_b),
    UNIQUE (page_key_a, page_key_b, conflict_reason, detector_version)
);

CREATE INDEX IF NOT EXISTS idx_site_cannibalization_conflicts_scope
    ON site.cannibalization_conflicts(scope_signature, status, severity);

CREATE TABLE IF NOT EXISTS site.page_briefs (
    page_brief_key    TEXT PRIMARY KEY,
    page_node_key     TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    blueprint_key     TEXT NOT NULL REFERENCES site.page_blueprints(blueprint_key) ON UPDATE CASCADE,
    title             TEXT NOT NULL,
    meta_description  TEXT NOT NULL DEFAULT '',
    required_sections JSONB NOT NULL DEFAULT '[]'::jsonb,
    required_links    JSONB NOT NULL DEFAULT '[]'::jsonb,
    evidence_manifest JSONB NOT NULL DEFAULT '[]'::jsonb,
    brief_version     INTEGER NOT NULL DEFAULT 1 CHECK (brief_version > 0),
    status            TEXT NOT NULL DEFAULT 'draft'
                      CHECK (status IN ('draft','ready','superseded','deprecated')),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (page_node_key, blueprint_key, brief_version)
);

CREATE TABLE IF NOT EXISTS site.page_drafts (
    page_draft_key       TEXT PRIMARY KEY,
    page_node_key        TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    page_brief_key       TEXT NOT NULL REFERENCES site.page_briefs(page_brief_key) ON UPDATE CASCADE,
    draft_revision       INTEGER NOT NULL DEFAULT 1 CHECK (draft_revision > 0),
    body_markdown        TEXT NOT NULL,
    traceability_manifest JSONB NOT NULL DEFAULT '[]'::jsonb,
    qa_verdict           TEXT NOT NULL DEFAULT 'not_run'
                         CHECK (qa_verdict IN (
                             'not_run','publish_ready','review_required',
                             'qa_failed','rebuild_required'
                         )),
    qa_blockers          JSONB NOT NULL DEFAULT '[]'::jsonb,
    truth_snapshot_ref   TEXT NOT NULL DEFAULT '',
    assembly_version     TEXT NOT NULL DEFAULT 'seo_draft_assemble@1',
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (page_node_key, page_brief_key, draft_revision)
);

CREATE INDEX IF NOT EXISTS idx_site_page_drafts_node_verdict
    ON site.page_drafts(page_node_key, qa_verdict, updated_at DESC);

CREATE TABLE IF NOT EXISTS site.page_support_bindings (
    page_support_binding_key TEXT PRIMARY KEY,
    page_node_key            TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    page_draft_key           TEXT REFERENCES site.page_drafts(page_draft_key) ON DELETE CASCADE,
    support_ref              TEXT NOT NULL,
    source_section_key       TEXT NOT NULL DEFAULT '',
    fragment_kind            TEXT NOT NULL DEFAULT 'factual',
    traceability_label       TEXT NOT NULL DEFAULT '',
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (page_node_key, page_draft_key, support_ref, source_section_key, fragment_kind)
);

CREATE INDEX IF NOT EXISTS idx_site_page_support_bindings_support_ref
    ON site.page_support_bindings(support_ref, page_node_key);

CREATE INDEX IF NOT EXISTS idx_site_page_support_bindings_page
    ON site.page_support_bindings(page_node_key, updated_at DESC);

CREATE TABLE IF NOT EXISTS site.cms_pages (
    page_node_key       TEXT PRIMARY KEY REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    scope_signature     TEXT NOT NULL,
    canonical_url_path  TEXT NOT NULL,
    locale_code         TEXT NOT NULL DEFAULT 'und',
    page_type_key       TEXT NOT NULL,
    dominant_intent_key TEXT NOT NULL,
    cms_document_id     TEXT NOT NULL,
    current_revision_id TEXT,
    current_status      TEXT NOT NULL DEFAULT 'planned'
                        CHECK (current_status IN (
                            'planned','blueprint_ready','draft_ready',
                            'review_required','approved','blocked',
                            'published','stale','needs_rebuild','deprecated'
                        )),
    rollback_revision_id TEXT,
    published_at        TIMESTAMPTZ,
    deprecated_at       TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (scope_signature, canonical_url_path),
    UNIQUE (cms_document_id)
);

CREATE INDEX IF NOT EXISTS idx_site_cms_pages_status
    ON site.cms_pages(current_status, updated_at DESC);

CREATE TABLE IF NOT EXISTS site.cms_page_revisions (
    revision_id           TEXT PRIMARY KEY,
    page_node_key         TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    page_draft_key        TEXT NOT NULL REFERENCES site.page_drafts(page_draft_key) ON UPDATE CASCADE,
    page_brief_key        TEXT NOT NULL REFERENCES site.page_briefs(page_brief_key) ON UPDATE CASCADE,
    title                 TEXT NOT NULL,
    meta_description      TEXT NOT NULL DEFAULT '',
    h1                    TEXT NOT NULL,
    body_payload          JSONB NOT NULL DEFAULT '{}'::jsonb,
    faq_payload           JSONB NOT NULL DEFAULT '[]'::jsonb,
    schema_markup_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    required_link_payload JSONB NOT NULL DEFAULT '[]'::jsonb,
    traceability_manifest JSONB NOT NULL DEFAULT '[]'::jsonb,
    blueprint_key         TEXT NOT NULL REFERENCES site.page_blueprints(blueprint_key) ON UPDATE CASCADE,
    qa_report_key         TEXT NOT NULL DEFAULT '',
    freshness_class       TEXT NOT NULL DEFAULT 'fresh'
                          CHECK (freshness_class IN ('fresh','watch','stale','unknown')),
    review_owner_role     TEXT NOT NULL DEFAULT 'seo_editor',
    revision_reason       TEXT NOT NULL DEFAULT 'seo_publish_gate',
    revision_status       TEXT NOT NULL DEFAULT 'review_required'
                          CHECK (revision_status IN (
                              'draft','review_required','approved',
                              'blocked','published','superseded',
                              'rolled_back','deprecated'
                          )),
    publish_notes         TEXT NOT NULL DEFAULT '',
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (page_node_key, page_draft_key)
);

CREATE INDEX IF NOT EXISTS idx_site_cms_page_revisions_page
    ON site.cms_page_revisions(page_node_key, revision_status, updated_at DESC);

CREATE TABLE IF NOT EXISTS site.cms_publish_events (
    event_key       TEXT PRIMARY KEY,
    event_type      TEXT NOT NULL
                    CHECK (event_type IN (
                        'seo_page_review_requested','seo_page_approved',
                        'seo_page_publish_blocked','seo_page_published',
                        'seo_page_deprecated','seo_page_rollback_requested',
                        'seo_page_rolled_back','seo_page_rebuild_requested',
                        'seo_page_canonical_changed'
                    )),
    page_node_key   TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    scope_signature TEXT NOT NULL,
    revision_id     TEXT REFERENCES site.cms_page_revisions(revision_id) ON DELETE SET NULL,
    actor_role      TEXT NOT NULL DEFAULT 'seo_system',
    causal_step     TEXT NOT NULL DEFAULT '',
    payload_version TEXT NOT NULL DEFAULT 'seo_cms_event@1',
    event_payload   JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_site_cms_publish_events_page
    ON site.cms_publish_events(page_node_key, occurred_at DESC);

CREATE TABLE IF NOT EXISTS site.cms_approval_decisions (
    decision_key  TEXT PRIMARY KEY,
    page_node_key TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    revision_id   TEXT NOT NULL REFERENCES site.cms_page_revisions(revision_id) ON DELETE CASCADE,
    actor_role    TEXT NOT NULL,
    decision      TEXT NOT NULL CHECK (decision IN ('approved','blocked','reopened')),
    reason        TEXT NOT NULL DEFAULT '',
    decided_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    decision_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (revision_id, actor_role, decision)
);

CREATE INDEX IF NOT EXISTS idx_site_cms_approval_decisions_revision
    ON site.cms_approval_decisions(revision_id, decision, decided_at DESC);

CREATE TABLE IF NOT EXISTS site.publish_artifacts (
    artifact_key  TEXT PRIMARY KEY,
    page_node_key TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    revision_id   TEXT NOT NULL REFERENCES site.cms_page_revisions(revision_id) ON DELETE CASCADE,
    artifact_type TEXT NOT NULL DEFAULT 'static_manifest'
                  CHECK (artifact_type IN ('static_manifest','deploy_bundle','headless_snapshot')),
    artifact_uri  TEXT NOT NULL DEFAULT '',
    manifest_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    status        TEXT NOT NULL DEFAULT 'created'
                  CHECK (status IN (
                      'created','pending_materialization','built_pending_validation',
                      'ready','blocked','build_failed','uploaded','deployed',
                      'failed','superseded'
                  )),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (revision_id, artifact_type)
);

CREATE INDEX IF NOT EXISTS idx_site_publish_artifacts_page
    ON site.publish_artifacts(page_node_key, created_at DESC);

CREATE TABLE IF NOT EXISTS site.publish_artifact_entries (
    artifact_entry_key TEXT PRIMARY KEY,
    artifact_key       TEXT NOT NULL REFERENCES site.publish_artifacts(artifact_key) ON DELETE CASCADE,
    relative_path      TEXT NOT NULL,
    entry_type         TEXT NOT NULL DEFAULT 'page'
                       CHECK (entry_type IN ('page','sitemap','robots','manifest','asset')),
    status             TEXT NOT NULL DEFAULT 'built'
                       CHECK (status IN ('built','ready','blocked','superseded')),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (artifact_key, relative_path)
);

CREATE INDEX IF NOT EXISTS idx_site_publish_artifact_entries_artifact
    ON site.publish_artifact_entries(artifact_key, entry_type, updated_at DESC);

CREATE TABLE IF NOT EXISTS site.expertise_signals (
    signal_key      TEXT PRIMARY KEY,
    page_node_key   TEXT REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    scope_signature TEXT NOT NULL,
    signal_type     TEXT NOT NULL
                    CHECK (signal_type IN (
                        'official_source_coverage','freshness','contradiction_status',
                        'procedural_completeness','reviewer_approval'
                    )),
    signal_value    TEXT NOT NULL,
    score           NUMERIC(6,5) NOT NULL DEFAULT 0 CHECK (score BETWEEN 0 AND 1),
    evidence_ref    TEXT NOT NULL DEFAULT '',
    observed_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    valid_until     TIMESTAMPTZ,
    status          TEXT NOT NULL DEFAULT 'active'
                    CHECK (status IN ('active','stale','rejected','deprecated')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_site_expertise_signals_scope
    ON site.expertise_signals(scope_signature, signal_type, status);

CREATE TABLE IF NOT EXISTS site.seo_hitl_tasks (
    task_key               TEXT PRIMARY KEY,
    task_type              TEXT NOT NULL
                           CHECK (task_type IN (
                               'cannibalization_conflict','unsafe_serp_pattern',
                               'unsupported_factual_fragment','registry_change',
                               'rebuild_suppression','canonical_url_conflict',
                               'publish_gate_blocker'
                           )),
    queue_state            TEXT NOT NULL DEFAULT 'open'
                           CHECK (queue_state IN (
                               'open','in_review','resolved','reopened',
                               'superseded','cancelled'
                           )),
    first_owner_role       TEXT NOT NULL DEFAULT 'seo_editor',
    current_owner_role     TEXT NOT NULL DEFAULT 'seo_editor',
    page_node_key          TEXT REFERENCES site.page_nodes(page_node_key) ON DELETE SET NULL,
    scope_signature        TEXT NOT NULL DEFAULT '',
    blocking_step_name     TEXT NOT NULL DEFAULT '',
    blocking_execution_key TEXT NOT NULL DEFAULT '',
    resolution_deadline_at TIMESTAMPTZ,
    severity               TEXT NOT NULL DEFAULT 'medium'
                           CHECK (severity IN ('low','medium','high','blocking')),
    decision_payload       JSONB NOT NULL DEFAULT '{}'::jsonb,
    audit_log_payload      JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_site_seo_hitl_tasks_queue
    ON site.seo_hitl_tasks(queue_state, severity, resolution_deadline_at);

CREATE TABLE IF NOT EXISTS site.page_lifecycle_events (
    lifecycle_event_id BIGSERIAL PRIMARY KEY,
    page_node_key      TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    from_state         TEXT,
    to_state           TEXT NOT NULL,
    reason             TEXT NOT NULL DEFAULT '',
    actor              TEXT NOT NULL DEFAULT 'seo_system',
    event_payload      JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_site_page_lifecycle_events_page
    ON site.page_lifecycle_events(page_node_key, created_at DESC);

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
    final_url       TEXT,
    title           TEXT,
    meta_desc       TEXT,
    canonical       TEXT,
    word_count      INTEGER,
    content         JSONB,                  -- структурированный: markdown, headings, links
    raw_html        TEXT,                   -- полный HTML (TOASTed)
    raw_html_bytes  INTEGER,
    content_hash    TEXT,                   -- blake3(raw_html), hex64
    redirect_chain  JSONB DEFAULT '[]'::jsonb,
    robots_trace    JSONB DEFAULT '{}'::jsonb,
    source_observation JSONB DEFAULT '{}'::jsonb,
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

CREATE TABLE IF NOT EXISTS raw.page_content_aliases (
    alias_page_id     BIGINT PRIMARY KEY REFERENCES raw.pages(id) ON DELETE CASCADE,
    canonical_page_id BIGINT NOT NULL REFERENCES raw.pages(id) ON DELETE CASCADE,
    source_url        TEXT NOT NULL,
    final_url         TEXT NOT NULL,
    content_hash      TEXT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_raw_page_content_aliases_canonical
    ON raw.page_content_aliases(canonical_page_id);
CREATE INDEX IF NOT EXISTS idx_raw_page_content_aliases_hash
    ON raw.page_content_aliases(content_hash);

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
    source_tier TEXT NOT NULL DEFAULT 'low_trust',
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
    source_domain     TEXT NOT NULL DEFAULT '',
    source_type       TEXT NOT NULL,   -- 'top10' | 'source_resolved'
    dtype             TEXT,            -- 'government' | 'niche_agency' | 'forum' | ...
    first_seen_run_id TEXT NOT NULL,
    first_seen_job_id TEXT NOT NULL,
    query_batch_key   TEXT NOT NULL DEFAULT '',
    first_seen_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    next_attempt_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    status            TEXT NOT NULL DEFAULT 'pending',
    http_status       INT,
    crawl_attempt_count INT NOT NULL DEFAULT 0,
    locked_until      TIMESTAMPTZ,
    last_error        TEXT,
    notes             TEXT
);

ALTER TABLE serp.crawl_queue
    ADD COLUMN IF NOT EXISTS source_domain TEXT NOT NULL DEFAULT '';
ALTER TABLE serp.crawl_queue
    ADD COLUMN IF NOT EXISTS query_batch_key TEXT NOT NULL DEFAULT '';
ALTER TABLE serp.crawl_queue
    ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE serp.crawl_queue
    ADD COLUMN IF NOT EXISTS crawl_attempt_count INT NOT NULL DEFAULT 0;
ALTER TABLE serp.crawl_queue
    ADD COLUMN IF NOT EXISTS locked_until TIMESTAMPTZ;
ALTER TABLE serp.crawl_queue
    ADD COLUMN IF NOT EXISTS last_error TEXT;

CREATE INDEX IF NOT EXISTS idx_serp_crawl_queue_status
    ON serp.crawl_queue(status);
CREATE INDEX IF NOT EXISTS idx_serp_crawl_queue_run_batch
    ON serp.crawl_queue(first_seen_run_id, query_batch_key, status, first_seen_at);
CREATE INDEX IF NOT EXISTS idx_serp_crawl_queue_domain_ready
    ON serp.crawl_queue(source_domain, status, next_attempt_at, first_seen_at);

CREATE TABLE IF NOT EXISTS raw.section_context_candidates (
    raw_section_id BIGINT NOT NULL REFERENCES raw.sections(id) ON DELETE CASCADE,
    context_key    TEXT NOT NULL REFERENCES kb.visa_contexts(context_key) ON UPDATE CASCADE,
    confidence     NUMERIC(5,4) NOT NULL DEFAULT 0.5000,
    source_type    TEXT NOT NULL DEFAULT '',
    mapping_reason TEXT NOT NULL DEFAULT '',
    status         TEXT NOT NULL DEFAULT 'candidate'
                   CHECK (status IN ('candidate','accepted','rejected','deprecated')),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (raw_section_id, context_key)
);

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

CREATE INDEX IF NOT EXISTS idx_raw_section_context_candidates_context
    ON raw.section_context_candidates(context_key, status);

-- ----------------------------------------------------------------------------
-- SEO intelligence structures derived from raw SERP ingestion.
-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS serp.query_batches (
    query_batch_key   TEXT PRIMARY KEY,
    scope_signature   TEXT NOT NULL,
    market            TEXT NOT NULL,
    locale            TEXT NOT NULL,
    source_system     TEXT NOT NULL DEFAULT 'serp_ingest',
    batch_version     INTEGER NOT NULL DEFAULT 1 CHECK (batch_version > 0),
    status            TEXT NOT NULL DEFAULT 'queued'
                      CHECK (status IN ('queued','running','blocked','done','failed','cancelled')),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_serp_query_batches_scope
    ON serp.query_batches(scope_signature, status, created_at DESC);

CREATE TABLE IF NOT EXISTS serp.serp_patterns (
    serp_pattern_key  TEXT PRIMARY KEY,
    query_batch_key   TEXT NOT NULL REFERENCES serp.query_batches(query_batch_key) ON DELETE CASCADE,
    scope_signature   TEXT NOT NULL,
    query             TEXT NOT NULL,
    pattern_type      TEXT NOT NULL,
    dominant_intent   TEXT NOT NULL,
    reliability_score NUMERIC(6,5) NOT NULL DEFAULT 0 CHECK (reliability_score BETWEEN 0 AND 1),
    evidence_ref      TEXT,
    pattern_version   TEXT NOT NULL DEFAULT 'seo_serp_pattern@1',
    status            TEXT NOT NULL DEFAULT 'active'
                      CHECK (status IN ('active','partial','rejected','deprecated')),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (query_batch_key, query, pattern_type, pattern_version)
);

CREATE INDEX IF NOT EXISTS idx_serp_patterns_scope
    ON serp.serp_patterns(scope_signature, dominant_intent, status);

CREATE TABLE IF NOT EXISTS serp.serp_pattern_observations (
    observation_key   TEXT PRIMARY KEY,
    serp_pattern_key  TEXT NOT NULL REFERENCES serp.serp_patterns(serp_pattern_key) ON DELETE CASCADE,
    url_norm          TEXT,
    rank_position     INTEGER CHECK (rank_position IS NULL OR rank_position > 0),
    title             TEXT,
    snippet           TEXT,
    observed_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    observation_payload JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS idx_serp_pattern_observations_pattern
    ON serp.serp_pattern_observations(serp_pattern_key, observed_at DESC);

CREATE TABLE IF NOT EXISTS serp.competitor_pages (
    competitor_page_key TEXT PRIMARY KEY,
    scope_signature     TEXT NOT NULL,
    url_norm            TEXT NOT NULL,
    domain_norm         TEXT,
    title               TEXT,
    page_type_guess     TEXT,
    first_seen_batch_key TEXT REFERENCES serp.query_batches(query_batch_key) ON DELETE SET NULL,
    status              TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active','ignored','deprecated')),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (scope_signature, url_norm)
);

CREATE INDEX IF NOT EXISTS idx_serp_competitor_pages_scope
    ON serp.competitor_pages(scope_signature, status);

CREATE TABLE IF NOT EXISTS serp.competitor_section_patterns (
    competitor_section_pattern_key TEXT PRIMARY KEY,
    competitor_page_key            TEXT NOT NULL REFERENCES serp.competitor_pages(competitor_page_key) ON DELETE CASCADE,
    section_role                   TEXT NOT NULL,
    heading_text                   TEXT,
    pattern_payload                JSONB NOT NULL DEFAULT '{}'::jsonb,
    detector_version               TEXT NOT NULL DEFAULT 'seo_competitor_section@1',
    created_at                     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_serp_competitor_section_patterns_page
    ON serp.competitor_section_patterns(competitor_page_key, section_role);

CREATE TABLE IF NOT EXISTS serp.opportunity_candidates (
    opportunity_key   TEXT PRIMARY KEY,
    scope_signature   TEXT NOT NULL,
    query_batch_key   TEXT REFERENCES serp.query_batches(query_batch_key) ON DELETE SET NULL,
    serp_pattern_key  TEXT REFERENCES serp.serp_patterns(serp_pattern_key) ON DELETE SET NULL,
    seed_keyword      TEXT NOT NULL,
    dominant_intent   TEXT NOT NULL,
    opportunity_score NUMERIC(6,5) NOT NULL DEFAULT 0 CHECK (opportunity_score BETWEEN 0 AND 1),
    recommended_action TEXT NOT NULL DEFAULT 'create_or_refresh_page',
    status            TEXT NOT NULL DEFAULT 'candidate'
                      CHECK (status IN ('candidate','accepted','rejected','expired','deprecated')),
    scoring_version   TEXT NOT NULL DEFAULT 'seo_opportunity@1',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (scope_signature, seed_keyword, dominant_intent, scoring_version)
);

CREATE INDEX IF NOT EXISTS idx_serp_opportunity_candidates_scope
    ON serp.opportunity_candidates(scope_signature, status, opportunity_score DESC);

-- ==============================================================================
-- 8. MONITORING — SEO operational signals
-- ==============================================================================

CREATE TABLE IF NOT EXISTS monitoring.seo_metric_snapshots (
    metric_snapshot_id BIGSERIAL PRIMARY KEY,
    metric_name        TEXT NOT NULL,
    scope_signature    TEXT,
    metric_value       NUMERIC NOT NULL,
    labels             JSONB NOT NULL DEFAULT '{}'::jsonb,
    recorded_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_monitoring_seo_metric_snapshots_lookup
    ON monitoring.seo_metric_snapshots(metric_name, recorded_at DESC);

CREATE TABLE IF NOT EXISTS monitoring.seo_freshness_alerts (
    freshness_alert_key TEXT PRIMARY KEY,
    artifact_type       TEXT NOT NULL,
    artifact_key        TEXT NOT NULL,
    severity            TEXT NOT NULL DEFAULT 'medium'
                        CHECK (severity IN ('low','medium','high','blocking')),
    status              TEXT NOT NULL DEFAULT 'open'
                        CHECK (status IN ('open','acknowledged','resolved','dismissed')),
    reason              TEXT NOT NULL DEFAULT '',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at         TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_monitoring_seo_freshness_alerts_open
    ON monitoring.seo_freshness_alerts(status, severity, created_at DESC);

CREATE TABLE IF NOT EXISTS monitoring.seo_rebuild_backlog (
    rebuild_request_key TEXT PRIMARY KEY,
    page_node_key       TEXT REFERENCES site.page_nodes(page_node_key) ON DELETE SET NULL,
    trigger_type        TEXT NOT NULL,
    priority            INTEGER NOT NULL DEFAULT 2 CHECK (priority BETWEEN 1 AND 3),
    status              TEXT NOT NULL DEFAULT 'queued'
                        CHECK (status IN ('queued','running','blocked','done','failed','cancelled')),
    reason              TEXT NOT NULL DEFAULT '',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_monitoring_seo_rebuild_backlog_status
    ON monitoring.seo_rebuild_backlog(status, priority, created_at);

CREATE TABLE IF NOT EXISTS monitoring.seo_rebuild_dependencies (
    rebuild_dependency_key TEXT PRIMARY KEY,
    page_node_key          TEXT NOT NULL REFERENCES site.page_nodes(page_node_key) ON DELETE CASCADE,
    dependency_type        TEXT NOT NULL
                           CHECK (dependency_type IN (
                               'truth_support','blueprint','section_template',
                               'keyword_cluster','serp_query','required_link',
                               'source_provenance','navigation_state'
                           )),
    dependency_ref         TEXT NOT NULL,
    reason_package         JSONB NOT NULL DEFAULT '{}'::jsonb,
    status                 TEXT NOT NULL DEFAULT 'active'
                           CHECK (status IN ('active','deprecated')),
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (page_node_key, dependency_type, dependency_ref)
);

CREATE INDEX IF NOT EXISTS idx_monitoring_seo_rebuild_dependencies_ref
    ON monitoring.seo_rebuild_dependencies(dependency_type, dependency_ref, status);

CREATE INDEX IF NOT EXISTS idx_monitoring_seo_rebuild_dependencies_page
    ON monitoring.seo_rebuild_dependencies(page_node_key, status, updated_at DESC);

CREATE TABLE IF NOT EXISTS monitoring.seo_quality_failures (
    quality_failure_key TEXT PRIMARY KEY,
    page_draft_key      TEXT REFERENCES site.page_drafts(page_draft_key) ON DELETE SET NULL,
    failure_type        TEXT NOT NULL,
    severity            TEXT NOT NULL DEFAULT 'medium'
                        CHECK (severity IN ('low','medium','high','blocking')),
    status              TEXT NOT NULL DEFAULT 'open'
                        CHECK (status IN ('open','acknowledged','resolved','dismissed')),
    details             JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_monitoring_seo_quality_failures_open
    ON monitoring.seo_quality_failures(status, severity, created_at DESC);

-- ==============================================================================
-- 9. PIPELINE — Operational tables
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
                        CHECK (workflow_type IN ('extract_facts','generate_content','seo_site_build')),
    context_key         TEXT        NOT NULL,
    status              TEXT        NOT NULL    DEFAULT 'created'
                        CHECK (status IN (
                            'created','normalizing','extracting','verifying',
                            'persisting','generating','validating','pending_hitl','done','failed'
                        )),
    created_at          TIMESTAMPTZ NOT NULL    DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL    DEFAULT now()
);

ALTER TABLE pipeline.execution_runs
    DROP CONSTRAINT IF EXISTS execution_runs_workflow_type_check;
ALTER TABLE pipeline.execution_runs
    ADD CONSTRAINT execution_runs_workflow_type_check
        CHECK (workflow_type IN ('extract_facts','generate_content','seo_site_build'));

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
