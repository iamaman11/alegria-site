# V6 SeoSiteBuildWorkflow Working Plan

**Status:** active versioned working plan
**Class:** `current-execution-satellite`
**Parent owner document:** [V6_Expert_Truth_Graph_Runtime.md](V6_Expert_Truth_Graph_Runtime.md)
**Purpose:** detailed implementation plan for converging the accepted canonical cutover workflow into the single active orchestration flow for Truth, Graph, Retrieval, and Serving planes.
**Editing rule:** this file is intentionally versioned and updated during execution.
**Current version:** `6.47`

---

## 1. Mission

The active forward orchestration path must remain a single workflow that:

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

### 1.2 Coverage boundary

The 56-step flow is the canonical product/runtime value stream for one accepted forward site-build run, executed by `SeoSiteBuildCanonicalCutoverWorkflow`.

It is not a list of every process, daemon, migration, CI check, backup script, adapter, or laboratory surface in the repository.

The rule is:

- a capability belongs inside the 56 steps when it mutates, validates, or gates source evidence, truth, graph/retrieval projection, planning, drafting, publish, or rebuild state for the current SEO site-build run;
- a capability belongs outside the 56 steps when it prepares the runtime substrate, verifies release safety, consumes outbox work asynchronously, runs scheduled monitoring, backs up/restores infrastructure, generates contracts, or serves as legacy/test/lab support.

Support-plane functionality must still be covered by acceptance gates, automation, runbooks, and release checks. It must not be forgotten, but it also must not be inserted into the workflow as if it were domain extraction or page-build logic.

### 1.3 Full project coverage map

This map is the result of reconciling the V6 plan with the V5 extraction protocol, SEO companion documents, ops runbooks, automation surface, and active Rust services.

| Area | Inside 56-step flow | Outside 56, but required support plane |
|---|---|---|
| Scope/run identity | `seo_preflight`, `load_verified_support_bundle.initial`, `truth_admissibility_gate` | starter CLI, run-mode/scenario policy, scope bootstrap, local DB baseline |
| Source discovery | `serp_ingest`, `serp_normalize` | DataForSEO credentials, SERP playbook artifacts, source registries |
| Crawl and raw evidence | `crawl_sources`, `whole_page_semantic_pass`, `page_utility_classifier`, `dom_block_relevance_filter`, `sectioning`, `sectioning_contract_gate`, `cas_gate`, `raw_evidence_register` | crawler adapter, robots/retry policy, raw page persistence, duplicate-content support code |
| Expert extraction | `layer_router` through `verified_truth_write` | truth LLM provider adapter, primitives validators, policy crates, golden fixtures, integration harness |
| Ontology | `canonical_mapping`, `ontology_intake_gate`, `resolution_loop`, `verified_truth_write`, retrieval invalidation through `voyage_qdrant_sync` | registry governance, impact analysis tooling, migration/backfill procedures |
| Graph and retrieval | `graph_admissibility_gate`, `retrieval_admissibility_gate`, `neo4j_sync`, `voyage_qdrant_sync`, projection barriers | outbox worker, reconcile service, Neo4j/Qdrant/Voyage adapters, stale outbox reclaim |
| Planning and supersite IA | `opportunity_build`, `ia_build`, `link_recommend`, `global_site_reconcile` | SEO registries, page-type policies, quality policy registry, graph analytics lab when needed |
| Draft and publish | `truth_admissibility_gate` through `projection_barrier(publish)` | CMS/static adapters, HITL queue surfaces, reviewer tooling, publish outbox consumers |
| Privacy, licensing, and content safety | `draft_assemble`, `content_contract_validate`, `draft_qa`, `render_preview_validate` | PII redaction pre-LLM, licensing gate, quality policy registry, HITL operator surface |
| Rebuild and update loop | `rebuild_detect` | scheduled freshness checks, GSC/analytics services, rebuild scheduler/backlog processors |
| Runtime safety | every first-class step ledger and every projection barrier | Temporal worker build-id discipline, replay/drain policy, DLQ, payload blobs, metrics, production gate |
| Schema/contracts | every step must consume typed contracts | Proto/FBS generation, SQL migrations, SQLx offline metadata, schema parity checks |
| Ops resilience | release gates observe the flow | CI, smoke checks, restore drill, backups, monitoring, Grafana/Prometheus |
| Legacy/test/lab surfaces | excluded unless explicitly promoted | `ContentGenerationWorkflow`, legacy `FactExtractionWorkflow` skeleton, `TestHitlWorkflow`, Python analytics lab |

### 1.4 Missing-process rule

If a future feature appears to be "missing from the 56 steps", classify it before changing the flow:

1. If it is a current-run domain phase, add it to the 56-step ledger or merge it into the correct existing step with an explicit exit contract.
2. If it is an asynchronous materializer or monitor, keep it as a support-plane process and add a barrier, metric, or acceptance gate where the workflow depends on its result.
3. If it is release safety, keep it in automation/runbooks and require it in production gates.
4. If it is legacy, test-only, or lab-only, quarantine it and document the promotion rule before it can affect production.

This prevents two failure modes:

- hiding real extraction or publish logic outside the canonical flow;
- bloating the workflow with infrastructure processes that should be independently operated and verified.

### 1.5 Support-process registry boundary

The required support processes outside the 56-step flow are owned by [V6_Support_Process_Registry.md](V6_Support_Process_Registry.md).

This plan must not duplicate their operational contracts. It only decides:

- whether the workflow depends on them;
- where barriers or gates must observe them;
- when a support process must be promoted into a first-class workflow or runner.

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

Additional first-class workflow surfaces now present for phased rollout.

These surfaces are migration-only. They are not the long-term target architecture and they do not create a second permanent production flow. The single accepted forward path is `SeoSiteBuildCanonicalCutoverWorkflow`, with `SeoSiteBuildWorkflow` retained only for explicit compat/drain.

- `ExpertExtractionWorkflow`:
  - isolated truth/extraction rollout path for `load_seo_site_build_input -> load_verified_support_bundle.initial -> serp_ingest -> crawl_sources -> raw_knowledge_ingestion -> optional support refresh`
  - now quarantine-only and startable only with `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`
- `ExpertDecomposedExtractionWorkflow`:
  - isolated decomposition rollout path for `load_seo_site_build_input -> load_verified_support_bundle.initial -> serp_ingest -> crawl_sources -> load_semantic_section_sample -> page_utility_classifier -> dom_block_relevance_filter -> layer_router -> entity_span_detection -> canonical_mapping -> procedural_extraction -> operational_extraction -> editorial_extraction -> completeness_judge -> triple_builder -> contradiction_gate -> hitl_decision -> raw_knowledge_ingestion -> optional support refresh`
  - now quarantine-only and startable only with `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`
