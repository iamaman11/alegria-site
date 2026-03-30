# V5 SEO Graph And Retrieval Projection Spec

**Status:** build-spec draft  
**Owner:** exact projection-layer contract for SEO-derived Neo4j and Qdrant surfaces  
**Depends on:** `V5_SEO_Foundation_Contracts.md`, `V5_SERP_Intelligence_Protocol.md`, `V5_SEO_Information_Architecture_Protocol.md`, `V5_SEO_Live_Schema_Design_Spec.md`

## 1. Purpose

This document defines the exact projection contract for SEO-derived graph and retrieval objects.

It is the canonical owner of:

- Neo4j SEO node labels and property shapes,
- Neo4j SEO edge types and upsert rules,
- allowed cross-layer links from SEO-derived nodes into truth-core,
- Qdrant SEO collections, point identity, payload schema, and rebuild semantics,
- projection freshness and invalidation semantics.

This document is not the owner of truth-core identity, truth merge policy, or V2 retrieval-plane semantics.

## 2. Projection Boundaries

### 2.1 Plane semantics

All objects defined here are `product-derived` or `retrieval-derived` SEO artifacts unless explicitly marked otherwise.

No node or point defined here may be treated as canonical truth.

### 2.2 Source-of-record rule

Source of record remains relational tables from the live schema spec.

- Neo4j is a graph projection and analytics surface.
- Qdrant is a retrieval projection and ranking support surface.
- Neither store may originate SEO owner-state independently.

### 2.3 Allowed source tables

Primary sources:

- `site.keyword_clusters`
- `site.page_nodes`
- `site.page_blueprints`
- `site.content_gaps`
- `site.search_features`
- `site.section_templates`
- `site.link_recommendations`
- `site.cannibalization_conflicts`
- `site.page_drafts`
- `serp.serp_patterns`
- `serp.opportunity_candidates`

Referenced truth-core sources only through existing V2-projected keys:

- verified concepts
- verified topics
- verified rules
- verified operational entities

## 3. Neo4j Projection Model

## 3.1 Node labels

### 3.1.1 `:KeywordCluster`

**Source table:** `site.keyword_clusters`

**Identity**

- `cluster_key` string, unique

**Required properties**

- `cluster_key`
- `cluster_version`
- `scope_signature`
- `dominant_intent_key`
- `status`
- `cluster_label`
- `primary_query`
- `language_code`
- `country_code`
- `market_code`
- `opportunity_score`
- `freshness_class`
- `updated_at`

### 3.1.2 `:PageNode`

**Source table:** `site.page_nodes`

**Identity**

- `page_node_key` string, unique

**Required properties**

- `page_node_key`
- `page_type_key`
- `scope_signature`
- `dominant_intent_key`
- `canonical_url_path`
- `status`
- `lifecycle_state`
- `title_candidate`
- `h1_candidate`
- `business_priority`
- `truth_coverage_score`
- `opportunity_score`
- `freshness_class`
- `updated_at`

### 3.1.3 `:PageBlueprint`

**Source table:** `site.page_blueprints`

**Identity**

- `blueprint_key` string, unique

**Required properties**

- `blueprint_key`
- `page_type_key`
- `scope_signature`
- `version`
- `status`
- `purpose`
- `dominant_intent_key`
- `metadata_policy_key`
- `faq_allowed`
- `table_allowed`
- `checklist_allowed`
- `updated_at`

### 3.1.4 `:ContentGap`

**Source table:** `site.content_gaps`

**Identity**

- `content_gap_key` string, unique

**Required properties**

- `content_gap_key`
- `gap_type`
- `scope_signature`
- `severity`
- `status`
- `source_system`
- `opportunity_score`
- `updated_at`

### 3.1.5 `:SearchFeature`

**Source table:** `site.search_features`

**Identity**

- `search_feature_key` string, unique

**Required properties**

- `search_feature_key`
- `feature_type`
- `query_fingerprint`
- `scope_signature`
- `status`
- `observed_frequency`
- `updated_at`

