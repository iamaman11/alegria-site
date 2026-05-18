# V6 SeoSiteBuildWorkflow Working Plan

**Status:** active versioned working plan
**Class:** `current-execution-satellite`
**Parent owner document:** [V6_Expert_Truth_Graph_Runtime.md](V6_Expert_Truth_Graph_Runtime.md)
**Purpose:** detailed implementation plan for evolving `SeoSiteBuildWorkflow` into the single active orchestration flow for Truth, Graph, and Retrieval planes.
**Editing rule:** this file is intentionally versioned and updated during execution.
**Current version:** `6.3`

---

## 1. Mission

`SeoSiteBuildWorkflow` must become the single active orchestration path that:

- keeps Truth Plane authoritative,
- restores the full rich semantic/graph/retrieval flow on top of truth-core,
- preserves clear separation between Truth, Graph, Retrieval, and Serving authority,
- classifies every relevant source fragment before extraction,
- supports real supersite planning and drafting without reintroducing ambiguous parallel authority paths.

This file is the working execution plan for that transition.

V6.3 restores the early extraction gates, `Completeness Judge`, and `Resolution Loop` from the V5 expert protocol as mandatory target runtime stages. These stages are a detailed expansion of the V6 target expert flow. They do not create a second workflow and they do not weaken the V6 rule that verified truth is produced only by the Truth Plane.

### 1.1 Expert-first implementation rule

The priority is expert correctness before orchestration completeness.

The system must not wait for all 56 target workflow steps to become first-class Temporal phases before it becomes expert-grade. The first executable slice may run as a Rust use-case, CLI/batch job, or existing workflow macro-step, as long as it enforces the expert contracts:

- early evidence gates before extraction;
- entity spans and canonical mapping before procedural extraction;
- strict procedural ontology for visa rules;
- deterministic candidate validation;
- mandatory completeness judging;
- mandatory resolution of loss, hallucination, ambiguity, and blocked mappings;
- deterministic adjudication before verified truth;
- truth admissibility before drafting;
- draft QA before publish.

This is not an MVP relaxation. It is the expert core extracted from the full target workflow. Orchestration may be added later; expert safety gates may not be postponed.

---

## 2. Current Runtime Shape

### 2.1 What is active today

Current active flow:

1. `load_verified_support_bundle`
2. `serp_ingest`
3. `crawl_sources`
4. `raw_knowledge_ingestion`
5. optional `load_verified_support_bundle.refresh`
6. `serp_normalize`
7. `opportunity_build`
8. `ia_build`
9. `link_recommend`
10. `global_site_reconcile`
11. per-page:
    - `draft_assemble`
    - `editorial_draft_generate`
    - `draft_normalize`
    - `content_contract_validate`
    - `draft_qa`
    - `cms_request_review`
    - `human_approval_wait`
    - `cms_publish_approved`
    - `publish_materialize`
    - `render_preview_validate`
    - `finalize_publish`
12. final:
    - `rebuild_detect`

Within the current `raw_knowledge_ingestion` macro-step:

1. truth-extraction LLM receives one `raw.section`
2. candidate JSON is parsed
3. deterministic validator assigns:
   - `structured`
   - `needs_hitl`
   - `rejected`
4. deterministic adjudicator decides:
   - `verified`
   - `needs_hitl`
   - `rejected`
5. admissible truth is written to `verified.rule_instances`

Current active guards already present:

- `truth_admissibility_gate` is enforced through `load_verified_support_bundle` and again before `draft_assemble`
- projection barrier checks already exist after `raw_knowledge_ingestion`
- projection barrier checks already exist after `global_site_reconcile`

Current runtime branching semantics already present in code:

- `run_mode` is normalized before execution plan construction
- `scenario` and `policy` decide whether:
  - planning phases run at all
  - `load_verified_support_bundle.refresh` is inserted
  - publish phases are included
  - `rebuild_detect` is included
- `crawl_only` stops before planning/drafting/publish
- `draft_only` and `dry_run` persist without publish
- publish path runs only when publish policy and scenario both allow it
- workflow may terminate early with `done:no_pages`

### 2.2 What is not yet active as mandatory runtime

- `whole_page_semantic_pass`
- `page_utility_classifier`
- `dom_block_relevance_filter`
- `sectioning_contract_gate`
- `cas_gate`
- `layer_router`
- `subspan_layer_router`
- `entity_span_detection`
- `canonical_mapping`
- rich layer-specific extraction for procedural, operational, editorial, SEO, and commercial objects
- mandatory `triple_builder`
- mandatory `completeness_judge`
- mandatory `resolution_loop`
- mandatory `contradiction_gate` as a first-class rich-flow gate
- mandatory `graph_admissibility_gate`
- mandatory `retrieval_admissibility_gate`
- first-class `neo4j_sync` and `voyage_qdrant_sync` workflow phases
- full evidence-grade entity/relation graph extraction

### 2.3 Current blocker

Current live blocker:

- no configured Gemini truth extraction provider

Canonical live envs:

- `GEMINI_API_KEY`
- `GOOGLE_API_KEY`

### 2.4 Important clarifications

- `seo_preflight` is a pre-run outer gate and operator entrypoint, not a current `SeoPhaseKey`.
- `neo4j_sync` and `voyage_qdrant_sync` already have partial outbox-backed surfaces, but are not current first-class synchronous workflow phases.
- current truth-core `validator` and `adjudication` are already active inside `raw_knowledge_ingestion`.
- deferred reactivation below refers to the richer semantic stages around early gates, router, entity spans, canonical mapping, triple building, completeness, and resolution. It does not mean removing the active truth-core validator/adjudicator.
- the current `raw_knowledge_ingestion` is a migration macro-step. In the V6.3 target it must be decomposed into explicit evidence preparation, rich extraction, validation, completeness, resolution, and truth-adjudication stages.

