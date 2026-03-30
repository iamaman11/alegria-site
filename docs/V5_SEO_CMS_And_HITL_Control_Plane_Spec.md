# V5 SEO CMS And HITL Control Plane Spec

**Status:** build-spec draft  
**Owner:** exact CMS publish contract and SEO HITL control-plane contract  
**Depends on:** `V5_SEO_Draft_Assembly_And_QA_Protocol.md`, `V5_SEO_Operations_And_Optimization_Protocol.md`, `V5_SEO_Live_Schema_Design_Spec.md`, `V5_SEO_Runtime_Step_Contracts_Spec.md`

## 1. Purpose

This document defines the operational control plane for SEO page publication, review, escalation, and resolution.

It is the canonical owner of:

- CMS page contract for SEO-generated pages,
- page revision and state-transition rules,
- publish gate states and blockers,
- canonical URL write and rollback behavior,
- HITL task types, queue states, and resolution payloads,
- blocking semantics between runtime steps and HITL decisions.

This document does not redefine truth admissibility or V2 publish-gate semantics. It applies them to the SEO operating layer.

## 2. Control-Plane Boundaries

### 2.1 Source-of-record rule

Source of record for publishable SEO state remains relational owner tables from the live schema spec.

- CMS is a serving system and content delivery target.
- HITL queue is an operational decision layer.
- Neither CMS nor HITL may mutate truth-core directly.

### 2.2 Owner roles

The following owner roles are referenced in this document:

- `seo_system` for machine-owned transitions
- `seo_editor` for editorial review decisions
- `seo_ops` for operational overrides and release control
- `taxonomy_owner` for registry and classification changes
- `compliance_owner` for restricted publishing decisions

## 3. CMS Page Contract

## 3.1 CMS page record identity

Each CMS page record must map one-to-one to a `site.page_nodes.page_node_key` and one active canonical URL path per `scope_signature`.

### 3.1.1 Required page identity fields

- `page_node_key`
- `scope_signature`
- `canonical_url_path`
- `locale_code`
- `page_type_key`
- `dominant_intent_key`
- `cms_document_id`
- `current_revision_id`
- `current_status`

## 3.2 Required content fields

Each publishable SEO page revision must carry:

- `title`
- `meta_description`
- `h1`
- `body_payload`
- `faq_payload`
- `schema_markup_payload`
- `required_link_payload`
- `traceability_manifest`
- `blueprint_key`
- `draft_key`
- `qa_report_key`
- `freshness_class`
- `review_owner_role`

## 3.3 Metadata and operational fields

- `slug`
- `breadcrumbs_payload`
- `robots_directive`
- `canonical_target_url`
- `open_graph_title`
- `open_graph_description`
- `publish_notes`
- `published_at`
- `deprecated_at`
- `rollback_revision_id`
- `revision_reason`
- `created_at`
- `updated_at`

## 3.4 Revision model

Each page must support immutable revisions.

Revision rules:

- a new publish attempt creates a new revision id
- published revisions remain immutable
- rollback restores a previous immutable revision as the active revision through a new rollback event, not by mutating historical content
- draft revisions may be superseded prior to approval

## 4. CMS State Machine

## 4.1 Canonical page states

Required states:

- `planned`
- `blueprint_ready`
- `draft_ready`
- `ready_for_review`
- `approved`
- `publish_blocked`
- `published`
- `stale`
- `needs_rebuild`
- `deprecated`

## 4.2 Allowed state transitions

| From | To | Allowed owner |
|---|---|---|
| `planned` | `blueprint_ready` | `seo_system` |
| `blueprint_ready` | `draft_ready` | `seo_system` |
| `draft_ready` | `ready_for_review` | `seo_system` |
| `ready_for_review` | `approved` | `seo_editor`, `seo_ops` |
| `ready_for_review` | `publish_blocked` | `seo_editor`, `seo_ops`, `compliance_owner` |
| `approved` | `published` | `seo_ops`, `seo_system` under approved publish workflow |
| `published` | `stale` | `seo_system` |
| `stale` | `needs_rebuild` | `seo_system`, `seo_ops` |
| `needs_rebuild` | `draft_ready` | `seo_system` |
| `published` | `deprecated` | `seo_ops`, `compliance_owner` |
| `publish_blocked` | `ready_for_review` | `seo_editor`, `seo_ops` |