### 3.1.6 `:SectionTemplate`

**Source table:** `site.section_templates`

**Identity**

- `section_template_key` string, unique

**Required properties**

- `section_template_key`
- `template_type`
- `scope_signature`
- `status`
- `section_role_key`
- `prompt_contract_key`
- `updated_at`

### 3.1.7 `:SERPPattern`

**Source table:** `serp.serp_patterns`

**Identity**

- `serp_pattern_key` string, unique

**Required properties**

- `serp_pattern_key`
- `pattern_type`
- `query_fingerprint`
- `scope_signature`
- `reliability_level`
- `freshness_bucket`
- `status`
- `support_count`
- `updated_at`

### 3.1.8 `:LinkRecommendation`

**Source table:** `site.link_recommendations`

**Identity**

- `link_recommendation_key` string, unique

**Required properties**

- `link_recommendation_key`
- `source_page_node_key`
- `target_page_node_key`
- `anchor_strategy_key`
- `required_flag`
- `link_score`
- `status`
- `updated_at`

### 3.1.9 `:CannibalizationConflict`

**Source table:** `site.cannibalization_conflicts`

**Identity**

- `cannibalization_conflict_key` string, unique

**Required properties**

- `cannibalization_conflict_key`
- `scope_signature`
- `conflict_type`
- `severity`
- `status`
- `left_page_node_key`
- `right_page_node_key`
- `updated_at`

## 3.2 Edge types

All edge upserts must be deterministic. Edge identity is the tuple:

- `edge_type`
- `from_key`
- `to_key`
- `scope_signature` when applicable

### 3.2.1 Page structure edges

- `(:PageNode)-[:GENERATED_FROM_BLUEPRINT]->(:PageBlueprint)`
- `(:PageNode)-[:BELONGS_TO_CLUSTER]->(:KeywordCluster)`
- `(:PageNode)-[:CHILD_OF_HUB]->(:PageNode)`
- `(:PageNode)-[:SATISFIES_INTENT]->(:KeywordCluster)`
- `(:PageNode)-[:HAS_CONTENT_GAP]->(:ContentGap)`
- `(:PageNode)-[:RISKS_CANNIBALIZATION_WITH]->(:PageNode)`

### 3.2.2 Linking edges

- `(:PageNode)-[:SHOULD_LINK_TO]->(:PageNode)`
- `(:LinkRecommendation)-[:LINK_SOURCE]->(:PageNode)`
- `(:LinkRecommendation)-[:LINK_TARGET]->(:PageNode)`
- `(:LinkRecommendation)-[:SUPPORTED_BY_PATTERN]->(:SERPPattern)`

### 3.2.3 SERP and search feature edges

- `(:KeywordCluster)-[:SUPPORTED_BY_SERP_PATTERN]->(:SERPPattern)`
- `(:PageNode)-[:ALIGNS_WITH_SEARCH_FEATURE]->(:SearchFeature)`
- `(:SERPPattern)-[:INDICATES_CONTENT_GAP]->(:ContentGap)`

### 3.2.4 Template and blueprint edges

- `(:PageBlueprint)-[:USES_SECTION_TEMPLATE]->(:SectionTemplate)`
- `(:SectionTemplate)-[:APPLIES_TO_CLUSTER]->(:KeywordCluster)`

## 3.3 Allowed cross-layer links to truth-core

SEO-derived nodes may link to truth-core only through read-only reference edges.

Allowed cross-layer edges:

- `(:PageNode)-[:COVERS_TOPIC]->(:Topic)`
- `(:PageNode)-[:COVERS_CONCEPT]->(:Concept)`
- `(:PageNode)-[:COVERS_RULE]->(:RuleInstance)`
- `(:KeywordCluster)-[:TARGETS_TOPIC]->(:Topic)`
- `(:SERPPattern)-[:CO_OCCURS_WITH_TOPIC]->(:Topic)`

Forbidden cross-layer actions:

- SEO nodes creating, mutating, or superseding truth-core nodes
- SEO edges participating in truth merge or canonical resolution
- SEO-derived reliability altering verified truth status

