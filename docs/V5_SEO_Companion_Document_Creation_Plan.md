# V5 SEO Companion Document Creation Plan

**Status:** planning master document for SEO companion protocols and build-spec readiness

## 1. Purpose

This document defines the full documentation stack required to extend `V5_Ultimate_Extraction_Protocol_V2.md` into an implementation-ready SEO super-site program without redefining V2-owned rules.

This file is a planning and coordination document. It is not the canonical owner of the rules that later live in the companion protocols or build-spec documents.

## 2. Documentation Stack

The documentation stack is split into three layers.

### 2.1 Architecture / ownership docs

These documents define normative ownership and operating semantics:

1. `V5_SEO_Foundation_Contracts.md`
2. `V5_SERP_Intelligence_Protocol.md`
3. `V5_SEO_Information_Architecture_Protocol.md`
4. `V5_SEO_Draft_Assembly_And_QA_Protocol.md`
5. `V5_SEO_Operations_And_Optimization_Protocol.md`

### 2.2 Implementation roadmap

This document defines phased delivery:

- `V5_SEO_SUPERSITE_IMPLEMENTATION_ROADMAP.md`

### 2.3 Build-spec docs

These documents are the final pre-code documentation gate:

1. `V5_SEO_Live_Schema_Design_Spec.md`
2. `V5_SEO_Runtime_Step_Contracts_Spec.md`
3. `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`
4. `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`

Documentation is not implementation-ready until all four build-spec documents exist and are linked from this master plan and the implementation roadmap.

## 3. Core Decisions

`V5_Ultimate_Extraction_Protocol_V2.md` remains the canonical owner of:

- extraction pipeline and step contracts,
- truth / retrieval / serving plane boundaries,
- graph identity, merge, dedup, and sync rules,
- ontology governance,
- validation, admissibility, publish gate,
- runtime auditability and HITL ownership.

The SEO layer is introduced as a product-layer operating model above V2, not as a replacement for V2.

The SEO companion architecture uses five owner-documents, plus one implementation roadmap, plus four build-spec documents.

## 4. Conflict And Precedence Rules

- One rule, one home.
- If a companion-spec conflicts with a V2-owned rule, V2 wins.
- If two SEO companion documents conflict, the owner named in the authority matrix wins.
- Non-owner documents may reference a rule, but must not restate it normatively.
- Build-spec documents refine implementation detail, but may not redefine owner semantics from V2 or the companion owner-docs.

**Canonical precedence chain**

`V5_Ultimate_Extraction_Protocol_V2.md > SEO companion owner > build-spec refinement > non-owner reference`

## 5. Build-Spec Prerequisites Before Code

No live implementation changes may begin in any of the following until the corresponding build-spec document exists:

- `app/db/schema.sql`
- `app/contracts/proto/temporal_payloads.proto`
- runtime model DTOs
- Temporal activities and workflows
- Neo4j projection code
- Qdrant projection code
- CMS publish path
- HITL task/runtime path

**No-Go for implementation** if any of the following are missing:

- exact table / column / constraint specification,
- exact runtime step Proto contracts,
- exact graph and retrieval projection shapes,
- exact CMS and HITL control-plane contracts.

## 6. Planned Documents

| Layer | Filename | Primary ownership |
|---|---|---|
| architecture | `V5_SEO_Foundation_Contracts.md` | cross-cutting SEO contracts |
| architecture | `V5_SERP_Intelligence_Protocol.md` | SERP and competitor intelligence |
| architecture | `V5_SEO_Information_Architecture_Protocol.md` | site structure and linking |
| architecture | `V5_SEO_Draft_Assembly_And_QA_Protocol.md` | page planning and draft assembly |
| architecture | `V5_SEO_Operations_And_Optimization_Protocol.md` | operations and optimization |
| roadmap | `V5_SEO_SUPERSITE_IMPLEMENTATION_ROADMAP.md` | phased implementation plan |
| build-spec | `V5_SEO_Live_Schema_Design_Spec.md` | exact SQL design |
| build-spec | `V5_SEO_Runtime_Step_Contracts_Spec.md` | exact runtime step contracts |
| build-spec | `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` | exact Neo4j/Qdrant projection |
| build-spec | `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` | exact CMS and HITL control plane |

## 7. Gap Audit / Mandatory Preconditions

The following gaps must be owned explicitly before implementation can be considered safe.

### 7.1 P0: mandatory before writing new protocols

| Gap | Canonical owner |
|---|---|
| Cross-document authority matrix | `V5_SEO_Foundation_Contracts.md` |
| Artifact classification matrix | `V5_SEO_Foundation_Contracts.md` |
| Deterministic identity for SEO artifacts | `V5_SEO_Foundation_Contracts.md` |
| Storage placement matrix | `V5_SEO_Foundation_Contracts.md` |
| Scope model for SEO pages | `V5_SEO_Foundation_Contracts.md` |
| SEO registries and governance | `V5_SEO_Foundation_Contracts.md` |
| SERP evidence reliability policy | `V5_SERP_Intelligence_Protocol.md` |
| Draft factual traceability contract | `V5_SEO_Draft_Assembly_And_QA_Protocol.md` |

