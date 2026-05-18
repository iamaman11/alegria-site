# V5 Ultimate Extraction Protocol V2

**Status:** Working canonical rewrite skeleton for normative consolidation
**Version:** 2.0-draft
**Source base:** `V5_Ultimate_Extraction_Protocol.md`
**Purpose of this file:** become the single normative protocol document after reconciliation of duplicates, conflicts, and rollout-vs-contract ambiguity.

> This file is intentionally organized as a canonical contract skeleton first. Content must be migrated from the V1 source into the section named here as the sole owner of each rule. Duplicate normative wording must not be copied into multiple sections.

---

## Document Rules

### Normative status classes

- `Normative`: defines mandatory behavior, invariants, admissibility, forbidden actions, or identity rules.
- `Reference only`: explanatory, roadmap, performance guidance, examples, or implementation notes that cannot override normative sections.
- `Appendix`: schemas, payloads, Cypher, examples, and migration notes.

### Editorial principles for this rewrite

1. One rule, one home.
2. No duplicated normative wording across sections.
3. Examples are never normative unless explicitly stated.
4. Rollout tiers cannot redefine protocol truth.
5. Schema-required fields must reflect all mandatory join and validation rules.
6. Storage admissibility, graph admissibility, and retrieval admissibility must each have one canonical section.
7. Conflicts discovered during migration must be resolved explicitly, not silently merged.

### Migration method

- Migrate by concept block, not by line order.
- Preserve source meaning, but normalize repeated rules into one canonical section.
- Mark every unresolved conflict with `TODO(RECONCILE)` until explicitly resolved.
- Keep implementation examples in appendices only.

---

## Migration Coverage Map

This section is operational metadata for the rewrite. It is not part of the future published protocol.

| V2 section | Primary source sections in V1 | Migration status |
|---|---|---|
| 1. Purpose, Scope, Boundaries | Intro, status note, architectural framing | seeded |
| 2. Core Terms And Axioms | `Архитектурные аксиомы`, `Whole Page Semantic Pass`, `External Job Context`, `Three Planes Architecture` | seeded |
| 3. Canonical Domain Model | `Entity Type Definitions`, `Procedural Scope`, `Frozen Roles`, `Layer Model vs Orthogonal Dimensions` | seeded |
| 4. Object Lifecycle And Quality Levels | `Storage Boundaries`, `Failure Classes`, `Definition of Done`, `Rule Completeness Levels`, `Truth Status` | seeded |
| 5. Pipeline Overview | `Полный поток`, `Failure Handling State Machine` | seeded |
| 6. Step Contracts | Steps `-2` through `7` | complete |
| 7. Validation, Invariants, And Enforcement | `Schema Registry`, `Invariants`, `Protocol Violations`, `Enforcement Matrix`, `Contradiction Gate`, `Over-extraction Check` | complete |
| 8. Storage And Plane Contracts | `Voyage AI`, `Neo4j`, `Three Planes Architecture` | complete |
| 9. Graph And Retrieval Contracts | `Triple Builder`, `Neo4j MERGE Contracts`, `Dedup & Conflict Resolution`, `Retrieval Sync Rule` | complete |
| 10. Ontology Governance | `Registry Governance`, `Deprecation Policy`, `Ontology Ops`, `Registry Governance as Operational Subsystem` | complete |
| 11. Runtime And Auditability | `Runtime Contract`, `Ownership Matrix`, `Pipeline Run State`, `HITL` | complete |
| 12. Operational Rollout | `Complexity Tiers` | complete |
| Appendices | schemas, examples, payloads, Cypher; constants and heuristics (Appendix E) | complete |

---

## 1. Purpose, Scope, Boundaries

### 1.1 Purpose

This document defines the canonical extraction protocol for converting source pages into verified, evidence-bound knowledge objects through a deterministic + LLM-assisted pipeline.

The governing principle is:

`Layer-first -> Detect Mentions -> Canonicalize -> Extract -> Build Graph -> Verify -> Publish`

### 1.2 Scope

This protocol governs:

- page and section understanding,
- extraction-layer routing,
- deterministic join contracts,
- canonical mapping rules,
- extraction outputs,
- triple construction,
- validation and completeness gates,
- storage admissibility,
- graph admissibility,
- ontology governance touchpoints,
- runtime auditability required for replay and HITL.

### 1.3 Out of scope

This file does not define low-level worker deployment, infra rollout, or operational runbook procedures beyond the minimum contracts required for deterministic replay and auditability.

Reference documents may exist for:

- runtime persistence,
- worker rollout,
- recovery operations,
- deployment modes,
- implementation-specific storage DDL,
- implementation-specific Cypher.

Those documents are subordinate to this protocol for domain meaning and admissibility rules.

### 1.4 Boundary rule

If an external runtime or implementation document conflicts with this file on domain rules, extraction admissibility, graph admissibility, or ontology semantics, this file wins.

---

## 2. Core Terms And Axioms

### 2.1 Core terms

- `page_mode`: coarse page utility class used to allow or block extraction paths.
- `section`: deterministic content unit produced before any extraction step.
- `mention`: evidence-bound textual span detected by Agent #2 and addressed by `mention_id` and/or span offsets.
- `canonical_key`: registry-controlled identifier for a concept or other canonical object.
- `RuleInstance`: final procedural truth object; not a direct relation-only representation.
- `deterministic job context`: orchestration-provided business context not generated by LLM.
- `truth plane`: verified knowledge and its evidence-bound canonical objects.
- `retrieval plane`: derived search/index artifacts.
- `serving plane`: assembled user-facing outputs derived from truth and retrieval.

### 2.2 Architectural axioms

1. Layer is determined before extraction.
2. The pipeline order is fixed.
3. Completeness outranks token economy.
4. Every accepted fact must be evidence-bound.
5. Symbolic canonical matching precedes vector similarity.
6. Qdrant is search support, not authority.
7. Procedural truth is modeled as `RuleInstance`, not relation-first shorthand.
8. `procedural`, `operational`, and `editorial` are truth-core layers; `seo` and `commercial` are product layers and must not become sources of truth.

### 2.3 Whole-page understanding principle

Whole-page semantic understanding is mandatory before section-local extraction. Final extracted facts remain section-bound or span-bound for evidence.

### 2.4 Deterministic job context

`deterministic job context` is the only source of truth for external business parameters supplied by orchestration.

Minimum contract:

- `country_code`
- `visa_type`
- `visa_key`
- `target_citizenship` when applicable
- `source_url`
- `source_tier`

LLM must not invent or finalize these fields. LLM may only operate in a way consistent with them.

### 2.5 Three-plane boundary axiom

Data flow is strictly downward:

`Raw Source -> Truth -> Retrieval -> Serving`

Forbidden upward flows:

- Retrieval cannot promote truth status.
- Serving outputs cannot create canonical knowledge.
- LLM cannot write directly into the truth plane.

### 2.6 Canonical numeric ownership by layer

This section defines the only allowed numeric ownership split across truth-core layers.

- `procedural` stores normative numbers and quantitative obligations (amounts, limits, durations, thresholds, deadlines when used as rules).
- `operational` stores operational temporal and validity numerics (dates, times, schedules, `valid_from`, `valid_until`, `ttl_days`).
- `editorial` does not store numeric truth facts as canonical values.

**Canonical hard rule**

No downstream section may redefine numeric ownership. Section `3.5` is the cross-reference target for layer-level interpretation.

---

## 3. Canonical Domain Model

### 3.1 Layer model

The protocol recognizes five layers:

- `procedural`
- `operational`
- `editorial`
- `seo`
- `commercial`

#### Canonical definitions

- `procedural`: normative or rule-like facts governing applicant obligations, eligibility, documents, fees, timelines, submission channels, appointment requirements, forms, and process steps.
- `operational`: current or scheduled state of external operational entities such as schedules, closures, notices, or holiday constraints.
- `editorial`: user-facing themes, pain points, informational angles, and contextual topics that may link to procedural concepts but do not themselves become procedural truth.
- `seo`: deterministic product-layer artifacts for search and clustering.
- `commercial`: business-owned product-layer claims and offers.

#### Canonical rule

Layer defines knowledge type, not authority, temporality, provenance, or audience applicability. Those belong to orthogonal dimensions.

#### 3.1.1 Classification algorithm (normative)

**Normative status:** mandatory for HITL arbitration and human classification. Layer Router uses independent scoring (Sec 6.5), not this sequential tree — but must produce results consistent with it.

Apply to one atomic statement at a time. First `YES` terminates the tree.

```
Q1  If this information is wrong or outdated — will it block the publish gate
    of a visa page?
    YES → PROCEDURAL CORE

Q2  Does this information describe the operating mode or availability of an
    external entity (office, VFS, consulate) AND does it have a concrete
    expiry (valid_until / TTL)?
    YES → OPERATIONAL LAYER

Q3  Is this information needed to write a useful article, FAQ, checklist,
    travel guide, or to connect pages by meaning?
    YES → EDITORIAL / TRAVEL LAYER

Q4  Is this a search query, demand cluster, site page, or link structure
    between pages?
    YES → SEO GRAPH

Q5  Is this an agency service, pricing tier, commercial offer, or CTA?
    YES → COMMERCIAL LAYER

    NO  → Classification error: return to Q1 and re-examine the statement
```

**Hard rules for this algorithm:**

- Questions are applied strictly in order. First `YES` terminates.
- The algorithm applies to one atomic statement, not to a document or section.
- If an object answers `YES` to more than one question it is multi-layer — decompose per Sec 3.1.2.
- This algorithm is the sole arbiter for HITL classification disputes. Do not duplicate its logic in other sections.

#### 3.1.2 Multi-layer decomposition rule (normative)

One real-world object may and should produce nodes in multiple layers. Layers do not compete — they complement each other through explicit edges.

**Normative rule:** numeric values are stored only in Procedural. Editorial references them via `LINKS_TO_PROCEDURE` and never duplicates them. SEO `Page` node pulls edges to all relevant layers.

**Canonical edge pattern:**

```
(SEO:Page) -[:COVERS]-> (EDITORIAL:Topic)
(EDITORIAL:Topic) -[:LINKS_TO_PROCEDURE]-> (PROCEDURAL:Concept)
```

**Reference examples of multi-layer objects:**

| Object | Procedural | Operational | Editorial | SEO / Commercial |
|---|---|---|---|---|
| Travel insurance for Schengen | ✅ `DOCUMENT_REQUIRED` (min 30 000 EUR) | — | ✅ Topic: `travel_insurance_guide` | ✅ Keyword, Page |
| Hotel booking for visa | ✅ `DOCUMENT_REQUIRED` | — | ✅ `preparation_task`: book_hotel | ✅ Keyword, Page |
| Holidays in Poland | — | ✅ `country_holiday` (office closures) | ✅ `travel_topic` (when to travel) | ✅ Keyword |
| How a self-employed person can get a visa | ✅ `ApplicantProfile` conditions | — | ✅ `audience_need` + `pain_point` | ✅ Keyword, Cluster |
| Visa refusal | — | — | ✅ `refusal_scenario` | ✅ Keyword, Page |
| Food in Italy | — | — | ✅ `travel_topic` | ✅ Keyword, Page |
| Consular fee 35 EUR | ✅ `FEE_ITEM` | — | — | — |
| VFS opening hours | — | ✅ `office_schedule` | — | — |
| Document package 150 BYN | — | — | — | ✅ Commercial: `service` |

**Invariant:** numbers stay in Procedural. If an Editorial topic mentions a number, that number must arrive via `LINKS_TO_PROCEDURE` from a Procedural node — never stored directly in the Editorial topic.

### 3.2 Entity type model

The fixed entity mention types are:

- `concept`
- `office`
- `topic`
- `pain_point`
- `risk`
- `task`
- `service`
- `fee`
- `timeline`
- `location`
- `organization`
- `profile`
- `legal_term`
- `date`

#### Canonical rule

Mention types are fixed. Canonical keys evolve under governance; mention types do not.

### 3.3 Procedural role model

The procedural role set is frozen at eight roles:

- `DOCUMENT_REQUIRED`
- `ELIGIBILITY_RULE`
- `FEE_ITEM`
- `TIMELINE_ITEM`
- `WHERE_TO_APPLY`
- `APPOINTMENT_RULE`
- `FORM_REQUIRED`
- `STEP`

#### Canonical rule

A ninth procedural role does not exist within this protocol version. If content does not fit one of these roles, it must be treated as a different layer or require a formal protocol revision.

### 3.4 Role-to-binding matrix

> This table is the sole normative owner of procedural binding requirements. No later section may redefine these requirements. Any capability matrix must derive from this table.

| role | requires_concept | requires_visa | requires_profile | requires_params | allows_null_object |
|---|---|---|---|---|---|
| `DOCUMENT_REQUIRED` | yes | yes | optional | yes | no |
| `ELIGIBILITY_RULE` | yes | yes | yes | yes | no |
| `FEE_ITEM` | yes | yes | optional | yes | no |
| `TIMELINE_ITEM` | yes | yes | optional | yes | no |
| `WHERE_TO_APPLY` | yes | yes | optional | yes | no |
| `APPOINTMENT_RULE` | yes | yes | optional | yes | no |
| `FORM_REQUIRED` | yes | yes | optional | yes | no |
| `STEP` | yes | yes | optional | yes | no |

#### Reconciliation notes

- This table is canonical and binding for all procedural pipelines.
- Capability or rollout tables must reference this table and must not duplicate or override binding columns.
- Merge and graph contracts consume this table as source of truth for required bindings.

### 3.5 Canonical numeric interpretation by layer

This subsection is the canonical interpretation reference for Section `2.6`.

- `procedural`: normative numeric facts.
- `operational`: schedule/validity temporal numerics.
- `editorial`: no numeric truth facts.

All extraction, storage, and graph sections must inherit this rule.

### 3.6 Controlled semantic extension

Semantic richness within a frozen procedural role is expressed through:

- `params`
- `conditions_raw`
- `exceptions_raw`
- `alternatives`
- `modality_raw`
- `applies_to_profiles`
- `subtype`
- `facets`

A new procedural role is forbidden unless a formal protocol revision proves that the structural graph shape, binding contract, and serving semantics change in a way that existing roles plus specialization cannot express.

### 3.7 Subtypes and facets

#### Subtype rule

`subtype` is a semantic family inside a role. It does not change mandatory bindings or graph shape.

#### Facet rule

`facets` are enrichment properties from a governed registry. They must not create new required fields or change graph merge semantics.

### 3.8 Orthogonal dimensions

The following dimensions are orthogonal to layer and must not be implemented as new layers:

- `truth_status`
- `authority_level`
- `temporal_status`
- `audience_scope`
- `source_provenance`

**Canonical hard rule:** a new type of authority, temporal semantics, audience scope, or provenance does not create a new layer. It extends an existing dimension. Any proposal for a sixth layer is a signal that a required dimension is absent or insufficiently formalized.

#### 3.8.1 truth_status

Epistemic status of a knowledge object. Supersedes the legacy `status: "verified|extracted|deprecated"` field with full backward compatibility.

| Value | Meaning |
|---|---|
| `draft` | Created manually or imported; has not entered extraction pipeline |
| `extracted` | Output of extraction pipeline; awaiting verification |
| `verified` | Passed full verification chain; L4 |
| `verified_with_limitations` | Verified with explicit caveats (incomplete params, low confidence, partially confirmed source) |
| `deprecated` | Obsolete; retained for audit trail |
| `superseded` | Replaced by a newer or more precise object; carries `replaced_by` reference |
| `blocked` | Publication blocked; cause recorded (contradiction, range, HITL required) |

Migration: existing `status = "verified"` maps to `truth_status = "verified"`. Existing `status = "deprecated"` maps to `truth_status = "deprecated"`. The `status` field remains as a backward-compatibility alias until migration window closes.

#### 3.8.2 authority_level

Normative weight of the source. Distinct from `source_tier` (origin category): describes the authoritative standing of the knowledge, not its provenance class.

**Canonical hard rule:** `authority_level` is assigned deterministically from `deterministic job context` + `source_tier`. LLM must not evaluate source authority.