## 3.4 Neo4j indexes and constraints

Required constraints:

- unique on every node identity key listed above
- unique composite on `PageNode(canonical_url_path, scope_signature)`
- unique composite on `SERPPattern(query_fingerprint, pattern_type, scope_signature)`

Required indexes:

- `PageNode(status, lifecycle_state)`
- `KeywordCluster(status, dominant_intent_key)`
- `ContentGap(status, severity)`
- `LinkRecommendation(status, required_flag)`
- `CannibalizationConflict(status, severity)`

## 4. Neo4j Projection Rules

### 4.1 Upsert rules

- Source rows marked inactive or deprecated must either remove or tombstone the corresponding graph node according to projection mode.
- Projection writes are idempotent by source row identity plus version.
- Projection must ignore rows that fail required status or completeness gates.

### 4.2 Freshness semantics

Each projected node must carry:

- `projection_updated_at`
- `source_updated_at`
- `projection_version`
- `freshness_bucket`

A node becomes stale when source freshness SLA is exceeded or when invalidation events mark the source version obsolete.

### 4.3 Rebuild and invalidation triggers

Rebuild of affected SEO graph nodes is required on:

- source table row insert or update
- registry change affecting page type, intent type, anchor strategy, or section template
- draft QA failure that invalidates page publication viability
- canonical URL change
- truth reference invalidation for covered topic, concept, or rule

## 5. Qdrant Retrieval Projection

## 5.1 Collections

The following SEO collections are required and separate from truth-core retrieval collections:

1. `seo_keyword_clusters`
2. `seo_page_blueprints`
3. `seo_serp_patterns`
4. `seo_section_templates`
5. `seo_link_targets`
6. `seo_content_gaps`

## 5.2 Collection contracts

### 5.2.1 `seo_keyword_clusters`

**Point id**

- `cluster_key`

**Embedding source text**

- `cluster_label`
- `primary_query`
- normalized supporting query set
- dominant intent description

**Payload schema**

- `cluster_key`
- `scope_signature`
- `dominant_intent_key`
- `page_type_candidates[]`
- `status`
- `opportunity_score`
- `freshness_class`
- `source_version`

**Filter fields**

- `scope_signature`
- `dominant_intent_key`
- `status`
- `freshness_class`

### 5.2.2 `seo_page_blueprints`

**Point id**

- `blueprint_key`

**Embedding source text**

- `purpose`
- required sections summary
- required facts summary
- dominant intent summary

**Payload schema**

- `blueprint_key`
- `page_type_key`
- `scope_signature`
- `dominant_intent_key`
- `status`
- `faq_allowed`
- `table_allowed`
- `checklist_allowed`
- `source_version`

**Filter fields**

- `page_type_key`
- `scope_signature`
- `dominant_intent_key`
- `status`

### 5.2.3 `seo_serp_patterns`

**Point id**

- `serp_pattern_key`

**Embedding source text**

- normalized pattern summary
- observed section skeleton summary
- normalized intent signal summary

**Payload schema**

- `serp_pattern_key`
- `pattern_type`
- `query_fingerprint`
- `scope_signature`
- `reliability_level`
- `status`
- `support_count`
- `freshness_bucket`

**Filter fields**

- `pattern_type`
- `query_fingerprint`
- `scope_signature`
- `reliability_level`
- `status`

### 5.2.4 `seo_section_templates`

**Point id**

- `section_template_key`

**Embedding source text**

- template purpose summary
- allowed evidence bindings summary
- section role summary

**Payload schema**

- `section_template_key`
- `template_type`
- `scope_signature`
- `section_role_key`
- `status`
- `source_version`

**Filter fields**

- `template_type`
- `scope_signature`
- `section_role_key`
- `status`

### 5.2.5 `seo_link_targets`

**Point id**

- `page_node_key`

**Embedding source text**

- page purpose
- title candidate
- H1 candidate
- related topic summaries
- linkable anchor descriptors

**Payload schema**

