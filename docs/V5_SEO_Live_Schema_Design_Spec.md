# V5 SEO Live Schema Design Spec

**Status:** build-spec draft; implementation status: partial live schema + SQLx persistence adapter  
**Owner:** exact live SQL design for SEO operating objects

---

## 1. Purpose

This document defines the exact relational schema to add for the SEO operating system.

This is the canonical owner of:

- table names
- columns and types
- PK/FK
- unique constraints
- check constraints
- indexes
- schema family placement
- source-of-record vs projection-store meaning
- migration notes for each table

This document does not redefine truth-plane semantics owned by `V5_Ultimate_Extraction_Protocol_V2.md`.

---

## 2. Conventions

### 2.1 Key and audit conventions

All persistent SEO tables must include:

- deterministic text primary key when the artifact has canonical identity
- `created_at TIMESTAMPTZ NOT NULL DEFAULT now()`
- `updated_at TIMESTAMPTZ NOT NULL DEFAULT now()`
- version field where derivation or template version matters

### 2.2 Scope fields

Every page-scoped artifact must include:

- `market TEXT NOT NULL`
- `locale TEXT NOT NULL`
- `country_code TEXT NOT NULL`
- `visa_type TEXT NOT NULL`
- `applicant_profile TEXT NOT NULL`
- `scope_signature TEXT NOT NULL`

`scope_signature` is the deterministic hash of the normalized scope tuple.

### 2.3 Store meaning

- canonical SQL source of record lives in `site.*`, `serp.*`, or `monitoring.*`
- Neo4j and Qdrant are projections only
- CMS is a serving destination only

---

## 3. Schema Family Placement

| Schema family | Ownership |
|---|---|
| `site.*` | canonical store for SEO operating artifacts and registries |
| `serp.*` | canonical store for SERP and competitor intelligence artifacts |
| `monitoring.*` | canonical store for SEO metrics, alerts, backlog, and quality failures |

---

## 4. `site.*` Tables

## 4.1 `site.registry_page_types`

| Column | Type | Constraints |
|---|---|---|
| `page_type_key` | `TEXT` | PK |
| `label` | `TEXT` | NOT NULL |
| `status` | `TEXT` | CHECK IN (`proposed`,`active`,`deprecated`,`blocked_for_new_use`,`archived`) |
| `description` | `TEXT` | NOT NULL |
| `owner_role` | `TEXT` | NOT NULL |
| `version` | `INTEGER` | NOT NULL DEFAULT 1 |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Indexes:
- PK only

Migration note:
- seed from `V5_SEO_Information_Architecture_Protocol.md`

## 4.2 `site.registry_intent_types`

Same shape as `site.registry_page_types`, with PK `intent_type_key`.

## 4.3 `site.registry_anchor_strategies`

Same shape as `site.registry_page_types`, with PK `anchor_strategy_key`.

## 4.4 `site.registry_section_templates`

Same shape as `site.registry_page_types`, with PK `section_template_key`.

## 4.5 `site.registry_cta_patterns`

Same shape as `site.registry_page_types`, with PK `cta_pattern_key`.

## 4.6 `site.keyword_clusters`

| Column | Type | Constraints |
|---|---|---|
| `cluster_key` | `TEXT` | PK |
| `market` | `TEXT` | NOT NULL |
| `locale` | `TEXT` | NOT NULL |
| `country_code` | `TEXT` | NOT NULL |
| `visa_type` | `TEXT` | NOT NULL |
| `applicant_profile` | `TEXT` | NOT NULL |
| `scope_signature` | `TEXT` | NOT NULL |
| `seed_keyword` | `TEXT` | NOT NULL |
| `dominant_intent` | `TEXT` | FK -> `site.registry_intent_types(intent_type_key)` |
| `keyword_count` | `INTEGER` | NOT NULL CHECK >= 1 |
| `cluster_version` | `TEXT` | NOT NULL |
| `status` | `TEXT` | CHECK IN (`candidate`,`active`,`deprecated`,`archived`) |
| `source_batch_key` | `TEXT` | FK -> `serp.query_batches(batch_key)` |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(scope_signature, seed_keyword, cluster_version)`

Indexes:
- `(scope_signature)`
- `(dominant_intent)`
- `(status)`

## 4.7 `site.search_features`

| Column | Type | Constraints |
|---|---|---|
| `search_feature_key` | `TEXT` | PK |
| `market` | `TEXT` | NOT NULL |
| `locale` | `TEXT` | NOT NULL |
| `normalized_query` | `TEXT` | NOT NULL |
| `feature_type` | `TEXT` | NOT NULL |
| `feature_payload` | `JSONB` | NOT NULL DEFAULT `'{}'::jsonb` |
| `captured_at` | `TIMESTAMPTZ` | NOT NULL |
| `fresh_until` | `TIMESTAMPTZ` | NOT NULL |
| `status` | `TEXT` | CHECK IN (`active`,`stale`,`archived`) |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(market, locale, normalized_query, feature_type, captured_at)`

