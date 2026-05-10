# V5 SEO Implementation Readiness Gate

**Status:** current implementation alignment gate and review record
**Purpose:** consolidated checklist for keeping live SEO code, schema, proto, automation, and documentation aligned.

## 1. Role Of This Document

This document is not a normative owner of architecture or runtime rules.

Its role is to:

- bind the four build-spec documents and the live execution plan into one execution checklist,
- distinguish document presence from actual cross-spec verification,
- record whether the documentation stack matches the live implementation,
- publish the current implementation and production go / no-go verdicts.

Normative rules remain owned by:

- `V5_SEO_Live_Schema_Design_Spec.md`
- `V5_SEO_Runtime_Step_Contracts_Spec.md`
- `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`
- `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`
- their upstream companion owner-documents and V2

## 2. Gate Rule

Live code, live schema, proto contracts, and automation are the highest-priority truth. Documentation is aligned only when it describes those live artifacts without claiming unimplemented runtime behavior.

Production launch is **No-Go** unless the production verdict in this document is `go`.

Implementation changes that must update this document when they change behavior include:

- SQL changes in `app/db/schema.sql`
- Proto changes in `app/contracts/proto/temporal_payloads.proto`
- new runtime DTOs
- new Temporal workflows or activities
- new Neo4j projection code
- new Qdrant projection code
- new CMS publish path logic
- new HITL task or queue runtime logic

## 3. Status Semantics

Checklist rows in this document use exactly three states:

- `verified`: the required live artifact exists and is consistent with code/schema/proto/automation
- `partial`: the artifact exists, but production completeness or live E2E proof is missing
- `blocked`: the artifact is missing, contradictory, or unsafe to treat as implemented

Production may move to `go` only when every launch-critical row is `verified` and live E2E certification has passed.

## 4. Current Verdict

- `spec_readiness_verdict`: `go`
- `implementation_readiness_verdict`: `partial`
- `production_readiness_verdict`: `no-go`
- `reason`: Live schema/proto/contracts, SQLx SEO persistence, `SeoSiteBuildWorkflow`, DataForSEO ingestion, crawl-to-raw storage, raw knowledge ingestion, trust-gated verification, Neo4j/Qdrant/CMS outbox projection, preflight diagnostics, global navigation persistence, draft/QA/CMS/HITL/static publish-control, and deterministic SEO smoke tests exist. Production remains blocked on live E2E certification, production crawler hardening, richer extraction/contradiction coverage, current-run scoped projection barriers, explicit SEO run modes, real external CMS integration policy, and rebuild scheduler execution.
- `last_reviewed_at`: `2026-05-11`
- `review_owner`: `live_contract_alignment`

## 5. Source Build-Spec Package

| Concern area | Source document |
|---|---|
| live SQL schema | `V5_SEO_Live_Schema_Design_Spec.md` |
| runtime steps and wire contracts | `V5_SEO_Runtime_Step_Contracts_Spec.md` |
| Neo4j and Qdrant projection | `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` |
| CMS and HITL control plane | `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` |
| current capability and 10/10 execution order | `SEO_SUPERSITE_10_10_EXECUTION_PLAN.md` |

## 5.1 Live Implementation Snapshot

This snapshot reflects the working tree as reviewed on 2026-05-11.

Implemented and automation-checked:

- `SeoSiteBuildWorkflow` is registered and startable through `temporal_starter`.
- Typed `SeoSiteBuildInputPayload` is persisted before workflow start.
- Workflow stages include SERP ingest, crawl sources, raw knowledge ingestion, SERP normalize, opportunity build, IA build, link recommend, global reconcile, draft assemble, editorial draft generation, draft normalize, content contract validation, draft QA, CMS review/publish control, static materialization, preview validation, finalize publish, and rebuild detect.
- DataForSEO credentials can drive live SERP ingestion and crawl queue population.
- `crawl_sources` stores fetched HTML into `raw.pages`/`raw.sections` and can emit Qdrant outbox events.
- Raw knowledge ingestion auto-verifies only official/VFS-like sources; competitor-only facts are persisted as pending.
- Preflight reports context readiness, verified/pending facts, Qdrant point ledger size, live Qdrant/Neo4j probes, and Neo4j/Qdrant/CMS projection outbox status.
- Global navigation persistence derives `site.navigation_items` from active `site.page_nodes`; it does not seed final menus before source-backed IA exists.
- Automation gates pass for SEO runtime registration, schema, proto, persistence, projection, CMS, publish gates, traceability, static site builder, step catalog, step execution, and status parity.

