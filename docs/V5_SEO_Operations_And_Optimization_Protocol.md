# V5 SEO Operations And Optimization Protocol

**Status:** working draft companion protocol  
**Purpose:** canonical owner of SEO review ownership, escalation, metrics, feedback-loop behavior, safe automation, and operational freshness management.

---

## 1. Relation To Other Companion Documents

This document depends on:

- `V5_SEO_Foundation_Contracts.md` for ownership roles and artifact semantics
- `V5_SERP_Intelligence_Protocol.md` for SERP signal freshness and safety
- `V5_SEO_Information_Architecture_Protocol.md` for structure and cannibalization rules
- `V5_SEO_Draft_Assembly_And_QA_Protocol.md` for lifecycle states and rebuild triggers

This document does not redefine artifact identity, scope, or draft field contracts.

---

## 2. Owner Roles

Canonical operational roles:

- `seo_protocol_owner`
- `seo_ia_owner`
- `seo_editorial_owner`
- `seo_ops_owner`
- `seo_registry_curator`
- `domain_reviewer`
- `cms_owner`

---

## 3. SEO Escalation Matrix

`seo_escalation_matrix`:

| Decision class | First owner | Escalation |
|---|---|---|
| intent conflict | `seo_ia_owner` | `seo_protocol_owner` |
| cannibalization conflict | `seo_ia_owner` | `seo_protocol_owner` |
| new page type | `seo_registry_curator` + `seo_ia_owner` | `seo_protocol_owner` |
| forbidden anchor pattern | `seo_ia_owner` | `seo_protocol_owner` |
| unsafe competitor-derived pattern | `seo_ops_owner` | `domain_reviewer` |
| unsupported factual draft claim | `seo_editorial_owner` | `domain_reviewer` |
| rebuild suppression request | `seo_ops_owner` | `seo_protocol_owner` |

Unowned operational decisions are forbidden.

---

## 4. Metrics Contract

This document owns the metrics catalog for:

- rankings
- impressions
- CTR
- clicks
- orphan risk
- link depth
- cannibalization incidence
- freshness decay
- coverage gaps

All metric records must carry:

- `scope_signature`
- `page_node_key` where applicable
- `captured_at`
- `metric_window`
- `metric_source`

---

## 5. Allowed And Forbidden Automations

### 5.1 Allowed automatic actions

- mark pages `stale`
- trigger rebuild jobs from approved triggers
- reprioritize opportunity backlogs
- generate link recommendations
- surface cannibalization findings
- refresh SERP intelligence batches

### 5.2 Forbidden automatic actions

- changing truth objects
- auto-publishing drafts
- auto-merging pages after cannibalization detection
- changing canonical URL ownership without human approval
- suppressing a rebuild caused by truth or scope change without owner approval

---

## 6. Feedback Loop Behavior

Feedback may modify:

- opportunity rank,
- rebuild urgency,
- stale flags,
- link recommendation priority,
- analyst review queues.

Feedback may not directly modify:

- truth status,
- page scope,
- canonical owner of URL,
- blueprint schema,
- registry status.

---

## 7. Freshness And Monitoring

Operational monitors must detect:

- expired SERP intelligence artifacts,
- stale published drafts,
- repeated QA failures,
- unresolved cannibalization conflicts,
- rising orphan risk,
- invalid or missing rebuild execution.

Critical freshness breaches route to `seo_ops_owner`.

---

## 8. Failure And Rollback Posture

If optimization logic behaves unsafely:

- stop automatic reprioritization,
- preserve audit history,
- keep existing published pages stable unless a truth-driven rebuild blocker exists,
- route unresolved conflicts to human owners,
- never roll back by reclassifying a draft or SEO artifact as truth.

---

## 9. Hard Rules

1. Every controversial SEO decision must have a first owner.
2. Metrics may drive prioritization, not truth mutation.
3. Safe automation is opt-in and bounded.
4. Feedback loops must preserve auditability.
5. No optimization workflow may override V2 truth boundaries.