Indexes:
- `(market, locale)`
- `(feature_type)`
- `(status, fresh_until)`

## 4.8 `site.content_gaps`

| Column | Type | Constraints |
|---|---|---|
| `content_gap_key` | `TEXT` | PK |
| `market` | `TEXT` | NOT NULL |
| `locale` | `TEXT` | NOT NULL |
| `country_code` | `TEXT` | NOT NULL |
| `visa_type` | `TEXT` | NOT NULL |
| `applicant_profile` | `TEXT` | NOT NULL |
| `scope_signature` | `TEXT` | NOT NULL |
| `missing_topic` | `TEXT` | NOT NULL |
| `competitor_hint` | `TEXT` | NOT NULL |
| `support_count` | `INTEGER` | NOT NULL CHECK >= 1 |
| `reliability_level` | `TEXT` | CHECK IN (`R0_forbidden`,`R1_weak`,`R2_supported`,`R3_strong`) |
| `status` | `TEXT` | CHECK IN (`candidate`,`accepted`,`rejected`,`archived`) |
| `source_batch_key` | `TEXT` | FK -> `serp.query_batches(batch_key)` |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(scope_signature, missing_topic, source_batch_key)`

Indexes:
- `(scope_signature)`
- `(status)`
- `(reliability_level)`

## 4.9 `site.section_templates`

| Column | Type | Constraints |
|---|---|---|
| `section_template_key` | `TEXT` | PK |
| `page_type_key` | `TEXT` | FK -> `site.registry_page_types(page_type_key)` |
| `section_role` | `TEXT` | NOT NULL |
| `template_version` | `TEXT` | NOT NULL |
| `required_fields` | `JSONB` | NOT NULL DEFAULT `'[]'::jsonb` |
| `allowed_traceability_labels` | `JSONB` | NOT NULL DEFAULT `'[]'::jsonb` |
| `status` | `TEXT` | CHECK IN (`active`,`deprecated`,`archived`) |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(page_type_key, section_role, template_version)`

Indexes:
- `(page_type_key)`
- `(status)`

## 4.10 `site.page_blueprints`