---

## 3. V6.3 Non-Negotiable 10/10 Invariants

These invariants define the target reliability bar. Any implementation that violates them is not considered the expert architecture, even if it passes a happy-path smoke.

### 3.1 Authority invariants

- LLM output is never verified truth.
- Graph output is never verified truth.
- Retrieval output is never verified truth.
- Page drafting output is never verified truth.
- Verified truth is created only through candidate extraction, deterministic validation, completeness/resolution handling where required, contradiction handling, and adjudication.
- `verified.rule_instances` remains the authoritative procedural truth store for drafting and publish.

### 3.2 Classification invariants

- Every extractable atomic statement must receive a layer disposition before layer-specific extraction:
  - `procedural`
  - `operational`
  - `editorial`
  - `seo`
  - `commercial`
  - `ignored_with_reason`
  - `blocked_by_gate`
- Visa rules and procedures use the strict procedural ontology.
- Non-visa and non-procedural information uses the evolutionary ontology workflow.
- Multi-layer statements must be decomposed; one layer must not swallow the others.
- Numbers that define visa requirements, fees, timelines, windows, or eligibility belong to procedural truth only.
- Editorial, SEO, commercial, graph, or retrieval layers may reference procedural concepts, but must not duplicate numeric visa truth as their own authority.

### 3.3 Evidence and join invariants

- Every truth-bearing candidate must carry source, section, exact evidence quote, `span_start`, `span_end`, and source snapshot hash.
- All required joins must use `mention_id` or exact source span offsets.
- Fuzzy string joins are forbidden for truth, graph-safe construction, and canonical binding.
- A missing required canonical binding is a blocking structural failure, not a warning.
- A null canonical key for a binding-required procedural object cannot reach verified truth or graph sync.

### 3.4 Completeness and resolution invariants

- `Completeness Judge` is mandatory before procedural truth promotion in the rich target flow.
- The judge must inspect missing numbers, conditions, exceptions, modality, alternatives, profiles, unmapped fragments, and hallucinated elements.
- If the judge finds unresolved loss or hallucination, the workflow must choose one of:
  - `targeted_patch`
  - `rerun_extractor`
  - `pause_for_hitl`
  - `drop_with_reason`
- Findings may not be logged and ignored.
- `Resolution Loop` ends only when the object is resolved, dropped, or formally paused with a HITL task.

### 3.5 Runtime verifiability invariants

- Every first-class step must persist a step execution record with input hash, output hash, idempotency key, schema versions, retry class, executor build id, and final status.
- Every side effect must go through a canonical store or outbox.
- Every phase exit must be proven by runtime evidence, not only by static documentation.
- `pending_hitl` is not failure and must be resumable.
- `poisoned` and `dead_letter` states must preserve forensic payloads.
- Replay must be possible from immutable inputs, payload blobs, version tuple, and attempt trail.

### 3.6 Projection and retrieval invariants

- `Neo4j` receives graph-safe projection payloads only.
- `Qdrant` receives retrieval-safe payloads only.
- `Voyage` or any embedding provider produces vectors only; embedding similarity is not truth confidence.
- Retrieval artifacts may become stale; stale retrieval must be excluded from auto-map and high-confidence serving until reindexed.
- Projection barriers must be run-scoped and must not silently ignore failed or open projection events when strict projection policy is active.

---

## 4. Target Workflow Shape

`SeoSiteBuildWorkflow` should evolve to this single ordered flow.

The target flow below is more detailed than the V6 owner-document summary. It expands the owner-document's target expert stages into executable gates and does not introduce a competing workflow.

### 4.1 Ordered target flow

1. `seo_preflight`
2. `load_verified_support_bundle.initial`
3. `serp_ingest`
4. `crawl_sources`
5. `whole_page_semantic_pass`
6. `page_utility_classifier`
7. `dom_block_relevance_filter`
8. `sectioning`
9. `sectioning_contract_gate`
10. `cas_gate`
11. `raw_evidence_register`
12. `projection_barrier(raw_evidence)`
13. `layer_router`
14. `subspan_layer_router`
15. `entity_span_detection`
16. `canonical_mapping`
17. `ontology_intake_gate`
18. `procedural_extraction`
19. `operational_extraction`
20. `editorial_extraction`
21. `seo_signal_extraction`
22. `commercial_signal_extraction`
23. `extraction_schema_validate`
24. `candidate_validation`
25. `triple_builder`
26. `completeness_judge`
27. `resolution_loop`
28. `contradiction_gate`
29. `truth_adjudication`
30. `verified_truth_write`
31. `load_verified_support_bundle.refresh`
32. `graph_admissibility_gate`
33. `retrieval_admissibility_gate`
34. `neo4j_sync`
35. `voyage_qdrant_sync`
36. `projection_barrier(semantic_projection)`
37. `serp_normalize`
38. `opportunity_build`
39. `ia_build`
40. `link_recommend`
41. `global_site_reconcile`
42. `projection_barrier(global_site_reconcile)`
43. `truth_admissibility_gate`
44. `draft_assemble`
45. `editorial_draft_generate`
46. `draft_normalize`
47. `content_contract_validate`
48. `draft_qa`
49. `cms_request_review`
50. `human_approval_wait`
51. `cms_publish_approved`
52. `publish_materialize`
53. `render_preview_validate`
54. `finalize_publish`
55. `projection_barrier(publish)`
56. `rebuild_detect`

