# V5 SEO Super-Site Implementation Roadmap

**Status:** implementation planning draft  
**Purpose:** decision-complete roadmap for turning the current V5 extraction platform into an expert-level SEO super-site application, grounded in the live codebase and automation invariants.

---

## 1. Grounding In Live Codebase

This roadmap is constrained by the actual application and automation rules already present in the repository.

### 1.1 Hard invariants already enforced by automation

From `automation/README.md`, `automation/OPS_CANONICALS.md`, `automation/00_ENTRYPOINT.md`, `automation/NEW_OPERATION_PLAYBOOK.md`, and the live checks in `automation/*.py`:

- `primitives = pure`
- `seo_steps = pure deterministic step library`, without raw SQL, raw JSON boundary logic, or direct SDK usage
- `infrastructure/adapters = the only IO / SQL / external SDK boundary`
- runtime payloads must be typed Proto messages, not ad hoc JSON
- every side-effectful runtime step must go through `pipeline.step_executions` and `pipeline.step_attempts`
- `blake3` is the canonical hash / idempotency / integrity mechanism
- production verdict requires:
  - `bash automation/ci_verify.sh`
  - `TEMPORAL_GATE_WORKER_MODE=docker bash automation/temporal_production_gate.sh`
  - `infra/backups/restore_drill.sh`

### 1.2 Current live baseline

Live source-of-truth code paths already exist:

- relational schema: `app/db/schema.sql`
- runtime Proto: `app/contracts/proto/temporal_payloads.proto`
- Rust layers:
  - `app/rust/crates/primitives`
  - `app/rust/crates/runtime_models`
  - `app/rust/crates/seo_steps`
  - `app/rust/crates/infrastructure/src/adapters`
  - `app/rust/services/temporal`

Current state relevant to SEO:

- `app/db/schema.sql` already has `serp.*` run ingestion tables and only one `site.*` mapping table: `site.page_context_map`
- `app/rust/crates/seo_steps/src/serp_ingest.rs` already exists, but only covers raw SERP ingest and crawl queueing
- extraction runtime steps already exist in `seo_steps/*_step.rs` and `services/temporal/src/activities/step_catalog.rs`
- there is no live typed SEO operating system yet for page planning, link recommendation, blueprinting, cannibalization, or draft QA

### 1.3 Core implementation constraint

The SEO super-site layer must be added as typed, runtime-owned, automation-gated application code. Markdown protocols are necessary but not sufficient.

---

## 2. Final Target State

The system is considered expert-level and super-site-ready only when all of the following are true:

- all SEO artifacts have live schema, typed runtime contracts, and runtime owners
- SEO runtime steps are registered in Temporal and enforced through step ledger / idempotency discipline
- Neo4j and Qdrant projections include SEO-derived objects without mixing them into truth-core
- draft assembly is blocked without verified traceability
- cannibalization and URL conflicts are detected before CMS publish
- linking and opportunity scoring are deterministic and testable
- CMS publication is fully gated and auditable
- HITL/backoffice flows exist for unsafe SERP patterns, unsupported claims, registry changes, and cannibalization conflicts
- evaluation harnesses and observability dashboards prove the system does not regress
- migration/backfill exists from current data to `PageNode` / `PageBlueprint` / related SEO artifacts

---

## 3. Implementation Principles

1. Do not bypass the existing layering formula.
2. Do not introduce SEO runtime state as JSON blobs when fields are stable.
3. Do not mix product-derived SEO artifacts into `verified.*` truth tables.
4. Do not add any new runtime operation without typed Proto, step ledger integration, metrics, and automation checks.
5. Do not treat companion docs as executable source of truth; executable source of truth must live in `app/` and automation gates.

---

## 4. Pre-Code Documentation Gate

Implementation is **No-Go** until all four build-spec documents exist and are linked from the master plan and this roadmap:

1. `V5_SEO_Live_Schema_Design_Spec.md`
2. `V5_SEO_Runtime_Step_Contracts_Spec.md`
3. `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`
4. `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`

The following work is blocked until the corresponding spec exists:

- SQL changes in `app/db/schema.sql`
- Proto changes in `app/contracts/proto/temporal_payloads.proto`
- new runtime DTO families
- new Temporal activities and workflows
- Neo4j projection changes
- Qdrant projection changes
- CMS publish path changes
- HITL/runtime review queue changes

The missing-spec set that blocks coding is:

- exact table / column / constraint specification
- exact step Proto and wire contracts
- exact graph / retrieval projection shapes
- exact CMS / HITL control-plane contracts

