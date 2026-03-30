# V5 SERP Intelligence Protocol

**Status:** working draft companion protocol  
**Purpose:** canonical owner of SERP and competitor intelligence ingestion, reliability, freshness, and opportunity-signal generation for the SEO layer.

---

## 1. Relation To V2 And Foundation

This document depends on:

- `V5_Ultimate_Extraction_Protocol_V2.md` for truth / retrieval / serving boundaries
- `V5_SEO_Foundation_Contracts.md` for artifact class, identity, storage, and scope semantics

This document does not create truth objects and does not authorize truth promotion from SERP or competitor evidence.

---

## 2. Source Classes And Admissibility

Allowed source classes:

- SERP result pages and features
- competitor page HTML and rendered page content
- deterministic keyword datasets
- deterministic search feature observations
- internal site inventory for gap comparison

Disallowed as truth inputs:

- competitor claims as factual authority
- competitor pricing or process claims as procedural fact
- stylistic copy patterns treated as content truth

---

## 3. Query And Crawl Contract

Every SERP intelligence run must declare:

- `market`
- `locale`
- `country_code`
- `visa_type`
- query seed set
- crawl window
- capture timestamp
- fetch method
- extraction version

Competitor crawl inputs must be normalized by canonical URL and tied to a query set or explicit competitor audit batch.

---

## 4. Allowed SERP Intelligence Artifacts

This document owns the production of:

- `SERPPattern`
- `SearchFeature`
- `ContentGap`
- competitor coverage summaries
- structure-pattern summaries
- intent-pattern summaries
- opportunity candidates

These artifacts are product-derived signals only.

---

## 5. Signal Reliability And Freshness

### 5.1 Reliability levels

`serp_signal_reliability_levels`:

- `R0_forbidden`: may not be used downstream except for audit
- `R1_weak`: may suggest exploration, may not drive page decisions alone
- `R2_supported`: may contribute to opportunity scoring when corroborated by multiple observations
- `R3_strong`: stable structural signal suitable for deterministic downstream use

### 5.2 Allowed use by level

| Level | Allowed use |
|---|---|
| `R0_forbidden` | audit only |
| `R1_weak` | analyst review, candidate backlog |
| `R2_supported` | opportunity scoring, content-gap candidate formation |
| `R3_strong` | page-type and section-pattern inputs, safe product heuristics |

### 5.3 Freshness

Every SERP-derived artifact must carry:

- `captured_at`
- `fresh_until`
- `freshness_class`
- `observation_window`

Default freshness classes:

- `volatile`: 7 days
- `standard`: 30 days
- `stable`: 90 days

If freshness expires, the artifact may remain in storage for audit but must not be treated as active decision input until refreshed.

---

## 6. Signal-Vs-Noise Rules

Allowed signals:

- repeated page-type presence across the top set
- repeated section structures
- repeated search-feature presence
- repeated linking patterns
- repeated topic coverage gaps
- repeated intent fulfillment patterns

Noise or forbidden patterns:

- single-competitor copy style
- rhetorical phrasing
- unverifiable claims
- unsupported comparison statements
- marketing promises

Competitor prevalence alone does not create truth, authority, or publishable claim eligibility.

---

## 7. Opportunity Extraction Contract

An opportunity candidate may be emitted only when:

- scope is explicit,
- the gap is tied to the internal site inventory,
- the supporting observations meet at least `R2_supported`,
- freshness is active,
- the artifact has deterministic identity,
- the candidate is explicitly labeled as derived SEO intelligence.

Minimum fields:

- `opportunity_key`
- `scope_signature`
- `missing_topic`
- `support_count`
- `reliability_level`
- `freshness_class`
- `source_batch_ref`

---

## 8. Storage And Rebuild Rules

- Canonical store for SERP-derived artifacts is `Postgres`.
- `SERPPattern`, `SearchFeature`, and `ContentGap` may be projected into `Qdrant`.
- `ContentGap` may also be projected into `Neo4j` when IA consumes it as a graph-visible planning signal.
- SERP-derived artifacts must be rebuildable from captured input batches.

Rebuild is mandatory when:

- crawl version changes,
- normalization logic changes,
- observation window rolls over,
- freshness expires,
- scope normalization changes.

---

## 9. Hard Boundaries

1. SERP and competitor signals are never truth-core.
2. A competitor page may suggest a topic gap, but may not establish a procedural fact.
3. Reliability level is not truth confidence.
4. Stale SERP artifacts may not drive page creation or draft assembly.
5. Unsafe or low-confidence competitor-derived patterns must route to human review through the operations protocol.

---

## 10. Blocking Conditions

Block downstream export when:

- `scope_signature` is missing,
- freshness is expired,
- reliability remains below `R2_supported`,
- the evidence batch is incomplete or corrupted,
- the artifact cannot be linked to a deterministic capture batch,
- the output attempts to classify itself as truth-bearing.

---

## 11. Observation Threshold Addendum

### 11.1 Observation unit

One observation is one distinct `(batch_key, competitor_page_key, pattern_type)` capture tuple after normalization.

### 11.2 Reliability thresholds

`multiple observations` is defined as follows:

- `R2_supported`: at least 3 observations across at least 2 distinct competitor domains within one active observation window, with the normalized pattern recurring in at least 60% of observations
- `R3_strong`: at least 5 observations across at least 3 distinct competitor domains over at least 2 distinct capture batches, with the normalized pattern recurring in at least 80% of observations

Threshold rules:

- stale observations do not count toward active thresholds
- excluded or corrupted captures do not count toward thresholds
- a single-domain burst may not promote a signal above `R1_weak`

### 11.3 Threshold failure behavior

If an artifact drops below its required threshold after freshness rollover or exclusion:

- reliability must be downgraded,
- downstream active-decision use must stop,
- rebuild or review routing must occur according to downstream owner rules.

---

## 12. SERP Analysis Outputs For The SEO Engine

Top-of-SERP analysis must produce machine-usable outputs for the SEO engine, not just descriptive research notes.

Required outputs from SERP analysis and competitor extraction:

- normalized query set
- intent signals
- candidate page types
- section-pattern signals
- search feature observations
- content-gap candidates
- competitor coverage signals
- linking-pattern hints
- blueprint-shaping signals

These outputs may influence:

- page opportunities
- deterministic page taxonomy selection
- site-structure proposals
- internal linking recommendations
- draft structure selection

These outputs may not establish truth.