This is one workflow, not three separate workflows.

The three planes live inside one `run_id` with different authority rules:

- Truth Plane decides verified truth.
- Graph Plane assembles and projects graph-safe semantics.
- Retrieval Plane indexes and recalls context.
- Serving Plane publishes only after truth, QA, HITL, and publish gates.

### 4.2 Macro-step migration rule

During migration, the existing `raw_knowledge_ingestion` step may continue to run the current candidate-only truth-core path.

The target V6.3 design must not keep both:

- current `raw_knowledge_ingestion` doing hidden extraction/adjudication, and
- explicit rich extraction/adjudication stages doing the same work later.

The migration must split the macro-step into explicit stages:

- raw evidence preparation,
- layer and entity classification,
- canonical mapping,
- layer-specific extraction,
- schema and candidate validation,
- triple building,
- completeness judging,
- resolution,
- contradiction handling,
- truth adjudication,
- verified write.

### 4.3 Branching rule

Run-mode, scenario, and policy branching may remove later segments, but must not bypass required upstream gates for any segment that remains present.

Examples:

- `crawl_only` may stop before planning, drafting, and publish, but if extraction is enabled it must still obey evidence, validation, and HITL gates.
- `draft_only` may remove publish phases, but it must not remove `truth_admissibility_gate`.
- `dry_run` may use warning projection policy, but must still expose projection status and step outcomes.
- `publish_with_hitl` must pause at human approval where required.
- `full_auto_after_approval` must require a pre-approved decision and must not invent approval.

---

## 5. Step Contract Ledger

Each target step must have:

- a typed input contract,
- a typed output contract,
- a persisted step execution record,
- deterministic idempotency inputs,
- a retry class,
- a clear blocking status on failure.

### 5.1 Pre-run, source discovery, and raw evidence

| Step | Plane | Purpose | Exit contract |
|---|---|---|---|
| `seo_preflight` | runtime | Validate credentials, dependency probes, projection backlog, scope identity, truth identity, output paths, and run policy before durable execution. | Clear go/no-go report. Strict mode blocks missing required dependencies or unsafe projection backlog. |
| `load_verified_support_bundle.initial` | truth | Load existing admissible verified truth for the current context and profile before deciding whether the run can draft immediately or must crawl/extract. | Support bundle persisted as runtime artifact. Empty bundle is allowed before crawl but blocks drafting later. |
| `serp_ingest` | planning/raw | Ingest real or configured SERP candidates and source candidates. | Query batch, source observations, rank/title/snippet/domain/source metadata persisted. Low-trust sources remain signals only. |
| `crawl_sources` | raw | Fetch pages, persist raw page snapshots, redirect/canonical metadata, crawl provenance, and raw fetch diagnostics. | `raw.pages` exists with immutable source snapshot hashes. Failed crawls are recorded without poisoning the run. |
| `whole_page_semantic_pass` | raw/semantic | Understand each page as a whole before section-level extraction. This step does not extract facts. | Page mode, dominant layers, page summary, page context profile, global entities, mixed-section hints. |
| `page_utility_classifier` | raw/semantic gate | Decide whether procedural, operational, editorial, SEO, commercial, or structural extraction is allowed for the page. | Hard allow/deny flags. Deny flags override router output. |
| `dom_block_relevance_filter` | raw gate | Remove or mark navigation, footer, header, cookie, sidebar, promo, and directory blocks before sectioning. | Only extraction-eligible blocks enter sectioning. Denied blocks are auditable with reason. |
| `sectioning` | raw | Create stable sections from headings, tables, lists, FAQ units, and fallback page sections. | `raw.sections` with stable section ids, block type, position, source snapshot binding, content hash, and source metadata. |
| `sectioning_contract_gate` | raw gate | Validate section quality before extraction. | Blocks empty, duplicate, unstable, over-large, under-anchored, or boilerplate-only sections unless explicitly accepted with reason. |
| `cas_gate` | raw gate | Content-addressable storage and duplicate-content gate. | Stable content hashes, idempotent rerun behavior, and skip decisions for unchanged or duplicate sections. |
| `raw_evidence_register` | raw/truth support | Bind raw sections to context candidates and source records without promoting truth. | Source registration, section-context candidates, and evidence-ready raw sections. No verified truth write. |
| `projection_barrier(raw_evidence)` | retrieval/runtime | Observe raw evidence projection status, especially raw-section Qdrant outbox where enabled. | Run-scoped barrier report. Strict policy blocks open or failed required projection events. |

### 5.2 Classification, mapping, and ontology intake

| Step | Plane | Purpose | Exit contract |
|---|---|---|---|
| `layer_router` | semantic | Assign full independent score vector across procedural, operational, editorial, SEO, and commercial layers for every extraction-eligible section. | Full score vector, primary layer, secondary layers, confidence, HITL flags. Returning only one label is invalid. |
| `subspan_layer_router` | semantic | Split mixed sections into sentences, list items, table rows, or FAQ units when multiple layers compete. | Subspans inherit section evidence and receive their own layer scores. Mandatory secondary layers are not ignored. |
| `entity_span_detection` | semantic | Detect entity mentions as evidence-bearing spans without extracting rules. | Stable `mention_id`, raw text, entity type, char offsets, numeric flag, centrality, confidence. |
| `canonical_mapping` | semantic/truth support | Map mentions to canonical registry keys through exact alias, normalized alias, rule/regex, then vector fallback. | Mapping output references `mention_id`, original span, target registry, canonical key or null, match method, confidence, HITL need. |
| `ontology_intake_gate` | ontology | Route unmapped or ambiguous mentions into evolutionary ontology intake. | New keys may become `detected` or `proposed`; they cannot participate in extraction until `indexed`. Ambiguous aliases block auto-map. |