---

## 5. Sequential Delivery Plan

## Phase 0A. Lock architecture and ownership

### Goal

Freeze the owner model and architectural semantics before implementation detail is added.

### Deliverables

- lock the owner model from:
  - `docs/V5_SEO_Foundation_Contracts.md`
  - `docs/V5_SERP_Intelligence_Protocol.md`
  - `docs/V5_SEO_Information_Architecture_Protocol.md`
  - `docs/V5_SEO_Draft_Assembly_And_QA_Protocol.md`
  - `docs/V5_SEO_Operations_And_Optimization_Protocol.md`
- maintain the master plan and authority mapping
- extend automation documentation to include SEO-specific operation rules:
  - `automation/NEW_OPERATION_PLAYBOOK.md`
  - `automation/OPS_CANONICALS.md`
  - `automation/00_ENTRYPOINT.md`

### Acceptance

- every planned SEO artifact and runtime step has an explicit owner
- no planned artifact is left without owner, store family, runtime path, and gate path

---

## Phase 0B. Write build-spec documents

### Goal

Produce the implementation-level documentation package required before any live schema or runtime work begins.

### Required documents

- `docs/V5_SEO_Live_Schema_Design_Spec.md`
- `docs/V5_SEO_Runtime_Step_Contracts_Spec.md`
- `docs/V5_SEO_Graph_And_Retrieval_Projection_Spec.md`
- `docs/V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`

### Acceptance

Each build-spec document is incomplete until it includes:

- a normative table or matrix section
- exact entity and state definitions
- exact producer and consumer ownership
- blocking rules
- rebuild or migration semantics where relevant
- acceptance checks mapped to automation or future automation

Implementation remains No-Go until all four are present.

---

## Phase 1. Executable contracts and relational schema

### Goal

Move SEO contracts from docs into live typed contracts and SQL schema.

### Code paths

- `app/contracts/proto/temporal_payloads.proto`
- `app/rust/crates/contracts/build.rs`
- `app/rust/crates/contracts/src/lib.rs`
- `app/rust/crates/runtime_models/src/lib.rs`
- `app/db/schema.sql`

### New runtime payload families

Add typed payloads for:

- `SerpNormalizeInputPayload` / `SerpNormalizeOutputPayload`
- `OpportunityBuildInputPayload` / `OpportunityBuildOutputPayload`
- `IaBuildInputPayload` / `IaBuildOutputPayload`
- `LinkRecommendInputPayload` / `LinkRecommendOutputPayload`
- `DraftAssembleInputPayload` / `DraftAssembleOutputPayload`
- `DraftQaInputPayload` / `DraftQaOutputPayload`
- `RebuildDetectInputPayload` / `RebuildDetectOutputPayload`
- SEO-specific HITL payloads for cannibalization, unsupported draft claims, unsafe SERP patterns, registry changes

### New relational tables

Use existing schema families instead of inventing new ones without reason.

#### `site.*` canonical SEO operating objects

Add:

- `site.keyword_clusters`
- `site.page_nodes`
- `site.page_blueprints`
- `site.page_briefs`
- `site.page_drafts`
- `site.link_recommendations`
- `site.cannibalization_conflicts`
- `site.content_gaps`
- `site.search_features`
- `site.section_templates`
- `site.page_lifecycle_events`
- `site.registry_page_types`
- `site.registry_intent_types`
- `site.registry_anchor_strategies`
- `site.registry_section_templates`
- `site.registry_cta_patterns`

#### `serp.*` intelligence-derived structures

Add:

- `serp.query_batches`
- `serp.serp_patterns`
- `serp.serp_pattern_observations`
- `serp.competitor_pages`
- `serp.competitor_section_patterns`
- `serp.opportunity_candidates`

#### `monitoring.*` SEO operational signals

Add:

- `monitoring.seo_metric_snapshots`
- `monitoring.seo_freshness_alerts`
- `monitoring.seo_rebuild_backlog`
- `monitoring.seo_quality_failures`

### Required schema constraints

- deterministic primary keys for all persistent SEO artifacts
- unique canonical URL per `scope_signature + dominant_intent`
- explicit `scope_signature` on page-scoped artifacts
- lifecycle/status checks for drafts and conflicts
- explicit foreign keys between `page_nodes`, `page_blueprints`, `page_drafts`, `keyword_clusters`, `content_gaps`

### Acceptance