---

## 10. Artifact Freshness SLA Catalog

| Artifact class | Freshness SLA | Breach action |
|---|---|---|
| `SERPPattern` | 30 days default, 7 days if volatile query class | downgrade to inactive decision input and enqueue refresh |
| `KeywordCluster` | 30 days | mark for opportunity recompute |
| `OpportunityCandidate` | 14 days | remove from create queue until refreshed |
| `PageBlueprint` | 90 days or immediate on blueprint version bump | rebuild dependent drafts |
| `PageNode` | 30 days planning freshness, immediate on truth invalidation | mark `stale` |
| `Draft` | 14 days or immediate on truth-change impact | block publish and re-run QA |
| `LinkRecommendation` | 14 days | recompute links before publish |
| `ContentGap` | 30 days | downgrade ranking priority until refreshed |

## 11. Metric Formulas And SLO Thresholds

### 11.1 Core formulas

- `rebuild_lag_hours = now - rebuild_requested_at`
- `draft_qa_fail_rate = failed_draft_qa_runs / total_draft_qa_runs` over 7 days
- `cannibalization_backlog = open_blocking_cannibalization_conflicts`
- `orphan_page_count = published_pages_without_required_parent_or_required_inbound_links`
- `stale_serp_ratio = stale_active_serp_artifacts / total_active_serp_artifacts`

### 11.2 SLO thresholds

| Metric | SLO | Breach action |
|---|---|---|
| rebuild lag | 95% under 24h | escalate to `seo_ops_owner`, freeze non-essential optimization jobs |
| draft QA fail rate | under 10% rolling 7d | inspect templates and traceability contracts |
| cannibalization backlog | zero blocking conflicts older than 72h | escalate to editorial owner |
| orphan page count | zero for published money pages, under 2% overall | trigger linking remediation |
| stale SERP ratio | under 15% | trigger refresh batch |

### 11.3 Rollback posture refinement

If any metric breaches hard SLO for two consecutive windows:

- stop automated promotion of affected derived artifacts,
- keep already-published safe pages stable,
- require human sign-off before resuming auto-publish behavior.

---

## 12. Minimum Update Management Loop

The SEO engine must manage updates continuously after publication.

Minimum update-management responsibilities:

- detect freshness expiry
- detect upstream truth impact
- detect SERP signal drift
- detect cannibalization emergence
- detect link decay or orphan risk
- enqueue rebuilds deterministically
- preserve auditability of all update decisions

### 12.1 Update loop outputs

The minimum operational outputs are:

- `refresh_required`
- `rebuild_required`
- `review_required`
- `publish_hold`
- `deprecate_candidate`

### 12.2 Hard rule for update management

Update management may change derived SEO artifacts, drafts, page priorities, and rebuild queues.

It may not rewrite truth-core or bypass publish governance.


## 13. Linking And Orphan Observability Addendum

### 13.1 Additional formulas

The operations layer must also track the following linking-health formulas:

- `orphan_risk_ratio = orphan_page_count / total_published_pages`
- `required_link_depth_p95 = p95(min_click_depth_to_required_hub_or_parent_for_published_pages)`

These formulas supplement, not replace:

- `orphan_page_count`
- `cannibalization_backlog`
- `rebuild_lag_hours`

### 13.2 Additional thresholds

The following thresholds are mandatory for linking-health monitoring:

- `orphan_page_count = 0` for published money pages
- `orphan_risk_ratio < 0.02` overall
- `required_link_depth_p95 < 3` for money pages
- `required_link_depth_p95 < 4` overall

### 13.3 Breach behavior

If any linking-health threshold is breached:

- affected pages are ineligible for silent auto-promotion,
- linking remediation enters the rebuild or review backlog,
- repeated breaches across two windows escalate to IA owner plus SEO ops owner.