Not yet production-complete:

- No live E2E certification has been recorded against real credentials and mutable external sources.
- The crawler is still an MVP fetcher, not a robots-aware production crawler.
- Extraction coverage is too narrow for complete visa-rule coverage.
- Projection barriers are global outbox barriers; current-run scoped barriers still need aggregate/run filtering.
- Explicit SEO run modes such as `crawl_only`, `draft_only`, and `full_auto_after_approval` are not implemented.
- Rebuild detection exists, but a full rebuild scheduler/workflow is still missing.

## 6. Blocking Checklist

### 6.1 Live schema completeness

| Item | Status | Source |
|---|---|---|
| all required SEO tables named | `verified` | `V5_SEO_Live_Schema_Design_Spec.md` |
| columns and scalar types explicit | `verified` | `V5_SEO_Live_Schema_Design_Spec.md` |
| PK/FK and uniqueness rules explicit | `verified` | `V5_SEO_Live_Schema_Design_Spec.md` |
| status enums and check constraints explicit | `verified` | `V5_SEO_Live_Schema_Design_Spec.md` |
| indexes explicit | `verified` | `V5_SEO_Live_Schema_Design_Spec.md` |
| source-of-record vs projection split explicit | `verified` | `V5_SEO_Live_Schema_Design_Spec.md` |
| migration/backfill hooks present | `verified` | `V5_SEO_Live_Schema_Design_Spec.md` |
| raw-scope normalization and signature enforcement explicit | `verified` | `V5_SEO_Live_Schema_Design_Spec.md` |
| JSONB payload contracts explicit | `verified` | `V5_SEO_Live_Schema_Design_Spec.md` |

### 6.2 Runtime contract completeness

| Item | Status | Source |
|---|---|---|
| every SEO step named | `verified` | `V5_SEO_Runtime_Step_Contracts_Spec.md` |
| typed Proto messages explicit | `verified` | `V5_SEO_Runtime_Step_Contracts_Spec.md` |
| `StepContractMeta` explicit | `verified` | `V5_SEO_Runtime_Step_Contracts_Spec.md` |
| failure classes explicit | `verified` | `V5_SEO_Runtime_Step_Contracts_Spec.md` |
| idempotency and retry policy explicit | `verified` | `V5_SEO_Runtime_Step_Contracts_Spec.md` |
| canonical hash algorithm explicit | `verified` | `V5_SEO_Runtime_Step_Contracts_Spec.md` |
| HITL pause conditions explicit | `verified` | `V5_SEO_Runtime_Step_Contracts_Spec.md` |
| HITL execution lifecycle explicit | `verified` | `V5_SEO_Runtime_Step_Contracts_Spec.md` |
| malformed-input and partial-failure handling explicit | `verified` | `V5_SEO_Runtime_Step_Contracts_Spec.md` |
| producer and consumer ownership explicit | `verified` | `V5_SEO_Runtime_Step_Contracts_Spec.md` |

### 6.3 Graph and retrieval projection completeness

| Item | Status | Source |
|---|---|---|
| all Neo4j node labels explicit | `verified` | `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` |
| all Neo4j edge types explicit | `verified` | `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` |
| cross-layer truth boundaries explicit | `verified` | `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` |
| source-table to projection mapping explicit | `verified` | `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` |
| Qdrant collections explicit | `verified` | `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` |
| Qdrant payload schema explicit | `verified` | `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` |
| projection deletion mode explicit | `verified` | `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` |
| rebuild and freshness semantics explicit | `verified` | `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` |
| truth-change propagation edges explicit | `verified` | `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` |