Forbidden transitions:

- direct `draft_ready -> published`
- direct `planned -> published`
- direct `publish_blocked -> published`
- any state transition that bypasses required QA or required HITL blockers

## 4.3 Publish blockers

A page must remain `publish_blocked` if any of the following are unresolved:

- missing `traceability_manifest`
- failing draft QA verdict
- active cannibalization conflict
- canonical URL conflict
- required link obligations unsatisfied
- unsupported factual fragment unresolved
- unsafe SERP pattern dependency unresolved
- registry dependency in pending or deprecated state

## 5. Canonical URL Contract

## 5.1 Write rules

- one active `canonical_url_path` per `scope_signature`
- two pages with different `scope_signature` values may not share canonical URL unless explicitly designated in localization policy
- canonical URL writes require URL conflict check before publish
- URL changes after publish require redirect plan and lifecycle event emission

## 5.2 Conflict handling

A detected URL collision must create a HITL task of type `canonical_url_conflict` and block publication until resolved.

## 5.3 Rollback behavior

When a rollback is executed:

- active page revision changes to the chosen safe revision through a new rollback event
- replaced revision remains historically intact
- required redirects or cache invalidations are emitted as outbox events
- any dependent rebuilds are queued

## 6. CMS Outbox Contract

Required outbox events:

- `seo_page_review_requested`
- `seo_page_approved`
- `seo_page_publish_blocked`
- `seo_page_published`
- `seo_page_deprecated`
- `seo_page_rollback_requested`
- `seo_page_rolled_back`
- `seo_page_rebuild_requested`
- `seo_page_canonical_changed`

Each outbox event must include:

- `event_key`
- `event_type`
- `page_node_key`
- `scope_signature`
- `revision_id`
- `actor_role`
- `causal_step`
- `occurred_at`
- `payload_version`

## 7. HITL Task Contract

## 7.1 Required task types

- `cannibalization_conflict`
- `unsafe_serp_pattern`
- `unsupported_factual_fragment`
- `registry_change`
- `rebuild_suppression`
- `canonical_url_conflict`

## 7.2 Required task fields

Every HITL task record must include:

- `task_key`
- `task_type`
- `queue_state`
- `first_owner_role`
- `current_owner_role`
- `page_node_key` when applicable
- `scope_signature`
- `blocking_step_name`
- `blocking_execution_key`
- `resolution_deadline_at`
- `severity`
- `created_at`
- `updated_at`
- `decision_payload`
- `audit_log_payload`

## 7.3 Queue states

Required queue states:

- `open`
- `in_review`
- `resolved`
- `reopened`
- `superseded`
- `cancelled`

## 7.4 Resolution payload contract

Each resolution payload must include:

- `task_key`
- `decision_key`
- `decision_status`
- `resolved_by_role`
- `resolved_by_actor_id`
- `resolution_reason`
- `resolution_notes`
- `affected_page_node_keys[]`
- `affected_registry_keys[]`
- `resume_step_name`
- `resume_execution_key`
- `apply_override_flag`
- `requested_followup_task_type`
- `resolved_at`

## 7.5 Reopen, supersede, cancel semantics

- `reopened` means prior resolution is no longer sufficient because upstream facts or scope changed
- `superseded` means a newer task replaces the current task; the superseding task key must be recorded
- `cancelled` means the underlying page, draft, or change request no longer exists and no resume is permitted

## 8. Runtime Blocking Matrix

