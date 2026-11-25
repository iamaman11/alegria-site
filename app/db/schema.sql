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

-- Canonical ontology vocabulary only; not visa claims or verified facts.
-- One owner on fresh installation; idempotent against existing databases.
INSERT INTO kb.concepts (concept_key, concept_type, label_ru, status) VALUES
  ('consular_fee', 'fee', 'Консульский сбор', 'active'),
  ('passport', 'document', 'Паспорт', 'active'),
  ('medical_insurance', 'document', 'Медицинская страховка', 'active'),
  ('schengen_visa_application_form', 'document', 'Анкета на шенгенскую визу', 'active'),
  ('standard_processing_time', 'timeline', 'Стандартный срок рассмотрения', 'active'),
  ('standard_tourist_eligibility', 'rule', 'Стандартные условия туристической визы', 'active'),
  ('standard_application_process', 'process', 'Порядок подачи заявления', 'active'),
  ('standard_application_channel', 'location', 'Канал подачи заявления', 'active')
ON CONFLICT (concept_key) DO NOTHING;

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
            'seo_page_blueprints',
            'seo_serp_patterns'
        )),
    embedding_model   TEXT NOT NULL,
    embedding_version TEXT NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (entity_type, entity_key, collection_name)
);

CREATE INDEX IF NOT EXISTS idx_kb_qdrant_points_entity
    ON kb.qdrant_points(entity_type, entity_key);

-- A verified-rule identity may not evade the transaction fence by claiming
-- another entity type inside its reserved Qdrant collection.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                   WHERE conrelid='kb.qdrant_points'::regclass
                     AND conname='qdrant_points_verified_rule_entity_type_check') THEN
        ALTER TABLE kb.qdrant_points
            ADD CONSTRAINT qdrant_points_verified_rule_entity_type_check
            CHECK (collection_name <> 'verified_rules_4' OR entity_type='rule_instance') NOT VALID;
    END IF;
END $$;

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
    projection_revision BIGINT,             -- causal per-rule counter; NULL for other targets and legacy rows
    staged_xid     xid8,                    -- top-level transaction that staged a verified-rule intent
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
    ADD COLUMN IF NOT EXISTS projection_revision BIGINT;
ALTER TABLE system.sync_outbox
    ADD COLUMN IF NOT EXISTS staged_xid xid8;
ALTER TABLE system.sync_outbox
    DROP COLUMN IF EXISTS payload_json;

-- Do not invent order for pre-revision events. NOT VALID deliberately leaves
-- historical rows untouched, but rejects all new inserts AND updates of an
-- unversioned verified-rule row (including attempts to claim it for delivery).
-- A real operator cutover is required for ANY historical unversioned intent.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                   WHERE conrelid='system.sync_outbox'::regclass
                     AND conname='sync_outbox_verified_rule_revision_check') THEN
        ALTER TABLE system.sync_outbox
            ADD CONSTRAINT sync_outbox_verified_rule_revision_check
            CHECK (NOT (target_system='qdrant' AND aggregate_type='rule_instance')
                   OR (projection_revision IS NOT NULL AND projection_revision > 0)) NOT VALID;
    END IF;
END $$;

-- This counter is an identity-local, durable ordering authority. Its updates
-- and Outbox insert commit in the canonical Truth transaction under the
-- identical per-rule advisory lock used by the dispatch fence.
CREATE TABLE IF NOT EXISTS system.verified_rule_projection_heads (
    aggregate_key TEXT PRIMARY KEY CHECK (aggregate_key <> '' AND btrim(aggregate_key)=aggregate_key),
    current_revision BIGINT NOT NULL CHECK (current_revision > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_sync_outbox_verified_rule_revision
    ON system.sync_outbox(aggregate_key, projection_revision)
    WHERE target_system='qdrant' AND aggregate_type='rule_instance'
      AND projection_revision IS NOT NULL;

-- Revision and immutable event identity cannot be rewritten after staging;
-- only status, delivery lease and retry metadata may change. In particular,
-- no old unversioned event can be silently 'repaired' via an UPDATE.
CREATE OR REPLACE FUNCTION system.guard_verified_rule_outbox_identity() RETURNS trigger AS $$
BEGIN
    IF ROW(OLD.run_id, OLD.aggregate_type, OLD.aggregate_key, OLD.target_system,
           OLD.event_type, OLD.payload_type, OLD.schema_version, OLD.idempotency_key,
           OLD.payload_bytes, OLD.payload_hash, OLD.projection_revision, OLD.staged_xid)
       IS DISTINCT FROM
       ROW(NEW.run_id, NEW.aggregate_type, NEW.aggregate_key, NEW.target_system,
           NEW.event_type, NEW.payload_type, NEW.schema_version, NEW.idempotency_key,
           NEW.payload_bytes, NEW.payload_hash, NEW.projection_revision, NEW.staged_xid) THEN
        RAISE EXCEPTION 'verified-rule Outbox identity and revision are immutable'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_verified_rule_outbox_identity ON system.sync_outbox;
CREATE TRIGGER trg_verified_rule_outbox_identity
    BEFORE UPDATE ON system.sync_outbox
    FOR EACH ROW
    WHEN ((OLD.target_system='qdrant' AND OLD.aggregate_type='rule_instance')
       OR (NEW.target_system='qdrant' AND NEW.aggregate_type='rule_instance'))
    EXECUTE FUNCTION system.guard_verified_rule_outbox_identity();

-- A caller cannot claim another transaction's outbox intent by supplying an
-- arbitrary xid. PostgreSQL stamps the *top-level* current transaction itself.
CREATE OR REPLACE FUNCTION system.stamp_verified_rule_outbox_xid() RETURNS trigger AS $$
BEGIN
    IF NEW.target_system='qdrant' AND NEW.aggregate_type='rule_instance' THEN
        NEW.staged_xid := pg_current_xact_id();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_verified_rule_outbox_xid ON system.sync_outbox;
CREATE TRIGGER trg_verified_rule_outbox_xid
    BEFORE INSERT ON system.sync_outbox
    FOR EACH ROW EXECUTE FUNCTION system.stamp_verified_rule_outbox_xid();

-- A new Truth transaction must not erase its older causal receipts or reset
-- its revision authority through ordinary DELETE. History retention is part
-- of the fence; deliberate operator repair is a separate reviewed action.
CREATE OR REPLACE FUNCTION system.reject_verified_rule_history_delete() RETURNS trigger AS $$
BEGIN
    IF TG_TABLE_NAME='sync_outbox' THEN
        IF OLD.target_system='qdrant' AND OLD.aggregate_type='rule_instance' THEN
            RAISE EXCEPTION 'verified-rule Outbox history cannot be deleted'
                USING ERRCODE='23514';
        END IF;
    ELSE
        RAISE EXCEPTION 'verified-rule revision head cannot be deleted'
            USING ERRCODE='23514';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_verified_rule_outbox_delete ON system.sync_outbox;
CREATE TRIGGER trg_verified_rule_outbox_delete
    BEFORE DELETE ON system.sync_outbox
    FOR EACH ROW EXECUTE FUNCTION system.reject_verified_rule_history_delete();
DROP TRIGGER IF EXISTS trg_verified_rule_revision_head_delete ON system.verified_rule_projection_heads;
CREATE TRIGGER trg_verified_rule_revision_head_delete
    BEFORE DELETE ON system.verified_rule_projection_heads
    FOR EACH ROW EXECUTE FUNCTION system.reject_verified_rule_history_delete();

-- NOT VALID quarantines unprovable historical receipts. New verified-rule
-- events are always stamped; old rows cannot be repaired by rewriting them.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                   WHERE conrelid='system.sync_outbox'::regclass
                     AND conname='sync_outbox_verified_rule_xid_check') THEN
        ALTER TABLE system.sync_outbox
            ADD CONSTRAINT sync_outbox_verified_rule_xid_check
            CHECK (NOT (target_system='qdrant' AND aggregate_type='rule_instance')
                   OR staged_xid IS NOT NULL) NOT VALID;
    END IF;
END $$;

-- Commit-time guard: a projected identity's final SQL state must have a
-- matching, latest-revision Qdrant command staged by THIS SAME transaction.
-- This protects low-level SQL/SQLx calls, including writes that bypass Rust
-- helpers, while remaining compatible with intentionally unprojected Truth.
CREATE OR REPLACE FUNCTION system.assert_verified_rule_tx_intent(
    rule_key TEXT, require_projection BOOLEAN
) RETURNS void AS $$
DECLARE
    latest BIGINT;
    point_exists BOOLEAN;
    expected_command TEXT;
BEGIN
    IF rule_key IS NULL OR btrim(rule_key)='' THEN
        RAISE EXCEPTION 'verified-rule projection identity is missing'
            USING ERRCODE='23514';
    END IF;
    SELECT current_revision INTO latest
      FROM system.verified_rule_projection_heads WHERE aggregate_key=rule_key;
    SELECT EXISTS (
        SELECT 1 FROM kb.qdrant_points
        WHERE collection_name='verified_rules_4'
          AND entity_type='rule_instance' AND entity_key=rule_key
    ) INTO point_exists;
    IF NOT require_projection AND latest IS NULL AND NOT point_exists THEN
        RETURN;
    END IF;
    IF latest IS NULL THEN
        RAISE EXCEPTION 'verified-rule projection % has no revision head', rule_key
            USING ERRCODE='23514';
    END IF;
    expected_command := CASE WHEN point_exists
        THEN 'QdrantUpsertCommand' ELSE 'QdrantDeleteCommand' END;
    IF NOT EXISTS (
        SELECT 1 FROM system.sync_outbox
        WHERE target_system='qdrant' AND aggregate_type='rule_instance'
          AND aggregate_key=rule_key AND projection_revision=latest
          AND staged_xid=pg_current_xact_id()
          AND event_type=expected_command
    ) THEN
        RAISE EXCEPTION 'verified-rule projection % requires same-transaction % at revision %',
            rule_key, expected_command, latest USING ERRCODE='23514';
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION system.guard_verified_rule_registry_commit() RETURNS trigger AS $$
BEGIN
    IF TG_OP='UPDATE' THEN
        IF (to_jsonb(OLD)-'updated_at')=(to_jsonb(NEW)-'updated_at') THEN
            RETURN NULL;
        END IF;
    END IF;
    IF TG_OP='UPDATE' OR TG_OP='DELETE' THEN
        IF OLD.collection_name='verified_rules_4'
           AND OLD.entity_type='rule_instance' THEN
            PERFORM system.assert_verified_rule_tx_intent(OLD.entity_key, TRUE);
        END IF;
    END IF;
    IF TG_OP='INSERT' THEN
        IF NEW.collection_name='verified_rules_4' AND NEW.entity_type='rule_instance' THEN
            PERFORM system.assert_verified_rule_tx_intent(NEW.entity_key, TRUE);
        END IF;
    ELSIF TG_OP='UPDATE' THEN
        IF NEW.collection_name='verified_rules_4' AND NEW.entity_type='rule_instance'
           AND (OLD.entity_key IS DISTINCT FROM NEW.entity_key
                OR OLD.collection_name IS DISTINCT FROM NEW.collection_name
                OR OLD.entity_type IS DISTINCT FROM NEW.entity_type) THEN
            PERFORM system.assert_verified_rule_tx_intent(NEW.entity_key, TRUE);
        END IF;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_verified_rule_registry_commit ON kb.qdrant_points;
CREATE CONSTRAINT TRIGGER trg_verified_rule_registry_commit
    AFTER INSERT OR UPDATE OR DELETE ON kb.qdrant_points
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION system.guard_verified_rule_registry_commit();

CREATE OR REPLACE FUNCTION system.guard_verified_rule_truth_commit() RETURNS trigger AS $$
BEGIN
    IF TG_OP='UPDATE' THEN
        IF (to_jsonb(OLD)-'updated_at')=(to_jsonb(NEW)-'updated_at') THEN
            RETURN NULL;
        END IF;
    END IF;
    IF TG_OP='UPDATE' OR TG_OP='DELETE' THEN
        PERFORM system.assert_verified_rule_tx_intent(OLD.rule_instance_id, FALSE);
    END IF;
    IF TG_OP='INSERT' THEN
        PERFORM system.assert_verified_rule_tx_intent(NEW.rule_instance_id, FALSE);
    ELSIF TG_OP='UPDATE' AND OLD.rule_instance_id IS DISTINCT FROM NEW.rule_instance_id THEN
        PERFORM system.assert_verified_rule_tx_intent(NEW.rule_instance_id, FALSE);
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_verified_rule_truth_commit ON verified.rule_instances;
CREATE CONSTRAINT TRIGGER trg_verified_rule_truth_commit
    AFTER INSERT OR UPDATE OR DELETE ON verified.rule_instances
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION system.guard_verified_rule_truth_commit();

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

-- A projection revision may have identical lookup payload bytes but different
-- canonical SQL content. Payload-byte uniqueness would discard that revision.
-- The existing logical idempotency index below is the sole deduplication owner.
DROP INDEX IF EXISTS system.idx_sync_outbox_dedup;

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
    CONSTRAINT content_gaps_owner_topic_detector_key
        UNIQUE NULLS NOT DISTINCT (scope_signature, page_node_key, missing_topic, detector_version)
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
    CONSTRAINT page_drafts_run_revision_uq UNIQUE (page_node_key, page_brief_key, truth_snapshot_ref, draft_revision)
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
                               'unsupported_factual_fragment','truth_candidate_review','registry_change',
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

-- Immutable evidence: every same-day re-fetch is a new snapshot, never an UPDATE.
CREATE INDEX IF NOT EXISTS idx_raw_pages_url_snapshot ON raw.pages (url, snapshot_at DESC);

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

-- Raw evidence writers are independent of Truth's context lock. PostgreSQL
-- enforces the narrow cross-owner boundary even for direct SQL writers:
-- V3 acquires matching read-only advisory keys before hashing and holds them
-- through the atomic HITL + receipt commit. No raw UPDATE grant is needed.
CREATE OR REPLACE FUNCTION raw.guard_evidence_writer_lock()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'TRUNCATE' THEN
        RAISE EXCEPTION 'raw evidence snapshots cannot be truncated'
            USING ERRCODE = '23514';
    END IF;
    -- `processed` is a crawler progress bit, explicitly excluded from V3's
    -- immutable source-identity fingerprint. Allow this one field to advance
    -- without aborting an admitted Truth effect; all other old/new fields,
    -- including future columns, remain protected by the writer advisory key.
    IF TG_TABLE_NAME = 'pages' AND TG_OP = 'UPDATE'
       AND (to_jsonb(NEW) - 'processed') IS NOT DISTINCT FROM
           (to_jsonb(OLD) - 'processed') THEN
        RETURN NEW;
    END IF;
    IF NOT pg_try_advisory_xact_lock(
        hashtextextended('raw-evidence|' || TG_TABLE_NAME || '|' || OLD.id::text, 0)
    ) THEN
        RAISE EXCEPTION 'raw evidence snapshot is locked by admitted Truth effect'
            USING ERRCODE = '55P03';
    END IF;
    IF TG_OP = 'UPDATE' THEN
        RETURN NEW;
    END IF;
    RETURN OLD;
END;
$$;
DROP TRIGGER IF EXISTS guard_raw_page_writer_lock ON raw.pages;
CREATE TRIGGER guard_raw_page_writer_lock
BEFORE UPDATE OR DELETE ON raw.pages
FOR EACH ROW EXECUTE FUNCTION raw.guard_evidence_writer_lock();
DROP TRIGGER IF EXISTS guard_raw_section_writer_lock ON raw.sections;
CREATE TRIGGER guard_raw_section_writer_lock
BEFORE UPDATE OR DELETE ON raw.sections
FOR EACH ROW EXECUTE FUNCTION raw.guard_evidence_writer_lock();
DROP TRIGGER IF EXISTS guard_raw_page_truncate ON raw.pages;
CREATE TRIGGER guard_raw_page_truncate
BEFORE TRUNCATE ON raw.pages
FOR EACH STATEMENT EXECUTE FUNCTION raw.guard_evidence_writer_lock();
DROP TRIGGER IF EXISTS guard_raw_section_truncate ON raw.sections;
CREATE TRIGGER guard_raw_section_truncate
BEFORE TRUNCATE ON raw.sections
FOR EACH STATEMENT EXECUTE FUNCTION raw.guard_evidence_writer_lock();

-- Direct candidate INSERT/UPDATE/DELETE must obey the same context lock as
-- canonical Truth writes, preventing phantoms during V3's source snapshot.
CREATE OR REPLACE FUNCTION extracted.lock_candidate_truth_context()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    old_context text;
    new_context text;
BEGIN
    IF TG_OP = 'INSERT' THEN
        PERFORM pg_advisory_xact_lock(
            hashtextextended('truth-context|' || NEW.context_key, 0)
        );
        RETURN NEW;
    ELSIF TG_OP = 'DELETE' THEN
        PERFORM pg_advisory_xact_lock(
            hashtextextended('truth-context|' || OLD.context_key, 0)
        );
        RETURN OLD;
    END IF;
    old_context := OLD.context_key;
    new_context := NEW.context_key;
    IF old_context <= new_context THEN
        PERFORM pg_advisory_xact_lock(
            hashtextextended('truth-context|' || old_context, 0)
        );
        IF old_context <> new_context THEN
            PERFORM pg_advisory_xact_lock(
                hashtextextended('truth-context|' || new_context, 0)
            );
        END IF;
    ELSE
        PERFORM pg_advisory_xact_lock(
            hashtextextended('truth-context|' || new_context, 0)
        );
        PERFORM pg_advisory_xact_lock(
            hashtextextended('truth-context|' || old_context, 0)
        );
    END IF;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS lock_candidate_truth_context ON extracted.rule_candidates;
CREATE TRIGGER lock_candidate_truth_context
BEFORE INSERT OR UPDATE OR DELETE ON extracted.rule_candidates
FOR EACH ROW EXECUTE FUNCTION extracted.lock_candidate_truth_context();

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

-- The admitted V3 Truth operation reads execution_runs.status under the
-- truth-context advisory lock and must keep that status valid through its
-- atomic HITL + receipt commit. Ordinary status writers do not otherwise
-- participate in that lock. Refuse a concurrent status/context/identity
-- rewrite with 55P03 instead of holding a row lock that could deadlock with
-- the receipt FK. This applies ONLY to runs carrying a V3 admission; legacy
-- runs and bookkeeping-only updated_at changes remain unblocked.
CREATE OR REPLACE FUNCTION pipeline.guard_execution_run_truth_context()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE
    old_context text;
    new_context text;
    canonical_run boolean;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.run_id, NEW.workflow_run_id, NEW.workflow_type,
            NEW.context_key, NEW.status)
           IS NOT DISTINCT FROM
           (OLD.run_id, OLD.workflow_run_id, OLD.workflow_type,
            OLD.context_key, OLD.status) THEN
            RETURN NEW;
        END IF;
    END IF;
    -- The operator uses this exact run key before reading a run and issuing
    -- its first admission. Acquire it even if no admission exists yet: an
    -- uncommitted status writer must not let the operator authorize an old
    -- status and then commit terminal during the resulting Activity.
    IF TG_OP = 'DELETE' THEN
        canonical_run := OLD.workflow_type = 'seo_site_build';
    ELSE
        canonical_run := OLD.workflow_type = 'seo_site_build'
                         OR NEW.workflow_type = 'seo_site_build';
    END IF;
    IF canonical_run AND NOT pg_try_advisory_xact_lock(
        hashtextextended('truth-run-status|' || OLD.run_id::text, 0)
    ) THEN
        RAISE EXCEPTION 'canonical Truth run status is locked by operator issuance'
            USING ERRCODE = '55P03';
    END IF;
    -- Never impose V3's fail-fast semantics on unrelated legacy runs. Keep
    -- historical (also expired/revoked) admissions in scope: an Activity that
    -- passed admission earlier may still be finishing under this lock.
    -- SECURITY DEFINER permits ordinary run-status writers to check this
    -- immutable authority table without granting them SELECT or INSERT.
    IF NOT EXISTS (
        SELECT 1 FROM pipeline.pg_effect_v3_admissions a
        WHERE a.run_id = OLD.run_id
    ) THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    old_context := OLD.context_key;
    IF TG_OP = 'DELETE' THEN
        new_context := old_context;
    ELSE
        new_context := NEW.context_key;
    END IF;
    IF old_context <= new_context THEN
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || old_context, 0)) THEN
            RAISE EXCEPTION 'Truth execution-run context is locked by admitted effect'
                USING ERRCODE = '55P03';
        END IF;
        IF old_context <> new_context AND NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || new_context, 0)) THEN
            RAISE EXCEPTION 'Truth execution-run destination context is locked'
                USING ERRCODE = '55P03';
        END IF;
    ELSE
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || new_context, 0)) THEN
            RAISE EXCEPTION 'Truth execution-run destination context is locked'
                USING ERRCODE = '55P03';
        END IF;
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || old_context, 0)) THEN
            RAISE EXCEPTION 'Truth execution-run context is locked by admitted effect'
                USING ERRCODE = '55P03';
        END IF;
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS guard_execution_run_truth_context ON pipeline.execution_runs;
CREATE TRIGGER guard_execution_run_truth_context
BEFORE UPDATE OR DELETE ON pipeline.execution_runs
FOR EACH ROW EXECUTE FUNCTION pipeline.guard_execution_run_truth_context();