### 6.4 CMS and HITL completeness

| Item | Status | Source |
|---|---|---|
| CMS page fields explicit | `verified` | `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` |
| revision model explicit | `verified` | `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` |
| publish state machine explicit | `verified` | `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` |
| publish blockers explicit | `verified` | `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` |
| canonical URL conflict handling explicit | `verified` | `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` |
| HITL task types explicit | `verified` | `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` |
| HITL resolution payloads explicit | `verified` | `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` |
| pause / resume / reopen / timeout protocol explicit | `verified` | `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` |
| runtime blocking matrix explicit | `verified` | `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` |

### 6.5 Companion-owner completeness

| Item | Status | Source |
|---|---|---|
| scope normalization algorithm explicit | `verified` | `V5_SEO_Foundation_Contracts.md` |
| derivation version-bump policy explicit | `verified` | `V5_SEO_Foundation_Contracts.md` |
| multi-version artifact coexistence policy explicit | `verified` | `V5_SEO_Foundation_Contracts.md` |
| claim validation algorithm explicit | `verified` | `V5_SEO_Draft_Assembly_And_QA_Protocol.md` |
| truth-change rebuild propagation explicit | `verified` | `V5_SEO_Draft_Assembly_And_QA_Protocol.md` |
| deterministic cannibalization final tie-break explicit | `verified` | `V5_SEO_Information_Architecture_Protocol.md` |
| explicit SERP observation thresholds present | `verified` | `V5_SERP_Intelligence_Protocol.md` |
| artifact freshness SLA catalog explicit | `verified` | `V5_SEO_Operations_And_Optimization_Protocol.md` |
| metric formulas and SLO thresholds explicit | `verified` | `V5_SEO_Operations_And_Optimization_Protocol.md` |

### 6.6 Cross-document readiness checks

| Item | Status | Source |
|---|---|---|
| each implementation concern has one owning spec | `verified` | master plan + build-spec package |
| build-spec execution order is explicit | `verified` | master plan |
| dependency references are non-cyclic ownership references | `verified` | master plan |
| roadmap contains historical implementation gate context | `verified` | `V5_SEO_SUPERSITE_IMPLEMENTATION_ROADMAP.md` |
| readiness gate linked into planning layer | `verified` | master plan |
| readiness gate linked into roadmap layer | `verified` | roadmap |
| V2 integration touchpoints explicit | `verified` | master plan |

## 7. Cross-Spec Review Completion Record

The following cross-spec review tasks are complete:

| Review task | Status |
|---|---|
| line-by-line consistency review across all 4 build-spec docs | `verified` |
| schema-field to runtime DTO name consistency review | `verified` |
| runtime step to store-write consistency review | `verified` |
| schema-source to graph/retrieval projection consistency review | `verified` |
| acceptance checks to future automation intent review | `verified` |

## 8. Go / No-Go Decision Procedure

The verdict may switch from `go` back to `no-go` if any of the following occur:

- a build-spec document changes without cross-spec re-verification,
- a new implementation-critical concern is introduced without an owning spec,
- a conflicting rule appears between owner docs and build-spec docs,
- SQL, Proto, Temporal, Neo4j/Qdrant, CMS, or HITL implementation requires inventing a missing contract.

## 9. Acceptance Criteria For This Document

This document is complete only when:

- it maps every implementation-critical concern to exactly one source build-spec document
- it distinguishes document presence from actual verification
- it exposes a single current implementation verdict
- it can be used as the alignment checklist before code, schema, proto, automation, or documentation changes ship

## 10. Acceptance Scenario

This document requires two acceptance levels:

- deterministic fixture acceptance for implementation alignment
- live E2E certification for production launch

The deterministic fixture path exists to prove that one complete controlled pipeline run can move from SERP-derived inputs to a publishable-or-blocked page verdict without inventing missing contracts. Live E2E remains required before production launch.

### 10.1 Scenario purpose

The scenario exists to prove that one complete deterministic pipeline run can move from SERP-derived inputs to a publishable-or-blocked page verdict without inventing missing contracts.

