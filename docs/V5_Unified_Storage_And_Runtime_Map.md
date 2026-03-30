# V5 Unified Storage And Runtime Map

**Status:** supporting architecture map  
**Purpose:** provide one practical view of what lives in Postgres, Neo4j, Qdrant, and CMS, and how data moves across them inside the unified Data Engine + SEO Engine platform.

## 1. Role Of This Document

This document is not a new canonical owner of rules.

It summarizes already-owned decisions from:

- `V5_SEO_Live_Schema_Design_Spec.md`
- `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`
- `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`
- `V5_SEO_Runtime_Step_Contracts_Spec.md`
- `V5_SEO_Expert_Data_Model_Plan.md`
- `V5_Unified_Data_And_SEO_Engine_Architecture_Note.md`

## 2. Store Responsibilities

## 2.1 Postgres

`Postgres` is the source-of-record layer.

It stores:

- raw and normalized SERP batches
- competitor page observations
- SEO registries
- keyword clusters
- page nodes
- page blueprints
- page briefs
- page drafts
- link recommendations
- cannibalization conflicts
- content gaps
- lifecycle events
- metric snapshots
- rebuild backlog and quality failures
- runtime step execution records and HITL decisions

Use Postgres for:

- canonical persistence
- state transitions
- auditability
- versioning
- deterministic write semantics

Do not use Postgres alone as the reasoning surface for graph traversal or semantic retrieval ranking.

## 2.2 Neo4j

`Neo4j` is the working decision graph.

It stores projected graph objects needed for:

- site structure generation
- hub / leaf decisions
- parent / child / sibling relationships
- truth coverage relationships
- internal linking decisions
- cannibalization detection
- content gap visibility
- rebuild impact traversal

It contains projected nodes such as:

- `KeywordCluster`
- `Intent`
- `PageNode`
- `PageBlueprint`
- `SectionTemplate`
- `SERPPattern`
- `SearchFeature`
- `ContentGap`
- `LinkRecommendation`
- `CannibalizationConflict`
- read-only truth reference nodes like `Topic`, `Concept`, `RuleInstance`, `OperationalEntity`

Use Neo4j for:

- graph traversal
- adjacency reasoning
- coverage reasoning
- structure and linking decisions
- impact analysis over explicit dependency edges

Do not use Neo4j as the source-of-record for mutable SEO state.

## 2.3 Qdrant

`Qdrant` is the retrieval support layer.

It stores projected retrieval collections needed for:

- blueprint lookup
- section template lookup
- link target lookup
- SERP pattern lookup
- content gap support lookup
- cluster retrieval
- draft assembly support retrieval

It contains collections such as:

- `seo_keyword_clusters`
- `seo_page_blueprints`
- `seo_section_templates`
- `seo_serp_patterns`
- `seo_link_targets`
- `seo_content_gaps`
- plus required V2 verified support retrieval surfaces

Use Qdrant for:

- ranked retrieval
- semantic support lookup
- retrieval-time narrowing of relevant support artifacts
- context assembly inputs

Do not use Qdrant for:

- canonical identity
- final page ownership decisions
- truth creation
- replacing graph decisions

## 2.4 CMS

`CMS` is the publish target and serving-state endpoint.

It stores or receives:

- approved page revisions
- publishable metadata
- canonical URL assignments
- publish states
- rollback references
- serving payloads and schema markup

Use CMS for:

- approved serving content
- revision history at the publish layer
- publication and deprecation actions
- rollback execution at the serving layer

Do not use CMS as the source-of-record for SEO planning or truth.

## 3. Runtime Flows Across Stores

## 3.1 Data Engine flow

```text
SERP / competitors / expert inputs
    -> Postgres ingestion and normalization
    -> verification and canonicalization
    -> source-of-record SEO and support state in Postgres
    -> projection into Neo4j and Qdrant
```

## 3.2 SEO Engine flow

```text
Postgres source-of-record + Neo4j decision graph + Qdrant retrieval support
    -> page opportunities
    -> site structure
    -> linking decisions
    -> page briefs
    -> drafts
    -> QA verdicts
    -> publish-ready revisions
```

## 3.3 Publish flow

```text
approved draft in Postgres
    -> CMS publish workflow
    -> CMS active revision
    -> super-site serving state
```

## 3.4 Update and rebuild flow

