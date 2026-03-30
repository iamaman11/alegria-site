-- Reference snapshot only. Live schema source of truth: app/db/schema.sql
-- Keep for historical/domain reference; do not treat as live migration input.

-- V5_Postgres_DDL.sql
-- Executable relational model for the V5 Ultimate Extraction Protocol.
-- Canonical policy: truth is persisted in PostgreSQL first; Neo4j is a graph projection.

begin;

create schema if not exists raw;
create schema if not exists extracted;
create schema if not exists verified;
create schema if not exists kb;
create schema if not exists runtime;

create extension if not exists pgcrypto;

-- ============================================================================
-- RAW
-- ============================================================================

create table if not exists raw.sections (
  section_id text primary key,
  source_url text not null,
  url_key text not null,
  crawl_version_id bigint not null,
  heading_level smallint not null,
  heading_text text not null,
  parent_heading text,
  raw_text text not null,
  content_hash text not null,
  block_type text not null,
  char_count integer not null check (char_count >= 0),
  position_on_page integer not null check (position_on_page >= 0),
  has_table boolean not null default false,
  has_list boolean not null default false,
  has_numbers boolean not null default false,
  source_domain text not null,
  source_tier text not null check (source_tier in ('government','official_partner','agency','forum','other')),
  source_priority numeric(4,3) not null check (source_priority >= 0 and source_priority <= 1),
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (url_key, position_on_page, content_hash)
);

create index if not exists idx_raw_sections_url_key on raw.sections(url_key);
create index if not exists idx_raw_sections_content_hash on raw.sections(content_hash);
create index if not exists idx_raw_sections_source_tier on raw.sections(source_tier);

-- ============================================================================
-- EXTRACTED
-- ============================================================================