### 10.2 Representative scenario shape

Use one stable scope-bearing page candidate with all of the following characteristics:

- one normalized `scope_signature`
- one dominant intent
- one accepted `KeywordCluster`
- one accepted `PageNode`
- one attached `PageBlueprint`
- one evidence-backed draft path
- at least one required internal link
- at least one traceable factual fragment

The scenario must use real platform object types but does not require production content volume.

### 10.3 Minimum scenario flow

The current implementation-alignment scenario must execute this chain successfully:

1. `load_seo_site_build_input`
2. `load_verified_support_bundle`
3. `serp_ingest`
4. `crawl_sources`
5. `raw_knowledge_ingestion`
6. `serp_normalize`
7. `opportunity_build`
8. `ia_build`
9. `link_recommend`
10. `global_site_reconcile`
11. `draft_assemble`
12. `editorial_draft_generate`
13. `draft_normalize`
14. `content_contract_validate`
15. `draft_qa`
16. `cms_publish` request-review verdict
17. HITL approval decision lookup when publication is requested
18. `cms_publish` approved-publish verdict
19. `publish_materialize`
20. `render_preview_validate`
21. `finalize_publish`
22. `rebuild_detect`

### 10.4 Required scenario outputs

The scenario passes only if it produces all of the following:

- persisted `SERPPattern` output with reliability metadata
- persisted `KeywordCluster` with dominant intent
- persisted `PageNode` with valid scope and canonical URL
- attached `PageBlueprint`
- at least one `LinkRecommendation`
- one `PageBrief`
- one `Draft` with traceability manifest
- one `DraftQaOutput` with explicit verdict
- required `Neo4j` projections for the page-planning path
- required `Qdrant` projections for the page-planning and draft-support path
- one terminal status of either `published`, `review_required`, `qa_failed`, `publish_blocked`, or `rebuild_required`

### 10.5 Required scenario blocking behavior

The scenario also fails unless the system proves these blocking behaviors:

- invalid scope blocks before persistence
- unsupported factual fragment blocks publication
- unresolved cannibalization blocker prevents silent publication
- missing required internal links prevents publication
- forbidden SERP-as-fact usage prevents publication

### 10.6 Acceptance verdict rule

Implementation alignment is considered proven only when deterministic automation passes against the current working tree. Production readiness is not proven until live E2E certification is documented against real credentials, real source pages, live Neo4j/Qdrant projections, HITL approval, and static/CMS output.

### 10.7 Required fixture package

The acceptance scenario must be backed by a stable fixture package with all of the following inputs:

- one fixed SERP query fixture and captured result set
- one normalized scope fixture with explicit raw and normalized forms
- one accepted keyword-cluster fixture
- one blueprint fixture
- one section-template fixture set
- one verified-fact support fixture set
- one link-target fixture set
- one expected draft fragment set with known traceability outcomes
- one expected blocking-case fixture for unsupported factual content
- one expected blocking-case fixture for scope mismatch or invalid scope

### 10.8 Fixture package hard rules

The fixture package must obey these rules:

- fixture inputs are versioned and immutable per acceptance run
- fixture scope values include both raw input and normalized canonical values
- fixture truth-support records are sufficient to validate at least one passing factual fragment
- fixture blocking records are sufficient to trigger at least one deterministic `qa_failed` or `review_required` outcome
- fixture data must not depend on live SERP fetches or mutable external pages at execution time

### 10.9 Fixture package expected outputs

The fixture package is complete only if it can be paired with expected results for:

- created canonical object keys
- required projection object keys
- expected link recommendation presence
- expected traceability verdicts
- expected terminal page outcome
- expected blocking reason codes for negative-path fixtures

### 10.10 Fixture package review rule

The fixture package must be reviewed whenever any of the following change:

- scope normalization contract
- dominant-intent assignment rules
- blueprint contract
- traceability labels or claim-validation rules
- mandatory Phase 1 graph objects
- mandatory Phase 1 retrieval collections