-- Experimental V3 PG effect admission is issued ONLY by a privileged operator
-- through SQL, never by a Workflow, an Activity, or an untrusted DAG manifest.
-- Deploy the PG effect worker with a distinct unprivileged DB role. Its start
-- guard checks that the role cannot INSERT/UPDATE/DELETE these admissions.
CREATE TABLE IF NOT EXISTS pipeline.pg_effect_v3_admissions (
    admission_id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    admission_schema_revision SMALLINT NOT NULL DEFAULT 3,
    -- V3 BLAKE3 workflow-command receipt key, independently pinned by the
    -- operator. Historical NULL admissions cannot start NEW effects.
    v3_expected_execution_key TEXT,
    run_id                 UUID NOT NULL REFERENCES pipeline.execution_runs(run_id),
    tenant_id              TEXT NOT NULL CHECK (length(trim(tenant_id)) > 0),
    context_key            TEXT NOT NULL CHECK (length(trim(context_key)) > 0),
    workflow_id            TEXT NOT NULL CHECK (length(trim(workflow_id)) > 0),
    workflow_execution_run_id TEXT NOT NULL CHECK (length(trim(workflow_execution_run_id)) > 0),
    operation_instance     TEXT NOT NULL CHECK (length(trim(operation_instance)) > 0),
    plan_hash              TEXT NOT NULL CHECK (length(plan_hash) = 64),
    operation_id           TEXT NOT NULL DEFAULT 'pg.truth.reconcile_candidate_reviews'
        CHECK (operation_id = 'pg.truth.reconcile_candidate_reviews'),
    operation_revision     INTEGER NOT NULL DEFAULT 1 CHECK (operation_revision = 1),
    contract_fingerprint   TEXT NOT NULL CHECK (length(contract_fingerprint) = 64),
    authorized_effect      TEXT NOT NULL DEFAULT 'pg.truth.review_queue.reconcile'
        CHECK (authorized_effect = 'pg.truth.review_queue.reconcile'),
    -- SHA-256 of scoped candidates, both source/evidence sections and parent
    -- page source identity/bytes, ordered by candidate ID (see postgres_store
    -- truth_review_effect_v3::CANDIDATE_SET_HASH_SQL).
    candidate_set_hash     TEXT NOT NULL CHECK (length(candidate_set_hash) = 64),
    issued_by              TEXT NOT NULL CHECK (length(trim(issued_by)) > 0 AND issued_by <> 'seo_system'),
    issued_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at             TIMESTAMPTZ NOT NULL,
    revoked_at             TIMESTAMPTZ,
    -- V4: workflow_id/run_id above refer to the CHILD Temporal Workflow.
    -- `pipeline.execution_runs.workflow_run_id` stays the canonical PARENT
    -- Temporal Run ID. This immutable operator attestation binds both actual
    -- server Histories and one CheckedDispatch to one exact child invocation.
    parent_workflow_id           TEXT,
    parent_workflow_run_id       TEXT,
    dispatch_execution_key       TEXT,
    dispatch_input_hash          TEXT,
    invocation_sha256            TEXT,
    bound_input_sha256           TEXT,
    owner_command_sha256         TEXT,
    temporal_attestation_sha256  TEXT,
    temporal_child_started_event_id BIGINT,
    UNIQUE (workflow_execution_run_id, operation_instance),
    CONSTRAINT pg_effect_v4_admission_revision_check
        CHECK (admission_schema_revision IN (3,4)),
    CONSTRAINT pg_effect_v3_expected_execution_key_shape
        CHECK (v3_expected_execution_key IS NULL OR length(v3_expected_execution_key) = 64),
    CONSTRAINT pg_effect_v4_admission_shape CHECK (
        (admission_schema_revision = 3 AND
            parent_workflow_id IS NULL AND parent_workflow_run_id IS NULL AND
            dispatch_execution_key IS NULL AND dispatch_input_hash IS NULL AND
            invocation_sha256 IS NULL AND bound_input_sha256 IS NULL AND
            owner_command_sha256 IS NULL AND temporal_attestation_sha256 IS NULL AND
            temporal_child_started_event_id IS NULL)
        OR (admission_schema_revision = 4 AND
            num_nonnulls(parent_workflow_id, parent_workflow_run_id,
                dispatch_execution_key, dispatch_input_hash,
                invocation_sha256, bound_input_sha256, owner_command_sha256,
                temporal_attestation_sha256, temporal_child_started_event_id) = 9 AND
            length(trim(parent_workflow_id)) > 0 AND
            length(trim(parent_workflow_run_id)) > 0 AND
            length(dispatch_execution_key) = 64 AND
            length(dispatch_input_hash) = 64 AND
            length(invocation_sha256) = 64 AND
            length(bound_input_sha256) = 64 AND
            length(owner_command_sha256) = 64 AND
            length(temporal_attestation_sha256) = 64 AND
            temporal_child_started_event_id > 0)
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_pg_effect_v4_parent_instance
    ON pipeline.pg_effect_v3_admissions(run_id, plan_hash, operation_instance)
    WHERE admission_schema_revision = 4;

CREATE TABLE IF NOT EXISTS pipeline.pg_effect_v3_receipts (
    receipt_id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    admission_id           UUID NOT NULL UNIQUE REFERENCES pipeline.pg_effect_v3_admissions(admission_id),
    admission_schema_revision SMALLINT NOT NULL DEFAULT 3
        CONSTRAINT pg_effect_v4_receipt_revision_check
        CHECK (admission_schema_revision IN (3,4)),
    execution_key          TEXT NOT NULL UNIQUE CHECK (length(execution_key) = 64),
    operation_id           TEXT NOT NULL CHECK (operation_id = 'pg.truth.reconcile_candidate_reviews'),
    operation_revision     INTEGER NOT NULL CHECK (operation_revision = 1),
    run_id                 UUID NOT NULL REFERENCES pipeline.execution_runs(run_id),
    context_key            TEXT NOT NULL,
    candidate_set_hash     TEXT NOT NULL,
    created_review_tasks   BIGINT NOT NULL CHECK (created_review_tasks >= 0),
    open_review_tasks      BIGINT NOT NULL CHECK (open_review_tasks >= 0),
    committed_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- These explicit revokes do not override table ownership or membership in a
-- privileged role. Runtime validates effective grants and FAILS CLOSED.
REVOKE INSERT, UPDATE, DELETE, TRUNCATE ON pipeline.pg_effect_v3_admissions FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE, TRUNCATE ON pipeline.pg_effect_v3_receipts FROM PUBLIC;

-- V3 operator admissions are append-only authority. Every allowed UPDATE
-- obtains the admission advisory try-lock or immediately fails with 55P03.
-- NEVER block here: PostgreSQL may take a key-row lock for an UPDATE that
-- merely *mentions* a key column, even if NEW = OLD. Waiting on the advisory
-- lock in that case could deadlock with the Activity receipt's FK KEY SHARE.
-- Operators retry a busy revocation after the effect+receipt COMMIT. All
-- authority fields (including the original expiry) are immutable.
-- This invoker trigger is NOT a privileged API and cannot grant access.
CREATE OR REPLACE FUNCTION pipeline.guard_pg_effect_v3_admission_mutation()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'TRUNCATE' THEN
        RAISE EXCEPTION 'V3 operator admissions cannot be truncated'
            USING ERRCODE = '23514';
    END IF;

    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'V3 operator admissions cannot be deleted'
            USING ERRCODE = '23514';
    END IF;

    IF ROW(NEW.admission_id, NEW.run_id, NEW.tenant_id, NEW.context_key,
           NEW.admission_schema_revision,
           NEW.v3_expected_execution_key,
           NEW.workflow_id, NEW.workflow_execution_run_id, NEW.operation_instance,
           NEW.plan_hash, NEW.operation_id, NEW.operation_revision,
           NEW.contract_fingerprint, NEW.authorized_effect, NEW.candidate_set_hash,
           NEW.issued_by, NEW.issued_at, NEW.expires_at,
           NEW.parent_workflow_id, NEW.parent_workflow_run_id,
           NEW.dispatch_execution_key, NEW.dispatch_input_hash,
           NEW.invocation_sha256, NEW.bound_input_sha256,
           NEW.owner_command_sha256, NEW.temporal_attestation_sha256,
           NEW.temporal_child_started_event_id)
       IS DISTINCT FROM
       ROW(OLD.admission_id, OLD.run_id, OLD.tenant_id, OLD.context_key,
           OLD.admission_schema_revision,
           OLD.v3_expected_execution_key,
           OLD.workflow_id, OLD.workflow_execution_run_id, OLD.operation_instance,
           OLD.plan_hash, OLD.operation_id, OLD.operation_revision,
           OLD.contract_fingerprint, OLD.authorized_effect, OLD.candidate_set_hash,
           OLD.issued_by, OLD.issued_at, OLD.expires_at,
           OLD.parent_workflow_id, OLD.parent_workflow_run_id,
           OLD.dispatch_execution_key, OLD.dispatch_input_hash,
           OLD.invocation_sha256, OLD.bound_input_sha256,
           OLD.owner_command_sha256, OLD.temporal_attestation_sha256,
           OLD.temporal_child_started_event_id)
       OR (OLD.revoked_at IS NOT NULL
           AND NEW.revoked_at IS DISTINCT FROM OLD.revoked_at)
    THEN
        RAISE EXCEPTION 'V3 admission authority and completed revocation are immutable'
            USING ERRCODE = '23514';
    END IF;

    IF NOT pg_try_advisory_xact_lock(
        hashtextextended('pg-v3-admission|' || OLD.admission_id::text, 0)
    ) THEN
        RAISE EXCEPTION 'V3 admission effect in flight; retry revocation'
            USING ERRCODE = '55P03';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS guard_pg_effect_v3_admission_update_delete
    ON pipeline.pg_effect_v3_admissions;
CREATE TRIGGER guard_pg_effect_v3_admission_update_delete
BEFORE UPDATE OR DELETE ON pipeline.pg_effect_v3_admissions
FOR EACH ROW EXECUTE FUNCTION pipeline.guard_pg_effect_v3_admission_mutation();

DROP TRIGGER IF EXISTS guard_pg_effect_v3_admission_truncate
    ON pipeline.pg_effect_v3_admissions;
CREATE TRIGGER guard_pg_effect_v3_admission_truncate
BEFORE TRUNCATE ON pipeline.pg_effect_v3_admissions
FOR EACH STATEMENT EXECUTE FUNCTION pipeline.guard_pg_effect_v3_admission_mutation();

CREATE OR REPLACE FUNCTION pipeline.pg_truth_candidate_set_hash_v1(p_context_key TEXT)
RETURNS TEXT LANGUAGE sql STABLE SECURITY INVOKER
SET search_path = pg_catalog AS $$
    SELECT encode(public.digest(COALESCE(jsonb_agg(
        jsonb_build_object(
            'candidate', to_jsonb(c),
            'raw_section', to_jsonb(s),
            'evidence_section', to_jsonb(es),
            'raw_page', CASE WHEN p.id IS NULL THEN NULL ELSE jsonb_build_object(
                'id', p.id, 'url', p.url, 'domain', p.domain, 'dtype', p.dtype,
                'status_code', p.status_code, 'content_type', p.content_type,
                'snapshot_at', p.snapshot_at, 'crawled_at', p.crawled_at,
                'final_url', p.final_url, 'title', p.title,
                'meta_desc', p.meta_desc, 'canonical', p.canonical,
                'word_count', p.word_count, 'content_hash', p.content_hash,
                'raw_html_bytes', p.raw_html_bytes,
                'content_sha256', CASE WHEN p.content IS NULL THEN NULL
                    ELSE encode(public.digest(p.content::text, 'sha256'), 'hex') END,
                'raw_html_sha256', CASE WHEN p.raw_html IS NULL THEN NULL
                    ELSE encode(public.digest(p.raw_html, 'sha256'), 'hex') END,
                'redirect_chain', p.redirect_chain, 'robots_trace', p.robots_trace,
                'source_observation', p.source_observation
            ) END,
            'evidence_page', CASE WHEN ep.id IS NULL THEN NULL ELSE jsonb_build_object(
                'id', ep.id, 'url', ep.url, 'domain', ep.domain, 'dtype', ep.dtype,
                'status_code', ep.status_code, 'content_type', ep.content_type,
                'snapshot_at', ep.snapshot_at, 'crawled_at', ep.crawled_at,
                'final_url', ep.final_url, 'title', ep.title,
                'meta_desc', ep.meta_desc, 'canonical', ep.canonical,
                'word_count', ep.word_count, 'content_hash', ep.content_hash,
                'raw_html_bytes', ep.raw_html_bytes,
                'content_sha256', CASE WHEN ep.content IS NULL THEN NULL
                    ELSE encode(public.digest(ep.content::text, 'sha256'), 'hex') END,
                'raw_html_sha256', CASE WHEN ep.raw_html IS NULL THEN NULL
                    ELSE encode(public.digest(ep.raw_html, 'sha256'), 'hex') END,
                'redirect_chain', ep.redirect_chain, 'robots_trace', ep.robots_trace,
                'source_observation', ep.source_observation
            ) END
        ) ORDER BY c.rule_candidate_id)::text, '[]'), 'sha256'), 'hex')
    FROM extracted.rule_candidates AS c
    LEFT JOIN raw.sections AS s ON s.id = c.raw_section_id
    LEFT JOIN raw.sections AS es ON es.id = c.evidence_section_id
    LEFT JOIN raw.pages AS p ON p.id = s.page_id
    LEFT JOIN raw.pages AS ep ON ep.id = es.page_id
    WHERE c.context_key = p_context_key
$$;

CREATE OR REPLACE FUNCTION site.reconcile_truth_candidate_reviews_v1(
    p_context_key TEXT, p_run_id TEXT
) RETURNS BIGINT LANGUAGE plpgsql SECURITY INVOKER
SET search_path = pg_catalog AS $$
DECLARE
    created_count BIGINT;
BEGIN
    IF length(btrim(p_context_key)) = 0 OR length(btrim(p_run_id)) = 0 THEN
        RAISE EXCEPTION 'truth HITL context and run are required'
            USING ERRCODE = '22023';
    END IF;
    UPDATE site.seo_hitl_tasks AS t
    SET queue_state = 'superseded',
        audit_log_payload = t.audit_log_payload || jsonb_build_array(
            jsonb_build_object('event', 'truth_candidate_rechecked',
                'candidate_status', c.epistemic_status,
                'run_id', p_run_id, 'at', now())),
        updated_at = now()
    FROM extracted.rule_candidates AS c
    WHERE t.task_type = 'truth_candidate_review'
      AND t.scope_signature = p_context_key
      AND t.blocking_execution_key = c.rule_candidate_id
      AND t.queue_state IN ('open', 'in_review', 'reopened')
      AND (
          (t.decision_payload->>'source_snapshot_hash')
              IS DISTINCT FROM c.source_snapshot_hash
          OR (c.epistemic_status <> 'needs_hitl'
              AND NOT (
                  c.epistemic_status = 'rejected'
                  AND t.decision_payload->'review'->>'decision' = 'reject'
              ))
      );

    UPDATE site.seo_hitl_tasks AS t
    SET queue_state = 'superseded',
        audit_log_payload = t.audit_log_payload || jsonb_build_array(
            jsonb_build_object('event', 'truth_candidate_source_missing',
                'run_id', p_run_id, 'at', now())),
        updated_at = now()
    WHERE t.task_type = 'truth_candidate_review'
      AND t.scope_signature = p_context_key
      AND t.queue_state IN ('open', 'in_review', 'reopened')
      AND NOT EXISTS (
          SELECT 1 FROM extracted.rule_candidates AS c
          WHERE c.rule_candidate_id = t.blocking_execution_key
            AND c.context_key = p_context_key
      );

    UPDATE site.seo_hitl_tasks AS t
    SET queue_state = 'open',
        current_owner_role = 'truth_reviewer',
        decision_payload = (
            t.decision_payload || jsonb_build_object(
                'raw_section_id', c.raw_section_id,
                'role', c.role, 'concept', c.concept_canonical_key,
                'params', c.params, 'scope', c.scope,
                'source_url', c.source_key,
                'source_snapshot_hash', c.source_snapshot_hash,
                'evidence_quote', c.evidence_quote,
                'span_start', c.span_start, 'span_end', c.span_end,
                'llm_provider', c.llm_provider, 'llm_model', c.llm_model,
                'prompt_version', c.prompt_version,
                'epistemic_status', 'needs_hitl', 'recheck_required', true)
        ) - 'review',
        audit_log_payload = t.audit_log_payload || jsonb_build_array(
            jsonb_build_object('event', 'truth_candidate_reopened_for_review',
                'run_id', p_run_id, 'at', now())),
        updated_at = now()
    FROM extracted.rule_candidates AS c
    WHERE t.task_key = 'truth_candidate_review@1:' ||
            c.rule_candidate_id || ':' || c.source_snapshot_hash
      AND t.task_type = 'truth_candidate_review'
      AND t.scope_signature = p_context_key
      AND c.context_key = p_context_key
      AND c.epistemic_status = 'needs_hitl'
      AND (
          t.queue_state = 'superseded'
          OR (t.decision_payload->>'raw_section_id')::bigint
              IS DISTINCT FROM c.raw_section_id
          OR (t.decision_payload->>'role') IS DISTINCT FROM c.role
          OR (t.decision_payload->>'concept') IS DISTINCT FROM c.concept_canonical_key
          OR t.decision_payload->'params' IS DISTINCT FROM c.params
          OR t.decision_payload->'scope' IS DISTINCT FROM c.scope
          OR (t.decision_payload->>'source_url') IS DISTINCT FROM c.source_key
          OR (t.decision_payload->>'evidence_quote') IS DISTINCT FROM c.evidence_quote
          OR (t.decision_payload->>'span_start')::integer IS DISTINCT FROM c.span_start
          OR (t.decision_payload->>'span_end')::integer IS DISTINCT FROM c.span_end
          OR (t.decision_payload->>'llm_provider') IS DISTINCT FROM c.llm_provider
          OR (t.decision_payload->>'llm_model') IS DISTINCT FROM c.llm_model
          OR (t.decision_payload->>'prompt_version') IS DISTINCT FROM c.prompt_version
      );

    INSERT INTO site.seo_hitl_tasks
        (task_key, task_type, queue_state, first_owner_role, current_owner_role,
         scope_signature, blocking_step_name, blocking_execution_key,
         severity, decision_payload, audit_log_payload)
    SELECT
        'truth_candidate_review@1:' || c.rule_candidate_id || ':' || c.source_snapshot_hash,
        'truth_candidate_review', 'open', 'truth_reviewer', 'truth_reviewer',
        c.context_key, 'truth_adjudication', c.rule_candidate_id,
        'blocking',
        jsonb_build_object(
            'schema_version', 1, 'candidate_id', c.rule_candidate_id,
            'context_key', c.context_key, 'raw_section_id', c.raw_section_id,
            'role', c.role, 'concept', c.concept_canonical_key,
            'params', c.params, 'scope', c.scope,
            'source_url', c.source_key,
            'source_snapshot_hash', c.source_snapshot_hash,
            'evidence_quote', c.evidence_quote,
            'span_start', c.span_start, 'span_end', c.span_end,
            'llm_provider', c.llm_provider, 'llm_model', c.llm_model,
            'prompt_version', c.prompt_version, 'epistemic_status', 'needs_hitl',
            'recheck_required', true),
        jsonb_build_array(jsonb_build_object(
            'event', 'truth_candidate_needs_review',
            'run_id', p_run_id,
            'source_snapshot_hash', c.source_snapshot_hash))
    FROM extracted.rule_candidates c
    WHERE c.context_key = p_context_key
      AND c.epistemic_status = 'needs_hitl'
    ON CONFLICT (task_key) DO NOTHING;
    GET DIAGNOSTICS created_count = ROW_COUNT;
    RETURN created_count;
END;
$$;

CREATE OR REPLACE FUNCTION pipeline.consume_pg_truth_review_v1(
    p_admission_id UUID,
    p_run_id UUID,
    p_tenant_id TEXT,
    p_context_key TEXT,
    p_workflow_id TEXT,
    p_workflow_run_id TEXT,
    p_operation_instance TEXT,
    p_plan_hash TEXT,
    p_contract_fingerprint TEXT,
    p_execution_key TEXT,
    p_admission_revision SMALLINT,
    p_parent_workflow_id TEXT,
    p_parent_workflow_run_id TEXT,
    p_dispatch_input_hash TEXT,
    p_invocation_bytes BYTEA,
    p_bound_input_bytes BYTEA,
    p_owner_command_bytes BYTEA
) RETURNS TABLE (
    receipt_id UUID, admission_id UUID, execution_key TEXT, run_id UUID,
    context_key TEXT, candidate_set_hash TEXT,
    created_review_tasks BIGINT, open_review_tasks BIGINT
) LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE
    a RECORD;
    v_newer_truth_done BOOLEAN;
    v_initial_refs JSONB;
    v_locked_refs JSONB;
    v_page BIGINT;
    v_section BIGINT;
    v_source_hash TEXT;
    v_created BIGINT;
    v_open BIGINT;
    v_receipt_key TEXT;
BEGIN
    -- SQL SECURITY DEFINER bypasses the caller's RLS. Never let a worker
    -- consume another tenant's receipt, including an old replay that returns
    -- before the receipt INSERT trigger. A custom GUC is not an identity.
    IF pipeline.current_session_tenant_v1() IS DISTINCT FROM p_tenant_id
       AND NOT EXISTS (
           SELECT 1 FROM pg_catalog.pg_roles r
           WHERE r.rolname = session_user AND r.rolsuper
       ) THEN
        RAISE EXCEPTION 'authenticated PostgreSQL LOGIN cannot consume this tenant effect'
            USING ERRCODE = 'AV001';
    END IF;
    IF p_admission_revision NOT IN (3,4)
       OR length(p_execution_key) <> 64
       OR length(p_plan_hash) <> 64
       OR length(p_contract_fingerprint) <> 64 THEN
        RAISE EXCEPTION 'unsupported or malformed admitted Truth effect'
            USING ERRCODE = 'AV001';
    END IF;

    PERFORM pg_advisory_xact_lock(
        hashtextextended('truth-context|' || p_context_key, 0));
    PERFORM pg_advisory_xact_lock(
        hashtextextended('pg-v3-admission|' || p_admission_id::text, 0));
    SELECT auth.*, er.status AS run_status INTO a
    FROM pipeline.pg_effect_v3_admissions AS auth
    JOIN pipeline.execution_runs AS er ON er.run_id = auth.run_id
    WHERE auth.admission_id = p_admission_id
      AND auth.admission_schema_revision = p_admission_revision
      AND auth.run_id = p_run_id
      AND auth.tenant_id = p_tenant_id
      AND auth.context_key = p_context_key
      AND auth.workflow_id = p_workflow_id
      AND auth.workflow_execution_run_id = p_workflow_run_id
      AND auth.operation_instance = p_operation_instance
      AND auth.plan_hash = p_plan_hash
      AND auth.contract_fingerprint = p_contract_fingerprint
      AND auth.operation_id = 'pg.truth.reconcile_candidate_reviews'
      AND auth.operation_revision = 1
      AND auth.authorized_effect = 'pg.truth.review_queue.reconcile'
      AND er.context_key = auth.context_key
      AND er.workflow_type = 'seo_site_build'
      AND (
          (p_admission_revision = 3 AND
              er.workflow_run_id = auth.workflow_execution_run_id AND
              (auth.v3_expected_execution_key = p_execution_key OR
                 (auth.v3_expected_execution_key IS NULL AND EXISTS (
                     SELECT 1 FROM pipeline.pg_effect_v3_receipts historic
                     WHERE historic.admission_id = auth.admission_id
                       AND historic.execution_key = p_execution_key
                       AND historic.admission_schema_revision = 3
                 ))) AND
              p_parent_workflow_id IS NULL AND p_parent_workflow_run_id IS NULL AND
              p_dispatch_input_hash IS NULL AND p_invocation_bytes IS NULL AND
              p_bound_input_bytes IS NULL AND p_owner_command_bytes IS NULL)
          OR (p_admission_revision = 4 AND
              er.workflow_run_id = auth.parent_workflow_run_id AND
              auth.v3_expected_execution_key IS NULL AND
              auth.parent_workflow_id = p_parent_workflow_id AND
              auth.parent_workflow_run_id = p_parent_workflow_run_id AND
              auth.dispatch_execution_key = p_execution_key AND
              auth.dispatch_input_hash = p_dispatch_input_hash AND
              auth.invocation_sha256 = encode(public.digest(p_invocation_bytes, 'sha256'), 'hex') AND
              auth.bound_input_sha256 = encode(public.digest(p_bound_input_bytes, 'sha256'), 'hex') AND
              auth.owner_command_sha256 = encode(public.digest(p_owner_command_bytes, 'sha256'), 'hex'))
      );
    IF NOT FOUND THEN
        RAISE EXCEPTION 'operator admission or canonical Temporal run denied'
            USING ERRCODE = 'AV001';
    END IF;
    -- Receipt replay is allowed after expiry/revocation, never with a
    -- different exact execution key or authorization revision.
    SELECT r.execution_key INTO v_receipt_key
    FROM pipeline.pg_effect_v3_receipts r
    WHERE r.admission_id = p_admission_id;
    IF FOUND THEN
        IF v_receipt_key <> p_execution_key THEN
            RAISE EXCEPTION 'effect receipt execution key mismatch'
                USING ERRCODE = 'AV001';
        END IF;
        RETURN QUERY SELECT r.receipt_id, r.admission_id, r.execution_key,
            r.run_id, r.context_key, r.candidate_set_hash,
            r.created_review_tasks, r.open_review_tasks
        FROM pipeline.pg_effect_v3_receipts r
        WHERE r.admission_id = p_admission_id
          AND r.admission_schema_revision = p_admission_revision;
        RETURN;
    END IF;
    -- Historic V3 admissions predating the immutable expected-key column may
    -- REPLAY an already-committed receipt above; they must never originate a
    -- new effect with a caller-chosen receipt execution key.
    IF p_admission_revision = 3 AND a.v3_expected_execution_key IS NULL THEN
        RAISE EXCEPTION 'historical admission lacks pinned V3 receipt key'
            USING ERRCODE = 'AV001';
    END IF;
    -- The legacy table predated per-parent instance uniqueness. A V3 and V4
    -- admission (or two completed same-revision admissions) must not each
    -- emit the same business effect under distinct child Workflow Run IDs.
    -- The context lock above serializes this check with every consuming
    -- Activity. Historical receipt replay returned before this guard.
    IF EXISTS (
        SELECT 1 FROM pipeline.pg_effect_v3_admissions other
        LEFT JOIN pipeline.pg_effect_v3_receipts other_receipt
            ON other_receipt.admission_id = other.admission_id
        WHERE other.run_id = a.run_id
          AND other.plan_hash = a.plan_hash
          AND other.operation_instance = a.operation_instance
          AND other.admission_id <> a.admission_id
          AND (other_receipt.receipt_id IS NOT NULL
               OR (other.admission_schema_revision <> a.admission_schema_revision
                   AND other.revoked_at IS NULL
                   AND other.expires_at > clock_timestamp()))
    ) THEN
        RAISE EXCEPTION 'competing versioned admission or completed instance effect'
            USING ERRCODE = 'AV001';
    END IF;
    IF a.revoked_at IS NOT NULL OR a.expires_at <= clock_timestamp() THEN
        RAISE EXCEPTION 'operator admission expired or revoked'
            USING ERRCODE = 'AV001';
    END IF;
    IF a.run_status NOT IN ('verifying','pending_hitl') THEN
        RAISE EXCEPTION 'canonical Truth run is no longer active'
            USING ERRCODE = 'TR001';
    END IF;
    SELECT EXISTS (
        SELECT 1 FROM pipeline.execution_runs newer
        JOIN pipeline.execution_runs this_run ON this_run.run_id = p_run_id
        JOIN pipeline.step_executions done ON done.run_id = newer.run_id
            AND done.step_name = 'verified_truth_write' AND done.status = 'done'
        WHERE newer.context_key = this_run.context_key
          AND newer.run_id <> this_run.run_id
          AND newer.created_at >= this_run.created_at
    ) INTO v_newer_truth_done;
    IF v_newer_truth_done THEN
        RAISE EXCEPTION 'newer verified Truth supersedes this run'
            USING ERRCODE = 'TR001';
    END IF;

    SELECT COALESCE(jsonb_agg(jsonb_build_array(ref.id, ref.page_id)
        ORDER BY ref.id, ref.page_id), '[]'::jsonb) INTO v_initial_refs
    FROM (
        SELECT DISTINCT s.id, s.page_id
        FROM extracted.rule_candidates c
        JOIN raw.sections s ON s.id IN (c.raw_section_id, c.evidence_section_id)
        WHERE c.context_key = p_context_key
    ) ref;
    FOR v_page IN
        SELECT DISTINCT s.page_id
        FROM extracted.rule_candidates c
        JOIN raw.sections s ON s.id IN (c.raw_section_id, c.evidence_section_id)
        WHERE c.context_key = p_context_key
        ORDER BY s.page_id
    LOOP
        PERFORM pg_advisory_xact_lock(
            hashtextextended('raw-evidence|pages|' || v_page::text, 0));
    END LOOP;
    FOR v_section IN
        SELECT DISTINCT s.id
        FROM extracted.rule_candidates c
        JOIN raw.sections s ON s.id IN (c.raw_section_id, c.evidence_section_id)
        WHERE c.context_key = p_context_key
        ORDER BY s.id
    LOOP
        PERFORM pg_advisory_xact_lock(
            hashtextextended('raw-evidence|sections|' || v_section::text, 0));
    END LOOP;
    SELECT COALESCE(jsonb_agg(jsonb_build_array(ref.id, ref.page_id)
        ORDER BY ref.id, ref.page_id), '[]'::jsonb) INTO v_locked_refs
    FROM (
        SELECT DISTINCT s.id, s.page_id
        FROM extracted.rule_candidates c
        JOIN raw.sections s ON s.id IN (c.raw_section_id, c.evidence_section_id)
        WHERE c.context_key = p_context_key
    ) ref;
    IF v_initial_refs IS DISTINCT FROM v_locked_refs THEN
        RAISE EXCEPTION 'raw source references changed while locking'
            USING ERRCODE = 'CS001';
    END IF;
    SELECT pipeline.pg_truth_candidate_set_hash_v1(p_context_key)
    INTO v_source_hash;
    IF v_source_hash <> a.candidate_set_hash THEN
        RAISE EXCEPTION 'candidate source snapshot changed since admission'
            USING ERRCODE = 'CS001';
    END IF;

    -- No DML grant to caller is needed; all three writes are within this
    -- single SECURITY DEFINER transaction and roll back together on error.
    SELECT site.reconcile_truth_candidate_reviews_v1(
        p_context_key, p_run_id::text) INTO v_created;
    SELECT count(*)::bigint INTO v_open
    FROM site.seo_hitl_tasks t
    WHERE t.scope_signature = p_context_key
      AND t.task_type = 'truth_candidate_review'
      AND t.queue_state IN ('open','reopened','in_review');
    INSERT INTO pipeline.pg_effect_v3_receipts
        (admission_id, admission_schema_revision, execution_key,
         operation_id, operation_revision, run_id, context_key,
         candidate_set_hash, created_review_tasks, open_review_tasks)
    VALUES (p_admission_id, p_admission_revision, p_execution_key,
            'pg.truth.reconcile_candidate_reviews', 1, p_run_id, p_context_key,
            v_source_hash, v_created, v_open);
    RETURN QUERY SELECT r.receipt_id, r.admission_id, r.execution_key,
        r.run_id, r.context_key, r.candidate_set_hash,
        r.created_review_tasks, r.open_review_tasks
    FROM pipeline.pg_effect_v3_receipts r
    WHERE r.admission_id = p_admission_id;
END;
$$;

REVOKE ALL ON FUNCTION pipeline.consume_pg_truth_review_v1(
    UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT,SMALLINT,
    TEXT,TEXT,TEXT,BYTEA,BYTEA,BYTEA
) FROM PUBLIC;

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

CREATE OR REPLACE FUNCTION pipeline.guard_verified_truth_step_context()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog AS $$
DECLARE
    old_context TEXT;
    new_context TEXT;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF ROW(OLD.run_id, OLD.step_name, OLD.status)
           IS NOT DISTINCT FROM ROW(NEW.run_id, NEW.step_name, NEW.status) THEN
            RETURN NEW;
        END IF;
        IF OLD.step_name <> 'verified_truth_write'
           AND NEW.step_name <> 'verified_truth_write' THEN
            RETURN NEW;
        END IF;
    ELSIF TG_OP = 'INSERT' THEN
        IF NEW.step_name <> 'verified_truth_write' THEN
            RETURN NEW;
        END IF;
    ELSE
        IF OLD.step_name <> 'verified_truth_write' THEN
            RETURN OLD;
        END IF;
    END IF;

    IF TG_OP <> 'INSERT' AND OLD.step_name = 'verified_truth_write' THEN
        SELECT context_key INTO old_context
        FROM pipeline.execution_runs WHERE run_id = OLD.run_id;
    END IF;
    IF TG_OP <> 'DELETE' AND NEW.step_name = 'verified_truth_write' THEN
        SELECT context_key INTO new_context
        FROM pipeline.execution_runs WHERE run_id = NEW.run_id;
    END IF;
    -- During an execution_run DELETE CASCADE, the parent may already be
    -- gone within this transaction. A deleted canonical run cannot authorize
    -- an admitted effect (its FK admission prevents such deletion).
    IF old_context IS NULL AND new_context IS NULL THEN
        IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
        RETURN NEW;
    END IF;
    IF old_context IS NULL THEN old_context := new_context; END IF;
    IF new_context IS NULL THEN new_context := old_context; END IF;

    -- A row-level writer already owns a step row lock. Fail fast rather than
    -- holding it waiting on an Activity transaction that reads chronology.
    IF old_context <= new_context THEN
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || old_context, 0)) THEN
            RAISE EXCEPTION 'verified Truth chronology is locked by admitted effect'
                USING ERRCODE = '55P03';
        END IF;
        IF old_context <> new_context AND NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || new_context, 0)) THEN
            RAISE EXCEPTION 'destination Truth chronology is locked'
                USING ERRCODE = '55P03';
        END IF;
    ELSE
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || new_context, 0)) THEN
            RAISE EXCEPTION 'destination Truth chronology is locked'
                USING ERRCODE = '55P03';
        END IF;
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || old_context, 0)) THEN
            RAISE EXCEPTION 'verified Truth chronology is locked by admitted effect'
                USING ERRCODE = '55P03';
        END IF;
    END IF;
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS guard_verified_truth_step_context ON pipeline.step_executions;
CREATE TRIGGER guard_verified_truth_step_context
BEFORE INSERT OR UPDATE OR DELETE ON pipeline.step_executions
FOR EACH ROW EXECUTE FUNCTION pipeline.guard_verified_truth_step_context();

