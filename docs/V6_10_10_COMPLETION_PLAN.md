# V6 10/10 Completion Plan

**Status:** current execution and closure satellite  
**Class:** `current-execution-satellite`  
**Parent owner document:** [V6_Expert_Truth_Graph_Runtime.md](V6_Expert_Truth_Graph_Runtime.md)  
**Execution satellite:** [V6_SeoSiteBuildWorkflow_Working_Plan.md](V6_SeoSiteBuildWorkflow_Working_Plan.md)  
**Production/supersite closure plan:** [PRODUCTION_10_10_PLAN.md](PRODUCTION_10_10_PLAN.md)

---

## 1. Purpose

This document tracks what separates the accepted V6 runtime from the intended Alegria product:

> a production system that can create, publish, maintain, and improve a high-quality supersite in any selected niche without rewriting the generic core for each vertical.

The audit standard is therefore not “does the current 56-step implementation agree with its own documents?” It is “does the implementation actually achieve the supersite objective, and are all declared capabilities materially used?”

The detailed ordered remediation program and all current audit findings live in `PRODUCTION_10_10_PLAN.md`. This file keeps the V6 owner/satellite completion policy aligned with that stricter product-level standard.

---

## 2. Corrected 10/10 closure rule

`10/10` is reached only when all of the following are true together:

1. the canonical forward runtime is singular, replay-safe, and fail-closed;
2. truth authority remains singular and machine-verifiable;
3. a selected niche can be compiled into a reviewed/versioned domain contract from discovery evidence without engineering changes to the generic core;
4. a fresh domain/context can bootstrap from zero pre-existing verified truth;
5. domain-specific identity, ontology, extraction, page structure, source policy, applicability, and conversion behavior are supplied through a versioned domain pack rather than hard-coded generic-core visa semantics;
6. Neo4j/GDS materially participates in the graph responsibilities declared by the architecture, and required graph reasoning cannot be silently bypassed;
7. Voyage/Qdrant materially participates in semantic discovery/retrieval responsibilities while remaining non-authoritative;
8. page generation and publish success are judged by product outcomes, not only process exit status;
9. approved candidate build, public published snapshot, deployment, live verification, and final publish are distinct states;
10. effective dates, freshness, applicability, evidence, and public citations are enforced on the active publish path;
11. the public renderer is secure, accessible, useful, scalable, and deploys atomically;
12. freshness/rebuild/GSC/analytics close the maintenance and outcome loop;
13. full local/CI and environment-specific operational certification remains green;
14. the same generic core passes certification in multiple materially different niches without core changes.

A successful process exit, a reachable provider, a non-empty synthetic graph/vector collection, or a locally rendered page is not sufficient evidence of 10/10.

---

## 3. Required pre-runtime capability: Niche Compiler / Domain Discovery

A manually authored domain pack for every new vertical is not sufficient to satisfy “any selected niche.” That would make Alegria a framework that engineers adapt per vertical, not an engine that can create a supersite for a newly selected niche.

Before ordinary site-build runtime starts, Alegria needs a first-class niche-discovery/compiler workflow.

### Input

At minimum:

- niche/topic seed;
- target market/geography;
- target language/locales;
- site/business goal and conversion model;
- risk class / review strictness;
- optional known competitors or authoritative sources.

### Discovery responsibilities

The compiler must use real SERP/source evidence and semantic analysis to propose:

- domain/entity taxonomy;
- relation/ontology schema;
- fact/rule types and parameter schemas;
- applicability dimensions;
- source classes, authority hierarchy, independence groups, jurisdiction/applicability rules, freshness TTLs;
- user-intent taxonomy and query families;
- page archetypes and section/content-block patterns;
- URL/localization strategy;
- graph node/edge model and required reasoning algorithms;
- retrieval collections/surfaces;
- completeness criteria;
- conversion/CTA/event model;
- review/HITL policy;
- freshness/withdrawal policy.