- `ExpertProjectionWorkflow`:
  - isolated truth-to-projection rollout path for `load_seo_site_build_input -> load_verified_support_bundle.initial -> serp_ingest -> crawl_sources -> raw_knowledge_ingestion -> graph_admissibility_gate -> neo4j_sync -> retrieval_admissibility_gate -> voyage_qdrant_sync -> projection_barrier.post_projection -> optional support refresh`
  - now quarantine-only and startable only with `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`
- `ExpertSemanticSliceWorkflow`:
  - isolated real-section rollout path for `load_seo_site_build_input -> load_verified_support_bundle.initial -> serp_ingest -> crawl_sources -> load_semantic_section_sample -> page_utility_classifier -> dom_block_relevance_filter -> layer_router -> entity_span_detection -> canonical_mapping -> procedural_extraction -> operational_extraction -> editorial_extraction -> completeness_judge -> triple_builder -> contradiction_gate -> hitl_decision`
  - now quarantine-only and startable only with `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`
- `ProjectionReconcileWorkflow`:
  - explicit reconcile surface for `neo4j` and `qdrant`
- `FreshnessCheckWorkflow`:
  - support-plane freshness monitor
- `SeoSiteBuildCanonicalCutoverWorkflow`:
  - accepted canonical-cutover surface now wired and runtime-accepted for canonical `steps 1-56`

Additional support-plane executable surfaces now present outside the 56-step run:

- `temporal_starter RebuildDispatch`
  - consumes queued rebuild backlog rows and starts a fresh scoped workflow run on a new workflow id
- `temporal_starter OntologyBackfillPlan`
  - plans ontology backfill and can materialize concepts into Neo4j and `Voyage/Qdrant` ontology retrieval projection
- `cli_tools SeoPostPublishFeedbackProbe`
  - verifies analytics/GSC support loop health without mutating truth
- `cli_tools SeoReleaseRestoreGate`
  - evaluates release/build-id/restore readiness and can explicitly run `ci_verify`, `temporal_production_gate`, and `restore_drill` with machine-readable evidence
- `cli_tools SeoCutoverShadowVerify`
  - compares current runtime against `SeoSiteBuildCanonicalCutoverWorkflow` by `run_id`, step ledger, candidate status counts, and projection barriers, and emits a machine-readable shadow report

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

### 2.2 What is not yet complete in steady-state

- production provider configuration for live truth extraction remains an external prerequisite;
- `SeoSiteBuildWorkflow` still exists as compat/drain workflow type outside the active forward path;
- `Expert*Workflow` surfaces are quarantined but not yet physically removed from the codebase;
- deeper graph-backed reasoning beyond planning artifacts remains deferred;
- some downstream materialization still uses outbox-backed async surfaces even though the canonical workflow already owns admissibility and barrier decisions.

### 2.3 Current blocker

Current live blocker:

- no configured Gemini truth extraction provider

Canonical live envs:

- `GEMINI_API_KEY`
- `GOOGLE_API_KEY`

### 2.4 Important clarifications

- `seo_preflight` is a pre-run outer gate and operator entrypoint, not a current `SeoPhaseKey`.
- the accepted forward path is `SeoSiteBuildCanonicalCutoverWorkflow`; descriptions of `raw_knowledge_ingestion` as the main runtime contour apply only to legacy compat/drain understanding.
- `neo4j_sync` and `voyage_qdrant_sync` are active canonical workflow phases in the forward path, even if downstream projection materialization still relies on async outbox/reconcile surfaces.
- current truth-core `validator` and `adjudication` remain part of the canonical truth path; the important distinction now is forward-path step ownership versus legacy macro-step compatibility.
- deferred items below refer only to genuinely deferred capabilities such as richer graph-first reasoning, not to already accepted canonical step coverage.
- `code-present, runtime-inactive` source files are not counted as active implementation until crate export, live invocation, and automation coverage all exist.
- `ExpertExtractionWorkflow` remains available only as a migration-only diagnostic extraction surface; it is not the forward runtime path.

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
| `whole_page_semantic_pass` | raw/semantic | Understand each page as a whole before section-level extraction. This step does not extract facts. | Page mode, page-mode confidence, dominant layers, layer scores, page summary, page context profile, global entities, mixed-section hints, and optional advisory retrieval diagnostics. |
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
| `canonical_mapping` | semantic/truth support | Map mentions to canonical registry keys through exact alias, normalized alias, heuristic rule/regex hint, then vector fallback. Regex may assist mapping, but it is not final authority for verified truth. | Mapping output references `mention_id`, original span, target registry, canonical key or null, match method, confidence, HITL need. |
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
| `draft_assemble` | serving preparation | Assemble draft plan from verified support, blueprint, section templates, graph/retrieval context, and required links. | LLM request is constrained by verified support and must pass PII redaction before external LLM use. Supplemental retrieval is marked non-fact support unless verified. |
| `editorial_draft_generate` | serving | Generate draft candidate under JSON/content contract. | LLM may write prose only. It may not create unsupported facts. |
| `draft_normalize` | serving | Normalize draft candidate into canonical draft blocks, claim ledger, support refs, metadata, and internal links. | Unsupported or malformed claims remain visible for QA and cannot be hidden. |
| `content_contract_validate` | serving gate | Validate page-type contract, required sections, metadata obligations, links, traceability labels, and schema readiness. | Contract failure blocks publish path. |
| `draft_qa` | serving gate | Validate factual support, claim coverage, unsupported numbers/dates/prices, licensing restrictions, quality-policy thresholds, duplicate risk, links, readability, and schema coverage. | Unsupported factual claims, restricted redistribution sources, or quality-policy failures block publish and may route to HITL. |
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

### Phase M - Canonical Cutover To One Workflow

#### Objective

- the problem is not that the architecture lacks a full 56-step target;
- the problem is that the runtime still lives in a migration state with several rollout surfaces;
- the goal of Phase M is to converge those rollout surfaces toward one replacement path without creating two permanent production architectures.

#### Canonical replacement target

- the long-term target remains the canonical 56-step flow in `Section 4.1`;
- the single accepted forward path is `SeoSiteBuildCanonicalCutoverWorkflow`;
- `SeoSiteBuildWorkflow` remains alive only for the compatibility window, replay safety, and explicit legacy drain;
- no second canonical candidate exists;
- no new `Expert*Workflow` surfaces should be added after this point.