### 5.3 Layer-specific extraction

| Step | Plane | Purpose | Exit contract |
|---|---|---|---|
| `procedural_extraction` | truth candidate | Extract visa/procedure rule candidates using the strict eight-role ontology. | Candidate roles limited to `DOCUMENT_REQUIRED`, `ELIGIBILITY_RULE`, `FEE_ITEM`, `TIMELINE_ITEM`, `WHERE_TO_APPLY`, `APPOINTMENT_RULE`, `FORM_REQUIRED`, `STEP`; exact evidence and deterministic anchors required. |
| `operational_extraction` | semantic/operational | Extract office schedules, notices, closures, holidays, blackout windows, and temporary operational state. | Validity semantics, TTL, source evidence, and entity type from fixed operational registry. No procedural requirement creation. |
| `editorial_extraction` | semantic/editorial | Extract topics, pain points, risks, preparation tasks, audience needs, refusal scenarios, destination topics, and seasonality. | Editorial objects may link to procedural concepts but must not carry procedural numeric truth. |
| `seo_signal_extraction` | product/retrieval | Extract or normalize search demand, page, cluster, content-gap, link, pattern, and structural site signals. | Product-layer artifacts only. No truth creation. Competitor-derived facts remain signals. |
| `commercial_signal_extraction` | product/serving | Extract agency services, offers, CTAs, partner offers, service tiers, and business-owned claims. | Commercial objects are governed by business/CMS workflow. They cannot become procedural requirements or consular fees. |

### 5.4 Validation, triple building, completeness, and resolution

| Step | Plane | Purpose | Exit contract |
|---|---|---|---|
| `extraction_schema_validate` | runtime gate | Validate every extractor output against schema, producer/consumer step, version, required fields, and failure class. | Invalid output blocks downstream. No verified write, graph sync, or indexing from invalid output. |
| `candidate_validation` | truth gate | Deterministically validate procedural candidates for role, concept key, params, evidence span integrity, freshness, completeness, modality, range, and ambiguity. | Candidate status becomes `structured`, `needs_hitl`, or `rejected`. Validator cannot assign `verified`. |
| `triple_builder` | semantic/graph support | Build graph-oriented objects and atomic semantic triples from canonical mappings and validated extraction outputs. | Deterministic joins only. Missing anchors reroute to resolution or HITL. No fuzzy matching. No truth verdict. |
| `completeness_judge` | truth gate | Compare source text, extracted outputs, unmapped mentions, and triples for semantic loss and hallucination. | Must check numbers, conditions, exceptions, modality, alternatives, profiles, unmapped fragments, and hallucinated elements. Findings require workflow action. |
| `resolution_loop` | truth/runtime gate | Resolve incompleteness, hallucination, ambiguity, blocked mapping, or structural loss before truth promotion. | Ends only with resolved output, `rerun_pending`, `paused_for_hitl`, or `dropped`. Missing findings may not be ignored. |
| `contradiction_gate` | truth gate | Detect conflicting candidates or verified rules for fees, timelines, requirements, locations, eligibility, appointment rules, and forms. | Unresolved contradiction blocks publish and graph sync for dependent objects, and routes to HITL or explicit resolution. |
| `truth_adjudication` | truth | Group structured and resolved candidates by semantic identity, corroboration, source signals, contradiction state, freshness, and completeness. | Decisions are `verified`, `needs_hitl`, or `rejected`. Source tier is signal only, never shortcut authority. |
| `verified_truth_write` | truth | Materialize verified procedural truth into `verified.rule_instances` with evidence, source, adjudication reason, admissibility, freshness, completeness, and version metadata. | Only admissible verified truth can support drafting. Demotions are explicit and auditable. |
| `load_verified_support_bundle.refresh` | truth | Reload verified support after truth changes and profile applicability filtering. | Runtime support bundle reflects current admissible verified truth for the run context and applicant profile. |

### 5.5 Graph and retrieval activation

| Step | Plane | Purpose | Exit contract |
|---|---|---|---|
| `graph_admissibility_gate` | graph gate | Validate graph-safe shape before projection. | Blocks null required canonical keys, missing visa/profile bindings, unresolved contradictions, missing evidence section, unsupported relation type, and fuzzy joins. |
| `retrieval_admissibility_gate` | retrieval gate | Validate indexing-safe payloads before Voyage/Qdrant sync. | Blocks missing deterministic context, stale ontology version, missing collection contract, missing payload schema, or unsupported source policy. |
| `neo4j_sync` | graph projection | Project graph-safe objects and relations from canonical stores/outbox into Neo4j. | Neo4j remains projection only. Writes are idempotent, keyed by stable identities, and run-auditable. |
| `voyage_qdrant_sync` | retrieval projection | Generate embeddings and index approved retrieval surfaces. | Raw chunks, canonical registry, verified rules, editorial topics, and SEO support collections are indexed under declared collection contracts. |
| `projection_barrier(semantic_projection)` | runtime | Ensure graph and retrieval projections required for planning are current enough for the run policy. | Strict policy blocks failed/open required projection events. Warn policy records lag. |