```text
truth change / SERP change / freshness expiry / IA change
    -> Postgres invalidation or backlog state
    -> Neo4j impact traversal
    -> Qdrant support refresh if needed
    -> draft rebuild or review path
    -> CMS update, hold, or rollback
```

## 4. Practical Ownership By Store

| Store | Primary role | Owned by | Must not own |
|---|---|---|---|
| `Postgres` | source of record and runtime state | Data Engine + shared platform state | semantic graph traversal as primary reasoning surface |
| `Neo4j` | working graph for SEO decisions | shared graph projection used mostly by SEO Engine | canonical mutable record ownership |
| `Qdrant` | retrieval support and ranking | shared retrieval projection used mostly by SEO Engine | truth, identity, or final page semantics |
| `CMS` | publication and serving endpoint | SEO Engine publish path | planning truth or source-of-record SEO state |

## 5. Practical Rule Set

1. Write canonical state first to `Postgres`.
2. Project decision graph state to `Neo4j`.
3. Project retrieval support to `Qdrant`.
4. Publish only approved revisions to `CMS`.
5. Never let `CMS`, `Neo4j`, or `Qdrant` silently redefine source-of-record meaning.
6. Never let `SEO Engine` write back into truth as authority.

## 6. Final Mental Model

- `Postgres` = canonical state
- `Neo4j` = decision graph
- `Qdrant` = retrieval support
- `CMS` = approved serving output

In one line:

```text
Postgres remembers, Neo4j reasons, Qdrant retrieves, CMS serves.
```

## 7. Object Placement Matrix

This section removes ambiguity for the most important SEO engine objects.

| Object | Source of record | Neo4j | Qdrant | CMS | Notes |
|---|---|---|---|---|---|
| `KeywordCluster` | Postgres | yes | yes | no | canonical planning object; projected for graph reasoning and retrieval |
| `PageNode` | Postgres | yes | yes via `seo_link_targets` | no | canonical SEO page-planning object |
| `PageBlueprint` | Postgres | yes | yes | no | source blueprint contract lives in Postgres |
| `SectionTemplate` | Postgres | yes | yes | no | retrieval helps assembly, graph helps attachment semantics |
| `SERPPattern` | Postgres | yes | yes | no | derived signal, never truth |
| `SearchFeature` | Postgres | yes | optional if retrieval use-case exists | no | graph-visible search-surface signal |
| `ContentGap` | Postgres | yes | yes | no | planning and reprioritization object |
| `LinkRecommendation` | Postgres | yes | optional | no | source-of-record for required/optional link decisions |
| `CannibalizationConflict` | Postgres | yes | no | no | conflict object for SEO decisioning, not retrieval |
| `PageBrief` | Postgres | no | no | no | planning artifact; should stay relational |
| `Draft` | Postgres | no | no | yes only after approved publish handoff | draft body is not a primary graph or retrieval object |
| `PublishedRevision` | CMS-facing publish state with relational linkage | no | no | yes | serving artifact only |
| `MetricSnapshot` | Postgres | no | no | no | monitoring data, not graph-native |
| `HitlTask` | Postgres | optional only in operational meta-graph, not main SEO graph | no | no | do not pollute the main decision graph |

### 7.1 Placement rules for the disputed objects

#### `PageBrief`

- canonical home: `Postgres`
- not a graph object
- not a retrieval collection object
- purpose: deterministic planning contract before draft assembly

#### `LinkRecommendation`

- canonical home: `Postgres`
- projected to `Neo4j` because linking is graph-native reasoning
- optional in `Qdrant`; only if retrieval-time link-target ranking needs it
- never stored in CMS as source-of-record

#### `ContentGap`

- canonical home: `Postgres`
- projected to `Neo4j` because gap visibility affects structure and missing-page reasoning
- projected to `Qdrant` because gap lookup can support planning and reprioritization
- never a CMS object

#### `SERPPattern`

- canonical home: `Postgres`
- projected to `Neo4j` for structural alignment reasoning
- projected to `Qdrant` for pattern lookup during planning and draft shaping
- never promoted to truth or CMS page-state

#### `Draft`

- canonical home: `Postgres`
- not a primary Neo4j node
- not a primary Qdrant collection object
- approved serving revision goes to `CMS`, but that does not move the draft's source-of-record out of `Postgres`

#### `CannibalizationConflict`

- canonical home: `Postgres`
- projected to `Neo4j` because it participates in page-competition reasoning
- not a retrieval object in normal architecture
- not a CMS object