#### Phase M1 - Steps 1-36

Phase M1 is the first cutover stage. It does not redefine the 56-step target and it does not introduce a second workflow ledger. It implements the canonical `Section 4.1` steps `1-36` under the replacement candidate while preserving replay safety and keeping the old publish path stable.

Coverage:

- steps `1-31`: preflight, support loading, source discovery, crawl, raw evidence, semantic routing, extraction, validation, completeness, resolution, adjudication, and verified truth write;
- steps `32-36`: graph/retrieval admissibility, `Neo4j` sync, `Voyage/Qdrant` sync, and post-projection barrier.

Rules:

- Phase M1 must reference canonical steps `1-36` from `Section 4.1`, not invent a parallel numbering scheme;
- if a target step is still temporarily executed inside an aggregate implementation boundary, it must be described as `implementation aggregation`, not as a new workflow ledger;
- `raw_knowledge_ingestion` may not remain a hidden second authority path inside the replacement flow.

#### Phase M2 - Steps 37-56

Phase M2 is the later cutover stage for the remainder of the canonical flow:

- steps `37-42`: planning;
- steps `43-55`: drafting, QA, review, publish control, publish materialization, and publish barrier;
- step `56`: rebuild detection.

Rules:

- publish is not considered cut over by Phase M1;
- until Phase M2 acceptance, old `SeoSiteBuildWorkflow` remained the canonical publish path;
- Phase M2 must preserve existing HITL, preview validation, finalize publish, and rebuild semantics.

#### Migration-only workflow status

The following workflows are migration-only rollout surfaces:

- `ExpertExtractionWorkflow`
  - `migration-only`
  - `not canonical`
  - `not permanent product runtime`
  - `eligible for retirement after full cutover convergence evidence`
- `ExpertDecomposedExtractionWorkflow`
  - `migration-only`
  - `not canonical`
  - `not permanent product runtime`
  - `eligible for retirement after full cutover convergence evidence`
- `ExpertProjectionWorkflow`
  - `migration-only`
  - `not canonical`
  - `not permanent product runtime`
  - `eligible for retirement after full cutover convergence evidence`
- `ExpertSemanticSliceWorkflow`
  - `migration-only`
  - `not canonical`
  - `not permanent product runtime`
  - `eligible for retirement after full cutover convergence evidence`

#### Cutover acceptance

Phase M1 acceptance:

- `SeoSiteBuildCanonicalCutoverWorkflow` covers canonical steps `1-36`;
- there is no hidden `raw_knowledge_ingestion` authority duplication inside the replacement path;
- truth and projection outputs are replay-safe;
- no second truth authority path exists;
- shadow comparison against the current runtime is green;
- `SeoReleaseRestoreGate` and rollout checks are green.

Phase M2 acceptance:

- `SeoSiteBuildCanonicalCutoverWorkflow` covers canonical steps `37-56`;
- publish, HITL, preview, finalize publish, and rebuild semantics are preserved;
- there is no regression in publish control;
- old `SeoSiteBuildWorkflow` can be drained safely.

#### Retirement rule

- rollout workflows may not remain undefined "just in case";
- after canonical cutover promotion, each migration-only workflow must either:
  - be marked `diagnostic-only`, or
  - be deleted after the compatibility window;
- no business logic may continue to land in migration-only workflows after `SeoSiteBuildCanonicalCutoverWorkflow` is declared the canonical candidate.

#### Execution protocol for Phase M1

Phase M1 must be executed as a strict expert migration program, not as opportunistic parallel coding across unrelated steps.

Execution unit:

- the implementation unit is canonical `Section 4.1` step coverage under `SeoSiteBuildCanonicalCutoverWorkflow`;
- every change must state which canonical step or contiguous step group it advances;
- work may not be described only as "improve extraction", "improve runtime", or "wire rollout surface" without naming the canonical steps affected.

Required implementation order:

1. `steps 1-11`
   - preflight, support loading, source discovery, crawl, semantic page prelude, section quality, CAS, raw evidence registration, and `projection_barrier(raw_evidence)`
2. `steps 12-17`
   - layer routing, subspan routing, entity spans, canonical mapping, and ontology intake
3. `steps 18-31`
   - procedural / operational / editorial / SEO / commercial extraction, schema validation, candidate validation, triple builder, completeness, resolution, contradiction handling, adjudication, verified write, and support refresh
4. `steps 32-36`
   - graph admissibility, retrieval admissibility, `Neo4j` sync, `Voyage/Qdrant` sync, and `projection_barrier(semantic_projection)`
5. shadow verification
   - run `cli_tools SeoCutoverShadowVerify` on paired legacy/cutover runs and compare `SeoSiteBuildCanonicalCutoverWorkflow` against current runtime on the same scope before promotion

The order above is mandatory unless a later change is purely a bug fix for an already accepted earlier slice.

#### Step completion status model

No canonical step in Phase M1 is considered implemented until it reaches all required states below:

- `doc_defined`
  - the step is correctly described in this file and not contradicted elsewhere
- `typed`
  - the step has typed input/output contracts
- `ledgered`
  - the step persists execution evidence through the canonical step ledger
- `wired`
  - the step is invoked by `SeoSiteBuildCanonicalCutoverWorkflow` or an explicitly named temporary implementation aggregation inside the cutover workflow
- `tested`
  - the step has at least one deterministic contract/unit/integration check
- `shadow_verified`
  - the step has been exercised in V2 against real or representative input alongside current runtime comparison where relevant
- `accepted`
  - the step is explicitly covered by the active cutover gate for its slice

Status shortcuts are forbidden:

- file exists != implemented;
- exported crate symbol != active step;
- runnable rollout workflow != canonical cutover;
- green happy-path smoke != accepted step coverage.

#### No-skip rule

Phase M1 implementation must obey the following control rules:

- no later slice may be declared complete while an earlier slice still lacks `typed`, `ledgered`, or `wired` coverage for a required canonical step;
- no step may bypass its upstream gate if that later step still remains present in the flow;
- no hidden authority logic may remain inside `raw_knowledge_ingestion` once the corresponding explicit canonical steps are declared wired in V2;
- no support-plane surface may be used to masquerade as canonical step coverage;
- no new migration-only workflow may be introduced to avoid wiring a missing canonical step into `SeoSiteBuildCanonicalCutoverWorkflow`.

#### Slice acceptance rule

Each implementation slice in Phase M1 must exit with all of the following:

- named canonical step coverage for that slice;
- machine-verifiable evidence for `typed`, `ledgered`, `wired`, and `tested`;
- explicit list of still-aggregated steps, if any;
- explicit list of remaining gaps that block promotion to the next slice;
- updated status in `Section 9` if the slice changes active/partial/deferred classification.

#### Context-loss recovery rule

If implementation context is lost, work must resume from stable artifacts rather than memory.

Recovery sequence:

1. read `Section 4.1` for canonical step order;
2. read `Section 8 / Phase M` for cutover target, current slice order, and no-skip rules;
3. read `Section 9` for active / partial / deferred status;
4. read the latest entry in `Section 13` to confirm the most recent planning change;
5. inspect current git status and separate:
   - user-owned unrelated dirty files,
   - accepted implementation changes,
   - unfinished cutover WIP that must be either completed or removed.

Resume rule:

- when ambiguity remains after the sequence above, resume from the earliest non-accepted Phase M1 slice;
- never resume from a later slice by assumption;
- never infer acceptance from code presence without ledger/test/shadow evidence.

#### Retired WIP preservation rule

The removed `ExpertRichFlowWorkflow` was an exploratory WIP, not an accepted runtime surface. Its deletion must not be interpreted as permission to lose reusable implementation knowledge.

Reusable achievements from that WIP are preserved conceptually as:

- combined semantic-to-projection orchestration for canonical Phase M1 coverage;
- pause / resume / status workflow control pattern for a cutover workflow;
- reuse of existing rich step activities across semantic, extraction, and projection slices;
- end-of-run summary expectations needed for later shadow comparison.

When implementing `SeoSiteBuildCanonicalCutoverWorkflow`, these reusable achievements must be recovered from stable sources, not by resurrecting `ExpertRichFlowWorkflow`:

- `ExpertDecomposedExtractionWorkflow` for semantic and extraction slice ordering;
- `ExpertProjectionWorkflow` for graph / retrieval admissibility and projection ordering;
- `ExpertSemanticSliceWorkflow` for real-section gate wiring and step catalog usage;
- the accepted `Phase M` rules in this file for naming, cutover scope, and no-skip execution.

Rule:

- no new code may reintroduce `ExpertRichFlowWorkflow` as a workflow identity;
- if code from that retired WIP is reused, it must be moved directly into `SeoSiteBuildCanonicalCutoverWorkflow` under canonical step names and cutover semantics.

#### Clean-development baseline

Before continuing development past planning, the repository must satisfy this cleanliness rule:

- only user-owned unrelated dirty files may remain outside accepted cutover work;
- there must be no unfinished migration-only workflow identity outside the four explicitly listed `Expert*Workflow` surfaces;
- the next implementation target must be post-acceptance convergence work: keep `SeoSiteBuildCanonicalCutoverWorkflow` as the single active canonical path, drain compat-only dependence on `SeoSiteBuildWorkflow`, and retire or quarantine migration-only rollout surfaces under the Phase M retirement rule;
- any future cutover code must begin from this baseline rather than reopening retired WIP directions.

#### Convergence closure policy

The remaining post-acceptance work is operational convergence, not missing workflow coverage.

- `SeoSiteBuildWorkflow` compat/drain window closes for an environment only when legacy replay evidence shows `total_runs=0` and `open_runs=0`, or an explicit production drain artifact proves all remaining histories safely drained.
- `Expert*Workflow` surfaces remain as a permanent narrow diagnostic toolset behind explicit opt-in `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`; they are diagnostic-only, they are not a landing zone for new product logic, they are not part of canonical execution, and they are not hidden fallbacks for the forward path.
- the runtime workflow type name remains `SeoSiteBuildCanonicalCutoverWorkflow` after drain; we do not create a new workflow type purely to rename it.
- a future rename is allowed only if another semantics change already requires a new workflow type, or if a zero-history cleanup window is explicitly approved.

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
- `SeoSiteBuildCanonicalCutoverWorkflow` with canonical `steps 1-56` wired through `rebuild_detect` as the current active forward workflow
- `cli_tools SeoCutoverShadowVerify` as the current machine-readable operator surface and paired-run evidence producer for accepted `Phase M1` and `Phase M2` cutover verification
- first real paired-run Phase M1 shadow report captured at [docs/runs/seo_cutover_shadow_verify_2026-05-21_phase_m1.json](/home/bose/projects/alegria-site/docs/runs/seo_cutover_shadow_verify_2026-05-21_phase_m1.json)
- accepted Phase M1 cutover evidence status:
  - legacy run id `d1e6a42c-f896-4420-8caa-effd07a0e28b`
  - accepted cutover run id `e7dcb050-e132-4592-84a8-6abd5d9add8e`
  - accepted paired-run shadow report: [docs/runs/seo_cutover_shadow_verify_2026-05-22_phase_m1_accepted.json](/home/bose/projects/alegria-site/docs/runs/seo_cutover_shadow_verify_2026-05-22_phase_m1_accepted.json)
  - canonical ledger coverage for `steps 1-36`: complete
  - `projection_barrier(semantic_projection)`: clear with `blocked_events=0`
- current Phase M2 planning status:
  - canonical `steps 37-56` are runtime-wired and now runtime-accepted in `SeoSiteBuildCanonicalCutoverWorkflow`
  - accepted full-cutover acceptance report captured at [docs/runs/seo_cutover_shadow_verify_2026-05-22_full_acceptance.json](/home/bose/projects/alegria-site/docs/runs/seo_cutover_shadow_verify_2026-05-22_full_acceptance.json)
  - accepted legacy full-flow run id `4bbb4100-5013-4e52-a307-0b74f882b2fd`
  - accepted cutover full-flow run id `35bbdec3-04f5-4f68-9035-df6ea9a128d1`
  - `Phase M2` runtime evidence proved preserved review, HITL, preview, finalize publish, publish barrier, and rebuild semantics on a shared acceptance fixture
  - the critical publish-tail bug fixed during acceptance was stale in-memory `qa_verdict` propagation between `draft_qa` and `cms_request_review`; both legacy and cutover runtimes now pass the updated draft state into CMS review/publish steps