| Value | Meaning | Example |
|---|---|---|
| `statutory` | Law, regulation, official normative act | EU Regulation, national visa law |
| `official_government` | Official government website or instruction | MFA website, consulate official page |
| `official_partner` | Official authorized partner (VAC) | VFS Global, BLS International official page |
| `institutional_operational` | Operational information from an institution | Bank processing SLA, medical lab requirements |
| `editorial_verified` | Verified editorial content | Curated explainer, verified guide |
| `editorial_advisory` | Non-normative advisory content | Blog post, forum answer, unverified guide |
| `commercial_claim` | Commercial assertion | Ad copy, sales page |

Conflict resolution order: `statutory` dominates `official_government`; `official_government` dominates `official_partner` and below.

#### 3.8.3 temporal_status

Semantic type of temporal applicability. Does not describe when a specific document expires (`valid_until` in params) — describes the temporal nature of the rule itself.

**Interaction with `valid_until`:** `temporal_status` is the semantic type; `valid_until` is a concrete expiry date. An object may carry `temporal_status = effective_window` AND a specific `valid_until`.

| Value | Meaning | Example |
|---|---|---|
| `timeless` | Rule has no temporal window; applies until superseded | "Passport must be valid for 3 months" |
| `effective_window` | Rule has explicit `valid_from` and/or `valid_until` | New fee amount from April 1 to December 31 |
| `temporary_override` | Temporarily overrides a timeless rule | Suspension of appointments for 2 weeks |
| `recurring` | Repeats on a schedule | Holiday closures, annual reviews |
| `historical` | No longer in force; retained as evidence | Previous fee amount |
| `superseded_by_new_rule` | Explicitly replaced by a newer object; carries `replaced_by` | Obsolete document requirement |

### 3.9 Audience scope

`audience_scope` is the structured applicability envelope for a knowledge object.

Canonical fields:

- `visa_scope`
- `profile_scope`
- `jurisdiction_scope`
- `citizenship_scope`
- `submission_channel_scope`
- `entry_type_scope`

#### Canonical rule

Scope is assembled deterministically from job context plus extraction outputs. LLM may surface applicability evidence but must not finalize the scope envelope.

### 3.10 Source provenance

`source_provenance` is the structured traceability envelope.

Canonical fields:

- `source_url`
- `source_domain`
- `source_tier`
- `source_document_type`
- `crawl_version_id`
- `evidence_section_id`
- `discovered_at`
- `last_verified_at`

#### Canonical rule

Any verified knowledge object without `evidence_section_id` is invalid for the truth plane.

### 3.11 Canonical object envelope

Every verified knowledge object must implement the canonical envelope:

- `node_id`
- `layer`
- `layer_version`
- `truth_status`
- `authority_level`
- `temporal_status`
- `audience_scope`
- `source_provenance`
- `confidence`
- `created_at`
- `updated_at`

#### Canonical rule

For `truth_status = verified`, all five orthogonal dimensions must be fully present.

### 3.12 Role capability matrix

> This section is the sole normative owner of per-role semantic capabilities. It extends the binding matrix (3.4) with capability columns only. Binding columns (`requires_concept`, `requires_visa`, `requires_profile`, `requires_params`, `allows_null_object`) are defined in 3.4 and must not be repeated here.

| role | allows_conditions | allows_exceptions | allows_alternatives | allows_temporal_window | allows_scope_override | subtype_supported | facets_supported |
|---|---|---|---|---|---|---|---|
| `DOCUMENT_REQUIRED` | yes | yes | yes | no | yes | yes | yes |
| `ELIGIBILITY_RULE` | yes | yes | yes | yes | yes | yes | yes |
| `FEE_ITEM` | yes | yes | yes | no | yes | yes | yes |
| `TIMELINE_ITEM` | yes | yes | yes | yes | yes | yes | yes |
| `WHERE_TO_APPLY` | yes | no | yes | no | yes | yes | yes |
| `APPOINTMENT_RULE` | yes | yes | yes | yes | yes | yes | yes |
| `FORM_REQUIRED` | yes | yes | yes | no | yes | yes | yes |
| `STEP` | yes | yes | yes | yes | yes | yes | yes |

**Column definitions:**

- `allows_temporal_window`: role can carry a semantic validity window beyond `params` quantities. For `DOCUMENT_REQUIRED` and `FEE_ITEM` this is expressed through params and versioning, not a dedicated temporal window.
- `allows_scope_override`: role can carry scope-narrowing conditions that override default audience scope.
- `allows_exceptions`: structural exceptions expressed as `exceptions_raw`. `WHERE_TO_APPLY` exceptions must be expressed as `conditions_raw` or a separate `WHERE_TO_APPLY` record — not as structural exceptions.
- `subtype_supported` and `facets_supported`: yes for all 8 roles without exception.

**Canonical hard rule:** this table is the single reference for capability decisions in extraction, validation, and serving. Any capability not listed here as `yes` for a role is treated as unsupported unless a formal protocol revision grants it.

---

## 4. Object Lifecycle And Quality Levels

### 4.1 Lifecycle levels

| Level | Semantic label | Allowed storage | Gate |
|---|---|---|---|
| `L0` | incomplete | nowhere | structural completeness |
| `L1` | extracted | `extracted.*` | Schema Validator |
| `L2` | verified | `verified.*` | Completeness Judge |
| `L3` | retrieval-ready | Qdrant (vectors via voyage-context-3) | business context complete |
| `L4` | graph-safe / publish-safe | Neo4j + CMS + final publish | full verification chain |

### 4.2 Canonical admissibility model

This section is the sole owner of destination admissibility.

All destination admissibility is canonicalized by Section `4.1` table. Other sections must reference `4.1` and must not redefine level semantics or storage labels.

### 4.3 Failure classes

- `structural failure`: missing required fields, missing join anchors, missing required binding keys, missing required validity fields.
- `semantic failure`: ambiguity, contradiction, lost conditions/exceptions/alternatives/modality, hallucinated elements, unresolved cross-layer meaning conflicts.

### 4.4 Definition of done for procedural RuleInstance

A procedural RuleInstance is done only when:

1. schema validation passes,
2. evidence binding exists,
3. required `params` exist,
4. deterministic binding exists where required,
5. no unresolved ambiguous canonical mapping remains,
6. completeness judge reports no unresolved missing or hallucinated elements,
7. dedup and conflict resolution has completed,
8. only then are verified storage, retrieval indexing, graph sync, and publish actions allowed according to destination level.

### 4.5 Truth status model

Canonical statuses:

- `draft`
- `extracted`
- `verified`
- `verified_with_limitations`
- `deprecated`
- `superseded`
- `blocked`

### 4.6 Verified storage definition

`verified.*` is the canonical truth-core store class at `L2`.

- `L2` admits verified truth objects that passed schema and completeness constraints.
- `L4` adds graph-safe and publish-safe admissibility; it does not redefine `verified.*`.
- Graph and publish destinations remain `L4`-gated as defined in `4.1`.

---

## 5. Pipeline Overview

### 5.1 Canonical step order

1. Whole Page Semantic Pass
2. Page Utility Classifier
3. DOM Block Relevance Filter
4. Sectioning
5. CAS Gate
6. Layer Router
7. Entity Span Detection
8. Canonical Mapping
9. Layer-Specific Extraction
10. Triple Builder
11. Completeness Judge
12. Resolution Loop
13. Verification Pipeline
14. Retrieval indexing
15. Graph sync
16. Publish gate
17. Serving assembly

### 5.2 Governing flow rule

No downstream step may silently bypass a blocking upstream gate.

### 5.3 Blocking gates

Canonical blocking gates include:

- page utility deny flags,
- CAS skip decisions,
- unresolved null-canonical requirements,
- schema validation failure,
- join-contract failure,
- completeness judge unresolved failures,
- contradiction gate blocks,
- unresolved HITL-required states,
- storage admissibility failure,
- graph admissibility failure.

### 5.4 Failure state machine

**Normative status:** canonical owner of pipeline object lifecycle states and allowed transitions.

**States**

| State | Meaning |
|---|---|
| `extracted` | Raw output from extraction agent; not yet validated |
| `validation_failed` | Failed Schema Validator or invariant check |
| `mapping_blocked` | `canonical_key = null` for a binding-required entity |
| `paused_for_hitl` | Awaiting human review; pipeline suspended for this object |
| `rerun_pending` | Extractor re-run scheduled |
| `patched` | Targeted patch applied by Triple Builder |
| `verified` | Passed Completeness Judge and verification chain |
| `graph_synced` | Successfully written to Neo4j at L4 |
| `dropped` | Declared invalid or duplicate; removed from pipeline |

**Allowed transitions**

```
extracted          → validation_failed | mapping_blocked | paused_for_hitl | rerun_pending
validation_failed  → rerun_pending | dropped
mapping_blocked    → paused_for_hitl | dropped
paused_for_hitl    → rerun_pending | dropped | verified  (verified only with HITL resolution artifact)
rerun_pending      → extracted
patched            → verified
verified           → graph_synced
graph_synced       → dropped  (on deprecation only)
```

**Forbidden transitions**

```
validation_failed  → verified         (schema failure cannot be auto-promoted)
mapping_blocked    → verified         (null canonical cannot become truth)
mapping_blocked    → graph_synced     (null canonical cannot enter graph)
extracted          → graph_synced     (bypass of Judge and Verification is forbidden)
paused_for_hitl    → graph_synced     (HITL block cannot be skipped)
dropped            → verified         (dropped is terminal)
```

**Canonical hard rules**

- The `mapping_blocked → graph_synced` transition is unconditionally forbidden. Any object with null canonical on a required binding must reach `dropped` or `paused_for_hitl`, never the graph.
- `paused_for_hitl → verified` requires a recorded HITL resolution artifact with `decision_id`, `owner_role`, `decision_type`, and `justification`.
- Every transition must be logged with actor, timestamp, and the triggering condition.

**Failure classes**

- **Structural failure:** missing required field, missing `params`, missing mention anchor for deterministic join, missing validity semantics for temporary operational entity, missing required graph binding key.
- **Semantic failure:** ambiguous canonical mapping, contradiction between rules, graph integrity violation, HITL-required item unresolved.

### 5.5 Pipeline profiles

Profiles may tune orchestration and rollout cadence, but they must not change normative step contracts.

Canonical profile rule:

- `6.7 Canonical Mapping` always includes vector similarity fallback using Qdrant after symbolic matching tiers.
- profiles may adjust runtime scaling or queueing behavior, but not resolution order or required outputs.

---

## 6. Step Contracts

> Each subsection below is the canonical owner of the local step contract. Keep step-local rules here. Do not duplicate global invariants, storage admissibility, ontology governance, or graph identity formulas in step sections.

### 6.1 Whole Page Semantic Pass

**Normative status:** mandatory pre-section semantic pass.

**Purpose**
Understand the page as a whole before sectioning so later local decisions can use page-level context without inventing page meaning downstream.

**What this step does**

- determines the overall `page_mode`,
- identifies dominant layers at page level,
- produces a concise semantic summary of the page,
- establishes the page-level context profile,
- marks obviously mixed sections for downstream care,
- lists globally recurring entities and cross-references.

**Normative inputs**

- rendered page content,
- source metadata,
- deterministic job context.

**Normative outputs**

Under `page_semantic_context` the step must produce:

- `page_mode`
- `dominant_layers[]`
- `page_summary`
- `page_context_profile`
- `mixed_sections[]`
- `cross_reference_map[]`
- `global_entities[]`

**Required output semantics**

- `page_mode` must be one of: `content_page | menu_page | directory_page | landing_page | utility_page`.
- `page_context_profile` may confirm page-level country/visa framing but must not override deterministic job context.
- `mixed_sections[]` identifies section positions expected to contain multiple layers or require subspan routing later.

**Canonical hard rules**

- Whole-page understanding is mandatory.
- This step does not extract facts.
- Final facts remain section-bound or span-bound.
- Deterministic job context remains authoritative for business fields; page understanding may only refine interpretation, not replace authoritative context.

**Downstream consumers**

- Page Utility Classifier
- Layer Router
- later scope/provenance enrichment in deterministic code

### 6.2 Page Utility Classifier

**Normative status:** mandatory gating step before block filtering and extraction routing.

**Purpose**
Determine page utility type and extraction permissions before block filtering and before any extraction pipeline is allowed to start.

**Normative outputs**

- `page_mode`
- `allow_procedural_extraction`
- `allow_operational_extraction`
- `allow_editorial_extraction`
- `allow_structural_extraction`
- `allow_seo_extraction`
- `allow_commercial_extraction`

**Page-mode rules**

- `menu_page` and `directory_page` block procedural extraction; `allow_commercial_extraction = false` by default.
- `landing_page` allows limited extraction only; `allow_seo_extraction = true`, all truth-core extractors restricted.
- `content_page` allows full pipeline subject to later gates.
- `utility_page` must be explicitly classified before any truth-core extraction is allowed; all extraction flags default to false until explicit override.

**Canonical hard rules**

- `allow_*_extraction` is a hard gate, not a hint.
- If `allow_procedural_extraction = false`, procedural extraction must not run, procedural router output becomes irrelevant, and procedural extractor invocation is forbidden.
- If `allow_operational_extraction = false`, operational extractor must not run.
- If `allow_editorial_extraction = false`, editorial extractor must not run.
- If `allow_structural_extraction = false`, structural extraction must not run.
- If `allow_seo_extraction = false`, SEO extractor must not run.
- If `allow_commercial_extraction = false`, commercial extractor must not run.
- Layer Router cannot override this step.

**Failure semantics**

- Missing gate fields blocks all downstream extraction.
- Conflicting page mode and allow flags must be treated as a structural configuration error.

### 6.3 DOM Block Relevance Filter

**Normative status:** mandatory pre-section filtering step.

**Purpose**
Remove navigation, menu, and boilerplate blocks before sectioning so truth extraction sees only relevant content.

**Canonical block roles**

- `content_main`
- `navigation`
- `footer`
- `header`
- `sidebar`
- `related_links`
- `toc`
- `breadcrumbs`
- `promo`
- `form`

**Normative outputs**

For each DOM block the filter must produce:

- `dom_block_id`
- `block_role`
- `content_relevance_score`
- `allow_extraction`

**Canonical hard rules**

- `navigation`, `menu`, and `directory` blocks must not enter the extraction pipeline.
- Blocks classified as boilerplate or navigational must be removed before sectioning.
- This step is allowed to be conservative; false negatives are safer than letting navigation contaminate extraction.

**Failure semantics**

- If block-role classification is unavailable, sectioning must not silently proceed over the full raw DOM without an explicit degraded-mode policy.

### 6.4 Sectioning And CAS Gate

**Normative status:** deterministic section production and re-extraction gating.

#### 6.4.1 Sectioning purpose

Produce deterministic, evidence-bearing sections before any LLM extraction step.

#### 6.4.2 Sectioning rules

A section is:

- one heading tag plus all content until the next heading of the same or higher level,
- or a special detached block when the content type requires independent evidence handling.

Special detached sections:

- each table becomes its own section,
- document lists may become dedicated list sections,
- each FAQ question-answer pair may become its own section.

Fallback rules:

- minimum section size is 50 characters; smaller fragments merge with a neighbor,
- if there are no heading tags, the full page becomes one section with `heading_level = 0`.

#### 6.4.3 Pre-section cleanup

The following must be removed before sectioning:

- navigation,
- header,
- footer,
- breadcrumbs,
- cookie banners,
- popups,
- ad blocks,
- sidebars,
- HTML comments,
- scripts,
- styles.

#### 6.4.4 Normative section object

Each section must carry at least:

- `section_id`
- `source_url`
- `url_key`
- `crawl_version_id`
- `heading_level`
- `heading_text`
- `parent_heading` when available
- `raw_text`
- `content_hash`
- `block_type`
- `char_count`
- `position_on_page`
- `has_table`
- `has_list`
- `has_numbers`
- `source_domain`
- `source_tier`

**Canonical rule**

`section_id` and `content_hash` must be deterministically computed before LLM involvement.

#### 6.4.5 CAS Gate

The CAS gate decides whether a changed section requires re-extraction.

If `content_hash` is unchanged:

- skip the section entirely.

If `content_hash` changed:

- evaluate a cheap semantic diff bundle containing:
  - `numeric_diff`
  - `modality_diff`
  - `condition_diff`
  - `exception_diff`
  - `entity_diff`

If all diff signals are false:

- skip re-extraction as likely layout-only change.

If any diff signal is true:

- send the section forward for extraction.

**Canonical hard rules**