- `page_node_key`
- `scope_signature`
- `page_type_key`
- `dominant_intent_key`
- `canonical_url_path`
- `status`
- `lifecycle_state`
- `truth_coverage_score`
- `source_version`

**Filter fields**

- `scope_signature`
- `page_type_key`
- `dominant_intent_key`
- `status`
- `lifecycle_state`

### 5.2.6 `seo_content_gaps`

**Point id**

- `content_gap_key`

**Embedding source text**

- gap description
- affected topic summary
- competitor deficit or opportunity summary

**Payload schema**

- `content_gap_key`
- `gap_type`
- `scope_signature`
- `severity`
- `status`
- `opportunity_score`
- `source_version`

**Filter fields**

- `gap_type`
- `scope_signature`
- `severity`
- `status`

## 5.3 Retrieval separation rules

- SEO collections must not be merged into truth-core collections.
- Retrieval responses from SEO collections are advisory inputs for planning, linking, and draft assembly.
- SEO retrieval results must preserve artifact provenance and reliability metadata.

## 5.4 Point rebuild triggers

Qdrant point rebuild is required on:

- source row content or version change
- registry update that changes normalized labels or prompt contract text
- canonical URL change for `seo_link_targets`
- pattern reliability downgrade or expiry for `seo_serp_patterns`
- scope-signature change

## 6. Projection Producer And Consumer Ownership

| Projection surface | Producer | Consumers |
|---|---|---|
| Neo4j SEO nodes/edges | infrastructure projection adapter | IA builder, linking engine, analytics, rebuild detector |
| `seo_keyword_clusters` | retrieval projection adapter | opportunity builder, IA builder |
| `seo_page_blueprints` | retrieval projection adapter | draft assembler |
| `seo_serp_patterns` | retrieval projection adapter | serp_normalize, opportunity builder, IA builder |
| `seo_section_templates` | retrieval projection adapter | draft assembler |
| `seo_link_targets` | retrieval projection adapter | link_recommend, draft assembler |
| `seo_content_gaps` | retrieval projection adapter | opportunity builder, rebuild detector |

## 7. Acceptance Checks

This document is incomplete until all of the following are true:

- every required SEO node label and edge type is named explicitly
- every node and collection has a deterministic identity key
- cross-layer allowed links and forbidden behaviors are explicit
- projection freshness and rebuild rules are explicit
- Qdrant collection separation from truth-core retrieval is explicit
- producer and consumer ownership is explicit
- future automation can verify node/edge/collection declarations against implementation

---

## 8. Projection Deletion And Propagation Addendum

### 8.1 Deletion mode selection

| Projected class | Deactivation mode |
|---|---|
| `PageNode` | tombstone if ever published or referenced; hard delete only for never-activated candidates |
| `KeywordCluster` | tombstone |
| `PageBlueprint` | tombstone |
| `ContentGap` | tombstone |
| `SearchFeature` | tombstone |
| `SectionTemplate` | tombstone |
| `SERPPattern` | tombstone |
| `LinkRecommendation` | hard delete when expired and not applied; tombstone when historically applied |

### 8.2 Truth-change propagation edges

Projection must preserve and index the following dependency edges for rebuild propagation:

- `(:PageNode)-[:COVERS_TOPIC]->(:Topic)`
- `(:PageNode)-[:COVERS_CONCEPT]->(:Concept)`
- `(:PageNode)-[:COVERS_RULE]->(:RuleInstance)`
- `(:PageNode)-[:USES_OPERATIONAL_ENTITY]->(:OperationalEntity)`

Required propagation behavior:

- a change on any referenced truth-core node marks dependent SEO nodes stale,
- staleness must propagate to retrieval projections sourcing the affected page or blueprint,
- projection refresh must not reactivate a tombstoned node without a source-of-record revival.

### 8.3 Embedding contract clarification

Each Qdrant collection must be homogeneous by:

- `embedding_model_key`
- `embedding_dimensions`

Collection creation is blocked until both values are declared in runtime configuration and stored as collection metadata.

---