- all planned SEO artifacts exist in live schema
- typed runtime payloads compile from Proto
- no stable SEO payload remains undefined as ad hoc JSON
- `automation/check_step_execution_contract.py` and `automation/check_reliability_contracts.py` still pass after extension

---

## Phase 2. Deterministic primitives, runtime models, and adapters

### Goal

Implement the pure and boundary layers for SEO artifacts and scoring.

### Code paths

- `app/rust/crates/primitives/src/...`
- `app/rust/crates/runtime_models/src/lib.rs`
- `app/rust/crates/infrastructure/src/adapters/...`
- `app/rust/crates/seo_steps/src/...`

### New primitives

Create pure deterministic modules for:

- SEO artifact key derivation using `blake3`
- `scope_signature` derivation
- `opportunity_score`
- `link_score`
- freshness priority / rebuild priority scoring
- cannibalization conflict detection heuristics
- draft traceability labeling logic

### New runtime models

Introduce typed DTOs for:

- `KeywordCluster`
- `PageNode`
- `SERPPattern`
- `PageBlueprint`
- `PageBrief`
- `Draft`
- `LinkRecommendation`
- `CannibalizationConflict`
- `ContentGap`
- `SearchFeature`
- `SectionTemplate`

### New infrastructure adapters

Add or extend adapters for:

- SQLX CRUD over `site.*`, `serp.*`, `monitoring.*`
- Neo4j materialization for SEO-derived graph nodes/edges
- Qdrant collection projection for SEO retrieval objects
- CMS outbox / publication adapter
- HITL queue and review adapter

### Acceptance

- `primitives` remain pure and pass automation purity checks
- `seo_steps` do not import SQL/SDK/JSON boundary logic directly
- adapters contain all SQL/SDK code for SEO storage and projection

---

## Phase 3. Runtime steps and Temporal orchestration

### Goal

Turn the SEO layer into real runtime operations governed by the existing step ledger discipline.

### New use-case step modules

Add under `app/rust/crates/seo_steps/src/`:

- `serp_normalize_step.rs`
- `opportunity_build_step.rs`
- `ia_build_step.rs`
- `link_recommend_step.rs`
- `draft_assemble_step.rs`
- `draft_qa_step.rs`
- `rebuild_detect_step.rs`

### Extend existing use cases where appropriate

- evolve `serp_ingest.rs` into the canonical raw ingest entrypoint for SEO query batches
- connect SEO steps to existing `seo_application::seo_runtime`, `outbox_builder.rs`, application HITL orchestration, and reconciliation flows

### New Temporal activities

Extend:

- `app/rust/services/temporal/src/activities/step_catalog.rs`
- `app/rust/services/temporal/src/activities/operations.rs`
- possibly `content_generation.rs` if part of draft assembly path

Every step must define:

- `input_hash`
- `output_hash`
- `idempotency_key`
- `retry_class`
- typed output payload
- HITL pause/resume semantics where needed

### New workflows

Add explicit workflows under `app/rust/services/temporal/src/workflows/`:

- `seo_site_generation.rs`
- `seo_rebuild.rs`
- `seo_review_resolution.rs`

Reuse existing `freshness.rs` where freshness/rebuild signals already exist, instead of duplicating logic.

### Acceptance

- every new SEO step is ledger-backed
- no new step bypasses `execute_step(...)`
- activities and workflows are registered in Temporal worker
- runtime metrics cover attempts, failures, reuse, duration, and SEO-specific gate failures

---

## Phase 4. Neo4j and Qdrant projection layer

### Goal

Project SEO-derived objects into graph and retrieval stores without polluting truth-core.

### Neo4j projection

Extend graph projection with product-derived node labels and edges such as:

- `KeywordCluster`
- `PageNode`
- `PageBlueprint`
- `ContentGap`
- `SearchFeature`
- `SectionTemplate`

Add edges such as:

- `TARGETS_INTENT`
- `BELONGS_TO_CLUSTER`
- `SHOULD_LINK_TO`
- `GENERATED_FROM_BLUEPRINT`
- `HAS_CONTENT_GAP`
- `RISKS_CANNIBALIZATION_WITH`

Hard rule:

- SEO-derived nodes must be stored as product-derived graph objects and must never masquerade as verified rule truth.

### Qdrant collections

Create dedicated collections for SEO retrieval surfaces:

- `seo_keyword_clusters`
- `seo_page_blueprints`
- `seo_serp_patterns`
- `seo_section_templates`
- `seo_link_targets`

Do not overload existing truth collections such as `content_chunks`, `kb_canonical`, and `ontology`.

### Acceptance