- `numeric_diff` is never a sufficient sole criterion for re-extraction decisions.
- Changes in modality, negation, conditions, exceptions, alternatives, or entity set are semantic changes even without numeric change.
- CAS logic must remain deterministic and code-owned.

### 6.5 Layer Router

**Normative status:** mandatory first extraction-routing step.

**Purpose**
Assign a complete layer score vector and route all subsequent extraction behavior before any layer-specific extraction occurs.

**Normative inputs**

- `heading_text`
- section `raw_text` or bounded excerpt
- `source_tier`
- `block_type`
- `page_semantic_context`
- Page Utility Classifier outputs

**Normative outputs**

- `layer_scores`
- `primary_layer`
- `secondary_layers`
- `reasoning_flags` when implemented
- `confidence`
- `needs_hitl`
- `hitl_reason`

**Canonical output rules**

- All five layers must receive independent scores in `[0,1]`.
- `primary_layer` is the layer with the highest score.
- `secondary_layers` includes all non-primary layers with score `>= 0.50`.

**Secondary-layer routing rules**

- secondary confidence `>= 0.70`: secondary extraction is mandatory if not blocked by utility gates,
- secondary confidence `0.50-0.69`: run secondary extraction and mark result as low-confidence,
- secondary confidence `< 0.50`: do not run the secondary pipeline.

**Canonical hard rules**

- Full score vector is mandatory.
- Primary layer does not suppress secondary layers.
- Utility classifier hard gates override router output.
- Router must evaluate layers independently rather than by sequential branching logic.

#### 6.5.1 Subspan re-routing policy

If a section is semantically mixed, specifically when:

- at least two layer scores are `>= 0.70`, or
- the delta between top two scores is `< 0.12`, or
- the section is large and mixed by structure,

then the section must be decomposed into subspans and Layer Router rerun over:

- sentences,
- list items,
- table rows,
- FAQ items.

**Failure semantics**

- Returning only a primary layer is structurally invalid.
- Ignoring a mandatory secondary layer is a routing failure.

### 6.6 Entity Span Detection

**Normative status:** mandatory mention detection step, parallelizable with Layer Router.

**Purpose**
Detect meaningful entity mentions as evidence-bearing spans without extracting rules.

**Normative behavior**

This step finds objects mentioned in text, classifies them at the fixed mention-type level, and anchors them deterministically inside the section text.

**Allowed entity types**

- `concept`
- `office`
- `topic`
- `pain_point`
- `risk`
- `task`
- `service`
- `fee`
- `timeline`
- `location`
- `organization`
- `profile`
- `legal_term`
- `date`

**Normative outputs per mention**

- `mention_id`
- `raw_text`
- `entity_type`
- `char_start`
- `char_end`
- `has_numeric`
- `is_central`
- `confidence`

**Canonical hard rules**

- This step must not invent new entity types.
- This step must not extract rules.
- `mention_id` must be stable within the section.
- Downstream joins must use `mention_id` and/or exact span offsets, not string similarity.

**Failure semantics**

- Missing span offsets make deterministic downstream binding impossible.
- Type drift outside the fixed registry is a protocol violation candidate, not an acceptable output.

### 6.7 Canonical Mapping

**Normative status:** mandatory registry-driven mapping step for canonical binding.

**Purpose**
Map mentions to canonical keys through deterministic registry logic, using vector similarity as mandatory subordinate fallback after symbolic matching tiers.

**Canonical resolution order**

1. exact alias match,
2. normalized alias match,
3. regex or rule match,
4. vector similarity (Qdrant) when earlier methods fail.

**Normative outputs per mapping**

- `mention_id`
- `raw_text`
- original span information
- `target_registry`
- `canonical_key`
- `mapping_type`
- `match_method`
- `confidence`
- `needs_hitl`
- `qdrant_score` when vector similarity is used

**Qdrant threshold policy from V1 full protocol**

- `>= 0.88`: `auto_map`
- `0.75-0.87`: `review`
- `< 0.75`: `new_candidate` and HITL-required

**Canonical hard rules**

- LLM does not choose canonical keys directly.
- Mapping must reference `mention_id` from mention detection.
- Mapping must carry original span information.
- Mapping must not rely on raw-text heuristics for downstream joins.
- If a required procedural canonical target is unresolved, procedural graph-safe construction is blocked.

### 6.8 Layer-Specific Extraction

**Normative status:** mandatory extraction stage after canonicalization, with one contract per layer pipeline.

**Cross-pipeline hard rule**

`CONDITIONAL REQUIRED` join anchors:

- `linked_mention_ids[]` OR `supporting_spans[]` is required when `role in {DOCUMENT_REQUIRED, ELIGIBILITY_RULE, FEE_ITEM, TIMELINE_ITEM, APPOINTMENT_RULE, FORM_REQUIRED, WHERE_TO_APPLY}`.
- `supporting_spans[].char_start` and `supporting_spans[].char_end` are required for canonical mapping outputs used by deterministic Triple Builder joins.

Without deterministic join anchors, Triple Builder must not create canonical joins.

#### 6.8.1 Procedural Extraction

**Purpose**
Extract procedural rule candidates only from explicit source claims that impose applicant requirements or describe procedural parameters.

**Normative outputs per candidate**

- `role`
- `concept_canonical_key` when available and valid
- `raw_mention`
- `linked_mention_ids[]` and/or exact `supporting_spans[]` as required by the cross-pipeline conditional contract
- `params`
- `severity`
- `applies_to_profiles[]` when present
- `exceptions_raw`
- `conditions_raw`
- `alternatives[]`
- `modality_raw`
- `is_numeric`
- `is_range`
- `is_incomplete`
- `confidence`
- `evidence_section_id`

**Canonical hard rules**

- No inference or unstated conclusions.
- Numeric values must be copied from text, not normalized by guess.
- Role must be one of the eight frozen procedural roles.
- Recommended or optional phrasing must not be converted into mandatory severity.
- Range values must set `is_range = true`.
- Incomplete numeric information must produce incomplete candidates, not fabricated numbers.
- Conditions, exceptions, alternatives, and modality must remain explicitly represented.

**Operational note for V2 placement**

Artifact decomposition between mention, mapping, and rule-candidate semantics remains local to this step, while identity and graph consequences belong later in Section 9.

#### 6.8.2 Operational Extraction

**Purpose**
Extract operational state of external entities such as schedules, notices, closures, blackout windows, and holiday structures.

**Allowed operational entity types**

- `office_schedule`
- `operational_notice`
- `closure_event`
- `holiday_calendar`
- `country_holiday`
- `submission_blackout`

**Normative outputs per entity**

- `entity_type`
- `entity_key_candidate`
- `office_key_candidate` and/or `country_code`
- `raw_mention`
- `params`
- `valid_from`
- `valid_until`
- `ttl_days`
- `confidence`
- `evidence_section_id`

**Type-specific parameter expectations from V1**

- `office_schedule`: weekdays, hours, timezone
- `operational_notice`: notice type and description
- `closure_event`: date range and reason
- `holiday_calendar`: country and year
- `country_holiday`: date and names
- `submission_blackout`: date range and reason

**Canonical hard rules**

- Operational extraction must not create procedural requirements.
- Entity type must come from the fixed operational list.
- Dates use ISO format.
- Times use `HH:MM`.
- Temporary operational entities require validity semantics.
- TTL policy must be deterministically assigned per entity class.

**Canonical alignment note**

Operational dates, times, and TTL fields are owned by `operational` layer numeric semantics defined in Sections `2.6` and `3.5`.

#### 6.8.3 Editorial Extraction

**Purpose**
Extract topics, pain points, and informational angles useful for articles, FAQs, explainers, and checklist framing.

**Allowed topic types**

- `topic`
- `travel_topic`
- `pain_point`
- `risk_factor`
- `preparation_task`
- `audience_need`
- `refusal_scenario`
- `destination_topic`
- `seasonality`

**Normative outputs per topic**

- `topic_type`
- `topic_key_candidate`
- `human_label`
- `raw_mention`
- `links_to_procedure`
- `procedure_concept_candidate` when linked
- `country_code`
- `visa_type`
- `audience_hint[]`
- `intent_type`
- `confidence`
- `evidence_section_id`

**Canonical hard rules**

- Editorial extraction describes what is discussed, not how it is regulated.
- Editorial topics must not become normative numeric fact carriers.
- Pain points are user problems, not procedural rules.
- If a topic links to procedure, the link must be explicit.

#### 6.8.4 SEO Extraction

**Normative status:** mandatory when `allow_seo_extraction = true`.

**Purpose**
Extract units of search demand and structural elements of the site. SEO extraction does not create procedural facts and is not a source of truth.

**Allowed SEO entity types**

| Type | Description |
|---|---|
| `keyword` | search query or variation |
| `keyword_cluster` | thematic group of queries (WCC result) |
| `page_node` | site page as a node (`url`, `page_type`, `cluster_id`) |
| `internal_link` | link relationship between pages with anchor text |
| `content_gap` | topic present at competitors, absent from this site |
| `page_rank_signal` | link weight of a page |

**Normative outputs per entity**

- `entity_type` (from fixed list above)
- `entity_key_candidate`
- `raw_mention`
- `country_code`
- `visa_type` (if applicable)
- `intent_type` (`informational | transactional | navigational`)
- `confidence`
- `evidence_section_id`

**Type-specific parameter expectations**

- `keyword`: `search_volume_hint`, `competition_hint`
- `keyword_cluster`: `cluster_id`, `seed_keyword`
- `page_node`: `url`, `page_type`, `cluster_id`
- `internal_link`: `source_url`, `target_url`, `anchor_text`
- `content_gap`: `competitor_hint`, `missing_topic`

**Canonical hard rules**

- SEO extraction must not create procedural requirements or operational facts.
- SEO extraction is deterministic-only by source policy (Sec 8.7).
- `keyword` and `keyword_cluster` entities must not carry numeric visa fact values.
- `page_node` entities must reference existing or planned pages, not inferred concepts.
- `content_gap` requires `competitor_hint` or explicit editorial source.

#### 6.8.5 Commercial Extraction

**Normative status:** mandatory when `allow_commercial_extraction = true`.

**Purpose**
Extract agency services, pricing tiers, and commercial offers. Commercial extraction describes what the agency sells — not what the consulate requires. Commercial objects never enter the truth-core graph.

**Allowed commercial entity types**

| Type | Description |
|---|---|
| `service` | agency service (document preparation, appointment booking) |
| `service_tier` | pricing plan (basic, premium, urgent) |
| `audience_segment` | audience segment for which the service is sold |
| `cta` | call to action (button, form, link) |
| `partner_offer` | partner offer (insurance via partner, visa photo) |

**Normative outputs per entity**

- `entity_type` (from fixed list above)
- `entity_key_candidate`
- `raw_mention`
- `service_description`
- `price_hint` (if price is mentioned — informational only)
- `currency_hint`
- `audience_hint[]`
- `country_code` (if country-scoped)
- `confidence`
- `evidence_section_id`

**Canonical hard rules**

- Commercial extraction must not capture consular or visa-centre fees — those are `FEE_ITEM` in Procedural.
- Source policy for commercial is `business-owned-only` (Sec 8.7) — not derived from government sources.
- `price_hint` is informational only; it must never flow into Procedural fee calculations.
- Commercial objects do not pass through the Publish Gate (Sec 7.10); they are governed by business CMS workflow.
- Commercial extraction must not create graph triples in the truth-core plane. `minimum_graph_level = none`.

**Critical boundary: Commercial vs Procedural**

| Object | Layer | Reason |
|---|---|---|
| VFS fee 26.5 EUR | PROCEDURAL `FEE_ITEM` | Money goes to VFS; fact from gov source |
| Agency service 150 BYN | COMMERCIAL `service` | Money goes to agency; business data |
| Insurance via partner | COMMERCIAL `partner_offer` | Agency's offer, not a requirement |
| Min. insurance 30 000 EUR | PROCEDURAL `ELIGIBILITY_RULE` | Numeric requirement from gov source |

### 6.9 Triple Builder

**Normative status:** deterministic code-owned construction step.

**Purpose**
Build final identity-bearing triples and graph-safe objects from mappings and extracted outputs.

**Normative inputs**

- canonical mappings,
- procedural rule candidates,
- operational entities,
- editorial topics,
- deterministic context needed for scope/provenance enrichment.

**What this step owns locally**

- conversion of extraction outputs into graph-oriented objects,
- decomposition of compound facts into atomic rule instances,
- deterministic join between mapping outputs and extracted outputs,
- enrichment of outputs with deterministic context fields.

**Canonical hard rules**

- Triple Builder must never use fuzzy string similarity for mention-to-rule binding.
- If required mention anchors are missing, the object is incomplete and must reroute to rerun or HITL.
- Complex multi-entity statements must decompose into atomic rule instances rather than remain ambiguous bundled facts.
- Structured scope-affecting conditions belong in structured params when they materially change applicability.

**Boundary rule**

Graph identity formulas, merge contracts, and dedup identity semantics are owned by Section 9 and must not be redefined here.

### 6.10 Completeness Judge

**Normative status:** mandatory completeness and over-extraction audit before verified truth promotion.

**Purpose**
Compare source text against extracted outputs and detect loss of meaning, lost fragments, and hallucinated additions.

**Normative inputs**

- source `raw_text`,
- constructed triples or equivalent derived outputs,
- unmapped mentions,
- evidence for what was extracted.

**Canonical checks — 7 mandatory loss types**

The judge must inspect all 7 loss types. Each is a named invariant check, not an optional heuristic.

| # | Loss type | What to detect |
|---|---|---|
| 1 | `numbers` | Amounts, deadlines, ranges, quantities, durations present in source text but absent from triples |
| 2 | `conditions` | "if", "when", "provided that", "in case of" phrases not reflected in `conditions_raw` or params |
| 3 | `exceptions` | "except", "excluding", "does not apply to" phrases not reflected in `exceptions_raw` |
| 4 | `modality` | "mandatory", "no later than", "no earlier than", "at least", "strictly before" — modality that changes rule semantics, absent from `modality_raw` |
| 5 | `alternatives` | "either A or B", "instead of X — Y", "or provide" — alternatives absent from `alternatives[]` |
| 6 | `profiles` | Specific applicant categories (children, pensioners, self-employed, students, sponsored) mentioned in source but not bound via `applies_to_profiles` |
| 7 | `fragments` | Whole sentences or clause groups with no mapping to any triple and no `unmapped_raw_text` record |

Additionally: `hallucinated_elements` — triples or claims present in extracted output but absent from source text. This is over-extraction, not a loss type, but is checked in the same pass.

**Canonical hard rule**

If any loss type or hallucination is flagged and `workflow_action` is not set, the Judge output is malformed.

**Additional integrity checks from V1**

The judge must also detect structural integrity loss such as:

- visa-specific procedural objects missing visa binding,
- profile-specific objects missing profile binding,
- role-requiring objects missing concept binding.

**Normative outputs**

- `completeness_score`
- `missing_elements[]`
- `hallucinated_elements[]`
- `workflow_action`
- `unmapped_raw_text`
- `needs_hitl`
- `hitl_reason`

**Canonical hard rules**

- If missing or hallucinated elements remain unresolved, the workflow must reopen extraction, patch deterministically, or pause for HITL.
- Judge findings must influence the workflow, not merely be logged.

### 6.11 Resolution Loop

**Normative status:** mandatory post-judge remediation loop.

**Purpose**
Resolve incompleteness, hallucination, ambiguity, and structural loss through bounded actions before truth promotion.

**Allowed workflow actions**

- `targeted_patch`
- `rerun_extractor`
- `pause_for_hitl`
- explicit drop only where downstream policy permits it

**Canonical hard rules**

- Missing/hallucinated findings may not be ignored.
- `pause_for_hitl` is mandatory when deterministic resolution is not available.
- A blocked mapping or unresolved contradiction cannot advance into graph sync or publish.
- Resolution loop ends only when the candidate is resolved, dropped, or formally paused pending HITL artifact.

**Boundary rule**

State-machine admissibility and cross-step enforcement details are owned by Sections 4, 7, and 11; this step owns only the remediation loop semantics.

---

## 7. Validation, Invariants, And Enforcement

> This section is the canonical owner of machine-enforceable validity. If a rule is mandatory, it must be enforced here through schema, invariant, gate, violation class, or explicit blocking semantics.

### 7.1 Schema validation

**Normative status:** mandatory validation layer for all pipeline outputs.