### 7.2 Practical decision rules

1. If the object drives state transitions, audit, versioning, or review, its source-of-record is `Postgres`.
2. If the object must participate in adjacency or dependency reasoning, project it to `Neo4j`.
3. If the object must be semantically retrieved or ranked, project it to `Qdrant`.
4. If the object represents approved serving state, send it to `CMS`.
5. Do not project an object into `Neo4j` or `Qdrant` unless there is a concrete reasoning or retrieval use-case.

## 8. Recommended Strictness Profile For Key SEO Objects

This section defines how strict each important object should be in storage and projection design.

## 8.1 `PageBrief`

Recommended profile: **strict relational object**

Reason:

- it is a planning contract,
- it drives deterministic draft assembly,
- it should be versioned, auditable, and diffable,
- it should not be retrieved semantically as a fuzzy object.

Guidance:

- keep required fields explicit in `Postgres`
- avoid graph projection unless a later ops-layer needs lineage only
- do not store as a primary retrieval collection

## 8.2 `SearchFeature`

Recommended profile: **strict core fields + flexible payload**

Reason:

- feature identity and lifecycle must be stable,
- captured feature detail may vary by SERP surface type.

Guidance:

- keep identity, scope, feature type, freshness, and status strict in `Postgres`
- allow structured payload extension for feature-specific detail
- project to `Neo4j` when it affects page planning or search-surface alignment
- project to `Qdrant` only if retrieval-time feature lookup is a real use-case

## 8.3 `LinkRecommendation`

Recommended profile: **strict relational object + graph projection**

Reason:

- link recommendations are decision outputs, not vague semantic artifacts,
- they must support scoring, review, expiration, and application tracking,
- they participate in page-to-page reasoning.

Guidance:

- keep canonical recommendation state in `Postgres`
- project to `Neo4j` as first-class linking edges or supporting nodes
- do not rely on Qdrant unless there is a clear retrieval-time ranking need
- applied vs expired vs rejected states must stay relational and auditable

## 8.4 `SectionTemplate`

Recommended profile: **strict contract object + retrieval support object**

Reason:

- section templates are assembly contracts,
- they must be deterministic enough for runtime and QA,
- but they also need retrieval support for matching the right template to the right page context.

Guidance:

- keep canonical template definition in `Postgres`
- project to `Neo4j` for attachment semantics to page types, intents, and blueprints
- project to `Qdrant` for template lookup during draft assembly
- avoid storing large free-form prompt blobs as opaque primary state without fielded contract metadata

## 8.5 `Draft`

Recommended profile: **strict relational object + CMS serving handoff**

Reason:

- draft state, QA, traceability, and revision control must be exact,
- full draft bodies do not need to be graph-native,
- retrieval over full drafts usually creates noise and stale coupling.

Guidance:

- keep source-of-record in `Postgres`
- keep QA state, traceability summary, revision, and publish linkage explicit
- send only approved serving revisions into `CMS`
- do not make full drafts a standard `Qdrant` collection unless a later explicit retrieval use-case is proven

## 8.6 Summary Rule

For this system, the default should be:

- planning and decision objects -> strict in `Postgres`
- adjacency and dependency reasoning -> projected to `Neo4j`
- ranked support lookup -> projected to `Qdrant`
- publishable serving state -> handed off to `CMS`

If an object has no concrete graph or retrieval use-case, keep it relational only.

## 9. Extended Strictness Profile For Core SEO Objects

## 9.1 `PageNode`

Recommended profile: **strict canonical planning object + graph projection + limited retrieval projection**

Reason:

- `PageNode` is the core SEO page-planning identity,
- it drives structure, linking, cannibalization, draft ownership, and publish lineage,
- its identity and lifecycle must be exact.

Guidance:

- keep canonical identity, scope, dominant intent, URL, lifecycle state, and blueprint attachment strict in `Postgres`
- project to `Neo4j` as a first-class working node
- project to `Qdrant` only through targeted retrieval surfaces such as `seo_link_targets`
- do not let retrieval redefine page identity or page ownership

## 9.2 `PageBlueprint`

Recommended profile: **strict contract object + graph projection + retrieval support object**

Reason:

- blueprints are assembly contracts,
- they bind page type, dominant intent, sections, metadata obligations, and review gates,
- they must support both deterministic attachment and retrieval-time lookup.

Guidance:

- keep canonical blueprint definition strict in `Postgres`
- project to `Neo4j` for blueprint-to-page and blueprint-to-template reasoning
- project to `Qdrant` for blueprint lookup during page planning and draft assembly
- keep versioning explicit and auditable

## 9.3 `KeywordCluster`

Recommended profile: **strict planning object + graph projection + retrieval support object**

Reason:

- clusters are the bridge between SERP analysis and site-generation decisions,
- they drive opportunities, structure, and page acceptance,
- they also benefit from retrieval-time similarity and lookup.

Guidance:

- keep canonical cluster membership, dominant intent, scope, and status strict in `Postgres`
- project to `Neo4j` as a first-class planning node
- project to `Qdrant` for cluster retrieval and candidate matching
- avoid fuzzy-only cluster ownership without canonical cluster keys

## 9.4 `SERPPattern`

Recommended profile: **strict derived signal object + graph projection + retrieval support object**

Reason:

- SERP patterns shape structure and prioritization,
- they must remain derived and non-truth-bearing,
- they are useful in both graph reasoning and retrieval-time structural lookup.

Guidance:

- keep pattern identity, reliability, freshness, and observation basis strict in `Postgres`
- project to `Neo4j` for structure and gap alignment reasoning
- project to `Qdrant` for pattern lookup during page planning and draft shaping
- never store them as CMS-serving or truth-bearing objects

## 9.5 `ContentGap`

Recommended profile: **strict planning object + graph projection + retrieval support object**

Reason:

- gaps influence both missing-page decisions and missing-section decisions,
- they must remain explicitly versioned and reviewable,
- they are useful in both structural reasoning and planning lookup.

Guidance:

- keep canonical gap identity, gap type, severity, status, and support basis strict in `Postgres`
- project to `Neo4j` for structure and coverage reasoning
- project to `Qdrant` for planning-time gap retrieval and reprioritization support
- distinguish missing-page gaps from missing-section gaps explicitly

## 9.6 `CannibalizationConflict`

Recommended profile: **strict conflict object + graph projection only**

Reason:

- cannibalization is a blocking SEO decision object,
- it needs auditable lifecycle, resolution, and severity,
- it is graph-relevant but not a semantic retrieval target in normal operation.

Guidance:

- keep source-of-record strict in `Postgres`
- project to `Neo4j` because it participates in page competition reasoning
- do not project to `Qdrant` in normal architecture
- keep detection version and resolution outcome explicit

## 9.7 `PublishedRevision`

Recommended profile: **strict serving object + CMS endpoint object**

Reason:

- published revision is the approved serving result,
- it must remain immutable, auditable, and rollback-safe,
- it is not a planning or graph-native object.

Guidance:

- keep relational linkage to page and draft lineage explicit
- store serving-active revision in `CMS`
- keep revision identity, rollback linkage, and publish history strict
- do not project published revisions into the main SEO graph or retrieval layer by default

## 9.8 Extended summary rule

For the core expert SEO system:

- identity-bearing planning objects stay strict in `Postgres`
- reasoning-bearing planning objects project to `Neo4j`
- lookup-bearing support objects project to `Qdrant`
- immutable serving outputs hand off to `CMS`
- no object gets a graph or retrieval projection without a concrete decision or lookup use-case

## 10. Remaining Strictness Decisions For The Implementation-Ready Object Catalog

## 10.1 `Intent`

Recommended profile: **strict registry-backed planning object + graph projection**

Reason:

- `Intent` is a canonical planning dimension,
- it must be stable enough for deterministic page assignment and cannibalization checks,
- it participates in graph reasoning but should not be treated as a fuzzy retrieval object by default.

Guidance:

- keep canonical intent identity, registry binding, and status strict in `Postgres`
- project to `Neo4j` because intent participates in page, cluster, and blueprint reasoning
- do not make `Intent` a default `Qdrant` collection unless there is a proven retrieval-time classification use-case
- enforce registry ownership so page planning cannot invent ad-hoc intents

## 10.2 `SiteSection`

Recommended profile: **strict structural object + graph projection**

Reason:

- `SiteSection` expresses the planned structural layout of the super-site,
- it must support deterministic hierarchy, navigation, and URL governance,
- it is graph-relevant but not normally a semantic retrieval object.

Guidance:

- keep source-of-record in `Postgres` with exact section identity, parent, ordering, and lifecycle
- project to `Neo4j` for sitemap traversal, hub-leaf reasoning, and navigation dependencies
- do not project to `Qdrant` unless section retrieval becomes a real runtime need
- keep section-level URL and canonical responsibilities explicit

