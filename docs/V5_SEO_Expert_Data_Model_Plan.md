# V5 SEO Expert Data Model Plan

**Status:** working expert planning document  
**Purpose:** define the direct-to-expert data model for Neo4j and Qdrant required to build a knowledge-driven SEO super-site without an MVP detour.

---

## 1. Decision

This project does not use a simplified MVP data model.

The target is a direct-to-expert data architecture where:

- `Postgres` remains the canonical source of record,
- `Neo4j` is the decision graph,
- `Qdrant` is the retrieval support layer,
- `CMS` is the approved serving target.

---

## 2. Scope

This plan covers:

- core SEO / knowledge graph in `Neo4j`
- retrieval layer in `Qdrant`
- minimum complete expert set for the super-site engine
- activation order inside the expert target architecture

This plan does not redefine:

- V2 truth semantics
- canonical SQL schema ownership
- runtime step ownership
- CMS / HITL control-plane ownership

---

## 3. Design Principle

`Neo4j` and `Qdrant` serve different jobs.

### 3.1 Neo4j is the decision graph

`Neo4j` owns graph reasoning needed for:

- page opportunity structure
- page-to-page relationships
- hub / leaf decisions
- cannibalization reasoning
- content gap reasoning
- blueprint attachment semantics
- truth-to-page dependency traversal

### 3.2 Qdrant is the retrieval support layer

`Qdrant` owns ranked retrieval needed for:

- cluster lookup
- blueprint lookup
- section-template lookup
- SERP pattern lookup
- link-target lookup
- verified support lookup
- evidence-backed draft support lookup
- content-gap lookup

`Qdrant` may rank and retrieve. `Neo4j` may decide structural semantics. Neither may create truth.

---

## 4. Neo4j Must-Have Expert Model

## 4.1 Primary working node classes

Required target node classes:

- `Topic`
- `Concept`
- `RuleInstance`
- `OperationalEntity`
- `KeywordCluster`
- `Intent`
- `SiteSection`
- `ContentHub`
- `PageNode`
- `PageBlueprint`
- `SectionTemplate`
- `SERPPattern`
- `SearchFeature`
- `ContentGap`
- `CannibalizationConflict`

`LinkRecommendation` is mandatory in the expert system, but as a canonical output plus graph-native reasoning edge rather than a required first-class node label.

## 4.2 Primary working edge classes

Required graph-native reasoning edges:

- `KeywordCluster -> TARGETS_INTENT -> Intent`
- `KeywordCluster -> PROPOSES_PAGE -> PageNode`
- `SiteSection -> CONTAINS_HUB -> ContentHub`
- `ContentHub -> CONTAINS_PAGE -> PageNode`
- `PageNode -> USES_BLUEPRINT -> PageBlueprint`
- `PageBlueprint -> USES_SECTION_TEMPLATE -> SectionTemplate`
- `PageNode -> COVERS_TOPIC -> Topic`
- `PageNode -> COVERS_CONCEPT -> Concept`
- `PageNode -> COVERS_RULE -> RuleInstance`
- `PageNode -> SUPPORTED_BY_PATTERN -> SERPPattern`
- `PageNode -> ALIGNS_WITH_SEARCH_FEATURE -> SearchFeature`
- `PageNode -> HAS_CONTENT_GAP -> ContentGap`
- `PageNode -> RECOMMENDS_LINK_TO -> PageNode`
- `PageNode -> COMPETES_WITH -> PageNode`
- `CannibalizationConflict -> INVOLVES_PAGE -> PageNode`

## 4.3 Primary working objects by function

For site structure:

- `KeywordCluster`
- `Intent`
- `SiteSection`
- `ContentHub`
- `PageNode`
- `PageBlueprint`

For linking:

- `PageNode`
- `ContentHub`
- `Topic`
- `Concept`
- `RuleInstance`
- `LinkRecommendation` as graph-native edge output

For draft assembly support:

- `PageNode`
- `PageBlueprint`
- `SectionTemplate`
- `SERPPattern`
- truth dependency nodes

For conflict and coverage reasoning:

- `ContentGap`
- `CannibalizationConflict`
- `SearchFeature`

## 4.4 What must not be put into the main graph

Do not use the main SEO graph as canonical home for:

- raw crawl HTML
- full draft bodies
- page briefs
- HITL task state
- metrics snapshots
- registry rows
- opaque prompt blobs
- protocol prose

## 4.5 Required graph traversals

The expert graph must support:

- `KeywordCluster -> candidate PageNode -> PageBlueprint`
- `PageNode -> owning SiteSection / ContentHub`
- `PageNode -> covered Topic / Concept / RuleInstance`
- `PageNode -> required links / recommended links`
- `PageNode -> active CannibalizationConflict`
- `SERPPattern -> supported cluster / page / gap`
- `ContentGap -> missing page or missing section candidate`
- `PageNode -> truth dependency set -> rebuild impact`

---

## 5. Qdrant Must-Have Expert Model

## 5.1 Required collections

### SEO-native collections

- `seo_keyword_clusters`
- `seo_page_blueprints`
- `seo_section_templates`
- `seo_serp_patterns`
- `seo_link_targets`
- `seo_verified_fact_support`
- `seo_draft_support_sections`
- `seo_content_gaps`

### Required truth-support retrieval dependency from V2

The SEO engine must also consume V2 retrieval surfaces for:

- verified rules retrieval
- verified topics retrieval
- verified operational support retrieval