**Canonical registry requirements**

Every pipeline schema must define at least:

- `schema_name`
- `schema_version`
- `required_fields`
- `validation_rules`
- `producer_step`
- `consumer_step`
- `failure_class`
- `minimum_storage_level`
- `minimum_graph_level`
- `allows_hitl_override`

**Required schema families**

- `LayerRouterOutput`
- `EntityMentionsOutput`
- `CanonicalMappingOutput`
- `ProceduralExtractionOutput`
- `OperationalExtractionOutput`
- `EditorialExtractionOutput`
- `TripleOutput`
- `CompletenessJudgeOutput`
- `QnAAnswerOutput`

**Canonical hard rule**

If an output fails schema validation:

- downstream steps must not run,
- verified storage writes are forbidden,
- graph sync is forbidden,
- the workflow must reroute to diagnostic handling, correction, or HITL.

**Normative minimum field expectations from V1**

- `LayerRouterOutput`: full score vector, primary, secondary, confidence, HITL state.
- `EntityMentionsOutput`: mention identity, raw span text, entity type, span offsets, centrality, confidence.
- `CanonicalMappingOutput`: mention identity, registry target, canonical result, mapping method, confidence, HITL state.
- `ProceduralExtractionOutput`: role, raw mention, params, severity, confidence, evidence.
- `OperationalExtractionOutput`: entity type, params, validity/TTL fields, confidence, evidence.
- `EditorialExtractionOutput`: topic type, human label, confidence, evidence.
- `TripleOutput`: triple identity, subject, relation, object, evidence, confidence.
- `CompletenessJudgeOutput`: score, missing elements, hallucinated elements, workflow action, HITL state.
- `QnAAnswerOutput`: answer class, answer text, evidence, confidence.

### 7.2 Deterministic join contract

**Normative status:** mandatory binding contract between mention detection, canonical mapping, extractors, and Triple Builder.

**Canonical join rule**

The only allowed deterministic joins are:

- by `mention_id`, or
- by exact span offsets in the original section text.

**Required join properties**

- `CanonicalMappingOutput` must reference `mention_id` from `EntityMentionsOutput`.
- Canonical mappings must carry original span information sufficient for deterministic downstream use.
- Extraction outputs that depend on canonical mapping must carry conditional-required anchors: `linked_mention_ids[]` or exact `supporting_spans[]` for roles `DOCUMENT_REQUIRED | ELIGIBILITY_RULE | FEE_ITEM | TIMELINE_ITEM | APPOINTMENT_RULE | FORM_REQUIRED | WHERE_TO_APPLY`.
- Triple Builder must reject objects that require canonical join but lack deterministic anchors.

**Forbidden join behavior**

- heuristic string similarity,
- fuzzy raw-text matching,
- fallback joins based on approximate phrase equality,
- LLM-inferred rebinding without deterministic anchors.

**Canonical hard rule**

Missing deterministic join anchors is a blocking structural failure, not a soft warning.

### 7.3 Pre-storage invariants

**Normative status:** mandatory guard for truth-bearing storage destinations below graph sync level.

The following invariants must hold before verified storage or equivalent truth promotion:

1. No procedural object without `evidence_section_id` may enter verified storage.
2. No procedural object without valid `params` may enter verified storage.
3. No temporary operational object without required validity semantics may enter verified storage.
4. No citizenship-specific retrieval payload may be built without `target_citizenship` from deterministic job context.

**Canonical hard rule**

Any violation of a pre-storage invariant blocks verified write and requires rejection, rerun, patch, or HITL.

### 7.4 Pre-graph invariants

**Normative status:** mandatory graph-admissibility guard.

The following invariants must hold before Neo4j or equivalent graph sync:

1. No procedural graph sync with `null canonical_key` for role-requiring objects.
2. No visa-specific procedural object without `APPLIES_TO -> Visa`.
3. No profile-specific procedural object without `FOR_PROFILE -> ApplicantProfile`.
4. No role-requiring procedural object without `ABOUT -> Concept`, except where the canonical binding matrix explicitly allows absence.
5. No Agent #2 to extractor join may be performed through heuristic string similarity.

**Canonical hard rule**

Graph sync admissibility is stricter than extracted-storage admissibility. A structurally valid extracted object is not automatically graph-safe.

### 7.5 Contradiction gate

**Normative status:** mandatory contradiction blocking mechanism.

**Canonical rule**

If conflicting verified rules exist without a resolution decision, generation and publish actions that depend on them are blocked.

**Minimum contradiction behavior**

- conflicting variants must not silently co-exist as publishable truth,
- workflow must require resolution or block publish,
- contradiction handling may escalate to HITL, conflict workflow, or explicit deprecation/supersession logic.

**Boundary rule**

Conflict-case object modeling and graph-level resolution artifacts belong to graph/governance sections; the blocking semantics belong here.

### 7.6 Over-extraction and hallucination policy

**Normative status:** mandatory negative-validity check.

**Canonical rule**

The Completeness Judge must verify not only missing information but also whether extracted triples contain information absent from the source text.

**Required output**

- `hallucinated_elements[]`

**Canonical hard rule**

If `hallucinated_elements[]` is non-empty, the workflow must reopen extraction, patch deterministically, or pause for HITL. Hallucinated outputs cannot be auto-verified.

### 7.7 Enforcement matrix

**Normative status:** owner mapping between rule, enforcing component, and blocking action.

| Rule | Enforced by | On failure | Forbidden action |
|---|---|---|---|
| `canonical_key != null` before required procedural upsert | workflow + graph sync guard | `pause_for_hitl` | concept/rule upsert |
| procedural candidate must have `params` | schema validator | reject candidate | verified insert |
| page utility deny flag | router/orchestrator | skip pipeline | extractor run |
| missing deterministic mention anchor | Triple Builder | rerun or HITL | heuristic join |
| range without `is_range = true` | Completeness Judge | reopen extraction | adjudication / verified insert |
| missing validity semantics for temporary operational entity | schema validator / writer | reject entity | verified write |
| missing deterministic business context for citizenship-specific retrieval payload | retrieval builder | block insert | retrieval write |

**Canonical hard rule**

This table must remain consistent with invariants and protocol violations. If a rule is here, at least one concrete enforcement component must exist in the system.

### 7.8 Invariant execution matrix

**Normative status:** execution placement for major invariants.

| Invariant | Checked at | Blocking layer | On violation |
|---|---|---|---|
| no null canonical before required procedural graph upsert | canonical mapping gate + workflow | graph sync | `pause_for_hitl` |
| no procedural rule without `params` | schema validator | verified storage | reject candidate |
| no visa-specific rule without visa binding | Triple Builder + graph integrity check | graph sync | block sync |
| no profile-specific rule without profile binding | Triple Builder + graph integrity check | graph sync | block sync |
| no temporary operational entity without validity semantics | schema validator + writer | verified storage | reject entity |
| no citizenship-specific retrieval payload without deterministic context | retrieval payload builder | retrieval storage | block insert |
| no mention binding by string similarity | Triple Builder | triple build | rerun or HITL |

### 7.9 Protocol violations

**Normative status:** hard-stop violation classes.

The following are protocol violations and must trigger immediate blocking of the affected pipeline path:

1. use of a procedural role outside the frozen list,
2. performing mention-to-extractor join by string similarity,
3. attempting graph sync with `null canonical_key` where binding is required,
4. writing a procedural rule to verified storage without `params`,
5. persisting a visa-specific rule without visa binding,
6. persisting a profile-specific rule without profile binding,
7. building a citizenship-specific retrieval payload without deterministic `target_citizenship`,
8. creating orphan graph evidence structures in violation of the canonical section/source contract.

### 7.10 Publish gate

**Normative status:** mandatory pre-publish blocking mechanism.

A procedural rule may reach publish-ready state only when all of the following conditions are simultaneously true:

- `status = verified` (L4, sec 4.1),
- `is_range = false` — numeric ranges without HITL resolution block publish,
- all required canonical bindings are present (concept, visa, profile where required by binding matrix sec 3.4),
- no unresolved HITL-required items remain open against this object.

**Blocking semantics when conditions are not met:**

- HITL-required item unresolved → procedural publish blocked,
- conflicting verified rule without resolution → rule blocked,
- operational entity with unresolved ambiguity → downgraded or expired,
- editorial output may survive publish only if it does not claim verified procedural facts.

**Canonical hard rule**

Publish gate enforcement is mandatory. A pipeline that writes to CMS or Neo4j without checking publish-gate conditions is a protocol violation (sec 7.9).

### 7.11 Schema vs prose parity rule

Every mandatory narrative rule must be encoded by at least one machine-enforceable mechanism:

- schema required or conditional-required field,
- invariant,
- blocking gate,
- protocol-violation class.

Canonical parity requirements:

- canonical mapping span fields required for deterministic joins,
- role-conditional mention anchors required for binding-sensitive procedural roles,
- temporality-sensitive operational validity fields required by entity class.

---

## 8. Storage And Plane Contracts

> This section is the canonical owner of storage meaning, plane boundaries, and allowed cross-plane data movement. Destination admissibility levels remain owned by Section 4; this section defines what each plane is and what may write to it.

### 8.1 Truth plane

**Normative status:** canonical owner of verified knowledge.

**Definition**

Truth plane is the only authoritative source of verified knowledge in the system.

**Core truth objects**

- canonical keys, aliases, scopes, and related registry objects,
- verified procedural RuleInstances,
- verified operational entities,
- verified editorial topics where the protocol allows them into truth,
- evidence bindings,
- HITL resolution artifacts,
- graph nodes and edges derived from fully admitted truth objects.

**Truth-plane properties**

- versioned,
- idempotent,
- auditable,
- provenance-bound,
- status-gated.

**Allowed writers**

Only verified workflow logic and deterministic activities may write to truth. LLM is not a direct writer to truth.

**Canonical hard rules**

- Truth writes require the verification chain.
- Evidence binding is mandatory unless an explicit documented exception exists.
- Retrieval output must never mutate truth directly.
- Serving output must never mutate truth directly.

### 8.2 Retrieval plane

**Normative status:** canonical owner of search and indexing artifacts.

**Definition**

Retrieval plane is a derived search layer used to find relevant context. It is not a source of truth.

**Core retrieval artifacts**

- raw section embeddings,
- verified rule vectors,
- editorial topic vectors,
- canonical registry retrieval collections,
- coarse page or section search surfaces.

**Retrieval-plane properties**

- derived from truth and raw source,
- rebuildable,
- search-optimized rather than normatively authoritative,
- potentially stale relative to truth without making truth invalid.

**Allowed writers**

Only deterministic indexing logic may write to retrieval artifacts.

**Canonical hard rules**

- Retrieval match score is similarity confidence, not truth confidence.
- Retrieval artifacts may be deleted and rebuilt without changing truth validity.
- Retrieval results must not promote truth status.

### 8.3 Serving plane

**Normative status:** canonical owner of assembled user-facing outputs.

**Definition**

Serving plane contains assembled outputs presented to users or CMS consumers. It is not a source of canonical authority.

**Core serving artifacts**

- Q&A responses,
- CMS fragments and blocks,
- checklists,
- explainers,
- structured UI outputs.

**Serving-plane properties**

- ephemeral,
- assembled from truth plus retrieval,
- non-authoritative,
- non-indexable back into retrieval by default.

**Allowed writers**

Assembly services and serving logic may write to serving artifacts. LLM may participate only under verified-fact supervision.

**Canonical hard rules**

- Serving artifacts do not create canonical facts.
- Serving artifacts do not create canonical keys.
- Serving artifacts do not mutate truth statuses.
- Serving outputs are not indexed back into retrieval unless a future protocol revision explicitly defines a different behavior.

### 8.4 Storage boundaries

**Normative status:** owner of storage-layer meaning.

**Storage layers**

- `raw.sections` and equivalent raw evidence stores,
- `extracted.*` for structurally valid but not yet truth-admitted outputs,
- verified truth storage for admitted facts,
- retrieval stores such as Voyage or Qdrant,
- graph stores such as Neo4j,
- serving destinations such as CMS or Q&A response layers.

**Canonical hard rules**

- Raw storage is not truth by itself.
- Extracted storage is not publishable truth by itself.
- Retrieval storage is not truth by itself.
- Graph storage is only for graph-safe admitted objects.
- Serving storage is never authoritative.

**Boundary rule**

Minimum admissibility by level is owned by Section 4. This section defines semantic boundaries, not level assignment.

### 8.5 Allowed transformations

**Normative status:** owner of allowed and forbidden cross-plane flows.

| From | To | Allowed | Constraint |
|---|---|---|---|
| Raw source | Truth | yes | only through the full extraction and verification chain |
| Truth | Truth | yes | verification, merge, HITL resolution, deprecation, supersession |
| Truth | Retrieval | yes | deterministic indexing only |
| Truth | Serving | yes | assembly only from admissible truth |
| Retrieval | Serving | yes | retrieval can supply context but not truth promotion |
| Retrieval | Truth | no | retrieval cannot produce verified truth |
| Serving | Truth | no | generated output cannot become canonical fact |
| Serving | Retrieval | no | user-facing output is not indexed back by default |

**Canonical hard rule**

Any forbidden transformation is a protocol violation.

### 8.6 Retrieval artifacts

**Normative status:** owner of retrieval artifact classes, timing, API contract, payload content, and metadata contracts.

**Embedding model:** `voyage-context-3` (Voyage AI). **Vector store:** Qdrant. These are distinct: Voyage produces vectors; Qdrant stores and serves them. All collections below live in Qdrant.

#### 8.6.0 voyage-context-3 API contract

voyage-context-3 uses a **list-of-lists** input structure. All sections (chunks) from one page must be grouped into a single inner list and sent in one request. Sending sections individually degrades contextual quality because the model infers document-level context from the co-presence of all chunks in the same inner list.

**Endpoint:** `POST https://api.voyageai.com/v1/contextualizedembeddings`

**Request shape:**
```python
vo.contextualized_embed(
    inputs=[
        # All chunks from page 1 in one inner list
        [section_1_text, section_2_text, section_3_text, ...],
        # All chunks from page 2 in next inner list
        [section_A_text, section_B_text, ...],
    ],
    model="voyage-context-3",
    input_type="document",   # "document" for indexing, "query" for search
    output_dimension=1024,   # default; 256/512/1024/2048 supported (Matryoshka)
)
```

**Per-request limits:**

| Constraint | Limit |
|---|---|
| Tokens per inner list (one page) | 32 000 |
| Total tokens per request | 120 000 |
| Inner lists per request | 1 000 |
| Total chunks per request | 16 000 |

**Canonical hard rules for voyage-context-3:**

- All sections from one page MUST be grouped into one inner list — never sent one-by-one.
- Sections MUST NOT overlap. Voyage AI explicitly prohibits chunk overlap for this model; overlap degrades contextual quality.
- `raw_chunks` vectorization MUST wait until all sections of the page are produced, then batch them in one call.
- `input_type = "document"` for indexing; `input_type = "query"` for search queries (single-element inner list: `inputs=[["query text"]]`).
- Page-level embeddings are coarse-search aids only — section-level vectors provide the deterministic evidence linkage via `evidence_section_id`.
- Identical section content from identical page context produces an identical vector — enables idempotent re-run without duplicate indexing.

#### 8.6.1 Unified vectorization contract

This is the single normative reference for when each object class is vectorized, what text is embedded, which Qdrant collection receives it, and what metadata the payload must carry.

| Trigger | What is vectorized | Qdrant collection | Required metadata |
|---|---|---|---|
| All sections of page ready (before extraction) | All `raw_text` sections grouped as one inner list | `raw_chunks` | `section_id`, `url_key`, `source_tier`, `content_hash`, `block_type` |
| Canonical key reaches `indexed` status | `canonical_key + description + aliases (joined)` | `kb_canonical` | `canonical_key`, `concept_type`, `layer` |
| `verified.rules` insert at L3+ | `"[ROLE] [concept_key] params: … severity: …"` | `verified_rules` | `triple_id`, `semantic_rule_key`, `evidence_binding_key`, `visa_key`, `country_code`, `visa_type`, `target_citizenship`, `role`, `concept_key`, `layer`, `source_tier`, `confidence` |
| `kb.editorial_topics` insert at L3+ | `human_label + raw_mention + audience_hint (joined)` | `editorial_topics` | `topic_key`, `topic_type`, `country_code`, `intent_type` |

