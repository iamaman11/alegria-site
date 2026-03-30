# V5 SEO Draft Assembly And QA Protocol

**Status:** working draft companion protocol  
**Purpose:** canonical owner of page brief contracts, blueprint minimum fields, draft assembly, metadata obligations, factual traceability, lifecycle states, rebuild triggers, and page-level QA.

---

## 1. Relation To V2, Foundation, And IA

This document depends on:

- `V5_Ultimate_Extraction_Protocol_V2.md` for serving and truth boundaries
- `V5_SEO_Foundation_Contracts.md` for artifact class, storage, and scope semantics
- `V5_SEO_Information_Architecture_Protocol.md` for page taxonomy, intent, URL policy, and cannibalization rules

This document does not authorize truth creation from drafts.

---

## 2. Draft Artifact Model

This document owns:

- `PageBrief`
- `PageBlueprint`
- `Draft`

Artifact classes:

- `PageBrief`: planning artifact
- `PageBlueprint`: planning artifact
- `Draft`: serving artifact with planning metadata

---

## 3. Page Brief Contract

Minimum fields:

- `page_node_key`
- `scope_signature`
- `page_type`
- `dominant_intent`
- `target_audience`
- `goal`
- `required_fact_domains`
- `link_obligations`
- `metadata_requirements`
- `truth_snapshot_ref`

`PageBrief` is the assembly input contract. Draft assembly may not start without a complete brief.

---

## 4. Page Blueprint Minimum Contract

Minimum blueprint fields:

- `blueprint_key`
- `page_type`
- `dominant_intent`
- `allowed_secondary_intents`
- `required_sections`
- `optional_sections`
- `forbidden_sections`
- `required_fact_types`
- `required_link_roles`
- `metadata_obligations`
- `review_gates`
- `blueprint_version`

Blueprints are reusable templates. They are not published content and are not truth objects.

---

## 5. Assembly And Metadata Rules

Draft assembly must:

- start from one `PageBrief`
- select exactly one `PageBlueprint`
- bind to one `truth_snapshot_ref`
- populate required sections before optional sections
- insert required links from IA obligations
- populate required metadata fields

Required metadata fields:

- `title_candidate`
- `meta_description_candidate`
- `canonical_url`
- `h1_candidate`
- `faq_eligible`
- `schema_markup_candidates`

---

## 6. Draft Traceability Contract

### 6.1 `draft_traceability_labels`

- `verified_fact`
- `verified_summary`
- `editorial_extrapolation`
- `serp_pattern_reference`
- `business_copy`

### 6.2 Allowed claim behavior

- `verified_fact`: may assert procedural or operational fact when supported by V2 truth objects
- `verified_summary`: may summarize verified facts without changing scope or meaning
- `editorial_extrapolation`: may explain, compare, or clarify, but may not introduce new factual claims
- `serp_pattern_reference`: may shape structure or prioritization, but may not appear as fact authority
- `business_copy`: may express service positioning, but may not impersonate verified rule truth

### 6.3 Hard blockers

Block draft promotion when:

- any factual sentence lacks `verified_fact` or `verified_summary` support,
- a required section contains only unsupported copy,
- `serp_pattern_reference` is used as fact evidence,
- scope-specific claims do not match `scope_signature`.

---

## 7. Lifecycle State Machine

Allowed states:

- `planned`
- `blueprint_ready`
- `draft_ready`
- `qa_failed`
- `ready_for_review`
- `approved`
- `published`
- `stale`
- `needs_rebuild`
- `deprecated`

Core transitions:

- `planned -> blueprint_ready -> draft_ready`
- `draft_ready -> qa_failed | ready_for_review`
- `ready_for_review -> approved | qa_failed`
- `approved -> published`
- `published -> stale -> needs_rebuild -> draft_ready`
- `published -> deprecated`

---

## 8. Rebuild Trigger Matrix

`rebuild_trigger_matrix`:

| Trigger | Rebuild required |
|---|---|
| truth change affecting required facts | yes |
| ontology change affecting scope or bindings | yes |
| SERP change affecting blueprint selection only | conditional |
| template or blueprint version change | yes |
| IA change affecting URL or dominant intent | yes |
| link graph change affecting required obligations | yes |

If a trigger changes `scope_signature`, `dominant_intent`, or `canonical_url`, rebuild is mandatory and the existing published page may not remain silently active.

---

## 9. QA Gates

A draft passes QA only when:

- blueprint contract is complete,
- required metadata exists,
- all factual claims are traceable,
- required links are satisfied,
- no unresolved cannibalization blocker exists,
- scope and canonical URL are valid,
- no forbidden section appears,
- freshness state of required evidence is acceptable.

---

## 10. Hard Rules

1. Drafts are never truth-core.
2. Unsupported factual fragments block readiness.
3. Draft assembly may use SERP signals for structure, not for factual authority.
4. A draft with unresolved scope mismatch may not move past `qa_failed`.
5. Lifecycle transitions must remain deterministic and auditable.

---

## 11. Claim Validation Algorithm Addendum

### 11.1 Factual claim detection

A draft fragment is treated as factual when it asserts one or more of the following:

- an eligibility condition
- a required document or procedural step
- a fee, time, place, authority, deadline, or actor responsibility
- a rule, exception, or applicability statement
- a scope-specific condition bound to country, visa type, or applicant profile

A fragment is treated as non-factual explanatory copy when it only:

- clarifies meaning,
- introduces structure,
- offers editorial framing,
- explains user journey without asserting procedural truth.