- accepted `SeoSiteBuildCanonicalCutoverWorkflow` forward path defined in `Phase M`, with the remaining work now limited to promotion/drain/retirement rather than missing canonical step coverage
- formal convergence policy artifact captured at [docs/runs/seo_cutover_convergence_policy_2026-05-22.json](/home/bose/projects/alegria-site/docs/runs/seo_cutover_convergence_policy_2026-05-22.json), recording:
  - local compat/drain window closed because the legacy replay inventory contains zero `SeoSiteBuildWorkflow` runs in the current environment;
  - `Expert*Workflow` family retained only as diagnostic-only opt-in tooling;
  - runtime naming decision fixed to keep `SeoSiteBuildCanonicalCutoverWorkflow` and avoid a cosmetic post-drain workflow-type rename
- `Neo4j` projection surface
- `Qdrant` retrieval surface
- `Voyage` embedding adapter surface
- ontology-backed canonical storage
- graph/retrieval specs and adapters
- outbox-backed downstream materialization surfaces that are not yet first-class workflow phases
- accepted whole-page semantic scaffolding that now includes weighted deterministic page-framing heuristics, optional `voyage-4-large` prototype retrieval hints under conservative fusion, and a dedicated baseline-backed fixture gate

### 9.3 Deferred now

- any future graph tranche that wants to affect `page_brief`, `draft_*`, `publish_*`, or `rebuild_detect`
- any new graph/retrieval path that would upgrade truth directly instead of staying inside planning artifacts

### 9.4 Legacy / quarantined now

- `ContentGenerationWorkflow`
- migration-only rollout workflows quarantined behind explicit opt-in `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`:
  - `ExpertExtractionWorkflow`
  - `ExpertDecomposedExtractionWorkflow`
  - `ExpertProjectionWorkflow`
  - `ExpertSemanticSliceWorkflow`
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

### Post-Gate 10 Tranche Order

After `Gate 10` is green, the execution order is fixed:

1. close certification drift with an accepted local/CI baseline artifact;
2. introduce truth-governance policy:
   - source independence
   - source trust weighting
   - authority override
   - freshness policy
3. only then remove regex as final authority from the expert truth path.

The single governance satellite for those rules is:

- [V6_Truth_Governance_Policy.md](V6_Truth_Governance_Policy.md)

Mandatory consequences:

- certification remains proof-only and never writes authority truth;
- corroboration must respect `independence_group_key`, not raw source count;
- single-source verification is allowed only through explicit override policy;
- regex may remain as heuristic or utility support, but not as final authority for `verified` truth.

Current accepted state:

- `Phase A / Knowledge Certification` is now accepted locally with baseline artifact at [docs/runs/truth_certification_local_ci_baseline.json](/home/bose/projects/alegria-site/docs/runs/truth_certification_local_ci_baseline.json);
- the accepted baseline now covers 16 fixtures, including explicit governance-proof cases for non-independent mirror corroboration, weak-source corroboration rejection, single authoritative override, override denial for non-authoritative source classes, and stale primary-authority override blocking;
- `automation/run_truth_certification_gate.sh` is the canonical local/CI certification wrapper;
- the canonical wrapper now runs the suite in an isolated Cargo target dir so certification proof does not depend on contaminated shared test artifacts;
- `automation/check_truth_certification_regression.py` is the canonical truth-diff gate against that accepted baseline;
- `automation/ci_verify.sh` now includes the truth certification regression gate;
- `Phase B / Truth Governance` is now accepted locally against the frozen certification baseline, with source independence, weak-source rejection, freshness blocking, and explicit authority-override checks wired into truth adjudication and CI policy checks;
- `Phase C / Regex De-Authority` has now started with the accepted `canonical_mapping_step` slice, where regex-symbolic acceptance was replaced by explicit typed-entity and lexicon-token mapping while the frozen certification baseline remained green;
- `Phase C / Regex De-Authority` is now accepted locally end-to-end against the frozen certification baseline;
- regex no longer acts as final authority inside `canonical_mapping_step`, `completeness_judge_step`, `procedural_extraction_step`, or `entity_span_detection_step`;
- remaining regex usage is restricted to utility or non-authoritative hint surfaces only.

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

### 6.47

- closed the legacy-surface policy tranche by accepting permanent narrow diagnostic retention for the four `Expert*Workflow` surfaces instead of leaving their fate as an unresolved future deletion question;
- tightened the convergence-policy artifact and automation so docs, starter, worker registration, and CI all agree that these workflows stay opt-in diagnostics only and never become a hidden forward-path fallback;
- updated the 10/10 completion plan so the remaining open blockers are now operational evidence and live-provider closure, not unresolved legacy-surface policy.

### 6.46

- formalized local env operational evidence bundling as a wrapper around existing proof surfaces instead of adding a new runtime gate or workflow path;
- added `automation/run_local_operational_evidence_bundle.sh` to aggregate clean acceptance, release/restore gate, legacy replay evidence, and live-provider evidence into one machine-readable env artifact;
- added `automation/check_local_operational_evidence_schema.py` and committed the accepted local artifact `docs/runs/local_operational_evidence_bundle.json`, with `BLOCKED_ON_LIVE_PROVIDER` documented as the honest local verdict while real provider credentials remain an external blocker.

### 6.45

- formalized build hygiene for proof surfaces by introducing an explicit build-residue cleanup script and a clean acceptance bundle that runs `cargo check`, the whole-page semantic gate, the truth-certification gate, and supporting policy/doc checks against isolated target roots;
- parameterized the truth-certification gate by target-root environment variables so clean acceptance runs do not need to share build artifacts with day-to-day development or with other gates;
- documented the accepted rule that `app/rust/target/` is disposable cache state rather than evidence, while `docs/runs/**` remains the canonical evidence surface.

### 6.44

- closed the remaining whole-page semantic hardening tranche by adding a dedicated machine-readable fixture pack, baseline artifact, and regression gate for `whole_page_semantic_pass`;
- replaced flat whole-text country/visa/authority promotion with weighted page-framing heuristics that consider URL, headings, primary content, and noise sections separately, so footer/menu noise no longer wins over true content framing;
- strengthened deterministic layer scoring for short English procedural/editorial pages and wired the whole-page fixture gate into `automation/ci_verify.sh` through an isolated target-dir runner.

### 6.43

- added noisy whole-page semantic hardening coverage for footer-heavy content pages, utility/login pages, menu-directory pages, and mixed procedural/editorial pages so page-level routing regressions are caught before they can weaken downstream semantic routing;
- mirrored the same utility and mixed-page expectations in the diagnostic extraction-core test surface so canonical runtime and debug-only semantics remain aligned during future whole-page hardening work.

### 6.42