**Canonical hard rules:**

- Business context fields (`visa_key`, `country_code`, `target_citizenship`) come from deterministic job context. LLM must not invent these values.
- If `target_citizenship` is absent, citizenship-specific retrieval cannot be treated as complete.
- `kb_canonical` and `verified_rules` use separate Qdrant collections — canonical mapping search and fact retrieval must not share a collection.

#### 8.6.2 Raw chunks

Vectorized page-wide after all sections are ready, before extraction begins. One voyage-context-3 call per page. Provides coarse retrieval over the raw evidence corpus with full contextual enrichment per section.

#### 8.6.3 Canonical registry retrieval surface

Vectorized when a canonical key reaches `indexed` status. Enables fuzzy concept matching in the Qdrant `kb_canonical` collection as the fourth-tier fallback after symbolic matching.

#### 8.6.4 Verified rule retrieval surface

Vectorized from normalized rule text after L3 promotion. The text field `text_for_search` combines role, concept, params, and business context deterministically so similarity search can match by semantic content, not raw extraction text.

Expected metadata: full set from unified contract table above (sec 8.6.1).

#### 8.6.5 Editorial retrieval surface

Vectorized after L3 promotion of editorial topics. Uses human-readable label and audience hints rather than procedural params. Must not be used as a source of procedural fact retrieval.

### 8.7 Layer source policy

**Normative status:** owner of layer-specific source admissibility posture.

| Layer | Source policy |
|---|---|
| `procedural` | verified-only |
| `operational` | verified and still-valid by TTL/validity semantics |
| `editorial` | verified-context or explicitly allowed verified editorial policy |
| `seo` | deterministic-only |
| `commercial` | business-owned-only |

**Canonical hard rule**

Layer source policy constrains what may feed publishable or graph-admitted outputs. Extracted-but-unverified artifacts do not satisfy verified-only policies.

### 8.8 Verified storage vs truth plane

The canonical model is:

- `verified.*` is an `L2` truth-plane destination.
- retrieval destinations are `L3`.
- graph and publish destinations are `L4`.

Section `4.1` is the sole owner of level semantics; this section inherits it.

---

## 9. Graph And Retrieval Contracts

> This section is the canonical owner of deterministic identity, triple semantics, graph merge admissibility, dedup policy, and retrieval synchronization after truth changes.

### 9.1 Identity derivation

**Normative status:** owner of deterministic identity formulas.

**Canonical principles**

- identity is code-derived, not LLM-derived,
- identity must be stable under idempotent replay,
- presentation text must not define object identity,
- changes in semantically relevant params must change object identity where required.

**Canonical identities currently required**

- `semantic_rule_key`
- `evidence_binding_key`
- `triple_id`
- `node_id`

**V1-derived formulas currently in scope**

- `triple_id = blake3(subject_key + relation_type + object_key + json_stable_serialize(params) + conditions_key)`
  where `conditions_key = blake3(conditions_raw.strip())` if `conditions_raw` is non-null, otherwise `""`.
  Two rules with identical params but different `conditions_raw` produce different `triple_id` values.
- `node_id` is a stable kind/context/key identity that must not depend on display text,
- `semantic_rule_key = blake3(role + concept_key + visa_key + profile_key + params)`,
- `evidence_binding_key = blake3(semantic_rule_key + evidence_section_id)`.

**Canonical hard rule**

Identity formulas may only be defined here. No other section may redefine the identity hash inputs.

### 9.2 Triple identity and decomposition

**Normative status:** owner of atomic fact decomposition.

**Canonical rule**

Complex statements must decompose into atomic rule instances. One RuleInstance represents one atomic semantic claim suitable for deterministic graph representation.

**Decomposition guidance**

The following semantic components may affect decomposition or identity:

- canonical parameters such as amount, currency, duration,
- scope-affecting conditions such as channel, citizenship, or age,
- applicant profile bindings,
- condition text when it cannot be normalized structurally,
- exceptions and alternatives as explicit semantic attachments.

**Canonical hard rules**

- Two triples with different effective params must not collapse into the same identity.
- Scope-changing conditions must be represented structurally when possible.
- Atomic decomposition is mandatory when a bundled sentence would otherwise mix multiple rule identities.

### 9.3 Procedural graph model

**Normative status:** owner of the procedural graph shape.

**Canonical procedural truth shape**

Procedural truth is modeled RuleInstance-first, not relation-first.

A procedural RuleInstance may bind to:

- `Concept` via `ABOUT`,
- `Visa` via `APPLIES_TO`,
- `ApplicantProfile` via `FOR_PROFILE`,
- `Section` via `SOURCED_FROM`.

**Canonical hard rules**

- Procedural graph sync is forbidden when a required concept binding is unresolved.
- Procedural graph sync is forbidden when required visa or profile bindings are missing.
- Section evidence binding is mandatory for graph-safe procedural objects.

### 9.4 Operational and editorial graph model

**Normative status:** owner of non-procedural graph contracts.

#### 9.4.1 Operational graph model

Operational entities are graph objects derived from deterministic relation typing by `entity_type`.

Deterministic relation mapping from V1:

- `office_schedule` -> `HAS_SCHEDULE`
- `operational_notice` -> `HAS_NOTICE`
- `closure_event` -> `HAS_CLOSURE`
- `holiday_calendar` -> `OBSERVES_HOLIDAY`
- `country_holiday` -> `OBSERVES_HOLIDAY`
- `submission_blackout` -> `HAS_SUBMISSION_BLACKOUT`

**Canonical hard rules**

- Operational relation type is code-derived, not LLM-derived.
- Unsupported operational entity types must not silently fall back to a generic graph relation.
- Temporary operational objects require validity semantics before graph sync.

#### 9.4.2 Editorial graph model

Editorial graph objects may bind:

- `Section -> Topic` via `MENTIONS_TOPIC`,
- `Topic -> Concept` via `LINKS_TO_PROCEDURE` when supported,
- `PainPoint -> ApplicantProfile` via `AFFECTS_PROFILE` when supported.

**Canonical hard rule**

Editorial graph objects may connect to procedural concepts, but they do not become substitutes for procedural truth.

### 9.5 Cross-layer edge policy

**Normative status:** owner of cross-layer admissibility.

Cross-layer edges are allowed only when:

- both sides already exist,
- both sides have stable identities,
- the relation is justified by extraction contract or deterministic code rule,
- the edge does not violate plane boundaries or truth precedence.

**Canonical hard rules**

- Cross-layer edges are bridges, not truth promotion mechanisms.
- Editorial or retrieval-derived links must not overwrite procedural truth.
- Page-level cross-layer edges may exist for navigation or discovery, but they are subordinate to fact-level evidence bindings.

### 9.6 Global graph merge rules

**Normative status:** owner of graph merge admissibility.

**Canonical merge rules**

- Graph MERGE must operate on stable `node_id` or stable canonical keys.
- `created_at` is set only on first creation.
- `updated_at` is set on each upsert.
- `layer`, truth/status fields, and confidence must be set deterministically.
- Freeform merge by display label or human text is forbidden.

**Section MERGE contract (normative)**

The Section graph node is the anchor for all evidence binding in the graph. Its creation is exclusive:

- Section nodes are created only by the deterministic Section MERGE contract, keyed on `section_id`.
- `created_at` is set on first creation only; all subsequent upserts update metadata fields.
- No other contract or activity may create a Section node by any other key, by display label, or as a side-effect of another merge.

**Canonical hard rule**

No other contract may create an empty or partial Section node. The Section MERGE is the sole source of truth for Section nodes in the graph. Creating a Section node outside this contract is Protocol Violation #8 (sec 7.9).

**Key derivation stability rule**

Changes to `raw_text` that do not change normalized semantic content (params, role, concept binding) must preserve the existing `semantic_rule_key`. A `semantic_rule_key` change implies a new semantic fact, not a text edit.

### 9.7 Dedup and conflict policy

**Normative status:** owner of semantic dedup and graph collision behavior.

**Dedup cases from V1**

- identical triple identities collapse at triple-build time,
- identical `semantic_rule_key` with identical `evidence_section_id` collapses at verified-write time,
- same semantic rule across multiple sections converges by `semantic_rule_key` with multiple evidence bindings,
- conflicting params for the same effective binding trigger contradiction handling rather than silent overwrite.

**Canonical hard rules**

- Duplicate identity must collapse deterministically.
- Conflicting semantic variants must block publish until resolved.
- Procedural truth dominates editorial overlap when the question is factual graph truth.
- Multi-section evidence accumulation must not create silent semantic duplication.

### 9.8 Retrieval synchronization rules

**Normative status:** owner of retrieval invalidation after truth or ontology changes.

**Canonical rule**

Changes to truth or ontology that affect retrieval semantics must trigger explicit retrieval refresh behavior. Retrieval does not stay correct automatically.

**Minimum synchronization actions**

- invalidate affected embeddings or mark them stale,
- queue affected objects for reindex,
- refresh vector artifacts before affected extraction or serving flows depend on them when the change is blocking.

**Important change classes**

- canonical registry updates,
- merge/split operations,
- deprecations affecting active keys,
- truth changes that modify retrieval payload semantics.

**Canonical invariants**

- Stale retrieval artifacts MUST NOT be used for high-confidence answers.
- Retrieval artifacts marked stale must be excluded from auto-map and serving until reindexed.

**Ontology → Graph → Retrieval strict dependency chain**

```
Ontology change →
  Graph update →
    Retrieval invalidation →
      Reindex
```

Graph and retrieval MUST NOT diverge from the ontology version that governs them. A verified truth object's `version_id` must match the registry version active at the time of its last verification.

### 9.9 Dependency graph for re-extraction

**Normative status:** owner of graph-aware re-extraction dependency semantics.

**Canonical dependency rule**

Re-extraction may be triggered not only by local section text change but also by upstream semantic changes, including:

- page-context-profile changes,
- canonical mapping changes,
- ontology changes that affect binding or identity.

**Minimum dependency chain from V1**

- `Section -> RuleInstance`
- `RuleInstance -> PageBlock`
- `PageBlock -> Page`

**Canonical hard rule**

Local text stability does not exempt a dependent object from re-extraction if its semantic dependencies changed.

### 9.10 Semantic identity vs evidence identity

The protocol distinguishes two identities:

- `semantic_rule_key`: semantic identity independent of source section.
- `evidence_binding_key`: evidence attachment identity per section.

Canonical merge behavior:

- semantic merge is keyed by `semantic_rule_key`,
- evidence attachment merge is keyed by `evidence_binding_key`,
- multi-section convergence is mandatory when semantic identity is equal.

---

## 10. Ontology Governance

> This section is the canonical owner of registry evolution, alias governance, merge/split, deprecation, ontology versioning, impact analysis, ambiguity control, and governance tooling.

### 10.0 Purpose and scope

**Purpose**

Ontology Ops defines how canonical knowledge evolves safely. Goals:

- prevent semantic drift between extraction, graph, and retrieval,
- ensure consistency across the canonical key registry, verified graph, and vector index,
- enable controlled evolution of canonical keys, profiles, scopes, and relation conventions.

**Scope**

Ontology includes:

- canonical keys (concepts, fees, documents, offices, etc.)
- aliases and locale-specific forms
- applicant profiles
- visa types and context keys
- scope definitions
- relation conventions (graph edge types)

### 10.1 Registry model

**Normative status:** canonical owner of registry authority.

**Canonical rule**

A new canonical key is admissible only when all of the following hold:

- it cannot be adequately expressed through alias or facet alone,
- there is sufficient evidence,
- there is schema support,
- there is at least one meaningful example,
- there is at least one downstream consumer.

**Governance principle**

Ontology is not static reference data. It is a governed operational subsystem whose maturity directly affects extraction quality.

### 10.2 Ontology ingestion

**Normative status:** owner of new ontology item intake.

**Canonical intake statuses**

- `detected`
- `proposed`
- `under_review`
- `accepted`
- `rejected`
- `backfilled`
- `indexed`

**Canonical transitions**

- `detected -> proposed`
- `proposed -> under_review`
- `under_review -> accepted | rejected`
- `accepted -> backfilled -> indexed`
- `rejected -> proposed` when new evidence appears

**Canonical hard rule**

No new canonical key may participate in extraction before status `indexed`. `accepted` is approval, not activation.

**Required intake fields**

- proposer identity or source,
- evidence count,
- entity type,
- locale,
- proposal/review/index timestamps,
- decision justification where relevant.

### 10.3 Alias governance

**Normative status:** owner of alias policy.

#### 10.3.1 Uniqueness

Alias-to-canonical mapping is 1:1 within the same `entity_type` and `locale`.

#### 10.3.2 Ambiguity

If one alias maps to multiple canonical keys in the same `entity_type` and `locale`:

- mark the alias ambiguous,
- block auto-mapping through that alias,
- force affected mappings into review or HITL,
- require curator resolution.

#### 10.3.3 Locale-awareness

- locale-specific aliases take precedence over wildcard aliases,
- wildcard aliases are permitted for codes and locale-agnostic forms,
- locale mismatch must not silently auto-resolve to the wrong canonical key.

#### 10.3.4 Deprecated aliases

Deprecated aliases remain auditable but must not continue as normal automatic mapping surfaces when they are known false-positive sources or have been superseded.

#### 10.3.5 Forbidden aliases

Certain overly generic or provably dangerous aliases are globally forbidden.

**Canonical hard rule**

Adding a forbidden alias requires protocol-owner level justification. Encountering a forbidden alias in production blocks mapping and may pause the affected flow.

### 10.4 Migration layer (Class C only)

**Normative status:** owner of all structural ontology change execution — merge, split, and deprecation of active keys.

These operations are Class C by definition. None may proceed without a complete change contract (sec 10.6.1) and full impact analysis (sec 10.7).

#### 10.4.1 Merge

Applies when two canonical keys are proven to represent the same semantic entity.

**Preconditions (both required before migration begins):**

- semantic equivalence proven and recorded in the change contract,
- impact_analysis completed with affected rule instances, triples, and embeddings enumerated.

**Required actions:**

- define one canonical winner key,
- create deterministic mapping `old_key → new_key` stored in registry,
- migrate all graph bindings from old key to winner key,
- reindex all affected retrieval artifacts (Voyage + Qdrant collections),
- preserve audit artifacts for the merge decision.

**Forbidden:**

- silent merge without a recorded migration mapping,
- merge applied before impact_analysis is complete,
- deletion of the old canonical key before grace period expires.

#### 10.4.2 Split

Applies when one canonical key is found to conflate multiple semantically distinct entities.

**Required before split begins:**

- parent key explicitly identified,
- all child keys defined with canonical definitions,
- remapping rules for each affected RuleInstance stated explicitly,
- full reindex plan covering all affected retrieval collections,
- graph migration plan for all edges referencing the parent key.

**Required actions:**

- affected historical bindings move to `needs_review` status,
- new extraction for concepts depending on the split key is blocked until split resolution is applied,
- graph objects remain auditable through the transition period.

**Canonical hard rule**

A split must not begin without a complete list of affected objects and a remediation plan signed off by `protocol_owner`.

#### 10.4.3 Deprecation of active keys

Applies when a key with existing verified rules or active graph bindings is retired.

**Required:**

- replacement key specified OR explicit nullification decision recorded,
- backward compatibility strategy defined (grace period, aliasing, re-extraction scope),
- affected RuleInstances marked `superseded` or migrated before key is fully blocked.

**Canonical key retirement statuses:**

- `proposed` → `active` → `deprecated` → `blocked_for_new_use` → `superseded` → `archived`

**Canonical hard rules:**

- Deprecated or archived keys are not deleted from the registry.
- Deprecated keys remain accessible for audit and historical replay.
- A key may not move to `blocked_for_new_use` without a documented grace period and backward compatibility strategy.
- Blocking an active key without a replacement or nullification record is forbidden.

### 10.5 Conflict ownership

**Normative status:** owner of explicit conflict assignment. Eliminates "someone will handle it" situations.

