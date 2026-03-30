# V5 SEO Build-Spec Fix List

**Status:** executed remediation plan for cross-spec blockers  
**Purpose:** exact fix list required to move the SEO documentation stack from architecture-ready to implementation-ready after the review findings.

## 1. Goal

This document converts the current review findings into an ordered remediation program.

It answers three questions:

- which gaps are real blockers,
- which document owns each fix,
- in what order the fixes must be applied before `V5_SEO_Implementation_Readiness_Gate.md` can move from `no-go` to `go`.

## 2. Priority Order

### P0. Must be fixed before any cross-spec approval

1. `scope_signature` normalization contract
2. `derivation_version` / version bump policy
3. HITL pause / resume / reopen / timeout protocol
4. draft claim validation algorithm
5. V2 truth-change to page rebuild propagation
6. readiness gate status semantics

### P1. Must be fixed before code work starts

7. JSONB internal payload schemas
8. deterministic final tie-break chain for cannibalization
9. precise SERP observation thresholds
10. projection deletion mode selection
11. runtime hash algorithm declaration
12. exact build-spec dependency order in execution terms

### P2. Must be fixed before implementation hardening is complete

13. artifact freshness SLA catalog across all SEO artifact classes
14. metric formulas and SLO thresholds
15. malformed-input / partial-failure handling contract
16. multi-version artifact coexistence policy
17. explicit V2 integration touchpoints

## 3. Exact Fix List

## Fix 1. `scope_signature` normalization contract

**Severity:** critical  
**Primary owner:** `V5_SEO_Foundation_Contracts.md`  
**Supporting docs:** `V5_SEO_Live_Schema_Design_Spec.md`, `V5_SEO_Runtime_Step_Contracts_Spec.md`

### Add to `V5_SEO_Foundation_Contracts.md`

Create new subsection:

- `7.2.1 Scope normalization algorithm`

It must define, for each dimension:

- accepted input form
- trim rule
- lowercase or uppercase rule
- enum validation rule
- sentinel values like `all`
- forbidden null semantics
- canonical serialization order

Required normalized tuple order:

- `market`
- `locale`
- `country_code`
- `visa_type`
- `applicant_profile`

Required output rule:

- `scope_signature = blake3(canonical_scope_tuple_string)`

### Add to `V5_SEO_Live_Schema_Design_Spec.md`

Create new subsection:

- `2.2.1 Scope uniqueness enforcement`

It must define:

- the five raw scope columns as source-of-record
- `scope_signature` as derived key only
- required unique constraint on raw normalized scope tuple where appropriate
- required check that stored `scope_signature` matches normalized tuple

### Add to `V5_SEO_Runtime_Step_Contracts_Spec.md`

Create new common contract note:

- every step receiving scope-bearing payloads must reject non-normalized scope input before persistence

### Acceptance

- two semantically identical scopes always yield the same `scope_signature`
- malformed scope dimensions fail before write
- automation can compare raw tuple to signature deterministically

## Fix 2. `derivation_version` / version bump policy

**Severity:** critical  
**Primary owner:** `V5_SEO_Foundation_Contracts.md`  
**Supporting docs:** all build-spec docs

### Add to `V5_SEO_Foundation_Contracts.md`

Create new section:

- `5.3 Derivation version policy`

It must define:

- what counts as a logic change
- what counts as a data refresh only
- which changes require version bump in identity inputs
- which changes require storage row supersession without key mutation
- artifact-specific version fields:
  - `cluster_version`
  - `pattern_version`
  - `blueprint_version`
  - `assembly_version`
  - `scoring_version`
  - `detector_version`

Required bump triggers:

- scoring formula change
- normalization logic change
- section-template rule change affecting artifact meaning
- conflict-detector logic change
- blueprint assembly semantics change

### Add to `V5_SEO_Implementation_Readiness_Gate.md`

New checklist row:

- `artifact version-bump policy explicit and cross-linked`

### Acceptance

- no artifact logic can change silently under the same derivation identity
- history and rebuild detection remain coherent across logic revisions

## Fix 3. HITL pause / resume / reopen / timeout protocol

**Severity:** critical  
**Primary owner:** `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`  
**Supporting docs:** `V5_SEO_Runtime_Step_Contracts_Spec.md`

### Add to `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`

Create new section:

- `8.1 Pause and resume protocol`

It must define:

- how a blocked step records pause state
- how a resolution payload addresses a specific paused execution
- what makes a resume payload valid
- what happens when a task is reopened after resume
- whether repeated pauses are allowed and how they version
- timeout thresholds by task type
- escalation behavior after timeout
- terminal conditions for abandoned tasks