### 7.2 P1: mandatory before implementation

| Gap | Canonical owner |
|---|---|
| Opportunity prioritization model | `V5_SEO_Information_Architecture_Protocol.md` |
| Cannibalization resolution policy | `V5_SEO_Information_Architecture_Protocol.md` |
| Internal linking contract | `V5_SEO_Information_Architecture_Protocol.md` |
| Page blueprint minimum contract | `V5_SEO_Draft_Assembly_And_QA_Protocol.md` |
| Page lifecycle and freshness states | `V5_SEO_Draft_Assembly_And_QA_Protocol.md` |
| Invalidation and rebuild propagation | `V5_SEO_Draft_Assembly_And_QA_Protocol.md` |
| Human review and escalation matrix | `V5_SEO_Operations_And_Optimization_Protocol.md` |

### 7.3 P2: mandatory before scale

| Gap | Canonical owner |
|---|---|
| Metrics and feedback contract | `V5_SEO_Operations_And_Optimization_Protocol.md` |
| Localization and canonical URL policy | `V5_SEO_Information_Architecture_Protocol.md` |
| Content reuse boundaries | `V5_SEO_Information_Architecture_Protocol.md` |

### 7.4 Lower-level build-spec gaps

The following implementation concerns are not closed by the architecture docs and roadmap alone:

- exact live schema specs by table,
- exact Proto contracts for each SEO runtime step,
- exact `StepContractMeta` and step I/O contracts,
- exact Neo4j node/edge projection shapes,
- exact Qdrant collection and payload schema,
- exact CMS fields and state transitions,
- exact HITL task and resolution payloads,
- exact evaluation datasets and pass/fail criteria,
- exact source-to-target migration/backfill mapping.

These are owned by the four build-spec documents.

## 8. Cross-Document Authority Model

### 8.1 Canonical owner map by rule class

| Rule class | Owner |
|---|---|
| extraction, truth, retrieval, serving semantics | `V5_Ultimate_Extraction_Protocol_V2.md` |
| cross-document SEO ownership and artifact semantics | `V5_SEO_Foundation_Contracts.md` |
| SERP source admissibility and reliability | `V5_SERP_Intelligence_Protocol.md` |
| page taxonomy, intent, structure, URL policy, linking, cannibalization | `V5_SEO_Information_Architecture_Protocol.md` |
| blueprint, draft, traceability, lifecycle, rebuild, QA | `V5_SEO_Draft_Assembly_And_QA_Protocol.md` |
| escalation, metrics, feedback loop, automation safety | `V5_SEO_Operations_And_Optimization_Protocol.md` |
| exact SQL shapes and constraints | `V5_SEO_Live_Schema_Design_Spec.md` |
| exact runtime step contracts and Proto messages | `V5_SEO_Runtime_Step_Contracts_Spec.md` |
| exact graph / retrieval projection contracts | `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` |
| exact CMS / HITL control-plane contracts | `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` |

### 8.2 Non-owner reference rule

A document may mention another document's rule only to:

- state a dependency,
- restate a boundary in non-normative prose,
- name the consuming interface,
- declare that enforcement happens elsewhere.

It may not:

- redefine required fields,
- weaken blocking semantics,
- change artifact class,
- change the precedence chain.

## 9. Build-Spec Document Dependencies

- `V5_SEO_Live_Schema_Design_Spec.md` depends on foundation + IA + draft + ops docs.
- `V5_SEO_Runtime_Step_Contracts_Spec.md` depends on foundation + SERP + IA + draft + ops docs + automation rules.
- `V5_SEO_Graph_And_Retrieval_Projection_Spec.md` depends on foundation + SERP + IA + live schema spec.
- `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md` depends on draft + ops + live schema spec + runtime step contracts spec.

## 10. Build-Spec Completeness Criteria

A build-spec document is incomplete until it includes:

- a normative table or matrix section,
- exact entity and state definitions,
- exact producer and consumer ownership,
- blocking rules,
- rebuild or migration semantics where relevant,
- acceptance checks mapped to automation or future automation.

## 11. Architecture / Ownership Document Specifications

## 11.1 Foundation Contracts

### Filename

`V5_SEO_Foundation_Contracts.md`

### Scope

- cross-document authority matrix,
- artifact classification matrix,
- deterministic identity rules,
- storage placement matrix,
- page scope contract,
- SEO registry catalog and governance posture.

## 11.2 SERP Intelligence

### Filename

`V5_SERP_Intelligence_Protocol.md`

### Scope

- SERP and competitor signal ingestion,
- allowed SERP-derived artifacts,
- source reliability and freshness,
- signal-vs-noise rules,
- opportunity inputs for IA.

## 11.3 Information Architecture

### Filename

`V5_SEO_Information_Architecture_Protocol.md`

### Scope