### Compilation contract

- the result is a machine-readable `DomainPackCandidate`;
- every inferred field carries evidence/reasoning provenance;
- ambiguity is explicit and routed to HITL;
- a human/domain expert may edit/approve the candidate;
- approval produces an immutable/versioned `DomainPack`;
- the generic runtime consumes the approved pack without vertical-specific core code changes.

### Evolution

The domain pack must be versioned and evolvable from new source/SERP/user evidence. Pack changes are migration events with impact analysis, not silent prompt changes.

### Acceptance

Starting from only a niche seed + market/locale/goals, the system can produce an approved domain pack and then execute the normal fresh-domain certification. Repeating this for materially different niches must not require editing generic Rust orchestration/domain-independent crates.

---

## 4. Corrected Graph/GDS position: core, not optional enrichment

The previous version of this completion plan classified graph influence on `page_brief`, `draft_*`, `publish_*`, and `rebuild_detect` as optional post-10/10 enrichment. That position is retired.

For the intended supersite engine, Graph/GDS is a first-class reasoning plane. It remains non-authoritative for truth, but its product responsibilities must be real and measurable.

### Required distinction

- **Truth Plane:** decides what can be treated as verified/admissible factual truth.
- **Graph Plane:** reasons over relationships/topology/coverage/dependencies; it cannot promote truth.
- **Retrieval Plane:** provides semantic recall/similarity/ranking; it cannot promote truth.

### Hard-fail rule

When a canonical generation/rebuild scenario declares graph reasoning required, missing/stale/incomplete Neo4j projection or required GDS algorithm output must block that new build/rebuild.

This must **not** make the already published site unavailable. Failure of a reasoning dependency blocks unsafe new publication; serving the last known good release is a separate availability concern.

### `100% functionality` rule

`100%` means all product-relevant graph capabilities declared by the architecture are actually:

- executed;
- consumed by downstream decisions;
- persisted with reason/version evidence;
- regression-tested;
- observable in final IA/linking/coverage/rebuild behavior.

It does not mean calling every algorithm provided by Neo4j GDS.

### Required graph responsibilities before 10/10

- domain-pack graph schema and integrity;
- community/topic/silo structure;
- hub/authority/centrality signals where applicable;
- orphan/disconnected-region detection;
- cannibalization/overlap neighborhoods;
- internal-link and journey topology;
- ontology/topic/rule-to-page coverage;
- dependency/rebuild impact expansion;
- graph-derived reason packages and algorithm/projection versioning;
- ablation tests proving graph reasoning changes expected product decisions.

A `gds.version()` probe proves capability availability only. It is not evidence that GDS is fulfilling these responsibilities.

---

## 5. Current high-severity gaps

The current codebase has strong truth/provenance/orchestration foundations, but these gaps prevent product-level 10/10:

### 5.1 Generic supersite boundary

- current core identity is visa-specific (`visa_contexts`, visa family/subtype, citizenship);
- SEO scope/page identity does not fully represent truth applicability identity;
- `/visa/...`, country mapping, applicant profiles, rule roles, page roles, and content blocks are hard-coded around the visa vertical;
- locale handling includes Russian-specific concept labels;
- there is no first-class versioned Domain Pack contract proving arbitrary-niche portability;
- there is no Niche Compiler that can derive a domain pack from a newly selected niche without engineering work.

### 5.2 Fresh-scope bootstrap

- canonical workflow requires non-empty verified support before discovery/crawl/extraction can create first truth;
- synthetic seeds can hide this flaw in local gates.

### 5.3 Graph/GDS use vs graph/GDS requirement

- graph capability is hard-required more strongly than current GDS reasoning contribution warrants;
- parts of “graph context” are reconstructed from relational/runtime state;
- some conflict/coverage/rebuild reasoning paths use SQL or retrieval rather than graph topology;
- GDS availability checks exist, but the intended algorithmic product responsibilities are not yet fully active/proven.