- expanded the `whole_page_semantic_pass` advisory prototype set across mixed, utility, menu-directory, SEO, commercial, and additional country/authority page families so `voyage-4-large` similarity hints are less brittle under real page variety;
- added advisory calibration knobs for minimum similarity score and hit-limit selection, and tightened the intended safety rule that low-score advisory hits must not change fused page-mode confidence or promote mixed-section ids without section evidence;
- added explicit fusion-policy tests for low-score no-op behavior and for mixed-section pressure that lacks section-level support, keeping the advisory lane explainable and non-authoritative.

### 6.41

- activated a non-authoritative `voyage-4-large` advisory retrieval lane inside `whole_page_semantic_pass` by building a page sketch, retrieving nearest whole-page prototypes from a dedicated Qdrant collection, and fusing only conservative diagnostics back into the page scaffold;
- kept the 56-step ledger unchanged and preserved the rule that whole-page semantics may widen routing analysis and confidence reporting but may not write truth, approve canonical mapping, gate publish, or trigger rebuild on their own;
- aligned the diagnostic extraction-core mirror to the richer whole-page output shape so canonical runtime and debug surfaces do not diverge on page-level semantic fields.

### 6.40

- aligned the active `whole_page_semantic_pass` runtime payload with the documented V5/V6 contract by emitting deterministic `page_context_profile` and mixed-section hints in both the canonical activity path and the diagnostic extraction core mirror;
- removed stale working-plan wording that still implied `layer_router` and `triple_builder` were merely prototype-only despite already being mandatory parts of the accepted canonical flow;
- added the explicit `V6 10/10 Completion Plan` satellite so the remaining closure work is tracked as code-ready integration, fresh implementation, or external blockers instead of being scattered across owner and ops docs.

### 6.39

- removed the last live-doc references that still described the canonical 56-step flow through legacy `SeoSiteBuildWorkflow` wording in the support-process registry and superseded gap-closure snapshot;
- extended doc-identity enforcement to cover the support-process registry and to fail if superseded reference docs present legacy workflow identity as current production truth.

### 6.34

- expanded the accepted truth-certification baseline from 15 to 16 fixtures by adding a full-flow freshness proof for stale primary-authority override blocking;
- fixed the cutover candidate-validation and adjudication path so stale deterministic procedural candidates keep freshness-derived `needs_hitl` semantics instead of being silently promoted or downgraded to generic rejection;
- refreshed the immutable baseline artifact and kept the full certification suite green against the stronger freshness proof pack.

### 6.35

- activated `Graph Reasoning Tranche 1` inside canonical planning steps `38-41` without changing the 56-step ledger or introducing a new workflow type;
- added planning-only `GraphPlanningContext` loading, richer planning provenance on `keyword_clusters`, `content_gaps`, and `link_recommendations`, and deterministic graph-aware planning tests while keeping truth authority, draft gating, publish semantics, and rebuild policy unchanged;
- fixed the runtime regression in `load_graph_planning_context` by decoding `site.link_recommendations.score` through an explicit `double precision` read path so `global_site_reconcile` remains certifiably green under the frozen truth baseline.

### 6.36

- hardened the Step 5 live-provider operational lane so missing truth-provider credentials are classified immediately as `PENDING_CREDENTIALS` instead of spending a full smoke run only to fail at `blocked:no_truth_extraction_provider`;
- added canonical wrapper `automation/run_live_provider_minimal_scope_gate.sh` so provider preflight, live smoke, and evidence-schema validation run through one machine-usable entrypoint;
- updated the runbook to treat Step 5 as an explicit operational gate with fast credential classification, while preserving the existing evidence artifact and without weakening live smoke semantics once credentials exist.

### 6.37

- removed the remaining live-doc wording that equated the canonical 56-step value stream with legacy `SeoSiteBuildWorkflow`; the accepted forward execution identity is now stated consistently as `SeoSiteBuildCanonicalCutoverWorkflow`;
- tightened reference-doc framing so historical mentions of `SeoSiteBuildWorkflow` remain clearly historical and cannot be misread as the current active path;
- extended doc-identity enforcement to keep `INDEX`, `V6` owner docs, and reference execution snapshots aligned on active-versus-historical workflow naming.

### 6.38

- wired the canonical Step 5 live-provider gate into `automation/temporal_production_gate.sh`, so production release can no longer pass while the real live-provider path is merely `PENDING_CREDENTIALS`;
- updated production-gate docs to treat `bash automation/run_live_provider_minimal_scope_gate.sh` as part of release discipline rather than as a detached operator helper;
- extended support-surface enforcement so the production gate must invoke the canonical live-provider wrapper instead of relying on indirect provider wording alone.

### 6.33

- expanded the accepted truth-certification baseline from 13 to 15 fixtures by adding full-flow governance proofs for weak-source corroboration rejection and override denial on non-authoritative source classes;
- kept the full suite green under the stronger governance proof pack and refreshed the immutable baseline artifact accordingly;
- clarified that the newly accepted fixture growth strengthens trust/override coverage, while freshness-specific full-flow proof remains a separate follow-up if runtime semantics are tightened further.

### 6.32

- expanded the accepted truth-certification baseline from 11 to 13 fixtures by adding governance-proof cases for non-independent mirror corroboration and single authoritative override;
- refreshed legacy fixture expectations so governance reason codes now match accepted source-independence and authority-class semantics instead of pre-governance single-source wording;
- moved the canonical truth-certification suite onto an isolated Cargo target dir inside `automation/run_truth_certification_gate.sh`, eliminating the shared-target `rustls` compile instability from the official proof path.

### 6.31

- accepted the final `entity_span_detection_step` regex de-authority slice by replacing regex-based mention detection with deterministic token and phrase parsing;
- closed `Phase C / Regex De-Authority` locally after the frozen truth-certification baseline stayed green across all four accepted slices;
- clarified that regex remains allowed only in utility and non-authoritative hint surfaces, not as final authority anywhere in the expert truth path.

### 6.30

- accepted the `procedural_extraction_step` regex de-authority slice by switching procedural rule formation from raw-text regex/substrings to structured mentions supplied by `entity_span_detection`;
- kept orchestration and truth-certification behavior stable by validating the slice against the frozen baseline before acceptance;
- reduced the remaining Phase C regex-authority backlog to a single pending step: `entity_span_detection_step`.

### 6.29

- accepted the `completeness_judge_step` regex de-authority slice by switching completeness comparison from raw-text regex rescans to upstream numeric evidence tokens from `entity_span_detection`;
- changed the canonical truth-certification wrapper so its transient current report defaults to `/tmp/truth_certification_local_ci_current.json` instead of polluting `docs/runs/**`;
- kept the frozen truth-certification baseline green after the completeness slice, so the remaining pending regex-authority steps are now only `procedural_extraction_step` and `entity_span_detection_step`.