| Column | Type | Constraints |
|---|---|---|
| `blueprint_key` | `TEXT` | PK |
| `page_type_key` | `TEXT` | FK -> `site.registry_page_types(page_type_key)` |
| `dominant_intent` | `TEXT` | FK -> `site.registry_intent_types(intent_type_key)` |
| `scope_class` | `TEXT` | NOT NULL |
| `blueprint_version` | `TEXT` | NOT NULL |
| `required_sections` | `JSONB` | NOT NULL DEFAULT `'[]'::jsonb` |
| `optional_sections` | `JSONB` | NOT NULL DEFAULT `'[]'::jsonb` |
| `forbidden_sections` | `JSONB` | NOT NULL DEFAULT `'[]'::jsonb` |
| `required_link_roles` | `JSONB` | NOT NULL DEFAULT `'[]'::jsonb` |
| `metadata_obligations` | `JSONB` | NOT NULL DEFAULT `'[]'::jsonb` |
| `review_gates` | `JSONB` | NOT NULL DEFAULT `'[]'::jsonb` |
| `status` | `TEXT` | CHECK IN (`draft`,`active`,`deprecated`,`archived`) |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(page_type_key, dominant_intent, scope_class, blueprint_version)`

Indexes:
- `(page_type_key, dominant_intent)`
- `(status)`

## 4.11 `site.page_nodes`

| Column | Type | Constraints |
|---|---|---|
| `page_node_key` | `TEXT` | PK |
| `market` | `TEXT` | NOT NULL |
| `locale` | `TEXT` | NOT NULL |
| `country_code` | `TEXT` | NOT NULL |
| `visa_type` | `TEXT` | NOT NULL |
| `applicant_profile` | `TEXT` | NOT NULL |
| `scope_signature` | `TEXT` | NOT NULL |
| `canonical_url` | `TEXT` | NOT NULL |
| `canonical_slug` | `TEXT` | NOT NULL |
| `page_type_key` | `TEXT` | FK -> `site.registry_page_types(page_type_key)` |
| `dominant_intent` | `TEXT` | FK -> `site.registry_intent_types(intent_type_key)` |
| `secondary_intents` | `JSONB` | NOT NULL DEFAULT `'[]'::jsonb` |
| `keyword_cluster_key` | `TEXT` | FK -> `site.keyword_clusters(cluster_key)` |
| `current_blueprint_key` | `TEXT` | FK -> `site.page_blueprints(blueprint_key)` |
| `lifecycle_state` | `TEXT` | CHECK IN (`planned`,`blueprint_ready`,`draft_ready`,`qa_failed`,`ready_for_review`,`approved`,`published`,`stale`,`needs_rebuild`,`deprecated`) |
| `status` | `TEXT` | CHECK IN (`active`,`deprecated`,`archived`) |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(canonical_url)`
- `UNIQUE(scope_signature, page_type_key, dominant_intent, canonical_slug)`

Indexes:
- `(scope_signature)`
- `(dominant_intent)`
- `(lifecycle_state)`
- `(status)`

## 4.12 `site.page_briefs`

| Column | Type | Constraints |
|---|---|---|
| `page_brief_key` | `TEXT` | PK |
| `page_node_key` | `TEXT` | FK -> `site.page_nodes(page_node_key)` |
| `blueprint_key` | `TEXT` | FK -> `site.page_blueprints(blueprint_key)` |
| `truth_snapshot_ref` | `TEXT` | NOT NULL |
| `goal` | `TEXT` | NOT NULL |
| `target_audience` | `TEXT` | NOT NULL |
| `required_fact_domains` | `JSONB` | NOT NULL DEFAULT `'[]'::jsonb` |
| `link_obligations` | `JSONB` | NOT NULL DEFAULT `'[]'::jsonb` |
| `metadata_requirements` | `JSONB` | NOT NULL DEFAULT `'[]'::jsonb` |
| `status` | `TEXT` | CHECK IN (`draft`,`ready`,`superseded`,`archived`) |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(page_node_key, truth_snapshot_ref)`

Indexes:
- `(page_node_key)`
- `(status)`

## 4.13 `site.page_drafts`

| Column | Type | Constraints |
|---|---|---|
| `draft_key` | `TEXT` | PK |
| `page_node_key` | `TEXT` | FK -> `site.page_nodes(page_node_key)` |
| `page_brief_key` | `TEXT` | FK -> `site.page_briefs(page_brief_key)` |
| `blueprint_key` | `TEXT` | FK -> `site.page_blueprints(blueprint_key)` |
| `draft_revision` | `INTEGER` | NOT NULL CHECK >= 1 |
| `assembly_version` | `TEXT` | NOT NULL |
| `truth_snapshot_ref` | `TEXT` | NOT NULL |
| `title_candidate` | `TEXT` | NOT NULL |
| `meta_description_candidate` | `TEXT` | NOT NULL |
| `h1_candidate` | `TEXT` | NOT NULL |
| `canonical_url` | `TEXT` | NOT NULL |
| `content_payload` | `JSONB` | NOT NULL |
| `traceability_summary` | `JSONB` | NOT NULL |
| `qa_state` | `TEXT` | CHECK IN (`draft_ready`,`qa_failed`,`ready_for_review`,`approved`,`published`,`deprecated`) |
| `published_revision_ref` | `TEXT` | |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(page_node_key, draft_revision)`

Indexes:
- `(page_node_key)`
- `(qa_state)`
- `(truth_snapshot_ref)`

## 4.14 `site.link_recommendations`