### Add to `V5_SEO_Runtime_Step_Contracts_Spec.md`

Create new section:

- `6.1 HITL execution lifecycle`

It must define:

- `paused`
- `waiting_for_resolution`
- `resumed`
- `reopened_after_resume`
- `expired`
- `cancelled`

### Acceptance

- no blocked step can hang indefinitely without timeout semantics
- reopen after resume is deterministic
- every resume targets one concrete paused execution

## Fix 4. Draft claim validation algorithm

**Severity:** critical  
**Primary owner:** `V5_SEO_Draft_Assembly_And_QA_Protocol.md`  
**Supporting docs:** `V5_SEO_Runtime_Step_Contracts_Spec.md`, `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`

### Add to `V5_SEO_Draft_Assembly_And_QA_Protocol.md`

Create new section:

- `6.4 Claim validation algorithm`

It must define:

- what counts as a factual claim
- what counts as non-factual explanatory copy
- sentence-level or fragment-level traceability granularity
- support resolution order:
  - direct verified fact binding
  - verified summary binding
  - allowed editorial extrapolation
  - forbidden SERP-as-fact usage
- exact blocker conditions
- ambiguous fragment fallback to HITL

### Add to `V5_SEO_Runtime_Step_Contracts_Spec.md`

Extend `draft_qa` contract with:

- exact output fields for unsupported fragments
- exact failure class names
- exact HITL trigger on ambiguous or unsupported claims

### Acceptance

- QA can classify fragments deterministically enough for automation
- unsupported claims and unsupported summaries are distinguished
- SERP structure can inform form, not factual authority

## Fix 5. V2 truth-change to page rebuild propagation

**Severity:** critical  
**Primary owner:** `V5_SEO_Draft_Assembly_And_QA_Protocol.md`  
**Supporting docs:** `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`, `V5_SEO_Runtime_Step_Contracts_Spec.md`, `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`

### Add to `V5_SEO_Draft_Assembly_And_QA_Protocol.md`

Create new section:

- `8.1 Rebuild propagation from upstream truth change`

It must define:

- which page dependencies are tracked
- how a changed topic, concept, rule, or operational entity maps to affected pages
- required propagation chain:
  - truth object changed
  - dependency lookup
  - page invalidation mark
  - rebuild queue decision
  - publish-state effect
- handling for pages in `publish_blocked`, `stale`, `deprecated`
- handling when `scope_signature` changes

### Add to `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`

Extend rebuild rules with:

- exact dependency edges used for propagation
- required invalidation queries or graph traversal logic class

### Acceptance

- every upstream truth change has a deterministic downstream impact path
- no affected published page stays silently active

## Fix 6. Readiness gate status semantics

**Severity:** critical  
**Primary owner:** `V5_SEO_Implementation_Readiness_Gate.md`

### Update the gate document

Replace current informal meaning of `ready` with explicit two-level semantics:

- `document_present`
- `cross_spec_verified`

Or simplify to status set:

- `present_not_reviewed`
- `verified`
- `blocked`

Recommended choice:

- `present_not_reviewed`
- `verified`
- `blocked`

Then update all current Section 5 rows from `ready` to `present_not_reviewed` unless they were explicitly cross-checked.

### Acceptance

- verdict `no-go` aligns with checklist state
- the document distinguishes existence from verification

## Fix 7. JSONB internal payload schemas

**Severity:** high  
**Primary owner:** `V5_SEO_Live_Schema_Design_Spec.md`  
**Supporting docs:** `V5_SEO_Runtime_Step_Contracts_Spec.md`, `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`

### Add appendix

Create new appendix:

- `Appendix A. JSONB payload contracts`

It must define exact shapes for at least:

- `feature_payload`
- `required_fields`
- `allowed_traceability_labels`
- `required_sections`
- `optional_sections`
- `forbidden_sections`
- `required_link_roles`
- `metadata_obligations`
- `review_gates`
- `secondary_intents`
- `content_payload`
- `traceability_summary`
- `payload` in lifecycle events
- `query_seed_set`
- `observation_payload`
- `coverage_payload`
- `pattern_payload`
- `failure_payload`

### Acceptance

- implementer can encode/decode every JSONB field without guessing shape

## Fix 8. Final deterministic tie-break chain for cannibalization

**Severity:** high  
**Primary owner:** `V5_SEO_Information_Architecture_Protocol.md`

### Extend Section 8

Add final tie-break order after current rules:

- hierarchy owner
- narrower valid scope owner
- higher truth coverage availability
- higher opportunity score
- lower canonical URL depth
- lexicographic fallback on `page_node_key`

### Acceptance

- same inputs always produce same winning page

## Fix 9. Precise SERP observation thresholds