### 5.4 Truth lifecycle

- support read path does not yet synchronously enforce all effective/freshness constraints;
- verification observation time and rule effective time are conflated in support state;
- public citation URL/evidence data is insufficiently propagated;
- source authority and jurisdiction/applicability need distinct enforcement.

### 5.5 Workflow/product result semantics

- page-level publication blockers can be skipped while the enclosing page loop completes;
- process completion and product success are not fully separated.

### 5.6 E2E/HITL certification

- current Python E2E harness does not faithfully prove the canonical durable HITL lifecycle;
- Cargo workspace invocation is not clean-checkout-safe from the current harness working directory;
- scenario verdicts need stronger business assertions.

### 5.7 Publish architecture

- a simple `published`-only static snapshot is insufficient because approved target revisions must be materialized before final publish;
- public snapshot and candidate build input must be separate;
- current branch's simple published-only change is therefore not merge-safe without candidate-build redesign;
- current publish completion is local-artifact/DB-centric rather than deploy + live verification;
- build output/promotion is not yet an immutable atomic release model.

### 5.8 Renderer/security/scaling

- raw generated Markdown/HTML safety boundary is not strong enough for untrusted source/LLM input;
- render validation is marker-based rather than semantic/security-complete;
- “incremental” build still performs whole-site rendering work before filtering target artifacts;
- global renderer navigation uses page catalog rather than bounded persisted navigation topology.

### 5.9 Maintenance/outcome loop

- freshness workflow is primarily passive reporting;
- GSC service is currently connectivity-oriented, not full performance/index ingestion;
- production DR is not proven by local Docker dumps alone.

---

## 6. Ordered completion tranches

### Tranche A — Niche Compiler / Domain Discovery

**Status:** missing product capability  
**Priority:** P0

Compile a selected niche into a reviewed, versioned domain-pack candidate using real SERP/source evidence and explicit HITL for ambiguity.

**Acceptance:** a new niche can be configured without editing generic core code.

### Tranche B — Domain Pack / arbitrary-niche runtime boundary

**Status:** missing core product abstraction  
**Priority:** P0

Build the versioned Domain Pack contract and move visa-specific identity, schemas, page roles, source policy, applicability, localization, graph schema/algorithms, retrieval policy, completeness, and conversion goals behind it.

**Acceptance:** the same generic core can load multiple materially different domain packs without core-code edits.

### Tranche C — Fresh-domain truth bootstrap

**Status:** current canonical ordering is blocking  
**Priority:** P0

Allow initial support to be empty, discover/crawl/extract/adjudicate/write first truth, then enforce non-empty complete fresh support before planning/drafting.

**Acceptance:** a zero-truth domain/context reaches a trustworthy reviewed candidate page without synthetic authority seeds.

### Tranche D — Full Graph/GDS reasoning closure

**Status:** partially active  
**Priority:** P0

Move from capability/projection proof to algorithmic product proof across IA, clusters/silos, hubs, linking, coverage, cannibalization, orphan detection, and rebuild impact.

**Acceptance:** graph ablation tests demonstrate expected product-decision changes and hard-required builds fail if required graph outputs are unavailable/stale.

### Tranche E — Retrieval reasoning closure

**Status:** partially active  
**Priority:** P0

Separate synthetic capability probes from real data readiness and prove semantic retrieval contribution on real launch scopes.

**Acceptance:** retrieval evaluation and traces demonstrate product use without authority leakage.

### Tranche F — Truth validity/applicability/freshness/public citations

**Status:** incomplete  
**Priority:** P0

Enforce effective/freshness validity at read/publish time; separate verification/effective timestamps; expose safe public provenance; enforce source jurisdiction/applicability.

**Acceptance:** stale/expired/inapplicable/untraceable critical truth cannot reach public output.

### Tranche G — Workflow result and HITL/E2E correctness

