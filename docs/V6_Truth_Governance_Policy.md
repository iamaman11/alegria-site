# V6 Truth Governance Policy

**Status:** current-runtime-satellite, accepted-locally-against-frozen-certification-baseline  
**Class:** `current-truth-governance-policy`  
**Parent owner document:** [V6_Expert_Truth_Graph_Runtime.md](V6_Expert_Truth_Graph_Runtime.md)  
**Execution satellite:** [V6_SeoSiteBuildWorkflow_Working_Plan.md](V6_SeoSiteBuildWorkflow_Working_Plan.md)

---

## 1. Purpose

This document owns the policy tables and hard rules for:

- source independence,
- source trust weighting,
- authority override,
- freshness gating,
- regex role boundaries in the expert truth path.

It does not create a second extraction path.  
`verified.rule_instances` remains the only authority truth store.

Current acceptance status:

- governance adjudication is wired into both persisted truth writes and the canonical workflow truth-adjudication sweep;
- the frozen local/CI certification baseline remains green under this policy layer;
- the accepted regex de-authority slices are `canonical_mapping_step` and `completeness_judge_step`;
- remaining regex de-authority work stays blocked until each future slice is proven against that same baseline.

---

## 2. Source Governance Model

Every active source registry record must carry:

- `source_type`
- `authority_class`
- `independence_group_key`
- `trust_level`
- `freshness_ttl_days`
- `override_eligible`

Default authority ordering:

1. `primary_authority`
2. `delegated_authority`
3. `official_publisher`
4. `editorial`
5. `agency`
6. `forum`
7. `unknown`

Trust level is a modifier, not the authority model by itself.

---

## 3. Adjudication Rules

### 3.1 Independence

- corroboration is valid only across different `independence_group_key`
- reposts, mirrors, and same-content aliases do not count as a second source
- same-group corroboration becomes `needs_hitl`, not `verified`

### 3.2 Trust weighting

- low-trust, forum, and editorial-only evidence cannot auto-promote by itself
- trust can strengthen corroboration or contradiction handling
- trust does not bypass independence or freshness

### 3.3 Authority override

Single-source auto-verification is allowed only when all are true:

- `override_eligible = true`
- `authority_class` is allowed for override
- candidate is `fresh`
- candidate is `complete`
- no unresolved contradiction exists

Default override allowlist:

- `primary_authority`
- `delegated_authority`
- selected `official_publisher`

### 3.4 Freshness

Freshness classes:

- `hard_freshness_required`
- `review_if_stale`
- `monitor_if_stale`

Freshness verdicts:

- `fresh`
- `stale`
- `expired`
- `unknown_freshness`

Policy effects:

- stale hard-freshness truth is not publish-admissible
- stale truth may remain historically stored
- stale truth may downgrade to `needs_hitl` or block draft/publish depending on class

---

## 4. Regex Boundary

Regex is allowed only as:

- PII redaction
- sanitation / cleanup
- HTML or block validation
- technical guards
- non-authoritative hint signals

Regex is not allowed as final authority for:

- verified truth promotion
- canonical mapping acceptance
- completeness sufficiency
- procedural fact promotion

During migration, regex may remain as heuristic input, but any `verified` promotion must depend on structured spans, ontology-aware mapping, and truth governance policy.

Current accepted slice order for regex de-authority:

1. `canonical_mapping_step` via typed-entity and lexicon-token mapping
2. `completeness_judge_step` via upstream numeric evidence tokens instead of raw-text regex rescans
3. `procedural_extraction_step`
4. `entity_span_detection_step`

---

## 5. Certification Rule

Certification is mandatory proof, not a second extraction path.

- certification never writes authority truth
- baseline refresh is a reviewed change
- `docs/runs/**` certification artifacts are immutable evidence
- local/CI certification must remain green before truth-policy or regex-authority changes are accepted