| Conflict type | First owner | Escalation |
|---|---|---|
| Alias collision (one alias → 2+ keys) | `ontology_curator` | → `domain_reviewer` if unresolved within SLA |
| Alias on forbidden list | `protocol_owner` | — |
| Semantic merge proposal (2 keys = one entity) | `ontology_lead` + `domain_reviewer` | → `protocol_owner` |
| Split proposal (1 key = 2 entities) | `ontology_lead` + `domain_reviewer` | → `protocol_owner` |
| Profile taxonomy change | `domain_reviewer` | → `protocol_owner` |
| Relation convention change | `protocol_owner` | — |
| New facet_key in registry | `ontology_curator` | → `domain_reviewer` if cross-role |
| Subtype restructuring | `ontology_curator` + `domain_reviewer` | → `protocol_owner` |
| Deprecated key remap (active verified rules) | `domain_reviewer` | → `protocol_owner` |
| Graph migration (Class C) | `protocol_owner` | — |

**Canonical hard rule**

Escalation must be configured before production deployment. An unresolved P0 ontology blocker with no escalation path is a governance failure that may halt affected extraction paths.

### 10.6 Change classes and workflow

**Normative status:** owner of governance routing by change risk.

**Change classes**

- `Class A`: low-risk additive or corrective changes,
- `Class B`: medium-risk semantic extensions,
- `Class C`: high-risk structural or backward-compatibility-impacting changes.

**Canonical workflow shape**

- proposals are triaged into a class,
- low-risk changes may use fast-path validation,
- medium-risk changes require curator review,
- high-risk changes require full impact analysis and formal sign-off,
- applied changes may trigger reindex, graph repair, or migration work.

**Change class examples**

| Class | Examples |
|---|---|
| A | alias addition (no collision), typo fix, metadata correction, new example in entity definition |
| B | new canonical key, new profile, new scope, new relation convention, new subtype, deprecation of alias |
| C | key merge, key split, deprecation of active key with existing verified rules, relation model change, remap verified rules |

**Change workflow state machine**

```
proposal_created
        ↓
[auto-triage: assign change class A / B / C]
        │
        ├── Class A ──→ fast_path_validation
        │                    ↓               ↓
        │             validation_passed  validation_failed
        │                    ↓               ↓
        │             auto_approved    ──→ upgrade to Class B
        │                    ↓
        │             registry_applied
        │
        ├── Class B ──→ curator_review
        │                    ↓
        │             approved | rejected | needs_more_info
        │                    ↓ (approved)
        │             registry_applied
        │
        └── Class C ──→ impact_analysis (mandatory, sec 10.7)
                             ↓
                     [impact report ready]
                             ↓
                     formal_decision (protocol_owner sign-off)
                             ↓
                     approved | rejected | deferred
                             ↓ (approved)
                     migration_planned
                             ↓
                     registry_applied
        │
        └── [after registry_applied for all classes]
                     reindex_required? ──→ reindex_queued
                     graph_repair_required? ──→ graph_repair_queued
                             ↓
                         completed
```

Every state transition must be recorded in audit log with `actor`, `timestamp`, `justification`.

**Fast-path validation checks (Class A — all must pass)**

| Check | Pass criterion |
|---|---|
| Alias uniqueness | No collision in same `entity_type` + `locale` |
| Forbidden aliases check | Alias is not in `forbidden_aliases` registry |
| Structural format | Fields correctly typed; key reference valid |
| Impact scope | Change is purely additive; does not touch existing verified rules |
| Ambiguity budget | `ambiguous_aliases` count has not exceeded Warning threshold (sec 10.10.3) |

If any check fails: Class A upgrades to Class B automatically; curator receives the escalated proposal with failure reason.

**Canonical hard rule**

Class assignment is automatic by policy. Manual down-classing requires explicit protocol-owner justification.

### 10.6.1 Mandatory change contract

Every ontology change MUST include a structured change record. This contract is required before any registry mutation is applied.

```json
{
  "change_id": "string",
  "change_class": "A | B | C",
  "proposed_by": "agent | human",
  "timestamp": "iso8601",
  "target_entities": ["canonical_key_1"],
  "impact_analysis": {
    "affected_rule_instances": "number",
    "affected_triples": "number",
    "affected_embeddings": "number",
    "affected_queries": "low | medium | high"
  },
  "version_id": "string",
  "requires_migration": "boolean",
  "migration_plan": "string | null",
  "reindex_required": "boolean"
}
```

**Canonical hard rule**

A change record without complete `impact_analysis` must be REJECTED. A Class C change record without `migration_plan` must be REJECTED.

### 10.7 Impact analysis

**Normative status:** owner of pre-change impact requirements.

Impact analysis is mandatory for at least medium and high risk ontology changes.

**Required impact fields**

- affected rule instances,
- affected triples,
- affected embeddings or retrieval collections,
- affected query patterns or QA surfaces,
- estimated reindex volume,
- migration plan where required,
- rollback plan for high-risk changes.

**Canonical hard rule**

No high-risk ontology change may be applied without complete impact analysis.

### 10.8 Versioning and rollback

**Normative status:** owner of ontology version semantics.

**Canonical rule**

Every applied registry change creates a new registry version identifier. Truth and retrieval artifacts must be reproducible against the registry version that governed them.

**Version metadata**

- `version_id`
- `applied_at`
- `change_class`
- `proposal_id`
- `delta`
- `snapshot_ref`

**Canonical hard rule**

Registry version must be traceable from verified truth objects so the system can reproduce knowledge state historically.

### 10.9 Conflict model and ownership

**Normative status:** owner of ontology conflict classification and first-responder ownership.

**Canonical conflict types**

- duplicate,
- ambiguity,
- partial overlap,
- cross-type collision.

**Canonical hard rules**

- ambiguity blocks auto-map until resolved,
- conflict ownership must be explicit,
- missing escalation path is governance failure,
- unresolved P0 ontology blockers may halt affected extraction paths.

### 10.10 SLA, ambiguity budget, and KPIs

**Normative status:** owner of operational ontology discipline.

#### 10.10.1 SLA

Ontology changes have explicit SLA by severity and type. Blocking ambiguity and forbidden-alias incidents are highest priority.

#### 10.10.2 Ambiguity budget

The protocol tracks ontology debt through metrics such as:

- unresolved candidates,
- ambiguous aliases,
- review-queued collisions,
- temporary fallback mappings,
- blocked split items.

Critical thresholds may block new extraction until debt is reduced.

#### 10.10.3 Per-domain ambiguity

Per-domain ambiguity metrics track debt at the level of a single domain slice (`country_code + visa_type`). They may independently block extraction for that slice even when global metrics remain healthy.

| Metric | Unit | Warning | Critical |
|---|---|---|---|
| `unresolved_candidates` per domain | per 1000 sections | > 5 | > 15 |
| `ambiguous_aliases` per domain | per domain slice | > 2 | > 5 |
| `temp_fallback_mappings` per domain | per domain slice | > 3 | > 10 |

**Domain-blocking rule:** when any per-domain metric reaches Critical, extraction passes for that `country + visa_type` are blocked until the metric drops below Warning. The global budget (sec 10.10.2) may be healthy — per-domain blocking is independent. This prevents a newly added country from silently degrading while global averages hide the problem.

#### 10.10.4 KPIs

Operational ontology KPIs measure time-to-decision, collision rate, auto-resolution rate, deprecated-key exposure, backlog, and backfill completion.

**Canonical hard rule**

KPI breaches may not always block the pipeline, but critical ambiguity-budget breaches do.

### 10.11 Governance tooling requirements

**Normative status:** owner of required governance tooling expectations.

**P0 tooling expected before production deployment**

- registry diff viewer,
- alias collision detector,
- impact analyzer.

**Higher-level tooling may include**

- reindex queue planner,
- merge preview,
- split analysis,
- graph migration preview,
- audit log viewer,
- ambiguity dashboard,
- ontology KPI reporting.

**Canonical hard rule**

Production deployment of ontology-dependent extraction without required P0 governance tooling is an operational-readiness failure.

### 10.12 Safety invariants

These invariants are non-negotiable and apply to every ontology change regardless of class or urgency.

1. No ontology change may be applied without impact analysis.
2. No Class C change may be applied without a migration plan.
3. No retrieval artifact known to be stale from an ontology change may serve high-confidence answers.
4. No ambiguous key may enter the verified graph until HITL resolves the ambiguity.
5. No silent merge or split — every structural change must be recorded in the mandatory change contract (sec 10.6.1).

---

## 11. Runtime And Auditability

> This section is the canonical owner of replayability, audit fields, workflow/activity ownership boundaries, and HITL decision artifacts required for deterministic operations.

### 11.1 Idempotency and replay

**Normative status:** owner of replay-critical runtime fields.

Each pipeline step must record at least:

- `input_hash`
- `output_hash`
- `idempotency_key`
- `retry_policy`
- `requires_hitl`

Replay must record at least:

- `prompt_version` for LLM steps,
- `model_version` for LLM steps,
- `registry_version`,
- `pipeline_version`.

**Canonical hard rule**

A non-auditable step is not publish-gate eligible.

### 11.2 Workflow vs activity ownership

**Normative status:** owner of orchestration boundaries.

**Canonical ownership split**

- workflow layer owns ordering, retry/backoff, wait/signal behavior, replay control, and deterministic state transitions,
- deterministic activities own external side effects and business logic execution such as HTTP, LLM calls, DB writes, Qdrant writes, and Neo4j operations,
- HITL pause/resume spans workflow orchestration plus deterministic HITL activities.

**Canonical hard rule**

Workflow logic must remain deterministic. Time, randomness, and ad-hoc non-deterministic identity generation must not leak into workflow state transitions.

### 11.3 Pipeline run state

**Normative status:** owner of execution-run audit fields.

Each execution-run record must include at least:

- `step_name`
- `status`
- `input_hash`
- `output_hash`
- `idempotency_key`
- `requires_hitl`
- `prompt_version` when applicable
- `model_version` when applicable
- `registry_version`
- `pipeline_version`
- `started_at`
- `finished_at`
- `attempt_no`

**Canonical hard rule**

Missing run-state fields makes the step non-auditable and therefore ineligible for publish-critical flows.

### 11.4 HITL resolution artifact

**Normative status:** owner of deterministic human-resolution output.

Every HITL decision must produce a deterministic resolution artifact containing at least:

- `decision_id`
- `owner_role`
- `decision_type`
- `target_entity_id`
- `previous_state`
- `next_state`
- `justification`
- `affects_registry_version`
- `affects_graph_sync`
- `applied_at`
- `applied_by`

**Canonical hard rule**

A paused-for-HITL object may not silently re-enter the pipeline without a resolution artifact when the relevant policy requires one.

### 11.5 Escalation semantics

**Normative status:** owner of escalation path requirements.

The system must define escalation paths for at least:

- new canonical candidates,
- ambiguity conflicts,
- contradictions,
- blocked publish conditions,
- operational ambiguity,
- missed high-priority ontology SLA.

**Canonical hard rule**

Missing escalation routing for a blocking state is an operational-governance failure.

---

## 12. Operational Rollout

> This section is `Reference only` unless a subsection is explicitly referenced by a normative core section. It governs staged implementation discipline and prevents rollout guidance from mutating protocol truth.

### 12.1 Complexity tiers

The protocol distinguishes staged implementation tiers so that rollout order does not become accidental architecture.

#### Tier 0: non-negotiable core

Includes the minimum system required for deterministic extraction correctness:

- deterministic sectioning,
- CAS gate,
- mention detection,
- symbolic-first canonical mapping,
- layer routing,
- procedural extraction,
- schema validation,
- extracted and verified storage foundations,
- evidence binding,
- canonical registry.

#### Tier 1: quality and operational gates

Adds production-grade control:

- Completeness Judge,
- Resolution Loop,
- HITL playbook,
- failure state machine,
- operational extraction,
- editorial extraction,
- retrieval indexing,
- graph sync,
- publish gate,
- page utility classifier,
- ambiguity budget controls.

#### Tier 2: advanced reasoning

Adds higher-complexity components justified only by production signal, such as:

- contradiction reasoning layers,
- cross-layer reasoning,
- advanced graph analytics,
- mandatory vector-assisted canonical fallback after symbolic matching tiers,
- subtype/facet-driven specialization where needed.

#### Tier 3: operational intelligence

Adds scale-driven operational tooling and analytics:

- ontology KPI dashboards,
- per-domain complexity reporting,
- automated coverage audit,
- advanced registry tooling,
- graph migration previews,
- serving-plane analytics.

### 12.2 Production-readiness gates

The intended progression is:

- Tier 0 establishes correctness foundations,
- Tier 1 establishes production readiness,
- Tier 2 and Tier 3 require measured signal, not architectural aspiration.

### 12.3 Do-not-build rules

The protocol explicitly discourages building advanced components before there is production signal, including:

- vector-assisted canonical mapping before symbolic-first limits are measured,
- contradiction graphs before real contradiction volume exists,
- subtype/facet specialization before role pressure is real,
- cross-layer graph analytics before graph density justifies them,
- per-domain ambiguity monitoring before multiple active domain slices exist,
- dashboard-heavy ontology tooling before Tier 1 stability.

### 12.4 Canonical rollout rule

Rollout guidance may delay or profile-enable features, but it must not silently change the meaning of any protocol rule defined in Sections 2 through 11.

---

## Appendix A. Schema Registry

> This appendix is the canonical machine-facing schema appendix. The normative meaning of each output is defined in Sections 6 and 7; this appendix defines required fields, schema metadata, and known conditional requirements.

### A.0 Registry-wide schema contract

Every schema in the registry must define at least:

- `schema_name`
- `schema_version`
- `required_fields`
- `validation_rules`
- `producer_step`
- `consumer_step`
- `failure_class`
- `minimum_storage_level`
- `minimum_graph_level`
- `allows_hitl_override`

**Canonical hard rule**

A schema entry without these registry-level fields is incomplete and cannot serve as a final normative schema artifact.

### A.1 LayerRouterOutput

```json
{
  "schema_name": "LayerRouterOutput",
  "schema_version": 1,
  "producer_step": "6.5 Layer Router",
  "consumer_step": "6.8 Layer-Specific Extraction",
  "failure_class": "structural",
  "minimum_storage_level": "L1",
  "minimum_graph_level": "L4",
  "allows_hitl_override": true,
  "required_fields": [
    "layer_scores",
    "primary_layer",
    "secondary_layers",
    "confidence",
    "needs_hitl"
  ],
  "validation_rules": [
    "all five layers must be present in layer_scores",
    "primary_layer must be the max-scoring layer",
    "secondary_layers may not include primary_layer"
  ]
}
```

**Conditional requirements**

- if `needs_hitl = true`, `hitl_reason` should be present.
- if any secondary layer is present, each item should carry both `layer` and `confidence`.

### A.2 EntityMentionsOutput

```json
{
  "schema_name": "EntityMentionsOutput",
  "schema_version": 2,
  "producer_step": "6.6 Entity Span Detection",
  "consumer_step": "6.7 Canonical Mapping",
  "failure_class": "structural",
  "minimum_storage_level": "L1",
  "minimum_graph_level": "L4",
  "allows_hitl_override": false,
  "required_fields": [
    "mentions[].mention_id",
    "mentions[].raw_text",
    "mentions[].entity_type",
    "mentions[].char_start",
    "mentions[].char_end",
    "mentions[].is_central",
    "mentions[].confidence"
  ],
  "validation_rules": [
    "entity_type must be from the fixed mention registry",
    "char_start must be <= char_end",
    "mention_id must be unique within the section"
  ]
}
```

**Recommended fields**

- `mentions[].has_numeric`

### A.3 CanonicalMappingOutput

```json
{
  "schema_name": "CanonicalMappingOutput",
  "schema_version": 2,
  "producer_step": "6.7 Canonical Mapping",
  "consumer_step": "6.8 Layer-Specific Extraction | 6.9 Triple Builder",
  "failure_class": "structural",
  "minimum_storage_level": "L1",
  "minimum_graph_level": "L4",
  "allows_hitl_override": true,
  "required_fields": [
    "mappings[].mention_id",
    "mappings[].raw_text",
    "mappings[].span.char_start",
    "mappings[].span.char_end",
    "mappings[].target_registry",
    "mappings[].canonical_key",
    "mappings[].mapping_type",
    "mappings[].match_method",
    "mappings[].confidence",
    "mappings[].needs_hitl"
  ],
  "validation_rules": [
    "mapping must reference an existing mention_id",
    "mapping_type and match_method must be internally consistent",
    "new_candidate mappings must set needs_hitl = true"
  ]
}
```

**Conditional requirements**

- if vector similarity is used, `qdrant_score` must be present.
- if `canonical_key = null` for a required procedural target, downstream graph-safe construction must block.

