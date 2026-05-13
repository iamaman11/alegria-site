# V5 SEO Runtime Step Contracts Spec

**Status:** build-spec draft; implementation status: partial live proto/runtime/workflow surface  
**Owner:** exact runtime contracts for all SEO operations

---

## 1. Purpose

This document defines the exact runtime contracts for all SEO pipeline steps.

This is the canonical owner of:

- step names
- typed Proto message names
- exact `StepContractMeta` expectations
- input/output contracts
- idempotency basis
- retry classes
- failure classes
- HITL pause conditions
- storage writes
- outbox writes
- downstream consumers

This document refines, but does not replace, the runtime invariants from:

- `automation/NEW_OPERATION_PLAYBOOK.md`
- `automation/check_step_execution_contract.py`
- `automation/check_step_contract_completeness.py`
- `docs/STEP_CATALOG_CONTRACT.md`

---

## 2. Common Runtime Contract

Every SEO runtime step must have:

- `step_name`
- `step_version`
- `entity_type`
- `entity_id`
- typed Proto input and output messages
- deterministic `input_hash`
- deterministic `output_hash`
- deterministic `idempotency_key`
- explicit `retry_class`
- explicit `max_retries`
- explicit failure-class mapping
- metrics emission
- ledger-backed execution through `execute_step(...)`

### 2.1 Required `StepContractMeta`

For all SEO steps:

| Field | Value shape |
|---|---|
| `step_name` | exact string from section 4 |
| `step_version` | semver text, starting at `1.0.0` |
| `entity_type` | one of `serp_batch`, `opportunity`, `page`, `draft`, `review_task` |
| `input_schema` | `alegria.temporal.v1.<MessageName>` |
| `output_schema` | `alegria.temporal.v1.<MessageName>` |
| `retry_class` | `safe`, `transient`, `never`, or `hitl_only` |
| `max_retries` | integer >= 0 |
| `requires_hitl` | boolean |
| `pipeline_version` | current SEO pipeline version |
| `registry_version` | current active SEO registry version |

### 2.2 Common failure classes

Allowed SEO failure classes:

- `schema_error`
- `invalid_scope`
- `missing_dependency`
- `stale_source`
- `forbidden_signal`
- `quality_gate_failed`
- `publish_blocked`
- `transient_io_failure`
- `hitl_required`

---

## 3. Proto Message Inventory

Messages to add in `app/contracts/proto/temporal_payloads.proto`:

- `SerpIngestInputPayload`
- `SerpIngestOutputPayload`
- `SerpNormalizeInputPayload`
- `SerpNormalizeOutputPayload`
- `OpportunityBuildInputPayload`
- `OpportunityBuildOutputPayload`
- `IaBuildInputPayload`
- `IaBuildOutputPayload`
- `LinkRecommendInputPayload`
- `LinkRecommendOutputPayload`
- `DraftAssembleInputPayload`
- `DraftAssembleOutputPayload`
- `DraftQaInputPayload`
- `DraftQaOutputPayload`
- `RebuildDetectInputPayload`
- `RebuildDetectOutputPayload`
- `SeoReviewResolutionInputPayload`
- `SeoReviewResolutionOutputPayload`
- `SeoHitlTaskContextPayload`
- `SeoHitlDecisionPayload`

Required DTO families in `runtime_models`:

- `SeoScopeState`
- `KeywordClusterState`
- `PageNodeState`
- `PageBlueprintState`
- `PageBriefState`
- `DraftState`
- `LinkRecommendationState`
- `CannibalizationConflictState`
- `ContentGapState`
- `SearchFeatureState`
- `SectionTemplateState`

---

## 4. Step Catalog

## 4.1 `serp_ingest`