### 6.28

- started `Phase C / Regex De-Authority` with the first accepted slice in `canonical_mapping_step`;
- removed regex-symbolic final authority from canonical mapping and replaced it with explicit typed-entity plus lexicon-token mapping;
- revalidated the frozen truth-certification baseline after that slice so Phase C remains proof-driven rather than refactor-driven;
- left the remaining regex-authority slices (`completeness_judge_step`, `procedural_extraction_step`, `entity_span_detection_step`) explicitly pending behind the same certification gate.

### 6.27

- accepted `Phase B / Truth Governance` locally after wiring governance adjudication into both persisted and workflow truth-adjudication paths and keeping the frozen certification baseline green;
- added explicit machine-checked governance protections for weak-source corroboration rejection, freshness blocking reason codes, and authority-override denial for non-authoritative source classes;
- added `automation/check_truth_governance_policy.py` to `automation/ci_verify.sh` so the governance satellite, schema fields, policy engine markers, and runtime wiring are enforced together;
- clarified that `Phase C / Regex De-Authority` remains the next blocked tranche and must be validated against the frozen certification baseline before merge.

### 6.26

- accepted `Phase A / Knowledge Certification` by calibrating the full certification fixture pack and capturing the immutable baseline artifact at `docs/runs/truth_certification_local_ci_baseline.json`;
- fixed draft certification allow-cases by aligning per-page required-section behavior with blueprint-required sections and by seeding explicit preverified support where fixtures required publish-ready draft coverage;
- added certification diagnostics for temporal step multisets, semantic draft counts, and missing required factual support refs to keep future calibration work machine-auditable;
- wired `automation/run_truth_certification_gate.sh` into `automation/ci_verify.sh` so truth-diff regression is mandatory in local/CI before further governance or regex-authority work lands;
- clarified that `Phase B` is frozen-prepared rather than accepted behavior and that `Phase C` stays blocked behind the certification baseline.

### 6.25

- added the fixed post-certification tranche order: certification closure, then truth governance, then regex de-authority;
- linked the new `V6_Truth_Governance_Policy.md` satellite as the single policy surface for source independence, trust weighting, authority override, freshness, and regex-boundary rules;
- clarified under `Gate 10` that certification is proof-only and cannot become a second truth path.

### 6.20

- accepted `Phase M2` with a strict machine-readable full-flow report at `docs/runs/seo_cutover_shadow_verify_2026-05-22_full_acceptance.json`, using legacy run `4bbb4100-5013-4e52-a307-0b74f882b2fd` and cutover run `35bbdec3-04f5-4f68-9035-df6ea9a128d1` on the shared `ES|tourist||BY` acceptance fixture;
- fixed the publish-tail runtime bug where `cms_request_review` could still see stale `draft.qa_verdict=not_run` after `draft_qa`, by propagating the updated QA verdict through the scenario path, legacy workflow, and canonical cutover workflow before CMS review/publish steps;
- extended `SeoCutoverShadowVerify` to validate `Phase M2` publish/HITL/rebuild semantics from stable run-scoped `pipeline.step_executions` evidence, instead of relying on mutable CMS event rows that can be overwritten by later runs on the same page/revision keys;
- updated the working-plan status so `SeoSiteBuildCanonicalCutoverWorkflow` is now accepted through canonical `steps 1-56`, while `SeoSiteBuildWorkflow` remains present only for drain/compat and not as the forward architecture target.

### 6.21

- switched convergence wording from "future replacement candidate" to "accepted forward path" now that `SeoSiteBuildCanonicalCutoverWorkflow` is runtime-accepted through canonical `steps 1-56`;
- updated active-surface status so `SeoSiteBuildCanonicalCutoverWorkflow` is the active forward workflow, while `SeoSiteBuildWorkflow` is documented only as compat/drain and replay-safe legacy support;
- reclassified the `Expert*Workflow` family everywhere in this working plan as migration-only diagnostic surfaces eligible for retirement after full convergence, not as current promotion targets.

### 6.22

- quarantined the `Expert*Workflow` family behind explicit worker/starter opt-in `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`, so canonical execution cannot accidentally fall back to migration-only diagnostic surfaces;
- updated the legacy/quarantine status block to reflect that these workflows no longer wait on cutover evidence and now remain only as explicit diagnostics during the compat/drain window;
- extended runtime-policy checks so worker registration and starter launch both fail closed for `Expert*Workflow` surfaces unless the explicit migration-diagnostics env flag is present.

### 6.24

- removed stale pre-cutover wording that still claimed rich semantic, truth, and projection stages were "not yet active as mandatory runtime" even after accepted canonical `steps 1-56` coverage;
- clarified that `raw_knowledge_ingestion` macro-step language now applies only to legacy compat/drain understanding, while the accepted forward path is `SeoSiteBuildCanonicalCutoverWorkflow`;
- aligned deferred/steady-state wording so remaining work now refers to provider configuration, legacy surface retirement, and deeper graph reasoning rather than to already accepted canonical workflow phases.

### 6.23

- formalized the `SeoSiteBuildWorkflow` compat/drain window with explicit closure criteria tied to legacy replay evidence and replay/drain signoff, rather than leaving the drain end-state implicit;
- fixed the post-cutover naming decision: keep `SeoSiteBuildCanonicalCutoverWorkflow` as the runtime workflow type after drain and do not introduce a new workflow type purely for cosmetic rename;
- recorded a machine-readable convergence policy artifact at `docs/runs/seo_cutover_convergence_policy_2026-05-22.json` and wired a dedicated automation check so the drain window, `Expert*` diagnostic-only status, and naming decision remain enforced.

### 6.19

- extended `SeoSiteBuildCanonicalCutoverWorkflow` through canonical `Phase M2` draft/publish/rebuild steps `43-56`;
- promoted previously hidden or merged publish gates into explicit canonical steps by adding wrappers for `truth_admissibility_gate`, `human_approval_wait`, `cms_request_review`, and `cms_publish_approved`, plus `projection_barrier(publish)`;
- advanced the cutover workflow from planning-only `steps 1-42` coverage to full runtime wiring of canonical `steps 1-56`, while keeping `Phase M2` acceptance pending runtime evidence for publish/HITL/rebuild semantics.

### 6.18