### 5.6 Planning, drafting, publish, and rebuild

| Step | Plane | Purpose | Exit contract |
|---|---|---|---|
| `serp_normalize` | planning | Normalize SERP observations into reusable search patterns and source evidence. | Patterns and opportunities are source-backed and scoped. |
| `opportunity_build` | planning | Build opportunity candidates from SERP, clusters, gaps, verified truth coverage, and product policy. | Opportunities carry evidence, scope, intent, confidence, and blocking reasons. |
| `ia_build` | planning | Build page nodes, page blueprints, hierarchy, and page ownership candidates. | Page identities are deterministic and scope-bound. No page is created from unsupported truth. |
| `link_recommend` | planning/graph | Recommend internal links from page nodes, graph relationships, retrieval support, and IA policy. | Link recommendations are deterministic, scoped, and do not override truth or cannibalization policy. |
| `global_site_reconcile` | planning/serving | Reconcile current scope into global navigation, hubs, directories, sitemap intent, and rebuild plan. | Navigation uses active source-backed pages and does not invent final menus before IA evidence exists. |
| `projection_barrier(global_site_reconcile)` | runtime | Ensure planning/global reconcile projections are materialized or explicitly recorded as pending according to policy. | Strict policy blocks open/failed required projection events. |
| `truth_admissibility_gate` | truth gate | Block drafting unless current page has admissible verified support after profile applicability filtering. | Empty or inadmissible support blocks `draft_assemble`. |
| `draft_assemble` | serving preparation | Assemble draft plan from verified support, blueprint, section templates, graph/retrieval context, and required links. | LLM request is constrained by verified support. Supplemental retrieval is marked non-fact support unless verified. |
| `editorial_draft_generate` | serving | Generate draft candidate under JSON/content contract. | LLM may write prose only. It may not create unsupported facts. |
| `draft_normalize` | serving | Normalize draft candidate into canonical draft blocks, claim ledger, support refs, metadata, and internal links. | Unsupported or malformed claims remain visible for QA and cannot be hidden. |
| `content_contract_validate` | serving gate | Validate page-type contract, required sections, metadata obligations, links, traceability labels, and schema readiness. | Contract failure blocks publish path. |
| `draft_qa` | serving gate | Validate factual support, claim coverage, unsupported numbers/dates/prices, duplicate risk, links, readability, and schema coverage. | Unsupported factual claims block publish and may route to HITL. |
| `cms_request_review` | publish control | Create review request for draft revision. | Review request persisted with revision id, reviewer surface, and blocking status. |
| `human_approval_wait` | publish control | Pause or verify pre-approval according to run policy. | `pending_hitl` is resumable. No approval is fabricated. |
| `cms_publish_approved` | publish control | Confirm approved decision before materialization. | Only approved revision proceeds. Rejected or change-requested revision blocks. |
| `publish_materialize` | serving | Materialize approved page to CMS/static target. | Output includes canonical URL, content blocks, schema data, breadcrumbs, and publish metadata. |
| `render_preview_validate` | serving gate | Render and validate preview before final publish. | Rendering failures, broken schema, broken links, or missing critical metadata block finalization. |
| `finalize_publish` | serving | Finalize publish state and durable page lifecycle event. | Published revision has immutable audit trail and serving metadata. |
| `projection_barrier(publish)` | runtime | Ensure CMS/static/materialized projection events are current where policy requires. | Strict policy blocks failed publish materialization events. |
| `rebuild_detect` | runtime/planning | Detect impacted pages from truth, ontology, graph, retrieval, navigation, or publish changes. | Rebuild reasons are explainable down to changed rule/source/link/ontology item. |

---

## 6. Plane Boundaries Inside One Flow

### 6.1 Truth Plane

Authoritative sequence:

1. raw evidence and source context
2. layer classification
3. entity span detection
4. canonical mapping
5. procedural candidate extraction
6. schema validation
7. deterministic candidate validation
8. triple builder support output
9. completeness judge
10. resolution loop
11. contradiction gate
12. truth adjudication
13. `verified.rule_instances`

Canonical rule:

- nothing downstream may promote truth status.

### 6.2 Graph Plane

Semantic assembly and graph projection sequence:

1. canonical mappings
2. resolved extraction outputs
3. `triple_builder`
4. graph admissibility
5. `Neo4j` sync

Canonical rule:

- graph uses truth and semantic bindings;
- graph does not create or upgrade truth.

### 6.3 Retrieval Plane

Retrieval and indexing sequence:

1. raw chunk indexing where allowed
2. canonical registry retrieval surface
3. verified rule vectors
4. editorial topic vectors
5. SEO support vectors
6. semantic recall and link support
7. `Voyage` embedding generation
8. `Qdrant` projection

Canonical rule:

- retrieval supports search, clustering, canonical mapping fallback, topic understanding, and recall;
- retrieval does not create or upgrade truth.

### 6.4 Evolutionary ontology plane

Evolutionary ontology is used for information that is not already covered by the strict visa/procedural ontology.

Lifecycle:

1. `detected`
2. `proposed`
3. `under_review`
4. `accepted`
5. `backfilled`
6. `indexed`

Canonical rule:

- `accepted` is not activation;
- only `indexed` ontology entries can participate in automatic extraction or mapping;
- ambiguity blocks auto-map;
- Class C changes require impact analysis, migration plan, graph repair, and retrieval reindex.