| Column | Type | Constraints |
|---|---|---|
| `link_recommendation_key` | `TEXT` | PK |
| `source_page_node_key` | `TEXT` | FK -> `site.page_nodes(page_node_key)` |
| `target_page_node_key` | `TEXT` | FK -> `site.page_nodes(page_node_key)` |
| `link_role` | `TEXT` | NOT NULL |
| `anchor_strategy_key` | `TEXT` | FK -> `site.registry_anchor_strategies(anchor_strategy_key)` |
| `anchor_text` | `TEXT` | NOT NULL |
| `link_score` | `NUMERIC(6,3)` | NOT NULL |
| `recommendation_status` | `TEXT` | CHECK IN (`required`,`optional`,`rejected`,`applied`,`expired`) |
| `scoring_version` | `TEXT` | NOT NULL |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(source_page_node_key, target_page_node_key, link_role, anchor_strategy_key, scoring_version)`

Indexes:
- `(source_page_node_key)`
- `(target_page_node_key)`
- `(recommendation_status)`

## 4.15 `site.cannibalization_conflicts`

| Column | Type | Constraints |
|---|---|---|
| `conflict_key` | `TEXT` | PK |
| `page_key_a` | `TEXT` | FK -> `site.page_nodes(page_node_key)` |
| `page_key_b` | `TEXT` | FK -> `site.page_nodes(page_node_key)` |
| `scope_signature` | `TEXT` | NOT NULL |
| `conflict_reason` | `TEXT` | NOT NULL |
| `severity` | `TEXT` | CHECK IN (`warning`,`blocking`) |
| `detector_version` | `TEXT` | NOT NULL |
| `resolution_status` | `TEXT` | CHECK IN (`open`,`in_review`,`resolved`,`superseded`,`rejected`) |
| `resolution_outcome` | `TEXT` | CHECK IN (`merge`,`split`,`hierarchy_fix`,`scope_fix`,`deprecate_lower_priority_page`) |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(page_key_a, page_key_b, detector_version)`

Indexes:
- `(scope_signature)`
- `(resolution_status, severity)`

## 4.16 `site.page_lifecycle_events`

| Column | Type | Constraints |
|---|---|---|
| `page_lifecycle_event_id` | `BIGSERIAL` | PK |
| `page_node_key` | `TEXT` | FK -> `site.page_nodes(page_node_key)` |
| `from_state` | `TEXT` | NOT NULL |
| `to_state` | `TEXT` | NOT NULL |
| `event_reason` | `TEXT` | NOT NULL |
| `actor_type` | `TEXT` | CHECK IN (`system`,`workflow`,`human`) |
| `actor_ref` | `TEXT` | |
| `payload` | `JSONB` | NOT NULL DEFAULT `'{}'::jsonb` |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Indexes:
- `(page_node_key, created_at DESC)`
- `(to_state, created_at DESC)`

---

## 5. `serp.*` Tables

## 5.1 `serp.query_batches`

| Column | Type | Constraints |
|---|---|---|
| `batch_key` | `TEXT` | PK |
| `market` | `TEXT` | NOT NULL |
| `locale` | `TEXT` | NOT NULL |
| `country_code` | `TEXT` | NOT NULL |
| `visa_type` | `TEXT` | NOT NULL |
| `applicant_profile` | `TEXT` | NOT NULL |
| `scope_signature` | `TEXT` | NOT NULL |
| `capture_window_start` | `TIMESTAMPTZ` | NOT NULL |
| `capture_window_end` | `TIMESTAMPTZ` | NOT NULL |
| `query_seed_set` | `JSONB` | NOT NULL |
| `status` | `TEXT` | CHECK IN (`captured`,`normalized`,`expired`,`archived`) |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Indexes:
- `(scope_signature)`
- `(status)`

## 5.2 `serp.serp_patterns`