### A.4 ProceduralExtractionOutput (ExtractedRuleCandidate)

```json
{
  "schema_name": "ProceduralExtractionOutput",
  "schema_version": 2,
  "producer_step": "6.8.1 Procedural Extraction",
  "consumer_step": "6.9 Triple Builder",
  "failure_class": "structural",
  "minimum_storage_level": "L1",
  "minimum_graph_level": "L4",
  "allows_hitl_override": true,
  "required_fields": [
    "extracted_rules[].role",
    "extracted_rules[].raw_mention",
    "extracted_rules[].params",
    "extracted_rules[].severity",
    "extracted_rules[].confidence",
    "extracted_rules[].evidence_section_id"
  ],
  "conditional_required_fields": [
    "for role in {DOCUMENT_REQUIRED, ELIGIBILITY_RULE, FEE_ITEM, TIMELINE_ITEM, APPOINTMENT_RULE, FORM_REQUIRED, WHERE_TO_APPLY}: extracted_rules[].linked_mention_ids OR extracted_rules[].supporting_spans",
    "if extracted_rules[].supporting_spans is present: supporting_spans[].char_start and supporting_spans[].char_end"
  ],
  "validation_rules": [
    "role must belong to the frozen procedural role set",
    "params must exist even if incomplete",
    "evidence_section_id must be present",
    "range values require explicit is_range handling"
  ]
}
```

**Conditional requirements**

- if canonical binding is required by role, extraction output must include `linked_mention_ids[]` or exact supporting spans.
- if the text contains range semantics, `is_range` should be present and true.
- if the rule is incomplete, `is_incomplete` should be present.
- if the rule binds to applicant profiles, `applies_to_profiles[]` should be present.

### A.5 OperationalExtractionOutput

```json
{
  "schema_name": "OperationalExtractionOutput",
  "schema_version": 2,
  "producer_step": "6.8.2 Operational Extraction",
  "consumer_step": "6.9 Triple Builder | graph sync",
  "failure_class": "structural",
  "minimum_storage_level": "L1",
  "minimum_graph_level": "L4",
  "allows_hitl_override": true,
  "required_fields": [
    "operational_entities[].entity_type",
    "operational_entities[].params",
    "operational_entities[].valid_until",
    "operational_entities[].ttl_days",
    "operational_entities[].confidence",
    "operational_entities[].evidence_section_id"
  ],
  "validation_rules": [
    "entity_type must belong to the fixed operational type registry",
    "temporary operational entities require validity semantics",
    "evidence_section_id must be present"
  ]
}
```

**Conditional requirements**

- `office_key_candidate` or `country_code` should be present depending on entity kind.
- `valid_from` should be present where the source encodes an effective start.
- if an entity is temporally temporary in meaning, missing `valid_until` or `ttl_days` is invalid for verified storage and graph sync.

### A.6 EditorialExtractionOutput

```json
{
  "schema_name": "EditorialExtractionOutput",
  "schema_version": 1,
  "producer_step": "6.8.3 Editorial Extraction",
  "consumer_step": "6.9 Triple Builder | retrieval indexing",
  "failure_class": "structural",
  "minimum_storage_level": "L1",
  "minimum_graph_level": "L4",
  "allows_hitl_override": true,
  "required_fields": [
    "topics[].topic_type",
    "topics[].human_label",
    "topics[].confidence",
    "topics[].evidence_section_id"
  ],
  "validation_rules": [
    "topic_type must belong to the fixed editorial topic-type registry",
    "evidence_section_id must be present"
  ]
}
```

**Conditional requirements**

- if `links_to_procedure = true`, `procedure_concept_candidate` should be present.
- `raw_mention` should be preserved for audit and retrieval utility.
- `country_code` and `visa_type` should be present where domain scoping is known.

### A.7 SeoExtractionOutput

```json
{
  "schema_name": "SeoExtractionOutput",
  "schema_version": 1,
  "producer_step": "6.8.4 SEO Extraction",
  "consumer_step": "SEO product pipeline | Triple Builder (links only)",
  "failure_class": "structural",
  "minimum_storage_level": "L1",
  "minimum_graph_level": "L4",
  "allows_hitl_override": false,
  "required_fields": [
    "seo_entities[].entity_type",
    "seo_entities[].entity_key_candidate",
    "seo_entities[].confidence",
    "seo_entities[].evidence_section_id"
  ],
  "validation_rules": [
    "entity_type must belong to the fixed SEO entity-type registry",
    "SEO entities must not carry procedural numeric values",
    "evidence_section_id must be present"
  ]
}
```

**Conditional requirements**

- `intent_type` required for `keyword` entities.
- `url` required for `page_node` entities.
- `anchor_text` required for `internal_link` entities.
- `competitor_hint` or explicit editorial source required for `content_gap` entities.

### A.8 CommercialExtractionOutput

```json
{
  "schema_name": "CommercialExtractionOutput",
  "schema_version": 1,
  "producer_step": "6.8.5 Commercial Extraction",
  "consumer_step": "business CMS workflow (not Triple Builder)",
  "failure_class": "structural",
  "minimum_storage_level": "L1",
  "minimum_graph_level": "none",
  "allows_hitl_override": false,
  "required_fields": [
    "commercial_entities[].entity_type",
    "commercial_entities[].entity_key_candidate",
    "commercial_entities[].confidence",
    "commercial_entities[].evidence_section_id"
  ],
  "validation_rules": [
    "entity_type must belong to the fixed commercial entity-type registry",
    "price_hint must not reference consular or visa-centre fee amounts",
    "commercial entities must not flow into the truth-core graph"
  ]
}
```

**Conditional requirements**

- `price_hint` and `currency_hint` should be present when price is mentioned in source.
- `audience_hint[]` should be present for `service_tier` and `audience_segment` entities.
- `country_code` should be present when the offer is country-scoped.

**Note on `minimum_graph_level = "none"`:** Commercial objects are governed by business CMS workflow and must never enter Neo4j truth-core graph. This is intentional and must not be corrected.

### A.9 TripleOutput

```json
{
  "schema_name": "TripleOutput",
  "schema_version": 1,
  "producer_step": "6.9 Triple Builder",
  "consumer_step": "7 Validation | graph sync | retrieval builders",
  "failure_class": "structural",
  "minimum_storage_level": "L2",
  "minimum_graph_level": "L4",
  "allows_hitl_override": false,
  "required_fields": [
    "triples[].triple_id",
    "triples[].subject_key",
    "triples[].relation_type",
    "triples[].object_key",
    "triples[].evidence_section_id",
    "triples[].confidence"
  ],
  "validation_rules": [
    "triple_id must be deterministic",
    "subject, relation, and object must be graph-resolvable for graph-safe use",
    "evidence_section_id must be present"
  ]
}
```

**Conditional requirements**

- `params` should be carried where relation semantics depend on structured values.
- graph-safe triples require all mandatory bindings implied by the role model.

### A.8 CompletenessJudgeOutput

```json
{
  "schema_name": "CompletenessJudgeOutput",
  "schema_version": 2,
  "producer_step": "6.10 Completeness Judge",
  "consumer_step": "6.11 Resolution Loop",
  "failure_class": "semantic",
  "minimum_storage_level": "L2",
  "minimum_graph_level": "L4",
  "allows_hitl_override": true,
  "required_fields": [
    "completeness_score",
    "missing_elements",
    "hallucinated_elements",
    "workflow_action",
    "needs_hitl"
  ],
  "validation_rules": [
    "workflow_action must be consistent with missing/hallucinated findings",
    "needs_hitl must be true when unresolved ambiguity requires human intervention"
  ]
}
```

**Conditional requirements**

- if `missing_elements` or `hallucinated_elements` is non-empty, `workflow_action` must be a blocking/remediation action rather than a no-op.
- `hitl_reason` should be present when `needs_hitl = true`.

### A.9 QnAAnswerOutput

```json
{
  "schema_name": "QnAAnswerOutput",
  "schema_version": 1,
  "producer_step": "serving / answer assembly",
  "consumer_step": "final answer delivery",
  "failure_class": "semantic",
  "minimum_storage_level": "L4",
  "minimum_graph_level": "L4",
  "allows_hitl_override": false,
  "required_fields": [
    "answer_class",
    "answer_text",
    "evidence_keys",
    "confidence"
  ],
  "validation_rules": [
    "answer_class must belong to the supported answer-class set",
    "evidence_keys must reference admissible evidence surfaces"
  ]
}
```

**Supported answer classes from V1**

- `exact_verified_answer`
- `verified_but_incomplete`
- `needs_more_input`
- `no_verified_data`
- `editorial_context_answer`

### A.10 Schema parity checklist

Before `V2` is considered normative-complete, verify for each schema:

- required fields match prose mandates,
- deterministic join anchors are enforceable,
- conditional required fields are explicit,
- failure classes are attached,
- minimum storage and graph levels are explicit,
- destination admissibility is unambiguous,
- HITL override behavior is explicit,
- role-sensitive and temporality-sensitive rules are represented structurally rather than only in prose.

### A.11 Priority reconciliation items for schemas

Highest-priority schema reconciliation work:

1. make deterministic span carriage explicit for canonical mappings,
2. formalize mention-link requirements for extraction outputs when canonical binding is required,
3. clarify conditional validity rules for operational temporality,
4. formalize graph-safe binding requirements as schema-level or invariant-level constraints,
5. align schema-level admissibility with the final resolved `verified storage` model.

---

## Appendix B. Examples

> Examples are illustrative and non-normative. If an example conflicts with Sections 2 through 11, the normative sections win.

### B.1 Whole-page semantic pass example

```json
{
  "page_semantic_context": {
    "page_mode": "content_page",
    "dominant_layers": ["procedural", "editorial"],
    "page_summary": "Official page about a tourist visa with blocks for documents, timelines, fees, and FAQ.",
    "page_context_profile": {
      "country": "PL",
      "visa_type": "tourist"
    },
    "mixed_sections": [3, 7],
    "cross_reference_map": [],
    "global_entities": ["passport", "consular_fee"]
  }
}
```

### B.2 Section object example

```json
{
  "section_id": "blake3(url + heading_text + position_on_page)",
  "source_url": "https://poland.mfa.gov.pl/en/visas/tourist",
  "url_key": "normalize_source_url(source_url)",
  "crawl_version_id": 42,
  "heading_level": 2,
  "heading_text": "Required Documents",
  "parent_heading": "Tourist Visa to Poland",
  "raw_text": "full block text including tables and lists",
  "content_hash": "blake3(raw_text)",
  "block_type": "requirements",
  "char_count": 847,
  "position_on_page": 3,
  "has_table": true,
  "has_list": true,
  "has_numbers": true,
  "source_domain": "poland.mfa.gov.pl",
  "source_tier": "government"
}
```

### B.3 Canonical mapping example

```json
{
  "mappings": [
    {
      "mention_id": "m1",
      "raw_text": "passport",
      "span": { "char_start": 12, "char_end": 24 },
      "target_registry": "kb.concepts",
      "canonical_key": "passport",
      "mapping_type": "alias",
      "match_method": "exact_alias",
      "qdrant_score": null,
      "confidence": 1.0,
      "needs_hitl": false
    },
    {
      "mention_id": "m6",
      "raw_text": "police clearance certificate",
      "span": { "char_start": 201, "char_end": 221 },
      "target_registry": "kb.concepts",
      "canonical_key": null,
      "mapping_type": "new_candidate",
      "match_method": "qdrant",
      "qdrant_score": 0.61,
      "confidence": 0.61,
      "needs_hitl": true
    }
  ]
}
```

### B.4 Procedural extraction example

```json
{
  "extracted_rules": [
    {
      "role": "FEE_ITEM",
      "concept_canonical_key": "consular_fee",
      "raw_mention": "Consular fee is 35 EUR",
      "linked_mention_ids": ["m2"],
      "params": {
        "amount": 35,
        "currency": "EUR"
      },
      "severity": "mandatory",
      "applies_to_profiles": ["adult", "student"],
      "exceptions_raw": "children under 6 are exempt",
      "conditions_raw": null,
      "alternatives": [],
      "modality_raw": null,
      "is_numeric": true,
      "is_range": false,
      "is_incomplete": false,
      "confidence": 0.97,
      "evidence_section_id": "{{section_id}}"
    }
  ]
}
```

### B.5 Operational extraction example

```json
{
  "operational_entities": [
    {
      "entity_type": "office_schedule",
      "entity_key_candidate": "pl_consulate_minsk_schedule_2026",
      "office_key_candidate": "pl_consulate_minsk",
      "country_code": "PL",
      "raw_mention": "Document intake: Mon-Fri 09:00-13:00",
      "params": {
        "weekdays": "mon-fri",
        "hours_open": "09:00",
        "hours_close": "13:00",
        "timezone": "Europe/Minsk"
      },
      "valid_from": null,
      "valid_until": null,
      "ttl_days": 90,
      "confidence": 0.92,
      "evidence_section_id": "{{section_id}}"
    }
  ]
}
```

### B.6 Editorial extraction example

```json
{
  "topics": [
    {
      "topic_type": "pain_point",
      "topic_key_candidate": "how_to_prove_income_self_employed",
      "human_label": "How can a self-employed applicant prove income for a Schengen visa?",
      "raw_mention": "Self-employed applicants provide a tax declaration",
      "links_to_procedure": true,
      "procedure_concept_candidate": "tax_declaration",
      "country_code": "PL",
      "visa_type": "tourist",
      "audience_hint": ["self_employed"],
      "intent_type": "informational",
      "confidence": 0.91,
      "evidence_section_id": "{{section_id}}"
    }
  ]
}
```

### B.7 Triple decomposition example

```text
Source statement:
Belarusian citizens applying via VFS Global pay 35 EUR, but children under 6 are exempt with notarized parental consent.

Decomposed outputs:
1. Fee rule for main applicant group
2. Fee exemption rule for child profile
3. Related document/condition rule for notarized parental consent
```

### B.8 Completeness judge example

```json
{
  "completeness_score": 0.88,
  "missing_elements": [
    {
      "loss_type": "exception",
      "raw_fragment": "children under 6 are exempt",
      "reason": "age-based exception was not captured",
      "action": "add_to_exceptions",
      "remediation_action": "targeted_patch",
      "target_triple_id": "blake3(...)"
    }
  ],
  "hallucinated_elements": [],
  "workflow_action": "pause_for_hitl",
  "unmapped_raw_text": "text not represented by any extracted object",
  "needs_hitl": true,
  "hitl_reason": "manual review required for unresolved alternative"
}
```

### B.9 SEO extraction example

```json
{
  "seo_entities": [
    {
      "entity_type": "keyword",
      "entity_key_candidate": "schengen_visa_minsk",
      "raw_mention": "виза в польшу минск",
      "country_code": "PL",
      "visa_type": "tourist",
      "intent_type": "transactional",
      "confidence": 0.95,
      "evidence_section_id": "{{section_id}}"
    },
    {
      "entity_type": "content_gap",
      "entity_key_candidate": "travel_insurance_guide_pl",
      "raw_mention": "страховка для шенгена",
      "competitor_hint": "present on 4 competitor sites",
      "missing_topic": "travel_insurance_selection",
      "country_code": "PL",
      "confidence": 0.82,
      "evidence_section_id": "{{section_id}}"
    }
  ]
}
```

### B.10 Commercial extraction example

```json
{
  "commercial_entities": [
    {
      "entity_type": "service",
      "entity_key_candidate": "document_package_preparation_pl",
      "raw_mention": "Подготовка пакета документов — 150 BYN",
      "service_description": "Full document package preparation for visa submission",
      "price_hint": "150",
      "currency_hint": "BYN",
      "audience_hint": ["general"],
      "country_code": "PL",
      "confidence": 0.97,
      "evidence_section_id": "{{section_id}}"
    },
    {
      "entity_type": "partner_offer",
      "entity_key_candidate": "partner_insurance_schengen",
      "raw_mention": "Страховка через партнёра",
      "service_description": "Schengen travel insurance via agency partner",
      "audience_hint": ["general"],
      "country_code": "PL",
      "confidence": 0.90,
      "evidence_section_id": "{{section_id}}"
    }
  ]
}
```

---

## Appendix C. Graph, Payload, And Query Reference

> This appendix is reference-only unless a specific snippet is explicitly elevated by a normative section. It contains implementation-oriented examples aligned to the canonical contracts.