- removed the dead `WorkflowStatus` runtime type instead of suppressing the warning, so `temporal_worker` now compiles without the stale unused-status warning;
- extended `SeoSiteBuildCanonicalCutoverWorkflow` through canonical `Phase M2` planning steps `37-42`: `serp_normalize`, `opportunity_build`, `ia_build`, `link_recommend`, `global_site_reconcile`, and `projection_barrier(global_site_reconcile)`;
- updated the execution state so the current cutover edge is `steps 1-42`, while the next non-accepted checkpoint moves to draft/publish `steps 43-55` and final `rebuild_detect`.

### 6.17

- accepted `Phase M1` by capturing a real green paired-run shadow report at `docs/runs/seo_cutover_shadow_verify_2026-05-22_phase_m1_accepted.json`;
- recorded the accepted legacy/cutover run ids and updated the partial-runtime status to show `projection_barrier(semantic_projection)` clear with `blocked_events=0`;
- documented that the `Qdrant` projection blocker was cleared by moving canonical cutover projection sync onto real run-scoped materialization and by enforcing deterministic valid Qdrant point ids;
- advanced the next non-accepted cutover checkpoint from `Phase M1` shadow acceptance to `Phase M2` starting at canonical `steps 37-42`.

### 6.15

- added `cli_tools SeoCutoverShadowVerify` as the machine-readable shadow verification surface for comparing legacy runtime and `SeoSiteBuildCanonicalCutoverWorkflow` by `run_id`;
- clarified that the remaining non-accepted `Phase M1` checkpoint is not inventing a shadow plan, but producing real paired-run shadow evidence before `Phase M2`;
- recorded the shadow verification surface in the active/partial runtime status and support-surface inventory.

### 6.16

- captured the first real paired-run `Phase M1` shadow evidence at `docs/runs/seo_cutover_shadow_verify_2026-05-21_phase_m1.json`;
- recorded the exact shadow baseline run ids for legacy and cutover execution so recovery does not depend on chat context;
- advanced the next non-accepted checkpoint from "produce shadow evidence" to "clear the `Qdrant` projection compatibility blocker that keeps `projection_barrier(semantic_projection)` blocked";
- documented that canonical cutover `steps 1-36` ledger coverage is complete, while `Phase M1` acceptance remains blocked by runtime projection compatibility rather than by missing workflow steps.

### 6.14

- extended `SeoSiteBuildCanonicalCutoverWorkflow` through canonical `steps 32-36`;
- wired `graph_admissibility_gate`, `retrieval_admissibility_gate`, `neo4j_sync`, `voyage_qdrant_sync`, and `projection_barrier(semantic_projection)` into the cutover runtime;
- updated the working plan status so `Phase M1` now reaches canonical `steps 1-36`, and the next required checkpoint is `shadow verification` before `Phase M2`.

### 6.13

- extended `SeoSiteBuildCanonicalCutoverWorkflow` from semantic routing into canonical `steps 18-31`;
- added first-class cutover runtime surfaces for `procedural_extraction`, `operational_extraction`, `editorial_extraction`, `seo_signal_extraction`, `commercial_signal_extraction`, `extraction_schema_validate`, `candidate_validation`, `triple_builder`, `completeness_judge`, `resolution_loop`, `contradiction_gate`, `truth_adjudication`, `verified_truth_write`, and support-bundle refresh after truth change;
- updated the working plan status so the next non-accepted Phase M1 target is now `steps 32-36`.

### 6.12

- extended `SeoSiteBuildCanonicalCutoverWorkflow` from the raw-evidence slice into canonical `steps 13-17`;
- added first-class cutover runtime surfaces for `layer_router`, `subspan_layer_router`, `entity_span_detection`, `canonical_mapping`, and `ontology_intake_gate`;
- updated the working plan status so the next non-accepted Phase M1 target is now `steps 18-31`.

### 6.11

- added the first real `SeoSiteBuildCanonicalCutoverWorkflow` runtime surface;
- wired the first cutover slice for `seo_preflight`, support loading, source discovery, crawl, semantic page prelude, sectioning, sectioning contract, CAS, raw evidence registration, and `projection_barrier(raw_evidence)`;
- updated the working plan status to record that the cutover workflow now exists as a partial runtime surface rather than only a planned replacement candidate.

### 6.10

- added a `Retired WIP preservation rule` so the useful orchestration lessons from deleted `ExpertRichFlowWorkflow` are preserved without keeping that workflow identity alive;
- added a `Clean-development baseline` that fixes what constitutes a clean starting point before further Phase M1 coding;
- clarified that reusable WIP achievements must be recovered from accepted rollout surfaces and the working plan, not by resurrecting the retired workflow.

### 6.9

- renamed the replacement candidate from `SeoSiteBuildV2Workflow` to `SeoSiteBuildCanonicalCutoverWorkflow` to avoid false `v1/v2` product-version semantics;
- added a `Context-loss recovery rule` so implementation can safely resume from the plan, status ledger, and git state without relying on conversational memory;
- removed `ExpertRichFlowWorkflow` from the intended cutover direction and kept the workflow family classification focused on the four explicitly migration-only `Expert*Workflow` surfaces.

### 6.8

- added a normative `Execution protocol for Phase M1` under `Phase M`;
- fixed the required implementation order for canonical cutover into slices `1-11`, `12-17`, `18-31`, `32-36`, then shadow verification;
- added a mandatory step completion status model: `doc_defined`, `typed`, `ledgered`, `wired`, `tested`, `shadow_verified`, `accepted`;
- added a no-skip rule so later slices cannot be declared complete while earlier canonical step coverage is still missing.

### 6.7

- added `Phase M - Canonical Cutover To One Workflow` under `Detailed Execution Program`;
- clarified the distinction between the full 56-step canonical target and the shorter Phase M1 cutover for canonical steps `1-36`;
- classified `Expert*Workflow` rollout surfaces as migration-only rather than long-term target architecture;
- introduced `SeoSiteBuildV2Workflow` as the single replacement candidate for future canonical cutover.

### 6.4

- clarified that the 56 steps are the canonical `SeoSiteBuildWorkflow` value stream, not every process in the repository;
- added a full project coverage map that reconciles source discovery, crawl, expert extraction, ontology, graph/retrieval, planning, drafting, publish, privacy/licensing/content-safety gates, rebuild, runtime safety, schema/contracts, ops resilience, and legacy/lab surfaces;
- added a missing-process classification rule so future capabilities are either placed inside the workflow, attached as support-plane gates, kept in release ops, or quarantined.
- clarified that dormant rich-step source files are not active functionality until exported, contract-tested, registered, and invoked.

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