| Column | Type | Constraints |
|---|---|---|
| `serp_pattern_key` | `TEXT` | PK |
| `batch_key` | `TEXT` | FK -> `serp.query_batches(batch_key)` |
| `pattern_type` | `TEXT` | NOT NULL |
| `pattern_signature` | `TEXT` | NOT NULL |
| `reliability_level` | `TEXT` | CHECK IN (`R0_forbidden`,`R1_weak`,`R2_supported`,`R3_strong`) |
| `freshness_class` | `TEXT` | CHECK IN (`volatile`,`standard`,`stable`) |
| `captured_at` | `TIMESTAMPTZ` | NOT NULL |
| `fresh_until` | `TIMESTAMPTZ` | NOT NULL |
| `status` | `TEXT` | CHECK IN (`active`,`stale`,`archived`) |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(batch_key, pattern_type, pattern_signature)`

Indexes:
- `(reliability_level)`
- `(status, fresh_until)`

## 5.3 `serp.serp_pattern_observations`

| Column | Type | Constraints |
|---|---|---|
| `observation_id` | `BIGSERIAL` | PK |
| `serp_pattern_key` | `TEXT` | FK -> `serp.serp_patterns(serp_pattern_key)` |
| `query_text` | `TEXT` | NOT NULL |
| `rank_position` | `INTEGER` | CHECK BETWEEN 1 AND 100 |
| `competitor_url` | `TEXT` | NOT NULL |
| `observation_payload` | `JSONB` | NOT NULL |
| `captured_at` | `TIMESTAMPTZ` | NOT NULL |

Indexes:
- `(serp_pattern_key)`
- `(competitor_url)`

## 5.4 `serp.competitor_pages`

| Column | Type | Constraints |
|---|---|---|
| `competitor_page_key` | `TEXT` | PK |
| `batch_key` | `TEXT` | FK -> `serp.query_batches(batch_key)` |
| `domain_norm` | `TEXT` | NOT NULL |
| `url` | `TEXT` | NOT NULL |
| `url_norm` | `TEXT` | NOT NULL |
| `page_type_hint` | `TEXT` | NOT NULL |
| `primary_intent_hint` | `TEXT` | NOT NULL |
| `coverage_payload` | `JSONB` | NOT NULL |
| `captured_at` | `TIMESTAMPTZ` | NOT NULL |
| `status` | `TEXT` | CHECK IN (`active`,`excluded`,`archived`) |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(batch_key, url_norm)`

Indexes:
- `(domain_norm)`
- `(status)`

## 5.5 `serp.competitor_section_patterns`

| Column | Type | Constraints |
|---|---|---|
| `competitor_section_pattern_key` | `TEXT` | PK |
| `competitor_page_key` | `TEXT` | FK -> `serp.competitor_pages(competitor_page_key)` |
| `section_role_hint` | `TEXT` | NOT NULL |
| `heading_text` | `TEXT` | NOT NULL |
| `pattern_payload` | `JSONB` | NOT NULL |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Unique:
- `UNIQUE(competitor_page_key, section_role_hint, heading_text)`

Indexes:
- `(competitor_page_key)`

## 5.6 `serp.opportunity_candidates`

| Column | Type | Constraints |
|---|---|---|
| `opportunity_key` | `TEXT` | PK |
| `batch_key` | `TEXT` | FK -> `serp.query_batches(batch_key)` |
| `scope_signature` | `TEXT` | NOT NULL |
| `candidate_type` | `TEXT` | NOT NULL |
| `candidate_topic` | `TEXT` | NOT NULL |
| `reliability_level` | `TEXT` | CHECK IN (`R0_forbidden`,`R1_weak`,`R2_supported`,`R3_strong`) |
| `support_count` | `INTEGER` | NOT NULL CHECK >= 1 |
| `opportunity_score` | `NUMERIC(6,3)` | |
| `status` | `TEXT` | CHECK IN (`candidate`,`accepted`,`rejected`,`implemented`,`archived`) |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Indexes:
- `(scope_signature)`
- `(status)`
- `(opportunity_score DESC)`

---

## 6. `monitoring.*` Tables

## 6.1 `monitoring.seo_metric_snapshots`

| Column | Type | Constraints |
|---|---|---|
| `metric_snapshot_id` | `BIGSERIAL` | PK |
| `metric_name` | `TEXT` | NOT NULL |
| `page_node_key` | `TEXT` | FK -> `site.page_nodes(page_node_key)` |
| `scope_signature` | `TEXT` | NOT NULL |
| `metric_window` | `TEXT` | NOT NULL |
| `metric_source` | `TEXT` | NOT NULL |
| `metric_value` | `NUMERIC(12,4)` | NOT NULL |
| `captured_at` | `TIMESTAMPTZ` | NOT NULL |