## 9. Core SEO / Knowledge Graph For Super-Site Generation

This document defines the primary working graph needed to generate the super-site.

The core SEO / knowledge graph must support:

- site structure generation
- page opportunity modeling
- internal linking
- content gap detection
- page brief generation
- draft assembly support
- cannibalization detection
- rebuild planning

### 9.1 Required working graph objects

Minimum graph-visible objects:

- `Topic`
- `Concept`
- `RuleInstance`
- `KeywordCluster`
- `PageNode`
- `PageBlueprint`
- `ContentGap`
- `SearchFeature`
- `SectionTemplate`
- `SERPPattern`
- `LinkRecommendation`
- `CannibalizationConflict`

### 9.2 Required graph traversals

The working graph must support deterministic traversal for:

- `KeywordCluster -> candidate PageNode -> PageBlueprint`
- `PageNode -> parent hub / child page / sibling page`
- `PageNode -> covered Topic / Concept / RuleInstance`
- `PageNode -> required links / recommended links`
- `PageNode -> active CannibalizationConflict`
- `SERPPattern -> supported cluster or gap`
- `ContentGap -> missing page or missing section candidate`

### 9.3 What must not be modeled as primary graph objects

Do not use the main SEO graph as the canonical home for:

- raw crawl HTML
- full draft bodies
- ad hoc runtime logs
- opaque analytics snapshots
- undifferentiated business copy
- protocol text or document ownership rules

## 10. Retrieval Layer For Super-Site Generation

The retrieval layer exists to fetch supporting inputs for generation and decision support.

It must support:

- draft assembly inputs
- blueprint lookup
- section-template lookup
- SERP pattern lookup
- link-target lookup
- content-gap lookup
- cluster lookup

### 10.1 Required retrieval collections for the minimum expert system

Minimum collections:

- `seo_keyword_clusters`
- `seo_page_blueprints`
- `seo_serp_patterns`
- `seo_section_templates`
- `seo_link_targets`
- `seo_content_gaps`

### 10.2 Retrieval responsibilities

Retrieval may:

- rank relevant supporting SEO artifacts
- retrieve blueprint and section candidates
- retrieve likely link targets
- retrieve SERP patterns for structure shaping
- retrieve content-gap evidence for planning

Retrieval may not:

- create canonical graph identity
- decide final page ownership
- override cannibalization rules
- create truth
- replace QA or publish gates

## 11. Minimum Viable Expert Set For Data Architecture

The minimum data architecture sufficient to generate a strong super-site consists of:

- the core SEO / knowledge graph objects in Section 9.1
- the retrieval collections in Section 10.1
- deterministic page taxonomy and dominant intent rules from IA
- blueprint contracts from draft assembly
- linking rules and scoring from IA
- draft traceability and QA gates from draft assembly
- publish governance from the CMS / HITL control-plane spec

Without this minimum set, Neo4j and Qdrant remain storage only and do not become a reliable SEO engine.

## 12. First-Class Neo4j Objects For The Expert SEO Engine

In this architecture, a `first-class` graph object is a node or relationship that has:

- a stable identity key,
- explicit projection ownership,
- deterministic upsert rules,
- direct participation in structure, linking, gap, or conflict reasoning.

### 12.1 Mandatory first-class node labels

The expert SEO graph must treat the following as first-class node labels:

- `Topic`
- `Concept`
- `RuleInstance`
- `KeywordCluster`
- `Intent`
- `SiteSection`
- `ContentHub`
- `PageNode`
- `PageBlueprint`
- `ContentGap`
- `SearchFeature`
- `SectionTemplate`
- `SERPPattern`
- `CannibalizationConflict`

### 12.2 Mandatory first-class relationship classes

The expert SEO graph must treat the following as first-class relationship classes or equivalent graph-native reasoning edges:

- `(:KeywordCluster)-[:TARGETS_INTENT]->(:Intent)`
- `(:KeywordCluster)-[:PROPOSES_PAGE]->(:PageNode)`
- `(:SiteSection)-[:CONTAINS_HUB]->(:ContentHub)`
- `(:ContentHub)-[:CONTAINS_PAGE]->(:PageNode)`
- `(:PageNode)-[:USES_BLUEPRINT]->(:PageBlueprint)`
- `(:PageBlueprint)-[:USES_SECTION_TEMPLATE]->(:SectionTemplate)`
- `(:PageNode)-[:COVERS_TOPIC]->(:Topic)`
- `(:PageNode)-[:COVERS_CONCEPT]->(:Concept)`
- `(:PageNode)-[:COVERS_RULE]->(:RuleInstance)`
- `(:PageNode)-[:SUPPORTED_BY_PATTERN]->(:SERPPattern)`
- `(:PageNode)-[:HAS_CONTENT_GAP]->(:ContentGap)`
- `(:PageNode)-[:ALIGNS_WITH_SEARCH_FEATURE]->(:SearchFeature)`
- `(:PageNode)-[:COMPETES_WITH]->(:PageNode)`
- `(:CannibalizationConflict)-[:INVOLVES_PAGE]->(:PageNode)`
- `(:PageNode)-[:RECOMMENDS_LINK_TO]->(:PageNode)`

### 12.3 Objects that are not first-class Neo4j nodes

The following must not be modeled as first-class nodes in the main SEO graph:

- `PageBrief`
- `Draft`
- `PublishedRevision`
- `RegistryEntry`
- `MetricSnapshot`
- `HitlTask`
- raw HTML or crawl snapshots
- full prompt text or long opaque draft bodies

These may remain relational only, or project only into a future ops-only meta-graph if that use-case is explicitly approved.

## 13. Mandatory Qdrant Collections For The Expert SEO Engine

These collections are mandatory for the expert retrieval layer.

### 13.1 `seo_keyword_clusters`

Purpose:

- retrieve cluster candidates during opportunity building and IA assignment.

Point identity:

- `keyword_cluster_key`

Required payload fields:

- `keyword_cluster_key`
- `scope_signature`
- `dominant_intent_key`
- `page_type_hint`
- `cluster_status`
- `source_version`

### 13.2 `seo_page_blueprints`

Purpose:

- retrieve blueprint candidates during page planning and draft assembly.

Point identity:

- `page_blueprint_key`

Required payload fields:

- `page_blueprint_key`
- `page_type_key`
- `dominant_intent_key`
- `scope_signature_mode`
- `status`
- `source_version`

### 13.3 `seo_section_templates`

Purpose:

- retrieve section templates during outline creation and section assembly.

Point identity:

- `section_template_key`

Required payload fields:

- `section_template_key`
- `page_type_key`
- `section_role_key`
- `allowed_intent_keys`
- `status`
- `source_version`

### 13.4 `seo_serp_patterns`

Purpose:

- retrieve reliable SERP patterns for opportunity shaping, structure design, and draft guidance.

Point identity:

- `serp_pattern_key`

Required payload fields:

- `serp_pattern_key`
- `pattern_type`
- `scope_signature`
- `reliability_level`
- `freshness_class`
- `source_version`

### 13.5 `seo_link_targets`

Purpose:

- retrieve candidate target pages during link recommendation and contextual-link insertion.

Point identity:

- `page_node_key`

Required payload fields:

- `page_node_key`
- `canonical_url`
- `scope_signature`
- `dominant_intent_key`
- `page_type_key`
- `lifecycle_state`
- `source_version`

### 13.6 `seo_content_gaps`

Purpose:

- retrieve missing-page and missing-section opportunities during planning and rebuild reprioritization.

Point identity:

- `content_gap_key`

Required payload fields:

- `content_gap_key`
- `gap_type`
- `scope_signature`
- `severity`
- `status`
- `opportunity_score`
- `source_version`

### 13.7 `seo_verified_fact_support`

Purpose:

- retrieve verified support fragments during draft assembly and claim validation.

Point identity:

- `support_fragment_key`

Required payload fields:

- `support_fragment_key`
- `truth_object_type`
- `truth_object_key`
- `scope_signature`
- `support_kind`
- `source_version`