**Severity:** high  
**Primary owner:** `V5_SERP_Intelligence_Protocol.md`

### Extend reliability section

Define `multiple observations` as explicit thresholds, for example:

- minimum domains
- minimum capture count
- minimum freshness window
- same-pattern stability threshold

### Acceptance

- `R2_supported` and `R3_strong` are assignable without human interpretation

## Fix 10. Projection deletion mode selection

**Severity:** high  
**Primary owner:** `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`

### Extend upsert rules

Specify for each projected class whether deactivation uses:

- hard delete
- tombstone
- archive flag

### Acceptance

- graph rebuild behavior is stable and analytics-safe

## Fix 11. Runtime hash algorithm declaration

**Severity:** high  
**Primary owner:** `V5_SEO_Runtime_Step_Contracts_Spec.md`

### Add common rule

State explicitly:

- `input_hash`, `output_hash`, and derived `idempotency_key` basis use `blake3`
- canonical serialization basis for hash input
- hash domain separation if used

### Acceptance

- runtime hashing aligns with automation and existing app invariants

## Fix 12. Exact build-spec dependency order

**Severity:** high  
**Primary owner:** `V5_SEO_Companion_Document_Creation_Plan.md`

### Add execution-order note

State practical resolution order:

1. `V5_SEO_Live_Schema_Design_Spec.md`
2. `V5_SEO_Runtime_Step_Contracts_Spec.md`
3. `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`
4. `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`
5. `V5_SEO_Implementation_Readiness_Gate.md`

Also clarify that dependency references are not cyclic ownership.

### Acceptance

- implementation order is unambiguous

## Fix 13. Artifact freshness SLA catalog

**Severity:** medium  
**Primary owner:** `V5_SEO_Operations_And_Optimization_Protocol.md`

### Add new section

- `7.1 Artifact freshness SLA catalog`

Define freshness windows for:

- `SERPPattern`
- `KeywordCluster`
- `OpportunityCandidate`
- `PageBlueprint`
- `PageNode`
- `Draft`
- `LinkRecommendation`
- `ContentGap`

## Fix 14. Metric formulas and SLO thresholds

**Severity:** medium  
**Primary owner:** `V5_SEO_Operations_And_Optimization_Protocol.md`

### Extend metrics contract

Define formulas and thresholds for at least:

- rebuild lag
- draft QA fail rate
- cannibalization backlog
- orphan page count
- stale SERP ratio

Also define SLO breach actions.

## Fix 15. Malformed-input and partial-failure handling

**Severity:** medium  
**Primary owner:** `V5_SEO_Runtime_Step_Contracts_Spec.md`

### Add failure-handling section

Define:

- malformed SERP HTML
- incomplete competitor capture
- partial Qdrant projection failure
- partial Neo4j projection failure
- CMS publish failure after approval
- retry vs quarantine vs HITL routing

## Fix 16. Multi-version artifact coexistence policy

**Severity:** medium  
**Primary owner:** `V5_SEO_Foundation_Contracts.md`

### Add new section

- `5.4 Multi-version artifact coexistence`

Define:

- when old and new versions may coexist
- how active version is selected
- how rebuilds choose source version
- how analytics distinguish superseded vs active records

## Fix 17. Explicit V2 integration touchpoints

**Severity:** medium  
**Primary owner:** `V5_SEO_Companion_Document_Creation_Plan.md`  
**Supporting docs:** build-spec package

### Add appendix

- `Appendix: V2 integration touchpoints`

Map:

- truth change inputs
- ontology change inputs
- graph reference edges
- publish-gate dependencies
- retrieval separation boundaries

## 4. Recommended Remediation Order

1. Fix readiness gate semantics first.
2. Fix `scope_signature` normalization.
3. Fix version bump policy.
4. Fix HITL pause/resume protocol.
5. Fix claim validation algorithm.
6. Fix truth-change rebuild propagation.
7. Fix JSONB payload schemas.
8. Fix cannibalization deterministic fallback.
9. Fix SERP thresholds.
10. Fix projection deletion mode.
11. Fix runtime hash algorithm declaration.
12. Fix execution dependency order.
13. Fix SLA catalog and metric formulas.
14. Fix failure-handling and multi-version policy.
15. Add explicit V2 integration appendix.
16. Re-run cross-spec review.
17. Update `V5_SEO_Implementation_Readiness_Gate.md` and switch verdict only after all review blockers are closed.

## 5. Completion Standard

This remediation plan is complete only when:

- every blocker maps to exactly one primary owner,
- every fix identifies the exact document and section to add or update,
- the remediation order removes ambiguity about what must be fixed first,
- the readiness gate can be updated directly from this list.