### C.1 Neo4j merge reference

#### Section MERGE reference

```cypher
MERGE (s:Section {section_id: $section_id})
ON CREATE SET s.created_at = $now
SET
  s.source_url = $source_url,
  s.heading_text = $heading_text,
  s.position_on_page = $position_on_page,
  s.raw_text = $raw_text,
  s.content_hash = $content_hash,
  s.block_type = $block_type,
  s.updated_at = $now
```

#### Procedural MERGE reference

```cypher
MERGE (ri:RuleInstance {node_id: $node_id})
ON CREATE SET ri.created_at = $now
SET
  ri.semantic_rule_key = $semantic_rule_key,
  ri.evidence_binding_key = $evidence_binding_key,
  ri.role = $role,
  ri.layer = 'procedural',
  ri.status = $status,
  ri.confidence = $confidence,
  ri.layer_version = $layer_version,
  ri.updated_at = $now,
  ri.evidence_section_id = $evidence_section_id
```

#### Operational MERGE reference

```cypher
MERGE (o:Office {key: $office_key})
ON CREATE SET o.created_at = $now
SET o.layer = 'operational', o.updated_at = $now
MERGE (e:OperationalEntity {node_id: $node_id})
ON CREATE SET e.created_at = $now
SET
  e.entity_type = $entity_type,
  e.layer = 'operational',
  e.status = $status,
  e.confidence = $confidence,
  e.valid_from = $valid_from,
  e.valid_until = $valid_until,
  e.updated_at = $now
```

#### Editorial MERGE reference

```cypher
MERGE (t:Topic {key: $topic_key})
ON CREATE SET t.created_at = $now
SET
  t.human_label = $human_label,
  t.topic_type = $topic_type,
  t.layer = 'editorial',
  t.status = $status,
  t.confidence = $confidence,
  t.updated_at = $now
```

### C.2 Voyage payload reference

```json
{
  "id": "blake3(triple_id)",
  "vector": ["...embedding..."],
  "payload": {
    "triple_id": "abc123",
    "semantic_rule_key": "srk_abc123",
    "text_for_search": "FEE_ITEM consular_fee: amount=35 EUR, mandatory, Poland tourist visa, target citizenship BY",
    "visa_key": "pl_tourist_by",
    "country_code": "PL",
    "visa_type": "tourist",
    "target_citizenship": "BY",
    "role": "FEE_ITEM",
    "concept_key": "consular_fee",
    "layer": "procedural",
    "source_tier": "government",
    "confidence": 0.97
  }
}
```

**Reference note**

Business context fields in retrieval payloads come from deterministic job context plus deterministic enrichment, not from LLM generation.

### C.3 Qdrant/canonical retrieval reference

```text
Resolution order:
1. exact alias
2. normalized alias
3. regex/rule match
4. vector similarity (Qdrant mandatory fallback)

Typical threshold policy from V1 full protocol:
- >= 0.88 -> auto_map
- 0.75-0.87 -> review
- < 0.75 -> new_candidate / HITL
```

### C.4 GDS examples

**Reference only**

Example use cases from V1:

- WCC for clustering themes and keys,
- PageRank for page weighting,
- NodeSimilarity for topic-concept similarity,
- shortest path across cross-layer graph bridges.

These examples are implementation illustrations, not protocol-defining semantics.

---

## Appendix D. Migration Notes

### D.1 Reconciliation status

Resolved in this V2 document:

- numbers by layer rule,
- binding matrix conflicts including `ELIGIBILITY_RULE`, `WHERE_TO_APPLY`, `STEP`,
- verified storage vs `L2/L4` semantics,
- mandatory Qdrant fallback in canonical mapping,
- semantic identity vs evidence identity (`semantic_rule_key` and `evidence_binding_key`),
- schema parity for deterministic join anchors.

### D.2 Migration completion rule

This V2 file can replace V1 as the canonical protocol only when:

1. every unresolved TODO is resolved,
2. every normative V1 rule has a unique home in V2,
3. duplicated normative wording is removed,
4. schema parity is complete,
5. appendices contain the moved examples and implementation references,
6. no rollout section changes a core contract.

---

## Appendix E. Constants & Heuristics

This appendix is the single canonical home for all numeric constants, heuristic weights, and enumerated flags used by the pipeline. Do not duplicate these values in prose sections — reference this appendix instead.

---

### E.1 Token Budget (per section, baseline)

These are orientation estimates, not hard limits. Complex pages may exceed them whenever completeness requires it.

| Agent / Step | System prompt | Input (section) | Output | Total |
|---|---|---|---|---|
| #1 Layer Router | ~200 | ~400 | ~100 | **~700** |
| #2 Entity Detection | ~300 | ~400 | ~200 | **~900** |
| Step 3 Canonical Mapping (symbolic-first, deterministic) | 0 | deterministic | deterministic | **~0** |
| #3A Procedural Extractor | ~500 | ~400 | ~400 | **~1300** |
| Step 5 Triple Builder (Rust, deterministic) | 0 | deterministic | deterministic | **~0** |
| #5 Completeness Judge | ~300 | ~800 (text + triples) | ~200 | **~1300** |
| **Baseline total per section** | | | | **~4200** |

---

### E.2 Operational Lifecycle TTL Matrix

Canonical reference for all timeline/calendar-type data. Used by Step 1 (Layer Router) to classify `lifecycle_class` and by Step 6 (Operational Extractor) to set `ttl` and `refresh_policy`.

| lifecycle_class | ttl | refresh_policy |
|---|---|---|
| `recurring_schedule` | 90d | periodic |
| `temporary_override` | fixed | event-based |
| `one_off_closure` | fixed | event-based |
| `blackout_window` | fixed | event-based |
| `country_holiday` | yearly | yearly |
| `office_holiday_adoption` | yearly | yearly |
| `notice` | 7d | frequent |

---

### E.3 Confidence & Freshness Decay

Effective confidence is computed once per verification cycle and stored in the object's `effective_confidence` field. It is never computed at query time.

```
effective_confidence = base_confidence * freshness_factor * source_factor
```

Where `source_factor` is derived from the source's `authority_level` using the weights in E.4.

---

### E.4 Source Priority Weights

Used to derive `source_factor` in the confidence formula (E.3). Also informs `authority_level` assignment in Sec 3.8.2.

```text
government       = 1.0
official_partner = 0.9
agency           = 0.7
forum            = 0.4
```

---

### E.5 Derivation Type

Mandatory field on every extracted object. Controls downstream merge and conflict-resolution logic.

```json
{
  "derivation_type": "direct | inferred | aggregated"
}
```

| Value | Meaning |
|---|---|
| `direct` | Extracted verbatim or near-verbatim from source text |
| `inferred` | Derived by logical inference from one or more source statements |
| `aggregated` | Synthesized from multiple source fragments (requires evidence list) |

---

### E.6 Extraction Mode

Set per pipeline run (not per object). Controls Procedural Extractor thresholds.

```json
{
  "mode": "strict | recall"
}
```

| Value | Behaviour |
|---|---|
| `strict` | Only high-confidence extractions are emitted; precision over recall |
| `recall` | Maximum completeness; all candidate extractions emitted for downstream validation |

Default: `strict` for production runs, `recall` for initial onboarding of new document sources.

---

### E.7 Meta-Ontology Classes

Four top-level meta-registry classes that make the pipeline domain-independent. Each class governs one category of ontology entries.

| Meta-class | Governs | Examples |
|---|---|---|
| `entity_class` | Node types and their allowed fields | `Person`, `FeeItem`, `VisaType` |
| `relation_class` | Edge types, direction, and cardinality | `requires`, `supersedes`, `applies_to` |
| `layer_class` | Pipeline layer definitions and L0–L4 lifecycle rules | `L0_incomplete`, `L3_retrieval_ready` |
| `evidence_class` | Evidence record types and binding requirements | `source_citation`, `binding_anchor` |

The meta-ontology registry lives under `ontology/meta/` and is governed by the Class C change process (Sec 10.4).

---

## Appendix F. Classification Guide

**Normative status:** normative reference for HITL arbitration and human classification. This appendix is the single canonical home for the classification algorithm (mirrored from Sec 3.1.1) and all layer-specific decision aids.

---

### F.1 Classification algorithm (full reference)

Apply to one atomic statement at a time. Questions are applied strictly in order. First `YES` terminates the tree.

```
Q1  If this information is wrong or outdated — will it block the publish gate
    of a visa page?
    YES → PROCEDURAL CORE

Q2  Does this information describe the operating mode or availability of an
    external entity (office, VFS, consulate) AND does it have a concrete
    expiry (valid_until / TTL)?
    YES → OPERATIONAL LAYER

Q3  Is this information needed to write a useful article, FAQ, checklist,
    travel guide, or to connect pages by meaning?
    YES → EDITORIAL / TRAVEL LAYER

Q4  Is this a search query, demand cluster, site page, or link structure
    between pages?
    YES → SEO GRAPH

Q5  Is this an agency service, pricing tier, commercial offer, or CTA?
    YES → COMMERCIAL LAYER

    NO  → Classification error: return to Q1 and re-examine the statement
```

**Quick membership test per layer:**

| Signal | → Layer | Key question |
|---|---|---|
| Participates in eligibility gate / required_keys | PROCEDURAL | Does it block publication? |
| Specific number from a gov source | PROCEDURAL | Is `evidence_section_id` present? |
| Operating mode + specific date / TTL | OPERATIONAL | Does it lose meaning after a date? |
| Closure / holiday / notice | OPERATIONAL | External entity + time-bounded? |
| Topic for an article or FAQ | EDITORIAL | Does it help write content? |
| User pain / risk / scenario | EDITORIAL | Is the user worried about this? |
| Search query / cluster | SEO | Unit of demand? |
| Page / link / PageRank | SEO | Site structure? |
| Agency service / tier / CTA | COMMERCIAL | Does the agency sell this? |

---

### F.2 PROCEDURAL CORE — classification guide

**Belongs here:**

- Requirement to provide a specific document (passport, insurance, photo)
- Numeric admission condition (passport valid 3 months after trip, min. 30 000 EUR insurance)
- Monetary fee amount (consular 35 EUR, VFS fee 26.5 EUR)
- Processing time, visa validity, submission window (10–15 business days)
- Submission location and channel (VFS Global, BLS, direct to consulate)
- Appointment requirement and booking method
- Process steps (biometrics, interview, passport surrender)
- Conditions for a specific applicant profile (children, self-employed, students)

**Does NOT belong here:**

- How to choose insurance — that is an Editorial Topic
- VFS opening hours — that is Operational (office schedule with TTL)
- Document preparation tips — that is Editorial (PreparationTask)
- Agency service price — that is Commercial

**Membership test:** Can a CHECK constraint be written in PostgreSQL for this value? → Procedural. Cannot → another layer.

---

### F.3 OPERATIONAL LAYER — classification guide

**Belongs here:**

- Consulate / VFS / embassy opening hours (Mon–Fri 09:00–17:00)
- Public holidays of the country when the office is closed
- Temporary closures (maintenance, renovation, relocation)
- Operating-mode change notices (suspension of submissions, urgent-only intake)
- Seasonal submission restriction dates
- Changes to office address, phone, email

**Why NOT in Procedural Core:**

- Operational data has TTL (opening hours change, holidays recur annually)
- Putting it in Procedural would cause the Publish Gate to block visa pages due to stale schedules
- Not eligible for CHECK constraint — no fixed enum
- Source is different: office websites, Google Maps, social media — not gov registries

**Membership test:** Does this information lose meaning after a specific date or season change? AND is it the operating mode of an external entity, not a requirement on the applicant? → Operational.

---

### F.4 EDITORIAL / TRAVEL LAYER — classification guide

**Belongs here:**

- Article or FAQ topic (how to prove income, what is a sponsorship letter)
- User pain point (no travel history, don't know how to prove finances)
- Refusal risk (weak application, suspicious itinerary, tourism as work pretext)
- Preparation task (book a hotel, gather 3-month bank statements)
- Travel topic (food in Italy, routes through Spain, Warsaw neighbourhoods)
- Audience need (visa for self-employed, trip with child, pensioner)
- Seasonality (best time to travel, avoid August)
- Refusal scenario (refused twice, what to do next)

**Critical invariant — Editorial does NOT carry numbers:**
If an Editorial Topic mentions a number, that number must come from Procedural Core via a `LINKS_TO_PROCEDURE` edge. Editorial nodes never store numeric fact values directly.

**Membership test:** Does it help write a useful article or answer a user question — but does not itself block or permit a visa application? → Editorial.

---

### F.5 SEO GRAPH — classification guide

**Belongs here:**

- Keyword / search query (poland visa minsk, schengen for belarusians)
- Keyword cluster (WCC cluster from Neo4j GDS)
- Site page as a node (Page with url, page_type, cluster_id)
- PageRank and link budget of a page
- Internal link relationships between pages
- Link anchors (link texts derived from keyword cluster)
- Content gap — topic present at competitors, page absent on our site

**Membership test:** Is this a unit of search demand (query, cluster) or a structural unit of the site (page, link, weight)? → SEO Graph.

---

### F.6 COMMERCIAL LAYER — classification guide

**Belongs here:**

- Agency service (document package preparation, appointment booking)
- Pricing plan (basic, premium, urgent)
- Audience segment to which the service is sold
- CTA (consultation button, application form)
- Partner offers (insurance via partner, visa photo)

**Critical boundary with Procedural:**
The determining criterion is **who receives the money and what is the source**. If money goes to the consulate or VFS and the fact comes from a government source → Procedural `FEE_ITEM`. If money goes to the agency and the fact is business data → Commercial `service`.

**Membership test:** Is this what the agency sells or offers — not what the consulate requires? → Commercial.

---

### F.7 Border cases — classification table

Objects that are commonly misclassified. Each row explains the correct layer and the reason.

| Object | Correct layer | Incorrect assumption | Why correct |
|---|---|---|---|
| VFS opening hours Mon–Fri | OPERATIONAL `office_schedule` | Procedural (WHERE_TO_APPLY attribute) | Has TTL, not eligibility condition; separate node |
| Poland public holidays | OPERATIONAL `country_holiday` | Editorial TravelTopic | If these are office closures — Operational. Travel article "when to go" → Editorial |
| VFS fee 26.5 EUR | PROCEDURAL `FEE_ITEM` | Commercial (agency collects it) | Money goes to VFS; fact from gov source |
| "Submission only via VFS, not directly" | PROCEDURAL `WHERE_TO_APPLY` | OPERATIONAL (VFS is an external entity) | Submission channel participates in required_keys; blocks publish if missing |
| "How to choose insurance for Schengen" | EDITORIAL `topic` | New procedural role INSURANCE_TOPIC | 8 procedural roles are frozen; this is article content |
| "Hotel booking for visa" | PROCEDURAL + EDITORIAL | Either/or | DOCUMENT_REQUIRED in Procedural; `preparation_task` in Editorial — both nodes, linked |
| "Travel insurance min. 30 000 EUR" | PROCEDURAL `ELIGIBILITY_RULE` | EDITORIAL (mentioned in article) | Numeric gov requirement; number lives only in Procedural |
| "Insurance via partner" | COMMERCIAL `partner_offer` | PROCEDURAL (it's insurance) | Agency's business offer, not a consulate requirement |
| "Agency consultation service" | COMMERCIAL `service` | EDITORIAL (useful for user) | Agency sells it; not a visa fact |
| "Visa refusal — what to do next" | EDITORIAL `refusal_scenario` | PROCEDURAL (affects visa outcome) | Not a rule; it is a user scenario for article/FAQ |
| "Schengen visa Minsk" | SEO `keyword` | EDITORIAL or PROCEDURAL | Unit of search demand; not a topic or visa rule |
| "Urgent processing +80 BYN" | COMMERCIAL `service_tier` | PROCEDURAL FEE_ITEM | Agency surcharge, not consular fee |
| "Schengen insurance min 30 000 EUR" in an article | PROCEDURAL (linked from Editorial) | EDITORIAL stores the number | Number stays in Procedural; Editorial topic uses `LINKS_TO_PROCEDURE` |
| Office address change | OPERATIONAL | PROCEDURAL WHERE_TO_APPLY | Address has TTL and is an office attribute, not an eligibility condition |
| "Page /poland/tourist/" | SEO `page_node` | EDITORIAL or PROCEDURAL | Structural unit of the site, not a topic or rule |