### 13.8 `seo_draft_support_sections`

Purpose:

- retrieve reusable support sections and evidence-backed section inputs during draft assembly.

Point identity:

- `draft_support_section_key`

Required payload fields:

- `draft_support_section_key`
- `page_type_key`
- `section_role_key`
- `scope_signature`
- `traceability_class`
- `source_version`

### 13.9 Collections that are not mandatory

The following are not mandatory expert collections unless a later runtime use-case proves the need:

- `seo_page_briefs`
- `seo_drafts_fulltext`
- `seo_cannibalization_conflicts`
- `seo_hitl_tasks`
- `seo_metric_snapshots`
- `seo_registry_entries`

## 14. Projection Write Path And Store Population Rules

### 14.1 Canonical write sequence

Objects must enter the platform in this order:

1. canonical record is created or updated in `Postgres`
2. runtime step completes and emits a projection-eligible outcome
3. projection adapter reads canonical state plus required dependencies
4. `Neo4j` receives graph-shaped upserts only
5. `Qdrant` receives retrieval-shaped upserts only
6. `CMS` receives serving-shaped writes only after approval and publish gates

### 14.2 What writes directly to each store

- application steps and operators write canonical state to `Postgres`
- projection adapters write to `Neo4j`
- retrieval adapters write to `Qdrant`
- publish adapters write to `CMS`

Direct manual writes from SEO runtime steps into `Neo4j`, `Qdrant`, or `CMS` are forbidden unless the adapter contract explicitly owns that write path.

### 14.3 Neo4j population rule

An object is projected to `Neo4j` only when all are true:

- canonical record exists in `Postgres`
- graph identity key is declared
- at least one approved graph reasoning use-case exists
- relationship set can be built deterministically

### 14.4 Qdrant population rule

An object is projected to `Qdrant` only when all are true:

- canonical record exists in `Postgres`
- collection contract exists in this document
- embedding source text is declared
- payload schema is declared
- at least one approved retrieval-time lookup use-case exists

### 14.5 Deactivation rule

Removal or deactivation must start in `Postgres` and then propagate outward:

- source-of-record state changes first
- `Neo4j` projection tombstones or deletes per Section 8
- `Qdrant` point is replaced, tombstoned, or deleted per collection contract
- `CMS` serving state is changed only through publish-control contracts

## 15. Phase-Ordered Activation Of The Expert Graph And Retrieval Model

This section does not define an `MVP` architecture. It defines the activation order inside the already-approved expert target model.

All objects and collections in Sections 12 and 13 remain part of the target architecture. The ordering below exists only to control implementation dependency order.

### 15.1 Phase 1 mandatory first-class Neo4j nodes

The first implementation must activate these first-class nodes because without them the system cannot reliably generate site structure, linking, and drafts:

- `Topic`
- `Concept`
- `RuleInstance`
- `KeywordCluster`
- `Intent`
- `PageNode`
- `PageBlueprint`
- `SERPPattern`
- `SectionTemplate`
- `ContentGap`

Reason:

- these nodes are the minimum expert reasoning surface for page opportunity evaluation,
- structure and blueprint attachment depend on them,
- draft assembly and traceability depend on them,
- link recommendation and gap detection depend on them.

### 15.2 Phase 1 mandatory first-class Neo4j relationships

The first implementation must activate these graph relationships:

- `(:KeywordCluster)-[:TARGETS_INTENT]->(:Intent)`
- `(:KeywordCluster)-[:PROPOSES_PAGE]->(:PageNode)`
- `(:PageNode)-[:USES_BLUEPRINT]->(:PageBlueprint)`
- `(:PageBlueprint)-[:USES_SECTION_TEMPLATE]->(:SectionTemplate)`
- `(:PageNode)-[:COVERS_TOPIC]->(:Topic)`
- `(:PageNode)-[:COVERS_CONCEPT]->(:Concept)`
- `(:PageNode)-[:COVERS_RULE]->(:RuleInstance)`
- `(:PageNode)-[:SUPPORTED_BY_PATTERN]->(:SERPPattern)`
- `(:PageNode)-[:HAS_CONTENT_GAP]->(:ContentGap)`
- `(:PageNode)-[:RECOMMENDS_LINK_TO]->(:PageNode)`