| Contract field | Value |
|---|---|
| `step_name` | `serp_ingest` |
| `purpose` | persist raw SERP query batch and enqueue crawl candidates |
| `entity_type` | `serp_batch` |
| `trigger` | external SERP batch arrival |
| `input_proto` | `SerpIngestInputPayload` |
| `output_proto` | `SerpIngestOutputPayload` |
| `idempotency_key` | `serp_ingest:<batch_key>:<input_hash>` |
| `retry_class` | `transient` |
| `max_retries` | `3` |
| `HITL` | no |
| `storage_writes` | `serp.query_batches`, existing `serp.raw_snapshots`, existing `serp.crawl_queue` |
| `outbox_writes` | none |
| `downstream_consumers` | `serp_normalize` |

Input minimum fields:
- `batch_key`
- `market`
- `locale`
- `country_code`
- `visa_type`
- `applicant_profile`
- `scope_signature`
- `query_seed_set`
- `raw_snapshot_refs[]`

Output minimum fields:
- `batch_key`
- `persisted_snapshot_count`
- `crawl_queue_write_count`
- `status`

## 4.2 `serp_normalize`

| Contract field | Value |
|---|---|
| `step_name` | `serp_normalize` |
| `purpose` | normalize SERP and competitor observations into typed intelligence records |
| `entity_type` | `serp_batch` |
| `trigger` | successful `serp_ingest` |
| `input_proto` | `SerpNormalizeInputPayload` |
| `output_proto` | `SerpNormalizeOutputPayload` |
| `idempotency_key` | `serp_normalize:<batch_key>:<input_hash>` |
| `retry_class` | `safe` |
| `max_retries` | `1` |
| `HITL` | only if batch contains malformed or forbidden mixed-source evidence |
| `storage_writes` | `serp.competitor_pages`, `serp.competitor_section_patterns`, `serp.serp_patterns`, `serp.serp_pattern_observations` |
| `outbox_writes` | none |
| `downstream_consumers` | `opportunity_build` |

## 4.3 `opportunity_build`

| Contract field | Value |
|---|---|
| `step_name` | `opportunity_build` |
| `purpose` | derive content gaps and opportunity candidates from normalized SERP signals |
| `entity_type` | `opportunity` |
| `trigger` | successful `serp_normalize` |
| `input_proto` | `OpportunityBuildInputPayload` |
| `output_proto` | `OpportunityBuildOutputPayload` |
| `idempotency_key` | `opportunity_build:<batch_key>:<input_hash>` |
| `retry_class` | `safe` |
| `max_retries` | `1` |
| `HITL` | yes when only `R1_weak` support exists for a candidate requested for promotion |
| `storage_writes` | `site.content_gaps`, `serp.opportunity_candidates` |
| `outbox_writes` | none |
| `downstream_consumers` | `ia_build` |

## 4.4 `ia_build`

| Contract field | Value |
|---|---|
| `step_name` | `ia_build` |
| `purpose` | create or update `PageNode`, `KeywordCluster`, and `PageBlueprint` assignment decisions |
| `entity_type` | `page` |
| `trigger` | accepted opportunity candidates or rebuild request |
| `input_proto` | `IaBuildInputPayload` |
| `output_proto` | `IaBuildOutputPayload` |
| `idempotency_key` | `ia_build:<scope_signature>:<input_hash>` |
| `retry_class` | `safe` |
| `max_retries` | `1` |
| `HITL` | yes for unresolved dominant intent or canonical URL conflict |
| `storage_writes` | `site.keyword_clusters`, `site.page_nodes`, `site.page_blueprints`, `site.cannibalization_conflicts` |
| `outbox_writes` | none |
| `downstream_consumers` | `link_recommend`, `draft_assemble` |

## 4.5 `link_recommend`