### 6.5 Planning and serving boundary

Planning and drafting may consume all planes, but may not redefine truth.

They may consume:

- admissible verified truth,
- graph-safe semantic outputs,
- retrieval support artifacts,
- SEO planning artifacts,
- commercial artifacts governed by business policy.

They may not:

- promote candidates into verified truth,
- convert retrieval snippets into verified facts,
- publish unsupported factual claims,
- override unresolved contradictions,
- bypass HITL approval where policy requires it.

---

## 7. Existing Expert Runtime Behaviors That Must Not Be Lost

- `truth_admissibility_gate`
- semantic link enrichment inside `link_recommend`
- `global_site_reconcile`
- projection barrier checkpoints after `raw_knowledge_ingestion` and `global_site_reconcile`
- run-mode/scenario/policy branching semantics
- `done:no_pages` early-exit behavior
- CMS review split:
  - request review
  - wait for approval
  - approved publish
- preview validation before final publish
- rebuild detection after final phases
- outbox-based downstream materialization for `Neo4j`, `Qdrant`, and CMS
- current-run and publish-gate semantics already enforced by runtime and automation
- candidate-only LLM truth extraction policy
- source-tier-as-signal-only policy
- legacy `ContentGenerationWorkflow` quarantine

---

## 8. Detailed Execution Program

### Phase A - Close live truth-core

Goal:

- prove the current truth-core end-to-end on live inputs.

Work:

- configure Gemini truth provider;
- rerun thin live-provider smoke on fresh baseline DB;
- capture immutable evidence artifact;
- confirm at least one live candidate persists in `extracted.rule_candidates`;
- confirm live path no longer blocks on `blocked:no_truth_extraction_provider`.

Exit:

- live smoke is green;
- blocker is no longer provider readiness.

### Phase B - Freeze Truth Plane as single authority

Goal:

- ensure no other surface competes with truth-core.

Work:

- keep regex extraction removed;
- keep source-tier shortcuts forbidden;
- keep validator/adjudication as only current route to `verified`;
- document and quarantine any remaining alternative authority paths.

Exit:

- all verified truth writes come only from the approved truth path.

### Phase C - Split the current raw ingestion macro-step

Goal:

- prepare the runtime to restore rich stages without duplicate hidden extraction paths.
- create an expert-grade extraction core before all target phases become first-class orchestration phases.

Work:

- separate raw evidence preparation from candidate extraction;
- preserve current truth-core behavior behind a compatibility flag or migration boundary;
- implement the expert extraction core as composable Rust use-cases that can run from CLI, batch, integration harness, or workflow activity;
- introduce first-class step records for each restored rich stage;
- keep typed input/output contracts and persisted evidence artifacts even when the step is not yet a Temporal phase;
- ensure no stage writes verified truth before adjudication.

Exit:

- `raw_knowledge_ingestion` no longer hides multiple target stages in the long-term architecture plan.
- the expert core can be executed and tested end-to-end without depending on full Temporal orchestration.

### Phase D - Restore early evidence gates

Goal:

- ensure extraction receives only clean, classified, auditable evidence.

Work:

- activate `whole_page_semantic_pass`;
- activate `page_utility_classifier`;
- activate `dom_block_relevance_filter`;
- activate `sectioning_contract_gate`;
- activate `cas_gate`;
- persist skip/block reasons as audit artifacts.

Exit:

- raw pages cannot silently bypass utility, relevance, sectioning, or CAS gates.

### Phase E - Reactivate semantic classification and mapping

Goal:

- classify every extractable atomic statement and bind mentions before extraction.

Work:

- activate `layer_router`;
- activate `subspan_layer_router` for mixed sections;
- activate `entity_span_detection`;
- activate `canonical_mapping`;
- activate `ontology_intake_gate`;
- require all rich extraction to bind through mention ids or exact spans.

Exit:

- no rich extraction runs on unclassified or unbound text unless explicitly routed to HITL or ontology intake.

### Phase F - Reactivate rich layer extraction

Goal:

- extract all relevant information while preserving strict truth boundaries.

Work:

- activate strict procedural extraction for visa rules;
- activate operational extraction for temporary/external entity state;
- activate editorial extraction for topics and user needs;
- activate SEO signal extraction for demand and site structure;
- activate commercial signal extraction for business-owned offers;
- keep non-procedural layers out of procedural verified truth.

Exit:

- every extracted object has layer, evidence, schema, and authority classification.

### Phase G - Restore triple builder, completeness judge, and resolution loop

Goal:

- prevent incomplete, hallucinated, ambiguous, or graph-unsafe outputs from becoming truth or graph projections.

Work:

- make `triple_builder` mandatory;
- require deterministic join anchors;
- make `completeness_judge` mandatory before procedural truth promotion;
- require all seven loss checks plus hallucination checks;
- make `resolution_loop` mandatory for unresolved findings;
- route deterministic fixes, reruns, HITL, and drops through explicit state transitions.

Exit:

- no procedural candidate can reach truth adjudication with unresolved loss, hallucination, missing required binding, or ignored judge findings.

### Phase H - Certify truth adjudication under the rich flow

Goal:

- make the rich flow converge into the same single Truth Plane.

Work:

- route resolved procedural candidates into `contradiction_gate`;
- route only structured/resolved candidates into `truth_adjudication`;
- write `verified.rule_instances` with evidence, adjudication, freshness, completeness, and version metadata;
- demote conflicting or no-longer-admissible semantic slots explicitly.