Indexes:
- `(metric_name, captured_at DESC)`
- `(page_node_key, captured_at DESC)`
- `(scope_signature, captured_at DESC)`

## 6.2 `monitoring.seo_freshness_alerts`

| Column | Type | Constraints |
|---|---|---|
| `freshness_alert_key` | `TEXT` | PK |
| `artifact_type` | `TEXT` | NOT NULL |
| `artifact_key` | `TEXT` | NOT NULL |
| `severity` | `TEXT` | CHECK IN (`warning`,`critical`) |
| `alert_reason` | `TEXT` | NOT NULL |
| `status` | `TEXT` | CHECK IN (`open`,`acknowledged`,`resolved`,`suppressed`) |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Indexes:
- `(artifact_type, artifact_key)`
- `(status, severity)`

## 6.3 `monitoring.seo_rebuild_backlog`

| Column | Type | Constraints |
|---|---|---|
| `rebuild_backlog_key` | `TEXT` | PK |
| `page_node_key` | `TEXT` | FK -> `site.page_nodes(page_node_key)` |
| `trigger_type` | `TEXT` | NOT NULL |
| `priority_score` | `NUMERIC(6,3)` | NOT NULL |
| `status` | `TEXT` | CHECK IN (`queued`,`running`,`blocked`,`done`,`cancelled`) |
| `source_ref` | `TEXT` | NOT NULL |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Indexes:
- `(status, priority_score DESC)`
- `(page_node_key)`

## 6.4 `monitoring.seo_quality_failures`