## 10.3 `ContentHub`

Recommended profile: **strict structural planning object + graph projection + limited retrieval support**

Reason:

- `ContentHub` is a first-class organizing object for the expert site architecture,
- it drives aggregation, child-page strategy, and link distribution,
- it may need retrieval support for hub selection during page planning.

Guidance:

- keep canonical hub identity, scope, owning section, and lifecycle strict in `Postgres`
- project to `Neo4j` as a primary structural node for hub-spoke reasoning
- project to `Qdrant` only if hub lookup is used during planning or draft support selection
- do not let retrieval redefine hub ownership or hierarchy

## 10.4 `AudienceScope`

Recommended profile: **strict scope dimension object + limited graph projection**

Reason:

- audience targeting affects page eligibility, linking, and draft obligations,
- scope dimensions must remain normalized and deterministic,
- audience scope is normally a classification dimension, not a retrieval target.

Guidance:

- keep canonical source-of-record in `Postgres`
- project to `Neo4j` only when audience applicability participates in page competition or support reasoning
- do not create a default `Qdrant` collection for `AudienceScope`
- enforce normalization and registry-backed allowed values

## 10.5 `RegistryEntry`

Recommended profile: **strict governance object only**

Reason:

- registry entries exist to govern the system,
- they are canonical control objects, not retrieval or publishing artifacts,
- they should stay auditable and tightly managed.

Guidance:

- keep source-of-record in `Postgres` only
- do not project registry entries into the main SEO graph by default
- do not project registry entries to `Qdrant`
- expose them through governance tooling, not content-generation flows

## 10.6 `MetricSnapshot`

Recommended profile: **strict monitoring object only**

Reason:

- metrics are operational measurements,
- they support monitoring, alerting, and optimization,
- they are not part of the primary SEO decision graph for site generation.

Guidance:

- keep source-of-record in `Postgres`
- do not project to `Neo4j` unless a future ops graph explicitly needs aggregate lineage only
- do not project to `Qdrant`
- keep retention and rollup rules explicit in monitoring specs

## 10.7 `HitlTask`

Recommended profile: **strict operational control-plane object only**

Reason:

- HITL tasks are workflow control objects,
- they must support exact queue state, escalation, resolution, and audit,
- they should not pollute the main SEO planning graph.

Guidance:

- keep source-of-record in `Postgres`
- do not project to the primary SEO graph by default
- do not project to `Qdrant`
- if an ops-only dependency graph is added later, project only minimal lineage references there

## 10.8 Final object-catalog rule

For implementation-ready placement decisions:

- planning identity objects stay strict first and project only for concrete planning use-cases
- structural navigation objects usually belong in `Postgres` plus `Neo4j`, not `Qdrant`
- governance, monitoring, and HITL control objects stay relational unless an explicit ops-only projection is later approved
- retrieval projection is justified only by a live lookup need, never by architectural symmetry

## 11. How Objects Are Added To The Stores

Objects are not written into `Neo4j` or `Qdrant` by hand as primary state.

The required population sequence is:

1. canonical object is created or updated in `Postgres`
2. runtime step marks the object projection-eligible
3. projection adapter reads canonical state and dependencies
4. graph-shaped projection is upserted into `Neo4j` if the object has an approved graph use-case
5. retrieval-shaped projection is upserted into `Qdrant` if the object has an approved retrieval use-case
6. approved serving revision is written to `CMS` only through publish-control flow

### 11.1 Practical examples

- `PageNode` is created in `Postgres`, then projected to `Neo4j`, and only its link-target surface may also go to `Qdrant`
- `PageBlueprint` is created in `Postgres`, then projected to `Neo4j` and `Qdrant`
- `Draft` is created in `Postgres`, reviewed there, and only approved serving output is handed to `CMS`
- `CannibalizationConflict` is created in `Postgres`, then projected to `Neo4j`, but not to `Qdrant`
- `RegistryEntry` stays in `Postgres` only

### 11.2 Store population rule

For this platform:

- `Postgres` owns canonical state
- `Neo4j` receives only graph-shaped reasoning objects
- `Qdrant` receives only retrieval-shaped support objects
- `CMS` receives only approved serving artifacts

No object should bypass `Postgres` and become canonical in `Neo4j`, `Qdrant`, or `CMS`.