- page taxonomy,
- dominant and secondary intent,
- hierarchy and canonical URL policy,
- opportunity prioritization,
- internal linking contract,
- cannibalization policy,
- content reuse boundaries,
- localization policy.

## 11.4 Draft Assembly And QA

### Filename

`V5_SEO_Draft_Assembly_And_QA_Protocol.md`

### Scope

- page brief contract,
- page blueprint minimum contract,
- draft assembly rules,
- metadata obligations,
- traceability labels,
- page lifecycle states,
- rebuild triggers,
- page-level QA gates.

## 11.5 Operations And Optimization

### Filename

`V5_SEO_Operations_And_Optimization_Protocol.md`

### Scope

- human review ownership,
- escalation matrix,
- metrics and feedback contract,
- safe automatic actions,
- freshness monitoring and optimization operations.

## 12. Build-Spec Document Specifications

## 12.1 Live Schema Design Spec

### Filename

`V5_SEO_Live_Schema_Design_Spec.md`

### Scope

- exact tables,
- columns and types,
- PK/FK,
- unique constraints,
- check constraints,
- status enums,
- indexes,
- schema family placement,
- source-of-record vs projection stores,
- migration notes.

## 12.2 Runtime Step Contracts Spec

### Filename

`V5_SEO_Runtime_Step_Contracts_Spec.md`

### Scope

- exact runtime step list,
- exact Proto payload set,
- exact `StepContractMeta`,
- exact I/O contracts,
- retry and failure contracts,
- HITL pause conditions,
- storage writes and outbox writes,
- downstream consumers.

## 12.3 Graph And Retrieval Projection Spec

### Filename

`V5_SEO_Graph_And_Retrieval_Projection_Spec.md`

### Scope

- exact Neo4j node labels,
- exact Neo4j edge types,
- exact upsert identity,
- exact Qdrant collection definitions,
- exact payload schema,
- projection rebuild and invalidation rules.

## 12.4 CMS And HITL Control Plane Spec

### Filename

`V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`

### Scope

- exact CMS fields,
- exact page state transitions,
- exact publish blockers,
- exact outbox event shapes,
- exact HITL task types,
- exact resolution payloads,
- exact review queue and escalation behavior.

## 13. Recommended Creation Order

1. Lock V2 and the five companion owner-docs.
2. Maintain the implementation roadmap.
3. Write the four build-spec documents.
4. Link the build-spec documents from this master plan and the roadmap.
5. Only then begin live changes in schema, Proto, runtime, graph, retrieval, CMS, and HITL code.

## 14. Acceptance Checklist

- every lower-level implementation concern maps to exactly one build-spec document,
- no concern is split across two documents without a declared owner,
- the master plan clearly separates architecture / ownership docs, implementation roadmap, and build-spec docs,
- build-spec prerequisites before code are explicit,
- documentation is not marked implementation-ready until all four build-spec docs exist,
- a future implementer can answer what SQL, Proto, Temporal steps, Neo4j/Qdrant projection shapes, and CMS/HITL states to add without inventing missing contracts.

## Readiness Gate Reference

Supporting readiness document:

- `V5_SEO_Implementation_Readiness_Gate.md`

This document consolidates the build-spec package into one go / no-go checklist before Phase 1 code work. It does not own normative rules and must not redefine owner semantics.

## Build-Spec Resolution Order Addendum

Practical implementation-order resolution is:

1. `V5_SEO_Live_Schema_Design_Spec.md`
2. `V5_SEO_Runtime_Step_Contracts_Spec.md`
3. `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`
4. `V5_SEO_CMS_And_HITL_Control_Plane_Spec.md`
5. `V5_SEO_Implementation_Readiness_Gate.md`

This order is execution order, not a redefinition of normative ownership.

Dependency references between build-spec docs are allowed and do not imply cyclic ownership.

## Appendix. V2 Integration Touchpoints

The SEO layer must stay coupled to V2 through explicit integration points only.

### A.1 Truth-change inputs from V2

- verified topic change events
- verified concept change events
- verified rule change events
- verified operational entity change events

### A.2 Ontology and registry touchpoints

- ontology change invalidation from V2 governance
- alias and canonical-name changes affecting scope or coverage manifests

### A.3 Graph touchpoints

Allowed cross-layer edges remain limited to read-only reference use:

- `COVERS_TOPIC`
- `COVERS_CONCEPT`
- `COVERS_RULE`
- `USES_OPERATIONAL_ENTITY`

### A.4 Publish-gate dependencies

SEO publish gates depend on V2 for:

- traceable verified fact support
- publish admissibility boundaries
- truth / retrieval / serving separation

### A.5 Retrieval separation boundaries

- SEO retrieval surfaces are derived-only
- truth retrieval remains independent and may not be reclassified by SEO outcomes
- SEO signals may prioritize assembly, but may not mutate truth status

## Expert Data Model Reference

Supporting implementation guide:

- `V5_SEO_Expert_Data_Model_Plan.md`

This guide defines the direct-to-expert Neo4j and Qdrant data model for super-site generation. It does not replace owner-documents or build-spec ownership.