| Contract field | Value |
|---|---|
| `step_name` | `link_recommend` |
| `purpose` | generate required and optional internal link recommendations |
| `entity_type` | `page` |
| `trigger` | successful `ia_build` or rebuild |
| `input_proto` | `LinkRecommendInputPayload` |
| `output_proto` | `LinkRecommendOutputPayload` |
| `idempotency_key` | `link_recommend:<page_node_key>:<input_hash>` |
| `retry_class` | `safe` |
| `max_retries` | `1` |
| `HITL` | no, unless blocked by unresolved cannibalization conflict |
| `storage_writes` | `site.link_recommendations` |
| `outbox_writes` | none |
| `downstream_consumers` | `draft_assemble` |

## 4.6 `draft_assemble`

| Contract field | Value |
|---|---|
| `step_name` | `draft_assemble` |
| `purpose` | assemble a page brief and draft from verified truth plus approved SEO inputs |
| `entity_type` | `draft` |
| `trigger` | `page_node` with valid blueprint and IA state |
| `input_proto` | `DraftAssembleInputPayload` |
| `output_proto` | `DraftAssembleOutputPayload` |
| `idempotency_key` | `draft_assemble:<page_node_key>:<truth_snapshot_ref>:<input_hash>` |
| `retry_class` | `safe` |
| `max_retries` | `1` |
| `HITL` | no during assembly |
| `storage_writes` | `site.page_briefs`, `site.page_drafts` |
| `outbox_writes` | none |
| `downstream_consumers` | `draft_qa` |

## 4.7 `draft_qa`

| Contract field | Value |
|---|---|
| `step_name` | `draft_qa` |
| `purpose` | apply machine QA gates to assembled draft |
| `entity_type` | `draft` |
| `trigger` | successful `draft_assemble` |
| `input_proto` | `DraftQaInputPayload` |
| `output_proto` | `DraftQaOutputPayload` |
| `idempotency_key` | `draft_qa:<draft_key>:<input_hash>` |
| `retry_class` | `safe` |
| `max_retries` | `1` |
| `HITL` | yes when unsupported factual fragments, unsafe SERP references, or publish-blocking URL conflicts are found |
| `storage_writes` | `site.page_drafts`, `site.page_lifecycle_events`, `monitoring.seo_quality_failures` |
| `outbox_writes` | none |
| `downstream_consumers` | CMS publish path or `seo_review_resolution` |

## 4.8 `rebuild_detect`

| Contract field | Value |
|---|---|
| `step_name` | `rebuild_detect` |
| `purpose` | convert truth, ontology, IA, link, or freshness changes into rebuild backlog items |
| `entity_type` | `page` |
| `trigger` | change event or scheduled freshness check |
| `input_proto` | `RebuildDetectInputPayload` |
| `output_proto` | `RebuildDetectOutputPayload` |
| `idempotency_key` | `rebuild_detect:<page_node_key>:<trigger_type>:<input_hash>` |
| `retry_class` | `safe` |
| `max_retries` | `1` |
| `HITL` | yes only for requested rebuild suppression |
| `storage_writes` | `monitoring.seo_rebuild_backlog`, `site.page_lifecycle_events` |
| `outbox_writes` | none |
| `downstream_consumers` | `ia_build`, `draft_assemble`, `seo_review_resolution` |

Required trigger taxonomy for `rebuild_detect`:

- `truth_change`
- `profile_applicability_change`
- `scope_change`
- `template_change`
- `serp_change`
- `locale_change`

Required implementation rules:

- `truth_change` and `profile_applicability_change` may invalidate published support freshness and move pages to `needs_rebuild`
- `scope_change` and `locale_change` are scope-only rebuild triggers and must not be mislabeled as truth changes
- `rebuild_detect` output must include structured per-page trigger metadata, not only page keys
- persistence may enrich reasons from dependency storage, but must not silently rewrite the trigger taxonomy emitted by the use-case

## 4.9 `seo_review_resolution`