create table if not exists extracted.section_layer_decisions (
  section_id text primary key references raw.sections(section_id) on delete cascade,
  primary_layer text not null check (primary_layer in ('procedural','operational','editorial','seo','commercial')),
  confidence numeric(5,4) not null check (confidence between 0 and 1),
  mode text not null default 'strict' check (mode in ('strict','recall')),
  needs_hitl boolean not null default false,
  hitl_reason text,
  prompt_version text not null,
  model_version text not null,
  pipeline_version text not null,
  registry_version text not null,
  schema_version text not null default 'layer_router_output@1',
  input_hash text not null,
  output_hash text not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists extracted.section_layer_scores (
  section_id text not null references raw.sections(section_id) on delete cascade,
  layer text not null check (layer in ('procedural','operational','editorial','seo','commercial')),
  score numeric(5,4) not null check (score between 0 and 1),
  rank smallint not null check (rank between 1 and 5),
  created_at timestamptz not null default now(),
  primary key (section_id, layer)
);

create table if not exists extracted.entity_mentions (
  mention_id text primary key,
  section_id text not null references raw.sections(section_id) on delete cascade,
  raw_text text not null,
  entity_type text not null check (entity_type in (
    'concept','office','topic','pain_point','risk','task','service','fee','timeline',
    'location','organization','profile','legal_term','date'
  )),
  has_numeric boolean not null default false,
  is_central boolean not null default false,
  confidence numeric(5,4) not null check (confidence between 0 and 1),
  schema_version text not null default 'entity_mention@1',
  input_hash text not null,
  output_hash text not null,
  created_at timestamptz not null default now()
);

create table if not exists extracted.canonical_mappings (
  mapping_id text primary key,
  mention_id text not null references extracted.entity_mentions(mention_id) on delete cascade,
  target_registry text not null,
  candidate_type text,
  canonical_key text,
  mapping_type text not null check (mapping_type in ('alias','regex','qdrant','review','new_candidate')),
  match_method text not null check (match_method in ('exact_alias','normalized_alias','regex','qdrant','manual_review')),
  qdrant_score numeric(5,4),
  confidence numeric(5,4) not null check (confidence between 0 and 1),
  needs_hitl boolean not null default false,
  hitl_reason text,
  schema_version text not null default 'canonical_mapping@1',
  created_at timestamptz not null default now(),
  check (qdrant_score is null or qdrant_score between 0 and 1)
);

create table if not exists extracted.rule_candidates (
  rule_candidate_id text primary key,
  section_id text not null references raw.sections(section_id) on delete cascade,
  role text not null check (role in (
    'DOCUMENT_REQUIRED','ELIGIBILITY_RULE','FEE_ITEM','TIMELINE_ITEM',
    'WHERE_TO_APPLY','APPOINTMENT_RULE','FORM_REQUIRED','STEP'
  )),
  concept_canonical_key text,
  raw_mention text not null,
  params jsonb not null default '{}'::jsonb,
  scope jsonb not null default '{}'::jsonb,
  severity text not null check (severity in ('mandatory','recommended','optional','unknown')),
  applies_to_profiles jsonb not null default '[]'::jsonb,
  exceptions_raw text,
  conditions_raw text,
  alternatives jsonb not null default '[]'::jsonb,
  modality_raw text,
  evidence_notes text,
  derivation_type text not null default 'direct' check (derivation_type in ('direct','inferred','aggregated')),
  is_numeric boolean not null default false,
  is_range boolean not null default false,
  is_incomplete boolean not null default false,
  confidence numeric(5,4) not null check (confidence between 0 and 1),
  evidence_section_id text not null references raw.sections(section_id) on delete cascade,
  schema_version text not null default 'extracted_rule_candidate@1',
  created_at timestamptz not null default now()
);

create table if not exists extracted.operational_entities (
  operational_entity_id text primary key,
  section_id text not null references raw.sections(section_id) on delete cascade,
  entity_type text not null check (entity_type in (
    'office_schedule','temporary_schedule_change','closure_event',
    'holiday_calendar','country_holiday','submission_blackout','operational_notice'
  )),
  validity_type text not null default 'unknown' check (validity_type in ('fixed','recurring','unknown')),
  entity_key_candidate text,
  office_key_candidate text,
  country_code text,
  raw_mention text not null,
  params jsonb not null default '{}'::jsonb,
  valid_from date,
  valid_until date,
  ttl_days integer check (ttl_days is null or ttl_days > 0),
  confidence numeric(5,4) not null check (confidence between 0 and 1),
  evidence_section_id text not null references raw.sections(section_id) on delete cascade,
  schema_version text not null default 'operational_entity@1',
  created_at timestamptz not null default now()
);

create table if not exists extracted.editorial_topics (
  topic_candidate_id text primary key,
  section_id text not null references raw.sections(section_id) on delete cascade,
  topic_type text not null check (topic_type in (
    'topic','travel_topic','pain_point','risk_factor','preparation_task',
    'audience_need','refusal_scenario','destination_topic','seasonality'
  )),
  topic_key_candidate text not null,
  human_label text not null,
  raw_mention text not null,
  links_to_procedure boolean not null default false,
  procedure_concept_candidate text,
  country_code text,
  visa_type text,
  audience_hint jsonb not null default '[]'::jsonb,
  intent_type text not null check (intent_type in ('informational','navigational','commercial','mixed')),
  confidence numeric(5,4) not null check (confidence between 0 and 1),
  evidence_section_id text not null references raw.sections(section_id) on delete cascade,
  schema_version text not null default 'editorial_topic@1',
  created_at timestamptz not null default now()
);

create table if not exists extracted.triples (
  triple_id text primary key,
  subject_key text not null,
  subject_label text not null,
  subject_layer text not null check (subject_layer in ('procedural','operational','editorial','seo','commercial')),
  relation_type text not null,
  object_key text not null,
  object_label text not null,
  object_layer text not null check (object_layer in ('procedural','operational','editorial','seo','commercial')),
  params jsonb not null default '{}'::jsonb,
  evidence_section_id text not null references raw.sections(section_id) on delete cascade,
  source_tier text not null,
  confidence numeric(5,4) not null check (confidence between 0 and 1),
  extraction_pipeline text not null,
  status text not null default 'extracted' check (status in ('extracted','verified','deprecated','rejected')),
  schema_version text not null default 'triple@1',
  created_at timestamptz not null default now()
);

create table if not exists extracted.completeness_reviews (
  review_id text primary key,
  section_id text not null references raw.sections(section_id) on delete cascade,
  completeness_score numeric(5,4) not null check (completeness_score between 0 and 1),
  missing_elements jsonb not null default '[]'::jsonb,
  hallucinated_elements jsonb not null default '[]'::jsonb,
  unmapped_raw_text text,
  needs_hitl boolean not null default false,
  hitl_reason text,
  schema_version text not null default 'completeness_judge@1',
  created_at timestamptz not null default now()
);

-- ============================================================================
-- VERIFIED
-- ============================================================================

create table if not exists verified.rule_instances (
  rule_instance_id text primary key,
  rule_candidate_id text unique references extracted.rule_candidates(rule_candidate_id) on delete set null,
  rule_key text not null,
  role text not null check (role in (
    'DOCUMENT_REQUIRED','ELIGIBILITY_RULE','FEE_ITEM','TIMELINE_ITEM',
    'WHERE_TO_APPLY','APPOINTMENT_RULE','FORM_REQUIRED','STEP'
  )),
  concept_key text not null,
  visa_key text,
  layer text not null default 'procedural' check (layer = 'procedural'),
  status text not null check (status in ('verified','deprecated','superseded')),
  confidence numeric(5,4) not null check (confidence between 0 and 1),
  effective_confidence numeric(5,4) not null check (effective_confidence between 0 and 1),
  source_factor numeric(5,4) not null check (source_factor between 0 and 1),
  freshness_factor numeric(5,4) not null check (freshness_factor between 0 and 1),
  source_priority numeric(4,3) not null check (source_priority between 0 and 1),
  derivation_type text not null check (derivation_type in ('direct','inferred','aggregated')),
  params jsonb not null default '{}'::jsonb,
  scope jsonb not null default '{}'::jsonb,
  applies_to_profiles jsonb not null default '[]'::jsonb,
  conditions_raw text,
  exceptions_raw text,
  alternatives jsonb not null default '[]'::jsonb,
  modality_raw text,
  evidence_notes text,
  evidence_section_id text not null references raw.sections(section_id) on delete restrict,
  registry_version text not null,
  prompt_version text not null,
  model_version text not null,
  pipeline_version text not null,
  published_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists verified.fact_instances (
  fact_instance_id text primary key,
  fact_key text not null,
  concept_key text not null,
  layer text not null check (layer in ('procedural','operational','editorial')),
  status text not null check (status in ('verified','deprecated','superseded')),
  confidence numeric(5,4) not null check (confidence between 0 and 1),
  effective_confidence numeric(5,4) not null check (effective_confidence between 0 and 1),
  derivation_type text not null check (derivation_type in ('direct','inferred','aggregated')),
  params jsonb not null default '{}'::jsonb,
  evidence_section_id text not null references raw.sections(section_id) on delete restrict,
  registry_version text not null,
  prompt_version text not null,
  model_version text not null,
  pipeline_version text not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

-- ============================================================================
-- KB / GOVERNANCE
-- ============================================================================

create table if not exists kb.registry_items (
  registry_item_id text primary key,
  registry_name text not null,
  key text not null,
  item_kind text not null check (item_kind in ('concept','rule','fact','entity_class','relation_class','layer_class','evidence_class')),
  status text not null check (status in ('active','deprecated','superseded','forbidden_for_new_extraction')),
  superseded_by text,
  schema jsonb not null default '{}'::jsonb,
  example jsonb,
  consumer_list jsonb not null default '[]'::jsonb,
  concept_facets jsonb not null default '{}'::jsonb,
  evidence_notes text,
  evidence_case_count integer not null default 0 check (evidence_case_count >= 0),
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (registry_name, key)
);

create table if not exists kb.registry_aliases (
  alias_id text primary key,
  registry_item_id text not null references kb.registry_items(registry_item_id) on delete cascade,
  alias_text text not null,
  alias_normalized text not null,
  is_exact boolean not null default true,
  regex_pattern text,
  created_at timestamptz not null default now(),
  unique (registry_item_id, alias_normalized)
);

create table if not exists kb.conflict_cases (
  conflict_case_id text primary key,
  conflict_key text not null unique,
  conflict_type text not null check (conflict_type in ('numeric','scope','logic','freshness','source_hierarchy')),
  target_kind text not null check (target_kind in ('rule_instance','fact_instance','triple')),
  target_key text not null,
  status text not null check (status in ('open','resolved','rejected','stale')),
  publish_blocking boolean not null default true,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists kb.conflict_variants (
  conflict_variant_id text primary key,
  conflict_case_id text not null references kb.conflict_cases(conflict_case_id) on delete cascade,
  variant_source_type text not null check (variant_source_type in ('rule_instance','fact_instance','candidate')),
  variant_source_id text not null,
  summary text not null,
  numeric_value jsonb,
  scope jsonb,
  evidence_section_id text references raw.sections(section_id) on delete set null,
  confidence numeric(5,4) not null check (confidence between 0 and 1),
  created_at timestamptz not null default now()
);

create table if not exists kb.resolution_decisions (
  resolution_decision_id text primary key,
  conflict_case_id text not null references kb.conflict_cases(conflict_case_id) on delete cascade,
  decision_type text not null check (decision_type in ('choose_variant','merge_variants','deprecate_old','reject_all','escalate')),
  chosen_variant_id text,
  rationale text not null,
  decided_by text not null,
  decision_source text not null check (decision_source in ('human','policy','system')),
  created_at timestamptz not null default now()
);

-- ============================================================================
-- RUNTIME
-- ============================================================================

create table if not exists runtime.pipeline_events (
  event_id text primary key,
  section_id text references raw.sections(section_id) on delete set null,
  step_name text not null,
  status text not null check (status in ('queued','running','succeeded','failed','dead_letter','poisoned','skipped')),
  input_hash text not null,
  output_hash text,
  idempotency_key text not null,
  retry_class text not null check (retry_class in ('never','safe','transient','hitl_only')),
  retry_count integer not null default 0 check (retry_count >= 0),
  max_retries integer not null default 0 check (max_retries >= 0),
  requires_hitl boolean not null default false,
  hitl_reason text,
  side_effects jsonb not null default '[]'::jsonb,
  prompt_version text,
  model_version text,
  registry_version text,
  pipeline_version text not null,
  schema_version text not null default 'pipeline_event@1',
  started_at timestamptz,
  finished_at timestamptz,
  created_at timestamptz not null default now(),
  unique (step_name, idempotency_key)
);

create table if not exists runtime.replay_manifests (
  replay_manifest_id text primary key,
  section_id text not null references raw.sections(section_id) on delete cascade,
  raw_content_hash text not null,
  prompt_version text not null,
  model_version text not null,
  registry_version text not null,
  pipeline_version text not null,
  mode text not null check (mode in ('strict','recall')),
  source_tier text not null,
  source_priority numeric(4,3) not null check (source_priority between 0 and 1),
  created_at timestamptz not null default now(),
  unique (section_id, raw_content_hash, prompt_version, model_version, registry_version, pipeline_version, mode)
);

create table if not exists runtime.block_generation_checks (
  block_check_id text primary key,
  page_id text not null,
  block_key text not null,
  has_unresolved_conflict boolean not null default false,
  has_unresolved_contradiction boolean not null default false,
  failed_reason text,
  checked_at timestamptz not null default now()
);

-- HITL queue (runtime-level review lane)
create table if not exists runtime.hitl_queue (
  hitl_task_id text primary key,
  run_id text,
  section_id text references raw.sections(section_id) on delete set null,
  task_type text not null,
  priority smallint not null check (priority in (1,2,3)),
  status text not null check (status in ('pending','in_progress','approved','rejected','auto_resolved','failed')),
  decision text,
  reason text,
  payload jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  resolved_at timestamptz
);

-- Execution runs for Temporal orchestration state (coarse registry only).
create table if not exists runtime.execution_runs (
  run_id text primary key,
  workflow_id text not null,
  workflow_type text not null check (workflow_type in ('extract_facts','generate_content','freshness_check')),
  context_key text,
  status text not null check (status in (
    'created','extracting','verifying','persisting','generating','validating','paused_hitl','done','failed'
  )),
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists runtime.execution_run_blobs (
  execution_run_blob_id bigserial primary key,
  run_id text not null references runtime.execution_runs(run_id) on delete cascade,
  field_name text not null check (field_name in (
    'input_payload','extracted_payload','generation_result','verify_report','persist_report','errors'
  )),
  payload_type text not null,
  schema_version integer not null,
  payload_bytes bytea not null,
  payload_hash text not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (run_id, field_name)
);

create table if not exists runtime.answer_traces (
  answer_trace_id text primary key,
  query_hash text not null,
  answer_class text not null check (answer_class in (
    'exact_verified_answer','verified_but_incomplete','needs_more_input',
    'no_verified_data','editorial_context_answer'
  )),
  rule_instance_ids jsonb not null default '[]'::jsonb,
  fact_instance_ids jsonb not null default '[]'::jsonb,
  evidence_section_ids jsonb not null default '[]'::jsonb,
  freshness_timestamp timestamptz,
  created_at timestamptz not null default now()
);

commit;