### 11.2 Validation granularity

Validation is fragment-level.

A fragment is one of:

- a sentence,
- a checklist item,
- a table cell carrying a procedural assertion,
- an FAQ answer clause.

### 11.3 Support resolution order

Every factual fragment must resolve support in this order:

1. direct `verified_fact` binding
2. allowed `verified_summary` binding
3. explicit `editorial_extrapolation` that does not introduce new procedural truth
4. otherwise block as unsupported

Forbidden support source:

- `serp_pattern_reference` as factual authority

### 11.4 Ambiguity handling

If a fragment cannot be classified deterministically as factual or non-factual:

- classify it as `ambiguous_fragment`
- route it to HITL through `unsupported_factual_fragment`
- block publish readiness until resolved

### 11.5 Validation outputs

The QA layer must emit for every draft:

- `fragment_key`
- `fragment_text`
- `fragment_kind`
- `traceability_label`
- `support_refs[]`
- `validation_verdict`

---

## 12. Rebuild Propagation Addendum

### 12.1 Upstream truth-change propagation

Truth-driven rebuild propagation must follow this deterministic chain:

1. a V2 truth object changes,
2. the changed object key is matched against page dependency manifests and graph reference edges,
3. affected `PageNode` and `Draft` objects are marked as impacted,
4. impacted published pages transition at least to `stale`,
5. `rebuild_detect` is queued with trigger type `truth_change`,
6. publishability is re-evaluated after rebuild.

Tracked upstream dependency classes:

- `Topic`
- `Concept`
- `RuleInstance`
- `OperationalEntity`

### 12.2 Dependency lookup sources

Affected pages are located through:

- draft traceability manifests
- page brief `required_fact_domains`
- graph cross-layer edges `COVERS_TOPIC`, `COVERS_CONCEPT`, `COVERS_RULE`
- current published revision traceability manifest

### 12.3 State handling

- `published`: mark `stale`, queue rebuild, block silent continued freshness
- `ready_for_review` or `approved`: invalidate review state and require re-QA
- `publish_blocked`: append new invalidation reason, do not auto-unblock
- `deprecated`: do not rebuild unless explicitly revived by owner decision

### 12.4 Scope-changing invalidation

If upstream change or ontology change causes `scope_signature` to change:

- the old page identity may not remain canonical by default,
- dependent artifacts must be superseded or archived,
- a new page lineage must be created under the new scope,
- URL and cannibalization checks must rerun before publication.

---

## 13. Minimum Brief, Draft, QA, And Publish System

This document defines the minimum expert content-generation layer required after IA decisions are made.

It must cover:

- page briefs
- page blueprints
- draft assembly
- draft traceability
- QA gates
- publish governance handoff

### 13.1 Page brief minimum role

A `PageBrief` is mandatory before draft assembly.

It must bind together:

- accepted `PageNode`
- dominant intent
- target audience
- required fact domains
- required link obligations
- metadata requirements
- truth snapshot reference
- blueprint candidate

### 13.2 Draft assembly minimum role

A `Draft` is publishable only after assembly from:

- verified knowledge inputs
- accepted page blueprint
- valid scope
- IA-approved URL and hierarchy placement
- required links
- allowed section templates

### 13.3 Draft traceability minimum role

Draft traceability must be present for:

- every factual fragment
- every required section carrying procedural meaning
- every scope-specific claim
- every FAQ answer with factual content

### 13.4 QA gates minimum role

Minimum page-level QA must verify:

- factual traceability
- scope correctness
- canonical URL validity
- required metadata presence
- required link obligations
- blueprint conformance
- no unresolved cannibalization blocker
- no forbidden SERP-as-fact usage

### 13.5 Publish governance handoff

A draft may pass from SEO generation into CMS publish flow only when:

- QA status is passing
- traceability manifest is complete
- page state is review-ready
- required human review cases are resolved
- CMS / HITL publish blockers are clear

This handoff is the bridge between SEO generation logic and publish governance.

## 14. Phase 1 Publish Gates For The First Working Super-Site Loop

This section defines the minimum publish gates that must already block unsafe publication in the first implementation.

### 14.1 Phase 1 mandatory blocking gates

A page must not enter CMS publish flow unless all of the following pass:

- `dominant_intent_assigned`
- `scope_signature_valid`
- `canonical_url_valid`
- `blueprint_attached`
- `required_sections_present`
- `required_metadata_present`
- `required_internal_links_present`
- `traceability_manifest_complete`
- `no_unsupported_factual_fragments`
- `no_forbidden_serp_as_fact_usage`
- `no_unresolved_cannibalization_blocker`
- `evidence_freshness_acceptable`
- `required_review_tasks_resolved`

### 14.2 Phase 1 blocking reason package

If publication is blocked, the system must emit an explicit blocking package containing at minimum:

- `page_node_key`
- `draft_key`
- `qa_verdict`
- `blocking_reason_codes[]`
- `blocking_task_type | null`
- `required_next_action`
- `recheck_trigger`

### 14.3 Phase 1 allowed outcomes after draft QA

After `draft_qa`, only these outcomes are allowed:

- `publish_ready`
- `review_required`
- `qa_failed`
- `rebuild_required`

Silent promotion from draft generation directly to `published` is forbidden.

### 14.4 Phase 1 publish-governance handoff rule

The first working SEO loop is complete only if:

- a passing draft can enter publish control deterministically,
- a failing draft is blocked deterministically,
- every blocked draft has machine-readable reasons,
- every review-required draft opens the correct HITL path.