| Contract field | Value |
|---|---|
| `step_name` | `seo_review_resolution` |
| `purpose` | consume HITL decision and apply deterministic state update |
| `entity_type` | `review_task` |
| `trigger` | human resolution payload |
| `input_proto` | `SeoReviewResolutionInputPayload` |
| `output_proto` | `SeoReviewResolutionOutputPayload` |
| `idempotency_key` | `seo_review_resolution:<task_key>:<decision_key>` |
| `retry_class` | `hitl_only` |
| `max_retries` | `0` |
| `HITL` | step exists only after HITL |
| `storage_writes` | affected `site.*`, `serp.*`, `monitoring.*`, `pipeline.hitl_decisions` |
| `outbox_writes` | optional CMS review outcome or rebuild resume event |
| `downstream_consumers` | blocked SEO steps waiting on review |

---

## 5. Producer / Consumer Ownership

| Step | Primary producer | Primary consumers |
|---|---|---|
| `serp_ingest` | external ingestion boundary | `serp_normalize` |
| `serp_normalize` | runtime workflow | `opportunity_build` |
| `opportunity_build` | runtime workflow | `ia_build` |
| `ia_build` | runtime workflow | `link_recommend`, `draft_assemble` |
| `link_recommend` | runtime workflow | `draft_assemble` |
| `draft_assemble` | runtime workflow | `draft_qa` |
| `draft_qa` | runtime workflow | CMS publish path, review queue |
| `rebuild_detect` | runtime workflow or scheduled monitor | `ia_build`, `draft_assemble` |
| `seo_review_resolution` | HITL control plane | blocked steps and status transitions |

---

## 6. HITL Pause Conditions

HITL pause is mandatory when:

- dominant intent conflict cannot be resolved deterministically
- canonical URL conflict remains after IA rules
- SERP pattern below `R2_supported` is requested for promotion
- unsupported factual fragment appears in draft QA
- registry change is required before proceeding
- rebuild suppression is requested on a truth-driven rebuild

HITL task types are defined in `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`.

---

## 7. Metrics Contract Per Step

Each step must emit at least:

- `activity_attempts_total{step_name}`
- `activity_failures_total{step_name}`
- `activity_duration_seconds{step_name}`
- `step_execution_reused_total{step_name}`

Additional SEO step metrics:

- `seo_opportunity_candidates_total`
- `seo_page_nodes_created_total`
- `seo_link_recommendations_total`
- `seo_draft_traceability_failures_total`
- `seo_rebuild_backlog_total`
- `seo_hitl_pauses_total`

---

## 8. Registration Expectations

### Activities

Expected additions under `app/rust/services/temporal/src/activities/`:

- catalog and operation handlers for all step names above
- integration through existing `execute_step(...)` runtime path

### Workflows

Expected additions under `app/rust/services/temporal/src/workflows/`:

- `seo_site_generation`
- `seo_rebuild`
- `seo_review_resolution`

---

## 9. Acceptance Checks

This spec is complete only if:

- every runtime step has exact Proto names
- every runtime step has exact idempotency and retry semantics
- every step has explicit storage writes and downstream consumers
- HITL pause conditions are explicit
- metrics and registration expectations are explicit
- no step is left as prose-only behavior

---

## 10. Runtime Clarifications Addendum

### 10.1 Canonical hash algorithm

The canonical runtime hash algorithm for all SEO steps is `blake3`.

Required rules:

- `input_hash = blake3(canonical_proto_binary(input_message))`
- `output_hash = blake3(canonical_proto_binary(output_message))`
- `idempotency_key` is derived from deterministic step prefix plus canonical identity fields plus `input_hash`
- mixed hash algorithms are forbidden within the SEO runtime path

Canonical serialization basis:

- typed Proto binary with deterministic field ordering
- no ad hoc JSON serialization for hashing
- no locale-sensitive formatting

### 10.2 Scope normalization gate

For every step that receives scope-bearing payloads:

- raw scope tuple must already be normalized,
- `scope_signature` must match the normalized tuple,
- non-normalized inputs fail before any storage write,
- failure class is `invalid_scope_signature`.