-- Migration 20261125_0006: a concurrent operator may independently edit
-- Truth HITL queue rows. Serialize those edits with the admitted Activity's
-- source/HITL/receipt transaction using the same per-context advisory key.
CREATE OR REPLACE FUNCTION site.guard_truth_review_hitl_context()
-- This must see ALL tenants during the statement-level TRUNCATE check:
-- RLS-filtered EXISTS could treat another tenant's queue as empty while
-- TRUNCATE itself bypasses row filtering. A database-owner definer sees all
-- rows; row_security=off also fails closed if FORCE RLS is later introduced.
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = off AS $$
DECLARE
    previous_scope TEXT;
    next_scope TEXT;
BEGIN
    IF TG_OP = 'TRUNCATE' THEN
        -- PostgreSQL's TRUNCATE CASCADE also fires on an *empty* referencing
        -- table during isolated fixture resets. No Truth history can be
        -- erased when there are zero rows; ACCESS EXCLUSIVE serializes a
        -- concurrent writer. Never permit truncating a nonempty HITL queue.
        IF EXISTS (SELECT 1 FROM site.seo_hitl_tasks) THEN
            RAISE EXCEPTION 'Truth HITL history cannot be truncated around admitted effects'
                USING ERRCODE = '23514';
        END IF;
        RETURN NULL;
    END IF;
    IF TG_OP <> 'INSERT' AND OLD.task_type = 'truth_candidate_review' THEN
        previous_scope := OLD.scope_signature;
    END IF;
    IF TG_OP <> 'DELETE' AND NEW.task_type = 'truth_candidate_review' THEN
        next_scope := NEW.scope_signature;
    END IF;
    IF previous_scope IS NULL AND next_scope IS NULL THEN
        IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
        RETURN NEW;
    END IF;
    IF (previous_scope IS NOT NULL AND length(btrim(previous_scope)) = 0)
        OR (next_scope IS NOT NULL AND length(btrim(next_scope)) = 0) THEN
        RAISE EXCEPTION 'Truth HITL context must be nonempty'
            USING ERRCODE = '23514';
    END IF;
    IF previous_scope IS NULL THEN previous_scope := next_scope; END IF;
    IF next_scope IS NULL THEN next_scope := previous_scope; END IF;
    IF previous_scope <= next_scope THEN
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || previous_scope, 0)) THEN
            RAISE EXCEPTION 'Truth HITL context is locked by admitted effect'
                USING ERRCODE = '55P03';
        END IF;
        IF previous_scope <> next_scope AND NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || next_scope, 0)) THEN
            RAISE EXCEPTION 'destination Truth HITL context is locked'
                USING ERRCODE = '55P03';
        END IF;
    ELSE
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || next_scope, 0)) THEN
            RAISE EXCEPTION 'destination Truth HITL context is locked'
                USING ERRCODE = '55P03';
        END IF;
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || previous_scope, 0)) THEN
            RAISE EXCEPTION 'Truth HITL context is locked by admitted effect'
                USING ERRCODE = '55P03';
        END IF;
    END IF;
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS guard_truth_review_hitl_context ON site.seo_hitl_tasks;
CREATE TRIGGER guard_truth_review_hitl_context
BEFORE INSERT OR UPDATE OR DELETE ON site.seo_hitl_tasks
FOR EACH ROW EXECUTE FUNCTION site.guard_truth_review_hitl_context();
DROP TRIGGER IF EXISTS guard_truth_review_hitl_truncate ON site.seo_hitl_tasks;
CREATE TRIGGER guard_truth_review_hitl_truncate
BEFORE TRUNCATE ON site.seo_hitl_tasks
FOR EACH STATEMENT EXECUTE FUNCTION site.guard_truth_review_hitl_context();

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