| Column | Type | Constraints |
|---|---|---|
| `quality_failure_key` | `TEXT` | PK |
| `artifact_type` | `TEXT` | NOT NULL |
| `artifact_key` | `TEXT` | NOT NULL |
| `failure_class` | `TEXT` | NOT NULL |
| `failure_payload` | `JSONB` | NOT NULL |
| `severity` | `TEXT` | CHECK IN (`warning`,`blocking`) |
| `status` | `TEXT` | CHECK IN (`open`,`resolved`,`ignored`) |
| `created_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL DEFAULT now() |

Indexes:
- `(artifact_type, artifact_key)`
- `(status, severity)`

---

## 7. Projection Boundaries

Source-of-record tables are all tables in this document.

Projection-only destinations:

- Neo4j projections from `site.keyword_clusters`, `site.page_nodes`, `site.page_blueprints`, `site.content_gaps`, `site.search_features`, `site.section_templates`
- Qdrant projections from `site.keyword_clusters`, `site.page_blueprints`, `site.search_features`, `serp.serp_patterns`, `site.section_templates`
- CMS serving destination from `site.page_drafts` after approval/publish gating

---

## 8. Migration Notes

### 8.1 Existing source mapping

- `site.page_context_map` -> seed `site.page_nodes` scope and URL mapping
- `raw.pages` / `raw.sections` -> seed page inventory and draft source references
- historical `serp.*` tables -> seed `serp.query_batches`, `serp.competitor_pages`, `serp.opportunity_candidates`

### 8.2 Migration rules

- no migration may create verified truth objects
- backfilled SEO objects must carry deterministic keys
- ambiguous scope mapping routes to review state, not silent normalization

---

## 9. Acceptance Checks

This spec is complete only if:

- every required table from the roadmap exists here
- every table has exact columns and types
- every canonical uniqueness rule is explicit
- every page-scoped table includes `scope_signature`
- every table has a declared source-of-record meaning
- migration entry points from existing sources are explicit

---

## 10. Scope Enforcement Addendum

### 10.1 Scope uniqueness enforcement

For every page-scoped or scope-bearing SEO table:

- the five raw scope columns remain source of record,
- `scope_signature` is a deterministic derived column,
- writes must be rejected if raw scope columns are not already normalized,
- uniqueness must be enforced on the raw normalized tuple plus the table's business identity fields.

Required raw tuple columns:

- `market`
- `locale`
- `country_code`
- `visa_type`
- `applicant_profile`

Required implementation rule:

- storage writes must validate `scope_signature == blake3(canonical_scope_tuple_string)`

Required uniqueness examples:

- `site.page_nodes`: `UNIQUE(market, locale, country_code, visa_type, applicant_profile, page_type_key, dominant_intent, canonical_slug)`
- `site.keyword_clusters`: `UNIQUE(market, locale, country_code, visa_type, applicant_profile, seed_keyword, cluster_version)`
- `serp.opportunity_candidates`: `UNIQUE(market, locale, country_code, visa_type, applicant_profile, candidate_type, candidate_topic, source_batch_key)`

### 10.2 Signature mismatch handling

When a persisted raw scope tuple and `scope_signature` disagree:

- the write must fail,
- no auto-correction may occur inside storage,
- runtime must emit a deterministic validation failure,
- automation must be able to recompute and compare the signature.

---

## Appendix A. JSONB Payload Contracts

Every JSONB payload listed here is schema-owned and may not change shape without a documented contract update.

### A.1 `feature_payload`

Object with:

- `feature_variant: string`
- `snippet_text: string | null`
- `rank_group: string`
- `source_engine: string`
- `raw_capture_ref: string`

### A.2 `required_fields`

Array of objects:

- `field_key: string`
- `required: boolean`
- `source_label: string`
- `notes: string | null`

### A.3 `allowed_traceability_labels`

Array of strings from:

- `verified_fact`
- `verified_summary`
- `editorial_extrapolation`
- `serp_pattern_reference`
- `business_copy`

### A.4 `required_sections`, `optional_sections`, `forbidden_sections`

Array of objects:

- `section_key: string`
- `section_role: string`
- `template_key: string | null`
- `required_traceability: string[]`

### A.5 `required_link_roles`

Array of objects:

- `link_role: string`
- `target_page_type: string`
- `required: boolean`
- `minimum_count: integer`

### A.6 `metadata_obligations` and `metadata_requirements`

Array of objects:

- `field_key: string`
- `required: boolean`
- `source_rule: string`
- `max_length: integer | null`

### A.7 `review_gates`

Array of objects:

- `gate_key: string`
- `blocking: boolean`
- `owner_role: string`
- `notes: string | null`

### A.8 `secondary_intents`

Array of objects:

- `intent_key: string`
- `support_level: string`
- `subordinate_to_dominant: boolean`

### A.9 `required_fact_domains`

Array of strings naming verified fact families required by the page brief.

### A.10 `link_obligations`

Array of objects:

- `link_role: string`
- `target_key: string | null`
- `target_page_type: string | null`
- `required: boolean`

### A.11 `content_payload`

Object with:

- `outline: object[]`
- `sections: object[]`
- `faq_items: object[]`
- `cta_blocks: object[]`
- `schema_markup_candidates: object[]`

Each `sections` item must include:

- `section_key: string`
- `heading: string`
- `body_markdown: string`
- `traceability_labels: string[]`
- `support_refs: string[]`

### A.12 `traceability_summary`

Object with:

- `fragment_count: integer`
- `verified_fact_refs: string[]`
- `verified_summary_refs: string[]`
- `editorial_extrapolation_refs: string[]`
- `unsupported_fragments: object[]`

### A.13 lifecycle event `payload`

Object with:

- `from_state: string | null`
- `to_state: string`
- `reason_key: string`
- `causal_step: string | null`
- `causal_execution_key: string | null`

### A.14 `query_seed_set`

Array of objects:

- `seed_query: string`
- `normalized_query: string`
- `priority_weight: number`

### A.15 `observation_payload`

Object with:

- `observation_type: string`
- `pattern_signature: string`
- `html_fragment_ref: string | null`
- `normalized_features: object`

### A.16 `coverage_payload`

Object with:

- `covered_topics: string[]`
- `covered_entities: string[]`
- `covered_rule_refs: string[]`
- `missing_topics: string[]`

### A.17 `pattern_payload`

Object with:

- `heading_text: string`
- `section_role_hint: string`
- `structure_signature: string`
- `confidence_notes: string | null`

### A.18 `failure_payload`

Object with:

- `failure_class: string`
- `artifact_key: string`
- `causal_step: string`
- `details: object`
- `retryable: boolean`

### A.19 JSONB invariants

- No JSONB payload may be used as an excuse to skip stable column design.
- Every JSONB field must have a documented top-level object or array shape.
- Unknown keys are forbidden unless the payload contract marks an `extensions` map explicitly.