**Status:** incomplete  
**Priority:** P0

Make page failures explicit, fix clean-checkout harness execution, and certify the real Temporal review/resume path.

**Acceptance:** workflow/product verdicts cannot hide zero/partial publish failure and one immutable run proves actual HITL.

### Tranche H — Candidate build, deploy, live verification, atomic publish

**Status:** architecture incomplete  
**Priority:** P0

Separate approved candidate from public snapshot, build immutable releases, validate, deploy, live-fetch/hash-check, atomically promote, and keep rollback.

**Acceptance:** `published` means live verified at the production origin.

### Tranche I — Renderer security and product utility

**Status:** incomplete  
**Priority:** P0

Sanitize generated content, validate schema/canonical/links/locale/accessibility, expose user-facing provenance, and render domain-specific structured blocks with a real design system.

**Acceptance:** malicious generated content is inert and representative users can solve the target intent from the page.

### Tranche J — Scale/incremental topology

**Status:** incomplete  
**Priority:** P1

Make rebuild cost dependency-bounded, navigation tree-driven, sitemap-sharded, release-atomic, and load-test large metadata/page sets.

### Tranche K — Active maintenance/outcome loop

**Status:** incomplete  
**Priority:** P1

Turn freshness into recrawl/revalidation/rebuild behavior; ingest GSC/analytics; feed search/user outcomes into planning; complete production DR/SLOs.

### Tranche L — Cross-niche certification

**Status:** not yet proven  
**Priority:** required before global 10/10 claim

Certify at least three materially different niche seeds through niche compilation, fresh bootstrap, publish, change/rebuild, dependency failure, and security scenarios with no generic core changes.

---

## 7. Current accepted foundations to preserve

The following are strong foundations and should be evolved rather than replaced:

- `SeoSiteBuildCanonicalCutoverWorkflow` as the forward durable orchestration surface;
- ports/adapters and Rust layering;
- deterministic truth validation/adjudication;
- singular verified truth authority;
- provenance/evidence snapshots;
- execution ledger, attempts, payload blobs, outbox, reconcile, DLQ;
- HITL review concept;
- projection barriers;
- dependency-aware rebuild concept;
- graph/retrieval non-authority boundaries;
- truth certification fixtures/gates;
- build-id/replay/drain discipline;
- local backup/restore drills as acceptance infrastructure.

The goal is not to delete complexity that has a product purpose. The goal is to make every retained capability earn its place in the actual supersite value stream.

---

## 8. Non-negotiable guardrails

Forbidden during closure:

- introducing a second truth authority path;
- letting Graph/GDS or retrieval promote truth;
- requiring engineers to rewrite generic core logic for every newly selected niche while claiming arbitrary-niche capability;
- claiming arbitrary-niche support while generic core still encodes visa semantics;
- keeping a hard-required subsystem that only health-checks but does not materially affect product decisions;
- bypassing graph/retrieval when the domain pack marks their reasoning outputs required;
- using synthetic seeds/probes as launch-scope readiness evidence;
- treating `PENDING_CREDENTIALS`, zero pages, zero verified facts, or partial publication as product success;
- marking local build completion as publication;
- exposing approved/unvalidated content through public sitemap/navigation;
- publishing stale/expired/inapplicable critical facts;
- accepting raw generated HTML/JSON-LD without a security boundary;
- hiding doc/runtime/product-goal mismatches behind internal contract consistency.

---

## 9. Completion evidence hierarchy

Evidence is ordered from weakest to strongest:

1. code exists;
2. unit/fixture test passes;
3. subsystem capability probe passes;
4. projection/materialization exists;
5. real scope uses the subsystem;
6. downstream product decision consumes the output;
7. final public artifact visibly reflects the decision;
8. production outcome/maintenance loop measures it;
9. the capability transfers unchanged across multiple automatically compiled domain packs.

A 10/10 claim requires reaching the highest applicable level for each core capability.