### 15.3 Phase 1 mandatory Qdrant collections

The first implementation must activate these collections because without them retrieval support for planning and draft generation is incomplete:

- `seo_keyword_clusters`
- `seo_page_blueprints`
- `seo_section_templates`
- `seo_serp_patterns`
- `seo_link_targets`
- `seo_verified_fact_support`
- `seo_draft_support_sections`

Reason:

- planning needs cluster, pattern, and blueprint lookup,
- draft generation needs template and verified support lookup,
- linking needs explicit target-page retrieval.

### 15.4 Phase 2 expert-required first-class Neo4j nodes

These nodes remain mandatory in the target expert model, but their activation may follow Phase 1 once the base site-generation loop is working:

- `SiteSection`
- `ContentHub`
- `SearchFeature`
- `CannibalizationConflict`

Reason:

- they strengthen navigation governance, search-surface alignment, and conflict control,
- but they depend on stable Phase 1 page identities and structure before they can be reasoned over correctly.

### 15.5 Phase 2 expert-required Neo4j relationships

These relationships may be activated after Phase 1, but before the system is considered scale-ready:

- `(:SiteSection)-[:CONTAINS_HUB]->(:ContentHub)`
- `(:ContentHub)-[:CONTAINS_PAGE]->(:PageNode)`
- `(:PageNode)-[:ALIGNS_WITH_SEARCH_FEATURE]->(:SearchFeature)`
- `(:PageNode)-[:COMPETES_WITH]->(:PageNode)`
- `(:CannibalizationConflict)-[:INVOLVES_PAGE]->(:PageNode)`

### 15.6 Phase 2 expert-required Qdrant collections

These collections remain part of the expert target state but may follow once the base planning loop and rebuild loop are stable:

- `seo_content_gaps`

Optional later collections remain non-mandatory unless a runtime use-case is approved:

- `seo_page_briefs`
- `seo_drafts_fulltext`
- `seo_cannibalization_conflicts`
- `seo_hitl_tasks`
- `seo_metric_snapshots`
- `seo_registry_entries`

### 15.7 Activation gate rule

A Phase 2 graph or retrieval surface must not be activated until all are true:

- its source-of-record object already exists and is stable in `Postgres`
- upstream Phase 1 identities are stable
- projection invalidation behavior is defined
- at least one runtime consumer is implemented or explicitly scheduled

### 15.8 Final sequencing rule

The platform does not change architecture between phases.

Instead:

- Phase 1 activates the minimum complete expert reasoning core
- Phase 2 activates the remaining expert governance and scale-strengthening surfaces
- optional surfaces remain disabled until a real runtime use-case exists

## 16. Alignment Addendum For Minimum Expert Sections

This addendum resolves any ambiguity between the earlier `minimum expert system` sections and the later first-class / mandatory projection sections.

### 16.1 Interpreting Section 9.1

Section 9.1 must be read as the minimum expert reasoning surface, not as an exhaustive final list of every target expert node class.

For implementation and activation authority:

- the authoritative full target node set is Section 12.1,
- the authoritative activation order is Section 15,
- `LinkRecommendation` is a mandatory canonical output and graph-native reasoning edge, not a required first-class node label.

### 16.2 Interpreting Section 10.1

Section 10.1 must be read as the minimum expert retrieval surface.

For implementation and activation authority:

- the authoritative full mandatory collection set is Section 13,
- the authoritative activation order is Section 15,
- `seo_verified_fact_support` and `seo_draft_support_sections` are part of the expert minimum and may not be omitted from the final expert target state.

### 16.3 Consistency rule

If an earlier summary section and a later explicit first-class / mandatory section appear to differ:

- the later explicit section wins,
- the expert target state remains unchanged,
- activation order is controlled only by Section 15.