- graph and Qdrant projections are rebuildable from canonical SQL stores
- SEO retrieval objects remain disjoint from truth-core retrieval objects
- automation and smoke checks prove that SEO artifacts do not promote truth status

---

## Phase 5. CMS contract and publish governance

### Goal

Make page publication deterministic, gated, and reversible.

### Deliverables

Define live CMS contract for:

- page identity and revision model
- draft status mapping
- canonical URL ownership rules
- publish / rollback behavior
- required metadata fields
- required link obligations
- allowed promotion path from `approved` to CMS write

### Implementation

- add typed CMS payloads and outbox events
- extend `system.sync_outbox` usage for SEO publication events
- add machine publish gate before CMS sync
- add owner approval path after machine gate and before publish

### Required publish blockers

- missing traceability
- unresolved cannibalization conflict
- missing required metadata
- invalid canonical URL
- missing required internal links
- stale or expired evidence beyond allowed freshness threshold

### Acceptance

- draft cannot reach CMS only because text exists
- publish path is ledger-backed, audited, and rollback-capable
- CMS events use typed payloads, not generic JSON runtime path

---

## Phase 6. Backoffice / HITL control plane

### Goal

Introduce the human control surface required by the SEO operating model.

### Required review queues

- cannibalization conflicts
- unsafe SERP patterns
- unsupported factual draft claims
- registry changes
- canonical URL conflict decisions
- rebuild suppression requests

### Implementation

- reuse `pipeline.hitl_tasks` / `pipeline.hitl_decisions` where possible
- add SEO-specific task types and typed payload context
- wire resolution flows into `seo_review_resolution` workflow
- persist review outcomes so rebuild/publish steps can consume them deterministically

### Acceptance

- each controversial SEO decision has a first owner and escalation path
- no human-only knowledge remains outside runtime state and audit trail

---

## Phase 7. Evaluation harness and regression suite

### Goal

Prove the system does not regress into page overlap, bad links, or unsupported drafts.

### Deliverables

Create gold datasets and regression suites for:

- SERP reliability classification
- opportunity ranking
- intent assignment
- page clustering
- link recommendation quality
- draft traceability labeling
- cannibalization detection
- rebuild triggering

### Automation extensions

Add new automation checks and smoke scenarios:

- `automation/check_seo_contract_boundaries.py`
- `automation/check_seo_artifact_schema.py`
- `automation/check_seo_runtime_steps.py`
- `automation/check_seo_traceability_contract.py`
- `automation/check_seo_publish_gates.py`
- `automation/check_seo_qdrant_collections.py`
- `automation/smoke_draft_gate_blocks_unsupported_claims.py`
- `automation/smoke_cannibalization_blocks_publish.py`
- `automation/smoke_rebuild_detect_from_truth_change.py`
- `automation/smoke_scope_signature_conflict_blocks_url_publish.py`

### Acceptance

- `bash automation/ci_verify.sh` includes SEO checks in the base verdict
- regression datasets are versioned and reproducible
- link recommendation and draft QA regressions become NO-GO signals

---

## Phase 8. Observability and operational dashboards

### Goal

Make freshness, rebuilds, QA, and SEO risk observable.

### Metrics to add

- SEO step duration / failure / reuse metrics
- draft QA fail rate
- cannibalization backlog size
- orphan risk count
- rebuild lag
- SERP staleness
- freshness SLA breach count
- publish gate reject count

### Dashboards / alerts

- freshness dashboard
- rebuild backlog dashboard
- QA failure dashboard
- cannibalization dashboard
- SERP ingestion health dashboard

### Acceptance

- SEO pipeline has production-grade metrics and alerts comparable to current runtime checks
- `temporal_production_gate.sh` is extended to validate critical SEO metrics where appropriate

---

## Phase 9. Migration and backfill

### Goal

Move existing site and graph state into the new SEO operating model without silent drift.

### Backfill sources

- existing `site.page_context_map`
- `raw.pages` and `raw.sections`
- `serp.*` historical runs
- existing Neo4j page/block/dependency graph
- existing CMS pages where available

### Required migration outputs

- initial `PageNode` population
- initial `PageBlueprint` assignment for known page families
- initial `ContentGap` candidates from historical SERP data
- initial `scope_signature` backfill
- initial cannibalization detection report

### Hard rule

Backfill may create planning and product-derived artifacts, but may not silently create verified truth objects.

### Acceptance

- migration is replayable
- every backfilled page can be traced to source inputs
- legacy pages with ambiguous scope are queued for review instead of silently normalized