### 10.3 HITL execution lifecycle

Blocked execution states:

- `paused`
- `waiting_for_resolution`
- `resumed`
- `reopened_after_resume`
- `expired`
- `cancelled`

Rules:

- each pause must record `pause_sequence` as a monotonic integer per execution
- a resume payload must target one concrete `execution_key + pause_sequence`
- repeated pauses are allowed only as new pause sequences
- a reopened task after resume invalidates the prior resume and forces a new pause sequence
- `expired` tasks may not auto-resume and require escalation

### 10.4 `draft_qa` output extension

`DraftQaOutputPayload` must include:

- `unsupported_fragments[]`
- `ambiguous_fragments[]`
- `traceability_verdict`
- `blocking_task_type | null`
- `blocking_reason_codes[]`

### 10.5 Failure-handling and partial-failure contract

| Failure case | Failure class | Runtime action |
|---|---|---|
| malformed SERP HTML | `malformed_source_capture` | quarantine capture, continue batch only if remaining evidence still satisfies minimum thresholds |
| incomplete competitor capture | `incomplete_capture_batch` | mark batch partial, block downstream export if required support falls below threshold |
| partial Neo4j projection failure | `projection_partial_failure` | retry safe projection subset, preserve source rows, alert monitoring |
| partial Qdrant projection failure | `retrieval_projection_partial_failure` | retry, keep source-of-record authoritative, block retrieval freshness promotion |
| CMS publish failure after approval | `cms_publish_failure` | preserve approved revision, emit failure event, do not mark page published |
| unresolved HITL timeout | `hitl_resolution_timeout` | escalate to owner, freeze blocked execution |

### 10.6 Runtime hard rules added by this addendum

1. All runtime hashing in the SEO pipeline uses `blake3`.
2. No scope-bearing step may persist invalid or non-normalized scope.
3. Every HITL pause must be resumable, expirable, or cancellable deterministically.
4. Partial projection failure may not mutate source-of-record semantics.

## 11. Phase 1 Runtime Outputs For The First End-To-End SEO Loop

This section defines the minimum runtime outputs that must exist before the first implementation can be considered a working expert SEO loop.

### 11.1 Phase 1 required canonical outputs in `Postgres`

The first end-to-end implementation must be able to persist all of the following canonical outputs:

- normalized `SERPPattern` records with reliability and freshness
- accepted `KeywordCluster` records with dominant intent
- accepted `PageNode` records with scope, dominant intent, canonical URL, and lifecycle state
- attached `PageBlueprint` references for every accepted `PageNode`
- generated `LinkRecommendation` records for each eligible page
- generated `PageBrief` records for each draft-eligible page
- generated `Draft` records with traceability manifests
- `DraftQaOutput` state with blocking and non-blocking findings
- `rebuild_detect` backlog records for invalidated pages

### 11.2 Phase 1 required projection outputs

The first end-to-end implementation must also produce:

- Neo4j projection for all Phase 1 mandatory nodes and relationships from `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`
- Qdrant population for all Phase 1 mandatory collections from `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`

A runtime step is not considered complete if its canonical source-of-record write succeeds but its required Phase 1 projection contract is absent without an explicit partial-failure state.

### 11.3 Phase 1 required consumer-visible outputs

A successful first loop must make the following outputs consumable by downstream steps and review flows:

- opportunity candidate list for IA acceptance
- accepted page structure inputs for linking and drafting
- page-level link recommendation set
- page brief package
- draft package with section-level traceability
- QA verdict package with blocking reasons
- publish-readiness decision package

### 11.4 Phase 1 loop-completeness rule

The first implementation is not considered end-to-end complete unless one deterministic pipeline execution can produce, for at least one valid scope:

1. SERP-derived planning signals,
2. accepted page-planning objects,
3. graph and retrieval projections,
4. draft and QA outputs,
5. a publishable-or-blocked verdict with explicit reasons.