-- Durable, provider-independent typed manual LLM inbox.
-- Manual LLM responses never imply verified truth or publication approval.
CREATE TABLE IF NOT EXISTS pipeline.manual_llm_jobs (
    task_id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    run_id TEXT NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('editorial_draft', 'truth_candidates')),
    input_sha256 TEXT NOT NULL CHECK (input_sha256 ~ '^[a-f0-9]{64}$'),
    input_json JSONB NOT NULL,
    job_status TEXT NOT NULL DEFAULT 'pending' CHECK (job_status IN ('pending', 'completed', 'superseded')),
    response_json JSONB,
    output_json JSONB,
    model_key TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, operation, input_sha256),
    CHECK (
        (job_status IN ('pending', 'superseded') AND response_json IS NULL AND output_json IS NULL)
        OR
        (job_status = 'completed' AND response_json IS NOT NULL AND output_json IS NOT NULL
         AND model_key IS NOT NULL)
    )
);
CREATE INDEX IF NOT EXISTS idx_manual_llm_jobs_pending
    ON pipeline.manual_llm_jobs (created_at, task_id) WHERE job_status = 'pending';

CREATE UNIQUE INDEX IF NOT EXISTS manual_llm_one_pending_editorial_owner
    ON pipeline.manual_llm_jobs
    (run_id, (input_json #>> '{request,editorial_brief,page_node_key}'))
    WHERE operation = 'editorial_draft' AND job_status = 'pending'
      AND nullif(input_json #>> '{request,editorial_brief,page_node_key}', '') IS NOT NULL;


CREATE TABLE IF NOT EXISTS pipeline.tenant_context_ownership (
    context_key TEXT PRIMARY KEY REFERENCES kb.visa_contexts(context_key) ON UPDATE CASCADE,
    tenant_id TEXT NOT NULL CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0),
    provisioned_by NAME NOT NULL DEFAULT session_user,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX IF NOT EXISTS idx_tenant_context_ownership_tenant
    ON pipeline.tenant_context_ownership(tenant_id, context_key);

CREATE TABLE IF NOT EXISTS pipeline.tenant_role_ownership (
    role_name NAME PRIMARY KEY,
    tenant_id TEXT NOT NULL CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0),
    provisioned_by NAME NOT NULL DEFAULT session_user,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX IF NOT EXISTS idx_tenant_role_ownership_tenant
    ON pipeline.tenant_role_ownership(tenant_id, role_name);

REVOKE ALL ON pipeline.tenant_context_ownership, pipeline.tenant_role_ownership FROM PUBLIC;

-- These administrative bindings cannot silently drift underneath a previously
-- issued admission. Tenant transfers need a separately reviewed drain/rebind.
CREATE OR REPLACE FUNCTION pipeline.deny_tenant_ownership_mutation_v1()
RETURNS trigger LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'immutable tenant ownership; drain and use an explicit transfer protocol'
        USING ERRCODE = 'AV001';
END;
$$;
DROP TRIGGER IF EXISTS tenant_context_owner_immutable ON pipeline.tenant_context_ownership;
CREATE TRIGGER tenant_context_owner_immutable
    BEFORE UPDATE OR DELETE OR TRUNCATE ON pipeline.tenant_context_ownership
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_ownership_mutation_v1();
DROP TRIGGER IF EXISTS tenant_role_owner_immutable ON pipeline.tenant_role_ownership;
CREATE TRIGGER tenant_role_owner_immutable
    BEFORE UPDATE OR DELETE OR TRUNCATE ON pipeline.tenant_role_ownership
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_ownership_mutation_v1();

ALTER TABLE pipeline.execution_runs ADD COLUMN IF NOT EXISTS tenant_id TEXT;
ALTER TABLE raw.pages ADD COLUMN IF NOT EXISTS tenant_id TEXT;
ALTER TABLE raw.sections ADD COLUMN IF NOT EXISTS tenant_id TEXT;
ALTER TABLE extracted.rule_candidates ADD COLUMN IF NOT EXISTS tenant_id TEXT;
ALTER TABLE site.seo_hitl_tasks ADD COLUMN IF NOT EXISTS tenant_id TEXT;
ALTER TABLE pipeline.pg_effect_v3_receipts ADD COLUMN IF NOT EXISTS tenant_id TEXT;

CREATE INDEX IF NOT EXISTS idx_execution_runs_tenant_context
    ON pipeline.execution_runs(tenant_id, context_key, run_id);
CREATE INDEX IF NOT EXISTS idx_raw_pages_tenant ON raw.pages(tenant_id, id);
CREATE INDEX IF NOT EXISTS idx_raw_sections_tenant ON raw.sections(tenant_id, page_id);
CREATE INDEX IF NOT EXISTS idx_rule_candidates_tenant_context
    ON extracted.rule_candidates(tenant_id, context_key);
CREATE INDEX IF NOT EXISTS idx_hitl_tasks_tenant_scope
    ON site.seo_hitl_tasks(tenant_id, scope_signature);

-- A custom GUC is never tenant authority: anybody can SET one. Only the
-- authenticated PostgreSQL LOGIN (session_user), bound by DBA-owned rows, is.
CREATE OR REPLACE FUNCTION pipeline.current_session_tenant_v1()
RETURNS TEXT LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog AS $$
    SELECT ownership.tenant_id
    FROM pipeline.tenant_role_ownership ownership
    WHERE ownership.role_name = session_user::name;
$$;
REVOKE ALL ON FUNCTION pipeline.current_session_tenant_v1() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION pipeline.current_session_tenant_v1() TO PUBLIC;

-- RLS is active for non-owner roles. No permissive NULL/legacy policy exists.
-- Superuser/table-owner bypass remains a distinct production acceptance NO-GO
-- until each legacy database role is replaced with tenant-scoped credentials.
ALTER TABLE pipeline.execution_runs ENABLE ROW LEVEL SECURITY;
ALTER TABLE raw.pages ENABLE ROW LEVEL SECURITY;
ALTER TABLE raw.sections ENABLE ROW LEVEL SECURITY;
ALTER TABLE extracted.rule_candidates ENABLE ROW LEVEL SECURITY;
ALTER TABLE site.seo_hitl_tasks ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.pg_effect_v3_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.pg_effect_v3_receipts ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS execution_runs_tenant_v1 ON pipeline.execution_runs;
CREATE POLICY execution_runs_tenant_v1 ON pipeline.execution_runs
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS raw_pages_tenant_v1 ON raw.pages;
CREATE POLICY raw_pages_tenant_v1 ON raw.pages
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS raw_sections_tenant_v1 ON raw.sections;
CREATE POLICY raw_sections_tenant_v1 ON raw.sections
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS extracted_candidates_tenant_v1 ON extracted.rule_candidates;
CREATE POLICY extracted_candidates_tenant_v1 ON extracted.rule_candidates
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS site_hitl_tasks_tenant_v1 ON site.seo_hitl_tasks;
CREATE POLICY site_hitl_tasks_tenant_v1 ON site.seo_hitl_tasks
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS pg_effect_admissions_tenant_v1 ON pipeline.pg_effect_v3_admissions;
CREATE POLICY pg_effect_admissions_tenant_v1 ON pipeline.pg_effect_v3_admissions
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS pg_effect_receipts_tenant_v1 ON pipeline.pg_effect_v3_receipts;
CREATE POLICY pg_effect_receipts_tenant_v1 ON pipeline.pg_effect_v3_receipts
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

-- Recheck every evidence edge under the same Truth context lock as the
-- admitted capability. A single NULL or cross-tenant source quarantines the
-- entire future effect, including older unassigned HITL tasks.
CREATE OR REPLACE FUNCTION pipeline.tenant_truth_scope_ready_v1(
    p_tenant TEXT, p_context TEXT, p_run UUID
) RETURNS BOOLEAN LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog AS $$
    SELECT EXISTS (
        SELECT 1 FROM pipeline.tenant_context_ownership o
        JOIN pipeline.execution_runs er ON er.context_key = o.context_key
        WHERE o.context_key = p_context AND o.tenant_id = p_tenant
          AND er.run_id = p_run AND er.tenant_id = o.tenant_id
    )
    AND EXISTS (
        SELECT 1 FROM extracted.rule_candidates c
        WHERE c.context_key = p_context AND c.tenant_id = p_tenant
    )
    AND NOT EXISTS (
        SELECT 1 FROM extracted.rule_candidates c
        LEFT JOIN raw.sections source_section ON source_section.id = c.raw_section_id
        LEFT JOIN raw.pages source_page ON source_page.id = source_section.page_id
        LEFT JOIN raw.sections evidence_section ON evidence_section.id = c.evidence_section_id
        LEFT JOIN raw.pages evidence_page ON evidence_page.id = evidence_section.page_id
        WHERE c.context_key = p_context AND (
            c.tenant_id IS DISTINCT FROM p_tenant
            OR source_section.tenant_id IS DISTINCT FROM p_tenant
            OR source_page.tenant_id IS DISTINCT FROM p_tenant
            OR evidence_section.tenant_id IS DISTINCT FROM p_tenant
            OR evidence_page.tenant_id IS DISTINCT FROM p_tenant
        )
    )
    AND NOT EXISTS (
        SELECT 1 FROM site.seo_hitl_tasks t
        WHERE t.task_type = 'truth_candidate_review' AND t.scope_signature = p_context
          AND t.tenant_id IS DISTINCT FROM p_tenant
    );
$$;
REVOKE ALL ON FUNCTION pipeline.tenant_truth_scope_ready_v1(TEXT,TEXT,UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION pipeline.tenant_truth_scope_ready_v1(TEXT,TEXT,UUID) TO PUBLIC;

CREATE OR REPLACE FUNCTION pipeline.enforce_pg_effect_admission_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE is_admin BOOLEAN;
BEGIN
    SELECT rol.rolsuper INTO is_admin FROM pg_catalog.pg_roles rol
        WHERE rol.rolname = session_user;
    IF NOT coalesce(is_admin, false) AND
       pipeline.current_session_tenant_v1() IS DISTINCT FROM NEW.tenant_id THEN
        RAISE EXCEPTION 'authenticated operator PostgreSQL LOGIN is not authorized for tenant'
            USING ERRCODE = 'AV001';
    END IF;
    IF NOT pipeline.tenant_truth_scope_ready_v1(NEW.tenant_id, NEW.context_key, NEW.run_id) THEN
        RAISE EXCEPTION 'canonical tenant run/source ownership not provisioned or stale'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS pg_effect_admission_tenant_v1 ON pipeline.pg_effect_v3_admissions;
CREATE TRIGGER pg_effect_admission_tenant_v1 BEFORE INSERT ON pipeline.pg_effect_v3_admissions
FOR EACH ROW EXECUTE FUNCTION pipeline.enforce_pg_effect_admission_tenant_v1();

-- Internal Truth-review task creation derives tenant only from the DBA-owned
-- canonical context binding, never the command's request/GUC. Existing
-- historical rows remain NULL until an explicit audited backfill.
CREATE OR REPLACE FUNCTION site.bind_new_truth_review_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE owner_tenant TEXT;
BEGIN
    IF NEW.task_type = 'truth_candidate_review' THEN
        SELECT tenant_id INTO owner_tenant FROM pipeline.tenant_context_ownership
            WHERE context_key = NEW.scope_signature;
        IF owner_tenant IS NOT NULL THEN
            IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id <> owner_tenant THEN
                RAISE EXCEPTION 'truth review task tenant differs from canonical context'
                    USING ERRCODE = 'AV001';
            END IF;
            NEW.tenant_id := owner_tenant;
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS bind_new_truth_review_tenant_v1 ON site.seo_hitl_tasks;
CREATE TRIGGER bind_new_truth_review_tenant_v1 BEFORE INSERT ON site.seo_hitl_tasks
FOR EACH ROW EXECUTE FUNCTION site.bind_new_truth_review_tenant_v1();

-- The pre-receipt SQL capability may have created HITL tasks. A denied tenant
-- receipt INSERT raises in that same transaction, rolling back ALL effects.
CREATE OR REPLACE FUNCTION pipeline.enforce_pg_effect_receipt_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE scope RECORD;
BEGIN
    SELECT tenant_id, context_key, run_id INTO scope
    FROM pipeline.pg_effect_v3_admissions a
    WHERE a.admission_id = NEW.admission_id AND a.context_key = NEW.context_key
      AND a.run_id = NEW.run_id;
    IF NOT FOUND OR NOT pipeline.tenant_truth_scope_ready_v1(
        scope.tenant_id, scope.context_key, scope.run_id) THEN
        RAISE EXCEPTION 'receipt refuses unowned or drifted canonical tenant evidence'
            USING ERRCODE = 'AV001';
    END IF;
    IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id <> scope.tenant_id THEN
        RAISE EXCEPTION 'receipt tenant mismatch' USING ERRCODE = 'AV001';
    END IF;
    NEW.tenant_id := scope.tenant_id;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS pg_effect_receipt_tenant_v1 ON pipeline.pg_effect_v3_receipts;
CREATE TRIGGER pg_effect_receipt_tenant_v1 BEFORE INSERT ON pipeline.pg_effect_v3_receipts
FOR EACH ROW EXECUTE FUNCTION pipeline.enforce_pg_effect_receipt_tenant_v1();

-- A tenant transfer cannot race an admission issued from the formerly
-- unowned canonical run. Writer-first and operator-first are serialized by
-- the same run key. No automatic tenant transfer is permitted post-admission.
CREATE OR REPLACE FUNCTION pipeline.guard_execution_run_tenant_transfer_v1()
RETURNS trigger LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF NEW.tenant_id IS NOT DISTINCT FROM OLD.tenant_id THEN
        RETURN NEW;
    END IF;
    IF NOT pg_try_advisory_xact_lock(
        hashtextextended('truth-run-status|' || OLD.run_id::text, 0)) THEN
        RAISE EXCEPTION 'tenant transfer blocked by active admitted Truth operation'
            USING ERRCODE = '55P03';
    END IF;
    IF EXISTS (SELECT 1 FROM pipeline.pg_effect_v3_admissions a
               WHERE a.run_id = OLD.run_id) THEN
        RAISE EXCEPTION 'admitted canonical run cannot change tenant owner'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS guard_execution_run_tenant_transfer_v1
    ON pipeline.execution_runs;
CREATE TRIGGER guard_execution_run_tenant_transfer_v1
BEFORE UPDATE OF tenant_id ON pipeline.execution_runs
FOR EACH ROW EXECUTE FUNCTION pipeline.guard_execution_run_tenant_transfer_v1();

COMMIT;

-- V6: an operator admission pins the authoritative registry status and trust
-- of every candidate's source, not just its raw HTML and extraction rows.
-- A registry editor shares the canonical Truth context advisory lock with
-- the admitted PostgreSQL Activity. This migration does not backfill trust,
-- mint a new admission, or switch the production Temporal Worker.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.pg_truth_candidate_set_hash_v1(p_context_key TEXT)
RETURNS TEXT LANGUAGE sql STABLE SECURITY INVOKER
SET search_path = pg_catalog AS $$
    SELECT encode(public.digest(COALESCE(jsonb_agg(
        jsonb_build_object(
            'candidate', to_jsonb(c),
            'raw_section', to_jsonb(s),
            'evidence_section', to_jsonb(es),
            -- A missing registry row (including a broken historical FK) is
            -- explicit NULL in the digest. Operational updated_at alone is
            -- excluded; trust_level/status/authority_class/independence,
            -- override eligibility, freshness and every other field are
            -- part of the operator's admitted source identity.
            'source_authority', CASE WHEN registry.source_key IS NULL THEN NULL
                ELSE to_jsonb(registry) - 'updated_at' END,
            'raw_page', CASE WHEN p.id IS NULL THEN NULL ELSE jsonb_build_object(
                'id', p.id, 'url', p.url, 'domain', p.domain, 'dtype', p.dtype,
                'status_code', p.status_code, 'content_type', p.content_type,
                'snapshot_at', p.snapshot_at, 'crawled_at', p.crawled_at,
                'final_url', p.final_url, 'title', p.title,
                'meta_desc', p.meta_desc, 'canonical', p.canonical,
                'word_count', p.word_count, 'content_hash', p.content_hash,
                'raw_html_bytes', p.raw_html_bytes,
                'content_sha256', CASE WHEN p.content IS NULL THEN NULL
                    ELSE encode(public.digest(p.content::text, 'sha256'), 'hex') END,
                'raw_html_sha256', CASE WHEN p.raw_html IS NULL THEN NULL
                    ELSE encode(public.digest(p.raw_html, 'sha256'), 'hex') END,
                'redirect_chain', p.redirect_chain, 'robots_trace', p.robots_trace,
                'source_observation', p.source_observation
            ) END,
            'evidence_page', CASE WHEN ep.id IS NULL THEN NULL ELSE jsonb_build_object(
                'id', ep.id, 'url', ep.url, 'domain', ep.domain, 'dtype', ep.dtype,
                'status_code', ep.status_code, 'content_type', ep.content_type,
                'snapshot_at', ep.snapshot_at, 'crawled_at', ep.crawled_at,
                'final_url', ep.final_url, 'title', ep.title,
                'meta_desc', ep.meta_desc, 'canonical', ep.canonical,
                'word_count', ep.word_count, 'content_hash', ep.content_hash,
                'raw_html_bytes', ep.raw_html_bytes,
                'content_sha256', CASE WHEN ep.content IS NULL THEN NULL
                    ELSE encode(public.digest(ep.content::text, 'sha256'), 'hex') END,
                'raw_html_sha256', CASE WHEN ep.raw_html IS NULL THEN NULL
                    ELSE encode(public.digest(ep.raw_html, 'sha256'), 'hex') END,
                'redirect_chain', ep.redirect_chain, 'robots_trace', ep.robots_trace,
                'source_observation', ep.source_observation
            ) END
        ) ORDER BY c.rule_candidate_id)::text, '[]'), 'sha256'), 'hex')
    FROM extracted.rule_candidates AS c
    LEFT JOIN kb.sources AS registry ON registry.source_key = c.source_key
    LEFT JOIN raw.sections AS s ON s.id = c.raw_section_id
    LEFT JOIN raw.sections AS es ON es.id = c.evidence_section_id
    LEFT JOIN raw.pages AS p ON p.id = s.page_id
    LEFT JOIN raw.pages AS ep ON ep.id = es.page_id
    WHERE c.context_key = p_context_key
$$;

CREATE OR REPLACE FUNCTION kb.guard_truth_source_authority_writer_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog
SET row_security = off AS $$
DECLARE
    affected_context TEXT;
BEGIN
    IF TG_OP = 'TRUNCATE' THEN
        -- TRUNCATE bypasses row triggers and can CASCADE to candidates.
        RAISE EXCEPTION 'source authority registry cannot be truncated'
            USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'UPDATE' AND
       (to_jsonb(NEW) - 'updated_at') IS NOT DISTINCT FROM
       (to_jsonb(OLD) - 'updated_at') THEN
        -- Registry bookkeeping, not an admission-authority change.
        RETURN NEW;
    END IF;

    -- Canonical contexts are fixed by authoritative extracted candidates.
    -- Sorted advisory keys prevent an opposite-order writer deadlock when
    -- one authority source supports multiple contexts. A direct DML writer
    -- cannot outrun an admitted effect's receipt/HITL transaction.
    FOR affected_context IN
        SELECT DISTINCT c.context_key
        FROM extracted.rule_candidates AS c
        WHERE c.source_key = OLD.source_key
        ORDER BY c.context_key
    LOOP
        IF NOT pg_try_advisory_xact_lock(
            hashtextextended('truth-context|' || affected_context, 0)) THEN
            RAISE EXCEPTION 'source authority is locked by admitted Truth effect'
                USING ERRCODE = '55P03';
        END IF;
    END LOOP;
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS guard_truth_source_authority_write_v1 ON kb.sources;
CREATE TRIGGER guard_truth_source_authority_write_v1
    BEFORE UPDATE OR DELETE ON kb.sources
    FOR EACH ROW EXECUTE FUNCTION kb.guard_truth_source_authority_writer_v1();
DROP TRIGGER IF EXISTS guard_truth_source_authority_truncate_v1 ON kb.sources;
CREATE TRIGGER guard_truth_source_authority_truncate_v1
    BEFORE TRUNCATE ON kb.sources
    FOR EACH STATEMENT EXECUTE FUNCTION kb.guard_truth_source_authority_writer_v1();

COMMIT;

-- Forward-only ingress owner provenance. An authenticated tenant's database
-- LOGIN is the only source of new raw snapshots; a request GUC or URL is not
-- sufficient identity. Existing unattributed historical rows stay NULL and
-- cannot become admitted Truth evidence without a separately audited backfill.
-- This migration deliberately does not FORCE RLS for legacy superuser/table-
-- owner paths: full role cutover is still an explicit production NO-GO.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.tenant_ingress_dba_v1()
RETURNS BOOLEAN LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog AS $$
    SELECT COALESCE((SELECT r.rolsuper FROM pg_catalog.pg_roles r
                     WHERE r.rolname = session_user), false);
$$;
REVOKE ALL ON FUNCTION pipeline.tenant_ingress_dba_v1() FROM PUBLIC;

-- No API-supplied tenant is trusted. A bound LOGIN gets its tenant from the
-- immutable owner ledger; a legacy unbound role may write only NULL rows.
CREATE OR REPLACE FUNCTION raw.stamp_ingress_page_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE
    actor_tenant TEXT := pipeline.current_session_tenant_v1();
    is_dba BOOLEAN := pipeline.tenant_ingress_dba_v1();
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF actor_tenant IS NOT NULL THEN
            IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id <> actor_tenant THEN
                RAISE EXCEPTION 'raw page tenant differs from authenticated LOGIN'
                    USING ERRCODE = 'AV001';
            END IF;
            NEW.tenant_id := actor_tenant;
        ELSIF NEW.tenant_id IS NOT NULL AND NOT is_dba THEN
            RAISE EXCEPTION 'unprovisioned LOGIN cannot claim raw page ownership'
                USING ERRCODE = 'AV001';
        END IF;
    ELSE
        IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id AND NOT is_dba THEN
            RAISE EXCEPTION 'raw page tenant reassignment requires audited DBA migration'
                USING ERRCODE = 'AV001';
        END IF;
        IF NOT is_dba AND (actor_tenant IS DISTINCT FROM OLD.tenant_id) THEN
            RAISE EXCEPTION 'raw page writer LOGIN does not own source evidence'
                USING ERRCODE = 'AV001';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION raw.stamp_ingress_page_tenant_v1() FROM PUBLIC;
DROP TRIGGER IF EXISTS stamp_ingress_page_tenant_v1 ON raw.pages;
CREATE TRIGGER stamp_ingress_page_tenant_v1
    BEFORE INSERT OR UPDATE ON raw.pages FOR EACH ROW
    EXECUTE FUNCTION raw.stamp_ingress_page_tenant_v1();

-- Section tenancy follows its actual parent page, never an independent
-- caller-supplied label. Reparenting across tenants is rejected atomically.
CREATE OR REPLACE FUNCTION raw.stamp_ingress_section_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE
    parent_tenant TEXT;
    actor_tenant TEXT := pipeline.current_session_tenant_v1();
    is_dba BOOLEAN := pipeline.tenant_ingress_dba_v1();
BEGIN
    SELECT p.tenant_id INTO parent_tenant FROM raw.pages p WHERE p.id = NEW.page_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'raw section has no canonical parent page'
            USING ERRCODE = 'AV001';
    END IF;
    IF parent_tenant IS NULL THEN
        IF actor_tenant IS NOT NULL OR NEW.tenant_id IS NOT NULL THEN
            RAISE EXCEPTION 'tenant writer cannot attach to unattributed raw page'
                USING ERRCODE = 'AV001';
        END IF;
    ELSE
        IF NOT is_dba AND actor_tenant IS DISTINCT FROM parent_tenant THEN
            RAISE EXCEPTION 'section writer LOGIN does not own parent raw page'
                USING ERRCODE = 'AV001';
        END IF;
        IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id <> parent_tenant THEN
            RAISE EXCEPTION 'raw section and parent page tenant mismatch'
                USING ERRCODE = 'AV001';
        END IF;
        IF TG_OP = 'INSERT' THEN
            NEW.tenant_id := parent_tenant;
        ELSIF OLD.tenant_id IS DISTINCT FROM parent_tenant OR
              NEW.tenant_id IS DISTINCT FROM OLD.tenant_id THEN
            RAISE EXCEPTION 'raw section tenant transfer requires audited backfill'
                USING ERRCODE = 'AV001';
        END IF;
    END IF;
    IF TG_OP = 'UPDATE' AND NOT is_dba AND
       OLD.tenant_id IS DISTINCT FROM actor_tenant THEN
        RAISE EXCEPTION 'unowned historical section is quarantined'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION raw.stamp_ingress_section_tenant_v1() FROM PUBLIC;
DROP TRIGGER IF EXISTS stamp_ingress_section_tenant_v1 ON raw.sections;
CREATE TRIGGER stamp_ingress_section_tenant_v1
    BEFORE INSERT OR UPDATE ON raw.sections FOR EACH ROW
    EXECUTE FUNCTION raw.stamp_ingress_section_tenant_v1();

-- A registered canonical run is attributed to its DBA-owned Truth context.
-- A tenant-bound LOGIN cannot create an unprovisioned run or impersonate
-- another tenant, even through a SECURITY DEFINER caller or forged GUC.
CREATE OR REPLACE FUNCTION pipeline.stamp_ingress_run_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE
    canonical_tenant TEXT;
    actor_tenant TEXT := pipeline.current_session_tenant_v1();
    is_dba BOOLEAN := pipeline.tenant_ingress_dba_v1();
BEGIN
    SELECT t.tenant_id INTO canonical_tenant
    FROM pipeline.tenant_context_ownership t WHERE t.context_key = NEW.context_key;
    IF canonical_tenant IS NULL THEN
        IF NOT is_dba AND (actor_tenant IS NOT NULL OR NEW.tenant_id IS NOT NULL) THEN
            RAISE EXCEPTION 'tenant run requires DBA-provisioned canonical context'
                USING ERRCODE = 'AV001';
        END IF;
    ELSE
        IF NOT is_dba AND actor_tenant IS DISTINCT FROM canonical_tenant THEN
            RAISE EXCEPTION 'run writer LOGIN does not own canonical context'
                USING ERRCODE = 'AV001';
        END IF;
        IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id <> canonical_tenant THEN
            RAISE EXCEPTION 'run tenant differs from canonical context owner'
                USING ERRCODE = 'AV001';
        END IF;
        IF TG_OP = 'UPDATE' AND OLD.tenant_id IS DISTINCT FROM canonical_tenant THEN
            RAISE EXCEPTION 'legacy run needs audited tenant backfill before use'
                USING ERRCODE = 'AV001';
        END IF;
        NEW.tenant_id := canonical_tenant;
    END IF;
    IF TG_OP = 'UPDATE' AND NOT is_dba AND
       (NEW.tenant_id IS DISTINCT FROM OLD.tenant_id OR
        NEW.context_key IS DISTINCT FROM OLD.context_key) THEN
        RAISE EXCEPTION 'run tenant/context identity cannot be reassigned'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.stamp_ingress_run_tenant_v1() FROM PUBLIC;
DROP TRIGGER IF EXISTS stamp_ingress_run_tenant_v1 ON pipeline.execution_runs;
CREATE TRIGGER stamp_ingress_run_tenant_v1
    BEFORE INSERT OR UPDATE ON pipeline.execution_runs FOR EACH ROW
    EXECUTE FUNCTION pipeline.stamp_ingress_run_tenant_v1();

-- A candidate can never claim a tenant other than both source/evidence
-- sections and the immutably provisioned context. This closes an ingress
-- path where RLS would otherwise filter the tenant mismatch from a reviewer.
CREATE OR REPLACE FUNCTION extracted.stamp_ingress_candidate_tenant_v1()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE
    canonical_tenant TEXT;
    source_tenant TEXT;
    evidence_tenant TEXT;
    actor_tenant TEXT := pipeline.current_session_tenant_v1();
    is_dba BOOLEAN := pipeline.tenant_ingress_dba_v1();
BEGIN
    SELECT t.tenant_id INTO canonical_tenant FROM pipeline.tenant_context_ownership t
    WHERE t.context_key = NEW.context_key;
    SELECT s.tenant_id INTO source_tenant FROM raw.sections s WHERE s.id = NEW.raw_section_id;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate source section missing' USING ERRCODE = 'AV001'; END IF;
    SELECT s.tenant_id INTO evidence_tenant FROM raw.sections s WHERE s.id = NEW.evidence_section_id;
    IF NOT FOUND THEN RAISE EXCEPTION 'candidate evidence section missing' USING ERRCODE = 'AV001'; END IF;
    IF canonical_tenant IS NULL THEN
        IF NOT is_dba AND (actor_tenant IS NOT NULL OR NEW.tenant_id IS NOT NULL) THEN
            RAISE EXCEPTION 'tenant candidate requires provisioned canonical context'
                USING ERRCODE = 'AV001';
        END IF;
    ELSE
        IF source_tenant IS DISTINCT FROM canonical_tenant OR
           evidence_tenant IS DISTINCT FROM canonical_tenant THEN
            RAISE EXCEPTION 'candidate sources differ from canonical tenant owner'
                USING ERRCODE = 'AV001';
        END IF;
        IF NOT is_dba AND actor_tenant IS DISTINCT FROM canonical_tenant THEN
            RAISE EXCEPTION 'candidate writer LOGIN does not own canonical context'
                USING ERRCODE = 'AV001';
        END IF;
        IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id <> canonical_tenant THEN
            RAISE EXCEPTION 'candidate tenant spoofing refused'
                USING ERRCODE = 'AV001';
        END IF;
        IF TG_OP = 'UPDATE' AND OLD.tenant_id IS DISTINCT FROM canonical_tenant THEN
            RAISE EXCEPTION 'legacy candidate needs audited tenant backfill'
                USING ERRCODE = 'AV001';
        END IF;
        NEW.tenant_id := canonical_tenant;
    END IF;
    IF TG_OP = 'UPDATE' AND NOT is_dba AND
       (NEW.tenant_id IS DISTINCT FROM OLD.tenant_id OR
        NEW.context_key IS DISTINCT FROM OLD.context_key) THEN
        RAISE EXCEPTION 'candidate tenant/context identity cannot be reassigned'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION extracted.stamp_ingress_candidate_tenant_v1() FROM PUBLIC;
DROP TRIGGER IF EXISTS stamp_ingress_candidate_tenant_v1 ON extracted.rule_candidates;
CREATE TRIGGER stamp_ingress_candidate_tenant_v1
    BEFORE INSERT OR UPDATE ON extracted.rule_candidates FOR EACH ROW
    EXECUTE FUNCTION extracted.stamp_ingress_candidate_tenant_v1();

COMMIT;

-- V6 experimental *second* external-effect owner: an immutable, tenant-scoped
-- Qdrant keyword projection with an explicit unknown-outcome barrier.
--
-- IMPORTANT: this does not authorize production use. Temporal parent/child
-- History must be attested independently by an operator before a real worker
-- role receives EXECUTE privileges. The values below are EXPECTED identities,
-- not cryptographic proof of a Temporal server History.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS pipeline.qdrant_projection_v1_admissions (
    admission_id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES pipeline.execution_runs(run_id),
    tenant_id TEXT NOT NULL CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0),
    context_key TEXT NOT NULL REFERENCES kb.visa_contexts(context_key),
    keyword_key TEXT NOT NULL CHECK (keyword_key = btrim(keyword_key) AND length(keyword_key) > 0),
    parent_workflow_id TEXT NOT NULL CHECK (length(btrim(parent_workflow_id)) > 0),
    parent_workflow_run_id TEXT NOT NULL CHECK (length(btrim(parent_workflow_run_id)) > 0),
    child_workflow_id TEXT NOT NULL CHECK (length(btrim(child_workflow_id)) > 0),
    child_workflow_run_id TEXT NOT NULL CHECK (length(btrim(child_workflow_run_id)) > 0),
    child_started_event_id BIGINT NOT NULL CHECK (child_started_event_id > 0),
    operation_instance TEXT NOT NULL CHECK (length(btrim(operation_instance)) > 0),
    plan_sha256 TEXT NOT NULL CHECK (plan_sha256 ~ '^[0-9a-f]{64}$'),
    command_sha256 TEXT NOT NULL CHECK (command_sha256 ~ '^[0-9a-f]{64}$'),
    -- The operator approves EXACT serialized bytes, never an editable URL or SQL.
    command_bytes BYTEA NOT NULL CHECK (octet_length(command_bytes) > 0),
    issued_by NAME NOT NULL DEFAULT session_user,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    UNIQUE (tenant_id, context_key, keyword_key),
    UNIQUE (run_id, plan_sha256, operation_instance),
    UNIQUE (child_workflow_run_id, operation_instance),
    CHECK (command_sha256 = encode(public.digest(command_bytes, 'sha256'), 'hex')),
    CHECK (expires_at > issued_at)
);

-- After START, provider outcome is UNKNOWN until independent exact readback.
-- An uncertain row MUST NEVER be automatically sent a second provider RPC.
CREATE TABLE IF NOT EXISTS pipeline.qdrant_projection_v1_attempts (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.qdrant_projection_v1_admissions(admission_id),
    tenant_id TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'uncertain' CHECK (state IN ('uncertain', 'confirmed')),
    started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    confirmed_at TIMESTAMPTZ,
    CHECK ((state = 'uncertain' AND confirmed_at IS NULL)
        OR (state = 'confirmed' AND confirmed_at IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS pipeline.qdrant_projection_v1_receipts (
    admission_id UUID PRIMARY KEY REFERENCES pipeline.qdrant_projection_v1_admissions(admission_id),
    tenant_id TEXT NOT NULL,
    command_sha256 TEXT NOT NULL CHECK (command_sha256 ~ '^[0-9a-f]{64}$'),
    point_id UUID NOT NULL UNIQUE,
    collection_name TEXT NOT NULL DEFAULT 'seo_keyword_clusters_4'
        CHECK (collection_name = 'seo_keyword_clusters_4'),
    -- Point was independently retrieved, including all 1024 floats and payload.
    exact_provider_readback BOOLEAN NOT NULL CHECK (exact_provider_readback),
    committed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

REVOKE ALL ON pipeline.qdrant_projection_v1_admissions,
    pipeline.qdrant_projection_v1_attempts,
    pipeline.qdrant_projection_v1_receipts FROM PUBLIC;

-- These are not application-insertable tenant claims. Only DBA-provisioned
-- PostgreSQL LOGINs can issue an admission for their canonical context/run.
CREATE OR REPLACE FUNCTION pipeline.guard_qdrant_projection_admission_v1()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE v_owner TEXT;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        RAISE EXCEPTION 'Qdrant effect admissions are append-only; no delete or rewrite'
            USING ERRCODE = 'AV001';
    END IF;
    v_owner := pipeline.current_session_tenant_v1();
    IF v_owner IS DISTINCT FROM NEW.tenant_id OR NEW.issued_by <> session_user THEN
        RAISE EXCEPTION 'Qdrant effect operator LOGIN is not provisioned for tenant'
            USING ERRCODE = 'AV001';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pipeline.tenant_context_ownership o
        JOIN pipeline.execution_runs r ON r.context_key = o.context_key
        WHERE o.context_key = NEW.context_key AND o.tenant_id = NEW.tenant_id
          AND r.run_id = NEW.run_id AND r.tenant_id = NEW.tenant_id
    ) THEN
        RAISE EXCEPTION 'Qdrant projection context/run has no authoritative tenant owner'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;
DROP TRIGGER IF EXISTS qdrant_projection_admission_insert_v1
    ON pipeline.qdrant_projection_v1_admissions;
CREATE TRIGGER qdrant_projection_admission_insert_v1
    BEFORE INSERT ON pipeline.qdrant_projection_v1_admissions
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_qdrant_projection_admission_v1();
DROP TRIGGER IF EXISTS qdrant_projection_admission_immutable_v1
    ON pipeline.qdrant_projection_v1_admissions;
CREATE TRIGGER qdrant_projection_admission_immutable_v1
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_v1_admissions
    FOR EACH ROW EXECUTE FUNCTION pipeline.guard_qdrant_projection_admission_v1();

-- Operator and worker must use different PostgreSQL LOGINs (both DBA bound to
-- the same tenant). The app role is deliberately granted NOTHING here.
CREATE OR REPLACE FUNCTION pipeline.begin_qdrant_projection_v1(
    p_admission_id UUID, p_command_bytes BYTEA
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_existing TEXT;
BEGIN
    SELECT * INTO a FROM pipeline.qdrant_projection_v1_admissions
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user
       OR a.revoked_at IS NOT NULL OR a.expires_at <= clock_timestamp()
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR encode(public.digest(p_command_bytes, 'sha256'), 'hex') <> a.command_sha256
       OR NOT EXISTS (
            SELECT 1 FROM pipeline.execution_runs r
            JOIN pipeline.tenant_context_ownership o ON o.context_key = r.context_key
            WHERE r.run_id = a.run_id AND r.context_key = a.context_key
              AND r.tenant_id = a.tenant_id AND o.tenant_id = a.tenant_id
       )
    THEN
        RAISE EXCEPTION 'Qdrant projection admission denied or stale' USING ERRCODE = 'AV001';
    END IF;
    SELECT state INTO v_existing FROM pipeline.qdrant_projection_v1_attempts
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF FOUND THEN
        RETURN v_existing; -- uncertain means NEVER retry the RPC blindly
    END IF;
    INSERT INTO pipeline.qdrant_projection_v1_attempts (admission_id, tenant_id)
        VALUES (p_admission_id, a.tenant_id);
    RETURN 'started';
END;
$$;

CREATE OR REPLACE FUNCTION pipeline.confirm_qdrant_projection_v1(
    p_admission_id UUID, p_command_bytes BYTEA
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_attempt RECORD;
BEGIN
    SELECT * INTO a FROM pipeline.qdrant_projection_v1_admissions
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR encode(public.digest(p_command_bytes, 'sha256'), 'hex') <> a.command_sha256
    THEN
        RAISE EXCEPTION 'Qdrant projection confirmation denied' USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO v_attempt FROM pipeline.qdrant_projection_v1_attempts
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR v_attempt.tenant_id IS DISTINCT FROM a.tenant_id THEN
        RAISE EXCEPTION 'provider RPC was never admitted' USING ERRCODE = 'AV001';
    END IF;
    IF v_attempt.state = 'confirmed' THEN
        RETURN TRUE;
    END IF;
    INSERT INTO pipeline.qdrant_projection_v1_receipts (
        admission_id, tenant_id, command_sha256, point_id, exact_provider_readback
    ) VALUES (p_admission_id, a.tenant_id, a.command_sha256, p_admission_id, TRUE);
    UPDATE pipeline.qdrant_projection_v1_attempts
       SET state = 'confirmed', confirmed_at = clock_timestamp()
     WHERE admission_id = p_admission_id;
    RETURN TRUE;
END;
$$;

REVOKE ALL ON FUNCTION pipeline.begin_qdrant_projection_v1(UUID, BYTEA) FROM PUBLIC;
REVOKE ALL ON FUNCTION pipeline.confirm_qdrant_projection_v1(UUID, BYTEA) FROM PUBLIC;

-- Role-identity RLS, never GUC identity. Superuser/owner bypass remains an
-- explicit production NO-GO until a separately attested worker is provisioned.
ALTER TABLE pipeline.qdrant_projection_v1_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.qdrant_projection_v1_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline.qdrant_projection_v1_receipts ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS qdrant_admission_tenant_v1 ON pipeline.qdrant_projection_v1_admissions;
CREATE POLICY qdrant_admission_tenant_v1 ON pipeline.qdrant_projection_v1_admissions
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS qdrant_attempt_tenant_v1 ON pipeline.qdrant_projection_v1_attempts;
CREATE POLICY qdrant_attempt_tenant_v1 ON pipeline.qdrant_projection_v1_attempts
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());
DROP POLICY IF EXISTS qdrant_receipt_tenant_v1 ON pipeline.qdrant_projection_v1_receipts;
CREATE POLICY qdrant_receipt_tenant_v1 ON pipeline.qdrant_projection_v1_receipts
    USING (tenant_id = pipeline.current_session_tenant_v1())
    WITH CHECK (tenant_id = pipeline.current_session_tenant_v1());

COMMIT;

-- RLS never applies to TRUNCATE. An accidentally overgranted native LOGIN,
-- table owner, or privileged maintenance session must not erase canonical
-- Truth evidence, executions, or Qdrant operator admission/receipts through
-- that bypass. This remains a forward-only fail-closed guard, not a historical
-- tenant backfill or authorization to use a production table-owner LOGIN.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.deny_tenant_scoped_truncate_v1()
RETURNS trigger LANGUAGE plpgsql
SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'tenant-bound canonical operation ledger cannot be truncated'
        USING ERRCODE = '23514';
END;
$$;
REVOKE ALL ON FUNCTION pipeline.deny_tenant_scoped_truncate_v1() FROM PUBLIC;

-- Revocation alone is insufficient when a specific operator LOGIN has an
-- inherited or mistakenly granted TRUNCATE privilege. The BEFORE triggers
-- enforce the prohibition even then, and also for table-owner DBA paths.
REVOKE TRUNCATE ON extracted.rule_candidates,
    pipeline.execution_runs,
    pipeline.pg_effect_v3_receipts,
    pipeline.qdrant_projection_v1_admissions,
    pipeline.qdrant_projection_v1_attempts,
    pipeline.qdrant_projection_v1_receipts FROM PUBLIC;

DROP TRIGGER IF EXISTS deny_tenant_scoped_truncate_v1 ON extracted.rule_candidates;
CREATE TRIGGER deny_tenant_scoped_truncate_v1
    BEFORE TRUNCATE ON extracted.rule_candidates
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_scoped_truncate_v1();

DROP TRIGGER IF EXISTS deny_tenant_scoped_truncate_v1 ON pipeline.execution_runs;
CREATE TRIGGER deny_tenant_scoped_truncate_v1
    BEFORE TRUNCATE ON pipeline.execution_runs
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_scoped_truncate_v1();

DROP TRIGGER IF EXISTS deny_tenant_scoped_truncate_v1 ON pipeline.pg_effect_v3_receipts;
CREATE TRIGGER deny_tenant_scoped_truncate_v1
    BEFORE TRUNCATE ON pipeline.pg_effect_v3_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_scoped_truncate_v1();

DROP TRIGGER IF EXISTS deny_tenant_scoped_truncate_v1 ON pipeline.qdrant_projection_v1_admissions;
CREATE TRIGGER deny_tenant_scoped_truncate_v1
    BEFORE TRUNCATE ON pipeline.qdrant_projection_v1_admissions
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_scoped_truncate_v1();

DROP TRIGGER IF EXISTS deny_tenant_scoped_truncate_v1 ON pipeline.qdrant_projection_v1_attempts;
CREATE TRIGGER deny_tenant_scoped_truncate_v1
    BEFORE TRUNCATE ON pipeline.qdrant_projection_v1_attempts
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_scoped_truncate_v1();

DROP TRIGGER IF EXISTS deny_tenant_scoped_truncate_v1 ON pipeline.qdrant_projection_v1_receipts;
CREATE TRIGGER deny_tenant_scoped_truncate_v1
    BEFORE TRUNCATE ON pipeline.qdrant_projection_v1_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION pipeline.deny_tenant_scoped_truncate_v1();

COMMIT;

-- Forward-only V6 correction: an immutable provider-attempt barrier is never
-- reopened by a DELETE/UPDATE, even when an executor LOGIN is overgranted.
-- Successful confirmation is DERIVED from the append-only receipt, not from
-- a mutable attempts.state bit. Historical confirmed attempts are preserved.
-- Admission revocation is a monotonic issuer-only NULL -> server-time change;
-- it denies a *new* begin, never claims to cancel an already-started RPC.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.guard_qdrant_projection_admission_v1()
RETURNS TRIGGER LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE v_owner TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'Qdrant effect admissions cannot be deleted'
            USING ERRCODE = 'AV001';
    END IF;
    v_owner := pipeline.current_session_tenant_v1();
    IF TG_OP = 'UPDATE' THEN
        IF v_owner IS DISTINCT FROM OLD.tenant_id OR OLD.issued_by <> session_user
           OR OLD.revoked_at IS NOT NULL OR NEW.revoked_at IS NULL
           OR (to_jsonb(NEW) - 'revoked_at') IS DISTINCT FROM
              (to_jsonb(OLD) - 'revoked_at') THEN
            RAISE EXCEPTION 'Qdrant admission allows only issuer-owned monotonic revocation'
                USING ERRCODE = 'AV001';
        END IF;
        -- Never trust a caller-selected timestamp, including a future date.
        NEW.revoked_at := clock_timestamp();
        RETURN NEW;
    END IF;
    IF v_owner IS DISTINCT FROM NEW.tenant_id OR NEW.issued_by <> session_user THEN
        RAISE EXCEPTION 'Qdrant effect operator LOGIN is not provisioned for tenant'
            USING ERRCODE = 'AV001';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pipeline.tenant_context_ownership o
        JOIN pipeline.execution_runs r ON r.context_key = o.context_key
        WHERE o.context_key = NEW.context_key AND o.tenant_id = NEW.tenant_id
          AND r.run_id = NEW.run_id AND r.tenant_id = NEW.tenant_id
    ) THEN
        RAISE EXCEPTION 'Qdrant projection context/run has no authoritative tenant owner'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION pipeline.deny_qdrant_projection_ledger_rewrite_v1()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'Qdrant provider attempt and receipt ledgers are append-only'
        USING ERRCODE = '23514';
END;
$$;
REVOKE ALL ON FUNCTION pipeline.deny_qdrant_projection_ledger_rewrite_v1() FROM PUBLIC;
DROP TRIGGER IF EXISTS deny_qdrant_projection_attempt_rewrite_v1
    ON pipeline.qdrant_projection_v1_attempts;
CREATE TRIGGER deny_qdrant_projection_attempt_rewrite_v1
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_v1_attempts
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_qdrant_projection_ledger_rewrite_v1();
DROP TRIGGER IF EXISTS deny_qdrant_projection_receipt_rewrite_v1
    ON pipeline.qdrant_projection_v1_receipts;
CREATE TRIGGER deny_qdrant_projection_receipt_rewrite_v1
    BEFORE UPDATE OR DELETE ON pipeline.qdrant_projection_v1_receipts
    FOR EACH ROW EXECUTE FUNCTION pipeline.deny_qdrant_projection_ledger_rewrite_v1();

CREATE OR REPLACE FUNCTION pipeline.begin_qdrant_projection_v1(
    p_admission_id UUID, p_command_bytes BYTEA
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_existing TEXT; v_receipt RECORD;
BEGIN
    -- Row lock serializes a first start against issuer revocation. If begin
    -- wins, a later revocation cannot recall an in-flight provider RPC.
    SELECT * INTO a FROM pipeline.qdrant_projection_v1_admissions
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user
       OR a.revoked_at IS NOT NULL OR a.expires_at <= clock_timestamp()
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR encode(public.digest(p_command_bytes, 'sha256'), 'hex') <> a.command_sha256
       OR NOT EXISTS (
            SELECT 1 FROM pipeline.execution_runs r
            JOIN pipeline.tenant_context_ownership o ON o.context_key = r.context_key
            WHERE r.run_id = a.run_id AND r.context_key = a.context_key
              AND r.tenant_id = a.tenant_id AND o.tenant_id = a.tenant_id
       )
    THEN
        RAISE EXCEPTION 'Qdrant projection admission denied or stale' USING ERRCODE = 'AV001';
    END IF;
    SELECT state INTO v_existing FROM pipeline.qdrant_projection_v1_attempts
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF FOUND THEN
        SELECT * INTO v_receipt FROM pipeline.qdrant_projection_v1_receipts
            WHERE admission_id = p_admission_id;
        IF FOUND THEN
            IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
               OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
               OR v_receipt.point_id IS DISTINCT FROM p_admission_id
               OR v_receipt.exact_provider_readback IS DISTINCT FROM TRUE THEN
                RAISE EXCEPTION 'Qdrant immutable receipt identity is inconsistent'
                    USING ERRCODE = 'AV001';
            END IF;
            RETURN 'confirmed';
        END IF;
        RETURN 'uncertain'; -- no second provider RPC, even for legacy state
    END IF;
    IF EXISTS (SELECT 1 FROM pipeline.qdrant_projection_v1_receipts
               WHERE admission_id = p_admission_id) THEN
        RAISE EXCEPTION 'Qdrant receipt exists without an admitted attempt'
            USING ERRCODE = 'AV001';
    END IF;
    INSERT INTO pipeline.qdrant_projection_v1_attempts (admission_id, tenant_id)
        VALUES (p_admission_id, a.tenant_id);
    RETURN 'started';
END;
$$;

CREATE OR REPLACE FUNCTION pipeline.confirm_qdrant_projection_v1(
    p_admission_id UUID, p_command_bytes BYTEA
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_attempt RECORD; v_receipt RECORD;
BEGIN
    SELECT * INTO a FROM pipeline.qdrant_projection_v1_admissions
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR encode(public.digest(p_command_bytes, 'sha256'), 'hex') <> a.command_sha256
    THEN
        RAISE EXCEPTION 'Qdrant projection confirmation denied' USING ERRCODE = 'AV001';
    END IF;
    -- Revocation after the durable start cannot undo Qdrant I/O. Reconcile
    -- and record the observed result even if the operator has since revoked.
    SELECT * INTO v_attempt FROM pipeline.qdrant_projection_v1_attempts
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR v_attempt.tenant_id IS DISTINCT FROM a.tenant_id THEN
        RAISE EXCEPTION 'provider RPC was never admitted' USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO v_receipt FROM pipeline.qdrant_projection_v1_receipts
        WHERE admission_id = p_admission_id;
    IF FOUND THEN
        IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
           OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
           OR v_receipt.point_id IS DISTINCT FROM p_admission_id
           OR v_receipt.exact_provider_readback IS DISTINCT FROM TRUE THEN
            RAISE EXCEPTION 'Qdrant immutable receipt identity is inconsistent'
                USING ERRCODE = 'AV001';
        END IF;
        RETURN TRUE;
    END IF;
    INSERT INTO pipeline.qdrant_projection_v1_receipts (
        admission_id, tenant_id, command_sha256, point_id, exact_provider_readback
    ) VALUES (p_admission_id, a.tenant_id, a.command_sha256, p_admission_id, TRUE);
    -- Do not mutate the uncertainty barrier. Durable confirmation is exactly
    -- the existence of this append-only receipt, including historical rows.
    RETURN TRUE;
END;
$$;

COMMIT;

-- V7 tenant cutover preflight only. No historical row is assigned a tenant,
-- no production role is granted DML, and RLS is not FORCE-enabled here.
-- This forward-only migration makes new malformed labels fail closed and
-- provides a DBA-only, source-verifiable orphan/inconsistency census.
BEGIN;
SET LOCAL lock_timeout = '5s';

-- Existing legacy NULLs and malformed labels are deliberately NOT VALIDATED:
-- they remain visible as historical quarantine in the census below. New
-- inserts/updates cannot create new whitespace or empty owner labels.
ALTER TABLE raw.pages ADD CONSTRAINT raw_pages_tenant_label_v1
    CHECK (tenant_id IS NULL OR (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0)) NOT VALID;
ALTER TABLE raw.sections ADD CONSTRAINT raw_sections_tenant_label_v1
    CHECK (tenant_id IS NULL OR (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0)) NOT VALID;
ALTER TABLE extracted.rule_candidates ADD CONSTRAINT extracted_candidates_tenant_label_v1
    CHECK (tenant_id IS NULL OR (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0)) NOT VALID;
ALTER TABLE pipeline.execution_runs ADD CONSTRAINT execution_runs_tenant_label_v1
    CHECK (tenant_id IS NULL OR (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0)) NOT VALID;
ALTER TABLE site.seo_hitl_tasks ADD CONSTRAINT seo_hitl_tasks_tenant_label_v1
    CHECK (tenant_id IS NULL OR (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0)) NOT VALID;
ALTER TABLE pipeline.pg_effect_v3_admissions ADD CONSTRAINT pg_effect_admissions_tenant_label_v1
    CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0) NOT VALID;
ALTER TABLE pipeline.pg_effect_v3_receipts ADD CONSTRAINT pg_effect_receipts_tenant_label_v1
    CHECK (tenant_id IS NULL OR (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0)) NOT VALID;
ALTER TABLE pipeline.qdrant_projection_v1_admissions ADD CONSTRAINT qdrant_admissions_tenant_label_v1
    CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0) NOT VALID;
ALTER TABLE pipeline.qdrant_projection_v1_attempts ADD CONSTRAINT qdrant_attempts_tenant_label_v1
    CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0) NOT VALID;
ALTER TABLE pipeline.qdrant_projection_v1_receipts ADD CONSTRAINT qdrant_receipts_tenant_label_v1
    CHECK (tenant_id = btrim(tenant_id) AND length(tenant_id) > 0) NOT VALID;

-- A read-only global inventory. Each artifact is counted once; unattributed
-- is NULL tenant_id, inconsistent is a non-NULL label that violates its
-- immutable context or source edge. This intentionally does not scan only
-- one caller-supplied tenant: missing attribution cannot safely be scoped.
CREATE OR REPLACE FUNCTION pipeline.tenant_historical_quarantine_v1()
RETURNS TABLE (relation_name TEXT, unattributed BIGINT, inconsistent BIGINT)
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
    SELECT 'raw.pages'::text,
           count(*) FILTER (WHERE p.tenant_id IS NULL),
           count(*) FILTER (WHERE p.tenant_id IS NOT NULL
                            AND (p.tenant_id <> btrim(p.tenant_id) OR length(p.tenant_id) = 0))
    FROM raw.pages p
    UNION ALL
    SELECT 'raw.sections'::text,
           count(*) FILTER (WHERE s.tenant_id IS NULL),
           count(*) FILTER (WHERE s.tenant_id IS NOT NULL
                            AND (s.tenant_id <> p.tenant_id OR p.tenant_id IS NULL
                                 OR s.tenant_id <> btrim(s.tenant_id) OR length(s.tenant_id) = 0))
    FROM raw.sections s LEFT JOIN raw.pages p ON p.id = s.page_id
    UNION ALL
    SELECT 'extracted.rule_candidates'::text,
           count(*) FILTER (WHERE c.tenant_id IS NULL),
           count(*) FILTER (WHERE c.tenant_id IS NOT NULL AND
                (c.tenant_id <> owner.tenant_id OR owner.tenant_id IS NULL
                 OR c.tenant_id IS DISTINCT FROM src.tenant_id
                 OR c.tenant_id IS DISTINCT FROM src_page.tenant_id
                 OR c.tenant_id IS DISTINCT FROM ev.tenant_id
                 OR c.tenant_id IS DISTINCT FROM ev_page.tenant_id))
    FROM extracted.rule_candidates c
    LEFT JOIN pipeline.tenant_context_ownership owner ON owner.context_key=c.context_key
    LEFT JOIN raw.sections src ON src.id=c.raw_section_id
    LEFT JOIN raw.pages src_page ON src_page.id=src.page_id
    LEFT JOIN raw.sections ev ON ev.id=c.evidence_section_id
    LEFT JOIN raw.pages ev_page ON ev_page.id=ev.page_id
    UNION ALL
    SELECT 'pipeline.execution_runs'::text,
           count(*) FILTER (WHERE r.tenant_id IS NULL),
           count(*) FILTER (WHERE r.tenant_id IS NOT NULL AND
                (r.tenant_id <> owner.tenant_id OR owner.tenant_id IS NULL))
    FROM pipeline.execution_runs r
    LEFT JOIN pipeline.tenant_context_ownership owner ON owner.context_key=r.context_key
    UNION ALL
    SELECT 'site.seo_hitl_tasks'::text,
           count(*) FILTER (WHERE t.tenant_id IS NULL),
           count(*) FILTER (WHERE t.tenant_id IS NOT NULL AND
                (t.tenant_id <> btrim(t.tenant_id) OR length(t.tenant_id)=0
                 OR (t.task_type='truth_candidate_review' AND
                     (t.tenant_id <> owner.tenant_id OR owner.tenant_id IS NULL))))
    FROM site.seo_hitl_tasks t
    LEFT JOIN pipeline.tenant_context_ownership owner
           ON owner.context_key=t.scope_signature
    UNION ALL
    SELECT 'pipeline.pg_effect_v3_admissions'::text,
           0::bigint,
           count(*) FILTER (WHERE a.tenant_id IS DISTINCT FROM r.tenant_id
                OR a.context_key IS DISTINCT FROM r.context_key
                OR a.tenant_id IS DISTINCT FROM owner.tenant_id)
    FROM pipeline.pg_effect_v3_admissions a
    LEFT JOIN pipeline.execution_runs r ON r.run_id=a.run_id
    LEFT JOIN pipeline.tenant_context_ownership owner ON owner.context_key=a.context_key
    UNION ALL
    SELECT 'pipeline.pg_effect_v3_receipts'::text,
           count(*) FILTER (WHERE receipt.tenant_id IS NULL),
           count(*) FILTER (WHERE receipt.tenant_id IS NOT NULL AND
                (receipt.tenant_id IS DISTINCT FROM a.tenant_id
                 OR receipt.run_id IS DISTINCT FROM a.run_id
                 OR receipt.context_key IS DISTINCT FROM a.context_key))
    FROM pipeline.pg_effect_v3_receipts receipt
    LEFT JOIN pipeline.pg_effect_v3_admissions a ON a.admission_id=receipt.admission_id
    UNION ALL
    SELECT 'pipeline.qdrant_projection_v1_admissions'::text,
           0::bigint,
           count(*) FILTER (WHERE a.tenant_id IS DISTINCT FROM r.tenant_id
                OR a.context_key IS DISTINCT FROM r.context_key
                OR a.tenant_id IS DISTINCT FROM owner.tenant_id)
    FROM pipeline.qdrant_projection_v1_admissions a
    LEFT JOIN pipeline.execution_runs r ON r.run_id=a.run_id
    LEFT JOIN pipeline.tenant_context_ownership owner ON owner.context_key=a.context_key
    UNION ALL
    SELECT 'pipeline.qdrant_projection_v1_attempts'::text,
           0::bigint,
           count(*) FILTER (WHERE attempt.tenant_id IS DISTINCT FROM a.tenant_id)
    FROM pipeline.qdrant_projection_v1_attempts attempt
    LEFT JOIN pipeline.qdrant_projection_v1_admissions a
           ON a.admission_id=attempt.admission_id
    UNION ALL
    SELECT 'pipeline.qdrant_projection_v1_receipts'::text,
           0::bigint,
           count(*) FILTER (WHERE receipt.tenant_id IS DISTINCT FROM a.tenant_id)
    FROM pipeline.qdrant_projection_v1_receipts receipt
    LEFT JOIN pipeline.qdrant_projection_v1_admissions a
           ON a.admission_id=receipt.admission_id;
$$;
REVOKE ALL ON FUNCTION pipeline.tenant_historical_quarantine_v1() FROM PUBLIC;

CREATE OR REPLACE FUNCTION pipeline.assert_no_tenant_quarantine_v1()
RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE v_quarantine RECORD;
BEGIN
    SELECT * INTO v_quarantine FROM pipeline.tenant_historical_quarantine_v1()
    WHERE unattributed > 0 OR inconsistent > 0 LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'historical tenant quarantine blocks cutover: %, missing %, inconsistent %',
            v_quarantine.relation_name, v_quarantine.unattributed, v_quarantine.inconsistent
            USING ERRCODE = 'AV001';
    END IF;
    RETURN TRUE; -- inventory only; does not attest production role cutover
END;
$$;
REVOKE ALL ON FUNCTION pipeline.assert_no_tenant_quarantine_v1() FROM PUBLIC;

-- Prospective, DBA-only proof for an ingress LOGIN. Session/GUC identity is
-- not a substitute for an immutable tenant_role_ownership row. No role grants
-- or table ownership transfers occur here. Operator/worker roles must have
-- their own narrower, separately attested capabilities.
CREATE OR REPLACE FUNCTION pipeline.assert_tenant_ingress_login_v1(
    p_role NAME, p_tenant TEXT
) RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog SET row_security = off AS $$
DECLARE
    v_role_oid OID;
    v_role RECORD;
    v_bound TEXT;
BEGIN
    SELECT r.oid, r.rolcanlogin, r.rolsuper, r.rolbypassrls, r.rolcreaterole,
           r.rolcreatedb, r.rolreplication
    INTO v_role FROM pg_catalog.pg_roles r WHERE r.rolname=p_role;
    IF NOT FOUND OR NOT v_role.rolcanlogin OR v_role.rolsuper OR v_role.rolbypassrls
       OR v_role.rolcreaterole OR v_role.rolcreatedb OR v_role.rolreplication THEN
        RAISE EXCEPTION 'tenant ingress LOGIN has unsafe role attributes'
            USING ERRCODE = 'AV001';
    END IF;
    v_role_oid := v_role.oid;
    SELECT tenant_id INTO v_bound FROM pipeline.tenant_role_ownership
    WHERE role_name=p_role;
    IF p_tenant IS NULL OR p_tenant <> btrim(p_tenant) OR length(p_tenant)=0
       OR v_bound IS DISTINCT FROM p_tenant THEN
        RAISE EXCEPTION 'tenant ingress LOGIN has no immutable exact tenant binding'
            USING ERRCODE = 'AV001';
    END IF;
    IF EXISTS (
        SELECT 1 FROM pg_catalog.pg_roles r
        WHERE (r.rolsuper OR r.rolbypassrls OR r.rolcreaterole)
          AND pg_catalog.pg_has_role(v_role_oid,r.oid,'MEMBER')
    ) OR EXISTS (
        SELECT 1 FROM pg_catalog.pg_class c
        WHERE c.oid = ANY(ARRAY[
            'raw.pages'::regclass, 'raw.sections'::regclass,
            'extracted.rule_candidates'::regclass, 'pipeline.execution_runs'::regclass,
            'site.seo_hitl_tasks'::regclass, 'kb.sources'::regclass,
            'pipeline.pg_effect_v3_admissions'::regclass,
            'pipeline.pg_effect_v3_receipts'::regclass,
            'pipeline.qdrant_projection_v1_admissions'::regclass,
            'pipeline.qdrant_projection_v1_attempts'::regclass,
            'pipeline.qdrant_projection_v1_receipts'::regclass])
          AND pg_catalog.pg_has_role(v_role_oid,c.relowner,'MEMBER')
    ) THEN
        RAISE EXCEPTION 'tenant ingress LOGIN can assume an RLS-bypassing owner role'
            USING ERRCODE = 'AV001';
    END IF;
    IF EXISTS (
        SELECT 1 FROM pg_catalog.pg_class c
        WHERE c.oid = ANY(ARRAY[
            'raw.pages'::regclass, 'raw.sections'::regclass,
            'extracted.rule_candidates'::regclass, 'pipeline.execution_runs'::regclass,
            'site.seo_hitl_tasks'::regclass,
            'pipeline.pg_effect_v3_admissions'::regclass,
            'pipeline.pg_effect_v3_receipts'::regclass,
            'pipeline.qdrant_projection_v1_admissions'::regclass,
            'pipeline.qdrant_projection_v1_attempts'::regclass,
            'pipeline.qdrant_projection_v1_receipts'::regclass])
          AND (pg_catalog.has_table_privilege(v_role_oid,c.oid,'TRUNCATE')
               OR pg_catalog.has_table_privilege(v_role_oid,c.oid,'DELETE'))
    ) OR EXISTS (
        SELECT 1 FROM pg_catalog.pg_namespace n
        WHERE n.nspname IN ('raw','extracted','pipeline','kb','site')
          AND pg_catalog.has_schema_privilege(v_role_oid,n.oid,'CREATE')
    ) THEN
        RAISE EXCEPTION 'tenant ingress LOGIN has unsafe destructive or DDL privileges'
            USING ERRCODE = 'AV001';
    END IF;
    IF EXISTS (
        SELECT 1 FROM pg_catalog.pg_class c
        WHERE c.oid = ANY(ARRAY[
            'pipeline.pg_effect_v3_admissions'::regclass,
            'pipeline.pg_effect_v3_receipts'::regclass,
            'pipeline.qdrant_projection_v1_admissions'::regclass,
            'pipeline.qdrant_projection_v1_attempts'::regclass,
            'pipeline.qdrant_projection_v1_receipts'::regclass])
          AND (pg_catalog.has_table_privilege(v_role_oid,c.oid,'INSERT')
               OR pg_catalog.has_table_privilege(v_role_oid,c.oid,'UPDATE'))
    ) THEN
        RAISE EXCEPTION 'tenant ingress LOGIN has operator admission or receipt write privileges'
            USING ERRCODE = 'AV001';
    END IF;
    IF EXISTS (
        SELECT 1 FROM pg_catalog.pg_class c
        WHERE c.oid = ANY(ARRAY[
            'raw.pages'::regclass, 'raw.sections'::regclass,
            'extracted.rule_candidates'::regclass, 'pipeline.execution_runs'::regclass])
          AND (NOT pg_catalog.has_table_privilege(v_role_oid,c.oid,'SELECT')
               OR NOT pg_catalog.has_table_privilege(v_role_oid,c.oid,'INSERT'))
    ) THEN
        RAISE EXCEPTION 'tenant ingress LOGIN lacks required scoped source writer privileges'
            USING ERRCODE = 'AV001';
    END IF;
    RETURN TRUE;
END;
$$;
REVOKE ALL ON FUNCTION pipeline.assert_tenant_ingress_login_v1(NAME,TEXT) FROM PUBLIC;

COMMIT;

-- Forward-only V7 security correction. Operator issuance is not enough:
-- a canonical Truth run can be terminal before a delayed provider Activity
-- reaches PostgreSQL. Serialize the FIRST Qdrant attempt with the same
-- advisory key held by the execution_runs status trigger (migration 0002).
-- An existing attempt remains an immutable unknown-outcome barrier even after
-- a terminal transition; it never authorizes a second provider RPC.
BEGIN;
SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION pipeline.begin_qdrant_projection_v1(
    p_admission_id UUID, p_command_bytes BYTEA
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE a RECORD; v_existing TEXT; v_receipt RECORD;
BEGIN
    -- A caller-chosen repeatable-read snapshot predating a terminal status
    -- must never be reused to approve fresh external effects.
    IF pg_catalog.current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'Qdrant first-start fence requires READ COMMITTED isolation'
            USING ERRCODE = 'AV001';
    END IF;
    SELECT * INTO a FROM pipeline.qdrant_projection_v1_admissions
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF NOT FOUND OR a.tenant_id IS DISTINCT FROM pipeline.current_session_tenant_v1()
       OR a.issued_by = session_user
       OR a.command_bytes IS DISTINCT FROM p_command_bytes
       OR encode(public.digest(p_command_bytes, 'sha256'), 'hex') <> a.command_sha256
       OR NOT EXISTS (
            SELECT 1 FROM pipeline.execution_runs r
            JOIN pipeline.tenant_context_ownership o ON o.context_key = r.context_key
            WHERE r.run_id = a.run_id AND r.context_key = a.context_key
              AND r.tenant_id = a.tenant_id AND o.tenant_id = a.tenant_id
       )
    THEN
        RAISE EXCEPTION 'Qdrant projection admission denied or stale' USING ERRCODE = 'AV001';
    END IF;
    SELECT state INTO v_existing FROM pipeline.qdrant_projection_v1_attempts
        WHERE admission_id = p_admission_id FOR UPDATE;
    IF FOUND THEN
        SELECT * INTO v_receipt FROM pipeline.qdrant_projection_v1_receipts
            WHERE admission_id = p_admission_id;
        IF FOUND THEN
            IF v_receipt.tenant_id IS DISTINCT FROM a.tenant_id
               OR v_receipt.command_sha256 IS DISTINCT FROM a.command_sha256
               OR v_receipt.point_id IS DISTINCT FROM p_admission_id
               OR v_receipt.exact_provider_readback IS DISTINCT FROM TRUE THEN
                RAISE EXCEPTION 'Qdrant immutable receipt identity is inconsistent'
                    USING ERRCODE = 'AV001';
            END IF;
            RETURN 'confirmed';
        END IF;
        RETURN 'uncertain'; -- no second provider RPC, even after run completion
    END IF;
    IF EXISTS (SELECT 1 FROM pipeline.qdrant_projection_v1_receipts
               WHERE admission_id = p_admission_id) THEN
        RAISE EXCEPTION 'Qdrant receipt exists without an admitted attempt'
            USING ERRCODE = 'AV001';
    END IF;

    -- The execution_runs BEFORE UPDATE/DELETE guard uses pg_try_advisory_xact_lock
    -- on this exact key. A status writer that wins first makes us wait and
    -- observe its terminal status; when this transaction wins first the writer
    -- fails closed with SQLSTATE 55P03. No user-supplied lock key is trusted.
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('truth-run-status|' || a.run_id::text, 0)
    );
    -- A revocation or expiry blocks a NEW provider RPC, never readback or
    -- confirmation of a prior immutable attempt. This permits safe recovery
    -- after a provider succeeded but PostgreSQL confirmation was interrupted.
    IF a.revoked_at IS NOT NULL OR a.expires_at <= clock_timestamp() THEN
        RAISE EXCEPTION 'Qdrant projection first start denied by revoked/expired admission'
            USING ERRCODE = 'AV001';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pipeline.execution_runs r
        JOIN pipeline.tenant_context_ownership o ON o.context_key = r.context_key
        WHERE r.run_id = a.run_id AND r.context_key = a.context_key
          AND r.tenant_id = a.tenant_id AND o.tenant_id = a.tenant_id
          AND r.workflow_run_id = a.parent_workflow_run_id
          AND r.workflow_type = 'seo_site_build'
          AND r.status IN ('verifying', 'pending_hitl')
        FOR SHARE OF r
    ) THEN
        RAISE EXCEPTION 'Qdrant projection first start denied by canonical run status'
            USING ERRCODE = 'AV001';
    END IF;
    INSERT INTO pipeline.qdrant_projection_v1_attempts (admission_id, tenant_id)
        VALUES (p_admission_id, a.tenant_id);
    RETURN 'started';
END;
$$;

COMMIT;