| Blocking runtime step | HITL task type | Blocking condition |
|---|---|---|
| `serp_normalize` | `unsafe_serp_pattern` | pattern reliability below allowed threshold but still relevant to planning |
| `ia_build` | `cannibalization_conflict` | two candidate pages share dominant intent and overlapping scope |
| `draft_assemble` | `registry_change` | required page type, section template, or CTA pattern unresolved |
| `draft_qa` | `unsupported_factual_fragment` | draft contains fragment without allowed traceability label |
| `draft_qa` | `canonical_url_conflict` | publishable revision collides on canonical URL |
| `rebuild_detect` | `rebuild_suppression` | automated rebuild suppression requested for operational reasons |

No blocked step may auto-resume without a valid resolution payload.

## 9. First Owner And Escalation Matrix

| Task type | First owner | Escalation owner |
|---|---|---|
| `cannibalization_conflict` | `seo_editor` | `seo_ops` |
| `unsafe_serp_pattern` | `seo_ops` | `compliance_owner` |
| `unsupported_factual_fragment` | `seo_editor` | `compliance_owner` |
| `registry_change` | `taxonomy_owner` | `seo_ops` |
| `rebuild_suppression` | `seo_ops` | `compliance_owner` |
| `canonical_url_conflict` | `seo_ops` | `compliance_owner` |

## 10. Audit Requirements

Every publish and HITL decision path must be auditable.

Required audit artifacts:

- state transition event
- acting role and actor id
- source draft or page revision key
- triggering runtime step and execution key
- blocking task references
- before/after canonical URL when changed
- traceability manifest hash for any approved publish

## 11. Producer And Consumer Ownership

| Surface | Producer | Consumers |
|---|---|---|
| CMS revision payload | draft assembler + publish adapter | CMS, serving layer, audit tools |
| publish state transition | publish workflow | CMS adapter, monitoring, rebuild detector |
| HITL task record | runtime review adapter | backoffice UI, reviewers, workflow resumer |
| HITL resolution payload | backoffice UI / review adapter | blocked workflow, audit log, monitoring |
| outbox events | publish adapter | cache invalidation, analytics, notification services |

## 12. Acceptance Checks

This document is incomplete until all of the following are true:

- CMS page fields and revision model are explicit
- state machine and forbidden transitions are explicit
- publish blockers are explicit
- canonical URL write and rollback rules are explicit
- HITL task types, queue states, and resolution payload fields are explicit
- runtime blocking semantics are explicit
- producer and consumer ownership is explicit
- future automation can verify publish-gate and HITL wiring against implementation

---

## 13. HITL Pause And Resume Addendum

### 13.1 Pause record contract

Every blocked execution must record:

- `execution_key`
- `step_name`
- `pause_sequence`
- `task_key`
- `paused_at`
- `pause_reason`
- `resume_deadline_at`

### 13.2 Resume validity rules

A resume payload is valid only when:

- it targets the exact `execution_key + pause_sequence`
- the referenced task is in `resolved`
- the task was not superseded or reopened after the resolution timestamp
- the blocking reason still matches the paused execution context

### 13.3 Reopen after resume

If a HITL task is reopened after a step has resumed:

- the prior resume becomes obsolete for future state transitions,
- downstream execution must pause again on a new `pause_sequence`,
- the reopened task or superseding task becomes the new blocker of record.

### 13.4 Repeated pauses

Repeated pauses are allowed only as distinct pause sequences on the same execution.

No runtime step may resume twice from the same pause sequence.

### 13.5 Timeout policy

| Task type | Timeout |
|---|---|
| `cannibalization_conflict` | 72 hours |
| `unsafe_serp_pattern` | 24 hours |
| `unsupported_factual_fragment` | 24 hours |
| `registry_change` | 72 hours |
| `rebuild_suppression` | 24 hours |
| `canonical_url_conflict` | 24 hours |

When timeout expires:

- task escalates to the configured escalation owner,
- blocked runtime step enters `expired`,
- no auto-resume is permitted,
- publication remains blocked where relevant.