The SEO engine is incomplete without those support surfaces.

## 5.2 Retrieval collections and use-cases

### `seo_keyword_clusters`

Used for:

- page opportunity grouping
- cluster lookup for structure generation
- candidate page-type matching

### `seo_page_blueprints`

Used for:

- blueprint lookup during page planning and draft assembly
- page-type-to-structure retrieval
- required-section retrieval

### `seo_section_templates`

Used for:

- section support lookup
- outline and section assembly guidance
- allowed traceability-label lookup

### `seo_serp_patterns`

Used for:

- SERP pattern lookup
- structure-shaping lookup
- title and section-pattern support for page planning

### `seo_link_targets`

Used for:

- link target lookup
- contextual anchor support
- support-page retrieval for draft assembly

### `seo_verified_fact_support`

Used for:

- factual support retrieval during draft assembly
- claim validation support resolution
- evidence-backed section support lookup

### `seo_draft_support_sections`

Used for:

- reusable evidence-backed section support
- section-role matching under valid scope
- draft assembly acceleration without losing traceability

### `seo_content_gaps`

Used for:

- missing-page detection support
- missing-section support
- opportunity reprioritization support

## 5.3 Retrieval contracts

Draft assembly must retrieve:

- page blueprint candidate
- section-template candidates
- relevant verified support
- relevant SERP structural patterns
- relevant draft support sections
- required link-target candidates

Link recommendation must retrieve:

- candidate target pages
- anchor-support surfaces
- scope-compatible related pages

Opportunity planning must retrieve:

- cluster candidates
- SERP patterns
- content-gap candidates

## 5.4 What retrieval must never do

Retrieval must never:

- create canonical identity
- assign final page ownership
- override cannibalization decisions
- create truth
- replace publish or QA gates

---

## 6. Minimum Complete Expert Set

Because this project skips MVP, the minimum acceptable set is already an expert baseline.

## 6.1 Minimum graph objects

Required:

- `Topic`
- `Concept`
- `RuleInstance`
- `OperationalEntity`
- `KeywordCluster`
- `Intent`
- `SiteSection`
- `ContentHub`
- `PageNode`
- `PageBlueprint`
- `SectionTemplate`
- `SERPPattern`
- `SearchFeature`
- `ContentGap`
- `CannibalizationConflict`

`LinkRecommendation` remains part of the minimum expert system as a canonical runtime output and graph-native reasoning edge, not as a mandatory first-class node label.

## 6.2 Minimum retrieval objects

Required:

- `seo_keyword_clusters`
- `seo_page_blueprints`
- `seo_section_templates`
- `seo_serp_patterns`
- `seo_link_targets`
- `seo_verified_fact_support`
- `seo_draft_support_sections`
- `seo_content_gaps`
- V2 verified support retrieval surfaces

## 6.3 Minimum registries

Required:

- `page_type`
- `intent_type`
- `anchor_strategy`
- `section_template`
- `cta_pattern`
- `search_feature_type`
- `scope_class`

## 6.4 Minimum runtime outputs

Required runtime outputs from the SEO engine:

- accepted `KeywordCluster`
- accepted `PageNode`
- `PageBlueprint` assignment
- `LinkRecommendation`
- `ContentGap`
- `PageBrief`
- `Draft`
- `CannibalizationConflict`
- `RebuildDecision`

## 6.5 Minimum QA gates

Required:

- scope validation
- canonical URL validation
- blueprint conformance
- traceability completeness
- no unsupported factual fragments
- no forbidden SERP-as-fact usage
- required links present
- no unresolved cannibalization blocker

## 6.6 Minimum publish gates

Required:

- page is in an allowed publish-ready state
- required review tasks are resolved
- traceability manifest is complete
- evidence freshness is acceptable
- CMS blockers are clear

## 6.7 Minimum rebuild triggers

Required:

- truth change
- ontology change
- SERP freshness expiry
- blueprint change
- section-template change
- link-graph invalidation

## 6.8 Minimum human review cases

Required:

- unresolved cannibalization
- unsupported factual fragment
- canonical URL conflict
- unsafe SERP promotion request
- registry change request
- rebuild suppression request

---

## 7. Step-By-Step Expert Implementation Order

1. Lock companion-owner architecture and build-spec contracts.
2. Implement canonical SQL schema and runtime payload contracts.
3. Activate Phase 1 graph objects and retrieval collections.
4. Implement page-planning loop from SERP patterns to accepted `PageNode`.
5. Implement linking and draft assembly with verified support retrieval.
6. Implement draft QA, publish gating, and CMS handoff.
7. Activate Phase 2 graph objects and collections for scale-strengthening behavior.
8. Add rebuild, freshness, review, and optimization loops.

---

## 8. Acceptance Standard

This plan is satisfied only when:

- `Neo4j` contains every required working node class and edge class needed for site generation
- `Qdrant` contains every required retrieval collection needed for support lookup and draft assembly
- the runtime produces the minimum canonical outputs
- the publish path enforces the minimum QA and governance gates
- no layer drifts into truth creation outside V2 boundaries

---

## 9. Final Recommendation

For this project, do not ask whether `Neo4j` and `Qdrant` are merely enough to store data.

The correct question is whether they are modeled richly enough to:

- design the site structure
- decide page ownership
- retrieve support deterministically
- generate drafts with traceability
- govern linking and cannibalization
- rebuild safely when truth or SERP conditions change

That is the standard this expert data model is intended to meet.