Exit:

- verified truth remains single-authority and richer than the current candidate-only path.

### Phase I - Reactivate Graph Plane

Goal:

- make `Neo4j` projection part of the canonical expert flow again.

Work:

- add `graph_admissibility_gate`;
- allow `Neo4j` sync only from graph-safe assembled outputs;
- block sync on unresolved bindings, contradictions, missing anchors, missing evidence, or unsupported relation types;
- preserve outbox idempotency and replay.

Exit:

- graph projection is active but still non-authoritative.

### Phase J - Reactivate Retrieval Plane

Goal:

- make retrieval and cluster/topic support first-class again.

Work:

- add `retrieval_admissibility_gate`;
- allow `Voyage/Qdrant` sync only from approved indexing surfaces;
- restore retrieval support for:
  - raw evidence recall,
  - canonical mapping fallback,
  - verified rule recall,
  - editorial topic recall,
  - cluster/topic understanding,
  - cross-page reasoning support,
  - navigation quality support;
- ensure stale ontology or truth changes invalidate affected vectors.

Exit:

- retrieval is active and clearly non-authoritative.

### Phase K - Enrich planning with graph and retrieval outputs

Goal:

- ensure planning consumes the enriched three-plane system.

Work:

- make planning consume:
  - admissible verified truth,
  - graph-safe semantic outputs,
  - retrieval support artifacts;
- keep drafting downstream of `truth_admissibility_gate`;
- improve cluster/topic understanding, cross-page reasoning, link recommendation, and global reconcile quality through graph/retrieval inputs.

Exit:

- planning and drafting are enriched by graph and retrieval without surrendering truth authority.

### Phase L - Certify rebuilt full workflow end-to-end

Goal:

- prove the rebuilt expert workflow as one coherent orchestration path.

Work:

- certify end-to-end flow with live truth-core plus restored early gates, semantic stages, completeness, resolution, graph, and retrieval;
- verify publish, projection, and rebuild behavior after rich-flow reactivation;
- produce machine-readable run reports for every phase.

Exit:

- one certified full workflow shape with no competing authority path.

---

## 9. Step-by-Step Status Ledger

### 9.1 Active now

- `load_verified_support_bundle`
- `serp_ingest`
- `crawl_sources`
- current `raw_knowledge_ingestion` macro-step
- current validator foundation
- current adjudication writer path
- projection barrier checkpoints
- `serp_normalize`
- `opportunity_build`
- `ia_build`
- `link_recommend`
- `global_site_reconcile`
- `truth_admissibility_gate`
- `draft_assemble`
- publish-control path:
  - `cms_request_review`
  - `human_approval_wait`
  - `cms_publish_approved`
  - `publish_materialize`
  - `render_preview_validate`
  - `finalize_publish`
- `rebuild_detect`

### 9.2 Partial now

- `seo_preflight`
- `Neo4j` projection surface
- `Qdrant` retrieval surface
- `Voyage` embedding adapter surface
- ontology-backed canonical storage
- graph/retrieval specs and adapters
- outbox-backed downstream materialization surfaces that are not yet first-class workflow phases
- prototype `layer_router` and `triple_builder` step code that is not yet mandatory in the production flow

### 9.3 Deferred now

- mandatory `whole_page_semantic_pass`
- mandatory `page_utility_classifier`
- mandatory `dom_block_relevance_filter`
- mandatory `sectioning_contract_gate`
- mandatory `cas_gate`
- mandatory `layer_router`
- mandatory `subspan_layer_router`
- mandatory `entity_span_detection`
- mandatory `canonical_mapping`
- mandatory `ontology_intake_gate`
- rich extraction stages above the current truth-core
- mandatory `triple_builder`
- mandatory `completeness_judge`
- mandatory `resolution_loop`
- mandatory `contradiction_gate` as explicit rich-flow phase
- mandatory `graph_admissibility_gate`
- mandatory `retrieval_admissibility_gate`
- first-class `neo4j_sync`
- first-class `voyage_qdrant_sync`
- full graph-backed cluster/topic reasoning

### 9.4 Legacy / quarantined now

- `ContentGenerationWorkflow`
- any direct truth path outside adjudication
- any source-tier shortcut truth verdict
- any regex/keyword path that writes verified truth
- any retrieval or graph path that upgrades truth status

---

## 10. Acceptance Gates

### Gate 1 - Live truth-core

Required:

- configured Gemini provider;
- live smoke green;
- persisted live candidates;
- admissible verified truth produced from live sections;
- no source-tier shortcut to verified.

### Gate 2 - Early evidence gates

Required:

- `whole_page_semantic_pass`, `page_utility_classifier`, `dom_block_relevance_filter`, `sectioning_contract_gate`, and `cas_gate` active or explicitly represented inside decomposed implementation steps;
- every skip/block decision persisted with reason;
- extraction cannot run over denied utility pages, denied blocks, unstable sections, or duplicate content without explicit policy.

### Gate 3 - Semantic classification and canonical mapping

Required:

- `layer_router`, `subspan_layer_router`, `entity_span_detection`, `canonical_mapping`, and `ontology_intake_gate` active in workflow order;
- full five-layer score vector for every extraction-eligible section or subspan;
- entity spans have stable ids and offsets;
- canonical mapping uses symbolic-first matching and vector fallback only after deterministic tiers;
- ambiguous or new ontology items do not auto-map.

### Gate 4 - Rich extraction and schema validation

Required:

- procedural, operational, editorial, SEO, and commercial extraction outputs are separated by layer;
- procedural roles remain limited to the eight-role strict ontology;
- non-procedural outputs cannot create procedural truth;
- schema validation blocks downstream on required-field failure.

### Gate 5 - Triple Builder, Completeness Judge, and Resolution Loop

Required:

- `triple_builder` uses deterministic anchors only;
- `completeness_judge` checks all seven loss types plus hallucinations;
- judge findings require workflow action;
- `resolution_loop` produces resolved, rerun, HITL, or dropped state;
- unresolved loss, hallucination, blocked mapping, or contradiction cannot reach verified truth, graph sync, or publish.

### Gate 6 - Truth adjudication and verified write

Required:

- candidates reach `verified`, `needs_hitl`, or `rejected` only through deterministic adjudication;
- verified writes include evidence, source snapshot hash, adjudication reason, publish admissibility, freshness, completeness, and version metadata;
- conflicting semantic variants block publish or route to HITL.

### Gate 7 - Graph Plane activation

Required:

- graph-admissibility gate active;
- `Neo4j` sync is downstream-only;
- no raw, unverified, unresolved, or graph-unsafe writes;
- graph writes use stable identities, not display labels.

### Gate 8 - Retrieval Plane activation

Required:

- retrieval-admissibility gate active;
- `Voyage/Qdrant` sync is downstream-only;
- no retrieval-driven truth promotion;
- stale ontology/truth changes invalidate affected retrieval artifacts;
- collection contracts and payload schemas exist before indexing.

### Gate 9 - Enriched planning/drafting

Required:

- planning consumes truth + graph + retrieval outputs;
- drafting remains blocked without admissible verified truth;
- supplemental retrieval context is visibly non-fact support unless tied to verified truth;
- improved cluster/topic and cross-page signals are observable.

### Gate 10 - Full expert certification

Required:

- fresh-scope live run proves acquisition, early gates, rich extraction, truth, graph, retrieval, planning, drafting, HITL, publish, and rebuild;
- mixed-page fixture proves procedural, operational, editorial, SEO, and commercial decomposition;
- contradiction fixture blocks publish;
- ontology ambiguity fixture blocks auto-map;
- replay proves deterministic output for unchanged inputs.

---

## 11. Machine-Verification Requirements

Every phase exit must be backed by at least one of:

- contract test,
- replay test,
- integration harness assertion,
- smoke artifact under `docs/runs`,
- SQL invariant check,
- outbox/projection status report,
- HITL resolution artifact,
- run report metric.

Minimum required machine checks for V6.3 reactivation:

- phase catalog includes restored early gates and rich stages;
- step execution rows include hashes, schema versions, retry class, and executor build id;
- layer router output contains all five scores;
- entity mention output contains span offsets;
- canonical mapping output references mention ids;
- extraction outputs contain evidence ids and span references;
- triple builder rejects missing anchors;
- completeness judge cannot emit findings without workflow action;
- resolution loop cannot silently continue on unresolved findings;
- adjudication cannot verify non-structured or unresolved candidates;
- graph sync rejects null required canonical keys;
- retrieval sync rejects stale or schema-less payloads.

---

## 12. Document Update Rules

When this file changes:

- increment `Current version`;
- append a change entry in the change log;
- keep `V6_Expert_Truth_Graph_Runtime.md` stable unless ownership or architecture changes;
- use this file for execution-state edits, not the owner-document.

V6.3 is an execution-satellite expansion of the existing V6 target expert architecture. It restores previously defined expert gates and does not move global ownership away from `V6_Expert_Truth_Graph_Runtime.md`.

---

## 13. Versioned Change Log

### 6.3

- restored early evidence gates from V5 into the target `SeoSiteBuildWorkflow` shape:
  - `whole_page_semantic_pass`
  - `page_utility_classifier`
  - `dom_block_relevance_filter`
  - `sectioning_contract_gate`
  - `cas_gate`
- restored mandatory `completeness_judge` and `resolution_loop` before rich-flow truth promotion;
- expanded the target workflow into a step-by-step 56-step flow;
- clarified that current `raw_knowledge_ingestion` is a migration macro-step and must be decomposed in the target architecture;
- added 10/10 invariants for authority, classification, evidence joins, completeness, runtime verifiability, projection, and retrieval;
- added the expert-first implementation rule: expert extraction safety gates must run before full orchestration completeness;
- added explicit strict procedural ontology vs evolutionary ontology routing;
- added detailed step contract ledger for every target flow step;
- added machine-verifiable acceptance gates for early gates, semantic classification, rich extraction, completeness/resolution, graph, retrieval, planning, drafting, and certification.

### 6.2

- added explicit run-mode, scenario, and policy branching semantics from the real execution plan
- clarified that `seo_preflight` is a pre-run outer gate, not a current workflow phase key
- clarified that current truth-core validator/adjudication are active now, while richer semantic stages remain deferred
- clarified that `neo4j_sync` and `voyage_qdrant_sync` are currently partial outbox-backed surfaces rather than active first-class workflow phases

### 6.1

- aligned the working plan with the actual active phase catalog
- preserved current publish, rebuild, and projection-control contours
- expanded target workflow shape to include the full orchestration surface
- clarified active, partial, and deferred steps
- added explicit non-regression inventory for existing expert runtime behaviors

### 6.0

- initial versioned working plan created
- defines target `SeoSiteBuildWorkflow` shape
- defines how Truth, Graph, and Retrieval planes live inside one flow
- defines activation order for rich expert architecture