---

## Phase 10. Security / compliance / launch bar

### Goal

Make the super-site platform safe for sustained operation.

### Security / compliance work

- crawl and competitor snapshot retention policy
- source usage policy for competitor-derived signals
- CMS publication permissions and rollback rights
- secret/config separation for fetchers and worker services
- audit trail for high-risk approvals and overrides

### Definition of ready for super-site launch

Launch is allowed only when all are true:

- all SEO artifacts exist in live schema and runtime
- all SEO runtime steps are typed, ledger-backed, and automation-gated
- rebuilds and freshness are automatic and observable
- drafts cannot bypass traceability gate
- cannibalization and URL conflicts block publication
- linking engine passes regression tests
- CMS publication is fully gated and auditable
- HITL queues exist for all high-risk SEO decisions
- metrics loop changes derived SEO objects only, never truth
- `ci_verify.sh`, Temporal production gate, and restore drill all pass after SEO expansion

---

## 6. Build-Spec Package

The roadmap depends on the following build-spec package.

### 6.1 Live Schema Design Spec

- exact table and column definitions
- PK/FK and uniqueness
- status enums and checks
- indexes
- schema-family placement
- source-of-record vs projections
- migration notes per table

### 6.2 Runtime Step Contracts Spec

- exact Proto payload list
- exact `StepContractMeta` for each step
- exact I/O contract for each runtime operation
- failure classes and retry contracts
- HITL pause conditions
- step-level producers and consumers

### 6.3 Graph And Retrieval Projection Spec

- exact Neo4j node labels and edge types
- exact projection identity/upsert rules
- exact Qdrant collections and payload schemas
- rebuild and invalidation semantics

### 6.4 CMS And HITL Control Plane Spec

- exact CMS page fields and revision model
- exact state transitions and publish blockers
- exact outbox event shapes
- exact HITL task types and resolution payloads

---

## 7. Recommended Build Order

1. Lock architecture and ownership docs.
2. Write the four build-spec documents.
3. Verify that build-spec docs answer the exact SQL / Proto / Temporal / Neo4j / Qdrant / CMS / HITL questions.
4. Only then begin live schema and runtime implementation.
5. Implement schema and Proto.
6. Implement primitives, runtime models, adapters, and use cases.
7. Register Temporal steps and workflows.
8. Implement graph / retrieval / CMS / HITL integrations.
9. Add evaluation harness and automation gates.
10. Add observability, migration, and launch-readiness gates.

---

## 8. Non-Negotiable Acceptance Gates

At the end of each implementation phase:

- `cargo check --workspace` passes
- `bash automation/contract_generate.sh` and `bash automation/contract_verify.sh` pass after contract changes
- `bash automation/ci_verify.sh` passes with new SEO checks included where applicable
- no layer violations are introduced into `primitives`, `seo_steps`, or `services/temporal`
- no new runtime path falls back to generic JSON payloads

Before production-grade verdict:

- `TEMPORAL_GATE_WORKER_MODE=docker bash automation/temporal_production_gate.sh`
- `infra/backups/restore_drill.sh`

must both pass after the SEO layer is integrated.

---

## Readiness Gate Reference

Before Phase 1 schema and runtime work starts, review:

- `docs/V5_SEO_Implementation_Readiness_Gate.md`

Phase 1 remains `No-Go` until that document records a `go` verdict after cross-spec review.

## 9. Phase 1 Acceptance Scenario Reference

Phase 1 delivery is not considered operationally proven until the representative end-to-end acceptance scenario defined in `V5_SEO_Implementation_Readiness_Gate.md` has been executed and recorded as passing.

That scenario must prove one deterministic chain from SERP-derived planning inputs through:

- accepted page-planning objects
- required Neo4j and Qdrant projections
- draft assembly and QA
- a terminal publish-control verdict

Implementation sequencing must preserve enough instrumentation and fixture control to run this scenario before broader rollout.

## 10. Phase 1 Fixture Package Reference

Before the representative Phase 1 acceptance scenario can be executed, the delivery record must include a stable fixture package as defined in `V5_SEO_Implementation_Readiness_Gate.md`.

That fixture package must provide:

- fixed SERP-derived inputs
- normalized scope fixtures
- blueprint and section-template fixtures
- verified support fixtures
- link-target fixtures
- positive and negative QA-path fixtures
- expected object keys, projections, and terminal outcomes

Implementation is not considered acceptance-ready until this fixture package exists and is versioned alongside the proving scenario.
