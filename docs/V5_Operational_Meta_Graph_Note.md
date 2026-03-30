# V5 Operational Meta-Graph Note

**Status:** short architectural note  
**Purpose:** define whether an operational meta-graph is required for the SEO super-site system and, if used, what its narrow role must be.

## 1. Decision

`Operational meta-graph` is **optional**, not a required foundation for building the SEO super-site.

The required foundation for the application is:

- core SEO / knowledge graph
- retrieval layer
- page generation and QA contracts

## 2. What It Is

Operational meta-graph is a derived ops-layer graph for tracking system behavior, not domain truth and not primary SEO generation logic.

It exists to support:

- rebuild propagation
- lineage
- dependency tracing
- blocked workflow analysis
- HITL and runtime debugging
- impact analysis after truth, ontology, or template changes

## 3. What It Is Not

It must not become:

- a second source of truth for protocol rules
- a replacement for the SEO / knowledge graph
- a general self-describing architecture graph of the whole system
- a place where SEO or truth semantics are redefined

## 4. When It Is Worth Adding

Add it only when the system needs deterministic answers to questions like:

- which pages are affected by a truth change?
- which drafts or links must rebuild after a blueprint change?
- which blocked runtime steps are waiting on which HITL decisions?
- which published page depends on which upstream objects?

If those questions can still be answered cheaply from relational state plus the main SEO graph, do not add a separate meta-graph yet.

## 5. Minimum Useful Scope

If implemented, the operational meta-graph should stay narrow.

Recommended node classes:

- `TruthObjectRef`
- `PageNodeRef`
- `DraftRef`
- `BlueprintRef`
- `StepExecutionRef`
- `HitlTaskRef`
- `ProjectionJobRef`

Recommended edge classes:

- `AFFECTS`
- `DEPENDS_ON`
- `BLOCKED_BY`
- `GENERATED_FROM`
- `PROJECTED_TO`
- `REBUILDS`
- `SUPERSEDES`

## 6. Recommendation For This Project

Do **not** treat operational meta-graph as a prerequisite for Phase 1 super-site implementation.

Recommended order:

1. build core SEO / knowledge graph
2. build retrieval layer
3. build page structure, linking, drafts, QA, and publish control
4. add operational meta-graph later only if rebuild, lineage, and ops-debugging complexity justifies it

## 7. Hard Rule

If operational meta-graph is added, it must remain a derived operational surface only.

It may observe runtime and dependency state, but it may not own truth, page semantics, or canonical SEO generation rules.
