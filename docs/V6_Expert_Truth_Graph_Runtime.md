# V6 Expert Truth Graph Runtime

**Status:** canonical owner document
**Class:** `current-runtime-and-knowledge-owner`
**Owner:** Alegria SEO runtime and knowledge architecture
**Supersedes:** `V5_Ultimate_Extraction_Protocol.md`, `SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md`
**Priority rule:** if any `V5` owner or execution document conflicts with this file, `V6` wins.
**Satellite docs:** `V5_Truth_Extraction_LLM_Contract.md`, `V5_Runtime_Contract.md`, `V5_SEO_Graph_And_Retrieval_Projection_Spec.md`, `OPS_RUNTIME_RUNBOOK.md`, `OPS_TEMPORAL_PRODUCTION_GATE.md`, `V6_SeoSiteBuildWorkflow_Working_Plan.md`, `V6_Support_Process_Registry.md`, `V6_Truth_Governance_Policy.md`

---

## 1. Purpose And Authority

This file is the single document the team works from and versions.

It exists to replace the split between:

- protocol intent in `V5_Ultimate_Extraction_Protocol.md`
- execution and migration intent in `SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md`

This file owns:

- the current active runtime shape,
- the target expert knowledge architecture,
- the authority boundaries between truth, graph, and retrieval,
- the status of active, partial, deferred, and legacy surfaces,
- the classification rule for `code-present, runtime-inactive` sources,
- the execution sequence for returning the full rich expert flow.

This file does not replace low-level implementation satellites that define:

- exact LLM prompt and wire format,
- exact runtime persistence and retry semantics,
- exact Neo4j/Qdrant projection contracts,
- support-process contracts outside the 56-step flow,
- rollout, replay, restore, and production-gate operations.

Those files remain active, but they are satellites, not competing owner-documents.

The detailed versioned execution plan for evolving `SeoSiteBuildWorkflow` lives in:

- [V6_SeoSiteBuildWorkflow_Working_Plan.md](V6_SeoSiteBuildWorkflow_Working_Plan.md)

The named support-plane contracts that are outside the 56-step flow live in:

- [V6_Support_Process_Registry.md](V6_Support_Process_Registry.md)

The current truth-governance policy tables and regex authority boundary live in:

- [V6_Truth_Governance_Policy.md](V6_Truth_Governance_Policy.md)

That working plan has been expanded to match the actual active runtime surface, including support loading, publish-control phases, projection barriers, rebuild detection, and explicit run-mode/scenario branching semantics.

The working plan's 56-step flow is the canonical `SeoSiteBuildWorkflow` value stream. It is not an exhaustive list of every support process in the repository. Runtime substrate, release gates, migrations, contract generation, asynchronous outbox workers, backup/restore, monitoring, analytics, and legacy/test/lab surfaces remain required support planes around the flow and must be listed in [V6_Support_Process_Registry.md](V6_Support_Process_Registry.md).

---

## 2. Current Runtime Truth

### 2.1 Active production workflow

The active forward workflow for new production-style site-build execution is:

- `SeoSiteBuildCanonicalCutoverWorkflow`

Compat/drain workflow still present during the rollout window:

- `SeoSiteBuildWorkflow`
  - retained for replay-safe compatibility and explicit legacy drain only
  - not the forward architecture target for new feature work

Additional first-class rollout/support workflows remain available only for controlled investigation and diagnostics:

- `ExpertExtractionWorkflow` as migration-only diagnostic extraction surface;
- `ExpertDecomposedExtractionWorkflow` as migration-only diagnostic decomposition surface;
- `ExpertProjectionWorkflow` as migration-only diagnostic truth-to-projection surface;
- `ExpertSemanticSliceWorkflow` as migration-only diagnostic real-section semantic surface;
- `FreshnessCheckWorkflow` for scheduled freshness monitoring;
- `ProjectionReconcileWorkflow` for explicit graph/retrieval reconcile execution.

`Expert*Workflow` surfaces are not registered by default in the worker fleet. They require explicit opt-in through `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true` and are retained only for controlled diagnostics during the compatibility window.

The current active runtime sequence for the forward path is the accepted canonical cutover flow:

- certification remains a proof layer only and never writes authority truth;
- corroboration must respect source independence rather than raw source count;
- single-source verification is allowed only through explicit authority-override policy;
- regex may remain in utility surfaces or heuristic hints, but not as final authority for `verified` truth promotion.
- the active truth-governance layer is accepted locally only because the frozen certification baseline remains green under it.
- `Phase C / Regex De-Authority` is accepted locally end-to-end; regex no longer serves as final authority anywhere in the expert truth path.

1. preflight, support loading, source discovery, crawl, evidence preparation, and section gating;
2. semantic routing, span detection, canonical mapping, ontology intake, layer-specific extraction, completeness, resolution, contradiction handling, adjudication, and verified truth write;
3. graph/retrieval admissibility, `Neo4j` sync, `Voyage/Qdrant` sync, and projection barriers;
4. planning and global reconcile;
5. per-page drafting, QA, review, publish-control, publish materialization, and publish barrier;
6. conditional `rebuild_detect`.

The legacy macro-step sequence centered on `raw_knowledge_ingestion` remains relevant only for compat/drain understanding of `SeoSiteBuildWorkflow`, not as the active forward runtime description.

Current truth extraction policy:

- `candidate_only_truth_extraction@1`
- LLM extracts candidates only
- validator assigns `structured | needs_hitl | rejected`
- adjudication is required before `verified`
- `truth_admissibility_gate` blocks drafting without admissible verified truth

Current rollout-safe promotion rule:

- `SeoSiteBuildCanonicalCutoverWorkflow` is the accepted active forward path;
- `SeoSiteBuildWorkflow` stays alive only for the explicit compatibility window and replay/drain discipline;
- no new product logic should land in the `Expert*Workflow` family.
- the explicit naming decision is to keep `SeoSiteBuildCanonicalCutoverWorkflow` as the runtime workflow type after drain, rather than creating another workflow type only to rename it.

Current certification state:

- local/CI truth certification is accepted against [docs/runs/truth_certification_local_ci_baseline.json](/home/bose/projects/alegria-site/docs/runs/truth_certification_local_ci_baseline.json);
- the accepted baseline currently contains 16 fixtures, including explicit proof cases for mirror non-independence, weak-source corroboration rejection, single-source authority override, override denial for non-authoritative sources, and stale primary-authority override blocking;
- certification remains evidence-only and never writes authority truth;
- `automation/run_truth_certification_gate.sh` is the canonical certification runner and diff gate against the accepted baseline, and it uses an isolated Cargo target dir for the suite itself so shared build-cache corruption cannot silently weaken proof execution;
- truth-governance changes and regex-authority changes must stay behind that regression gate.

Current execution-plan semantics:

- `run_mode` is normalized before execution plan build
- `scenario` and `policy` decide whether planning, publish, and rebuild phases are present
- publish phases are conditional, not unconditional
- workflow may terminate early with `done:no_pages`

### 2.2 Current live blocker

The current live blocker is external:

- no configured Gemini truth extraction provider
- canonical envs: `GEMINI_API_KEY` or `GOOGLE_API_KEY`

The current live blocker is not:

- DB baseline drift
- raw page persist drift
- regex extraction
- source-tier auto-verify

---

## 3. Three Planes Architecture

### 3.1 Truth Plane

Authority lives here.

Canonical stores and stages:

- `extracted.rule_candidates`
- deterministic validator
- deterministic adjudication
- `verified.rule_instances`

Rules:

- LLM may extract and normalize
- LLM may not assign `verified`
- retrieval may not promote truth status
- graph may not promote truth status
- page drafting may consume truth but may not mutate it

### 3.2 Graph Plane

Graph is a projection and semantic assembly plane, not truth authority.

Canonical duties:

- build graph-safe semantic objects and relations
- enforce graph integrity contracts
- project admissible knowledge into `Neo4j`

Rules:

- `Neo4j` is projection only
- graph sync happens after truth and semantic assembly gates
- graph richness may increase reasoning quality but must not redefine truth

### 3.3 Retrieval Plane

Retrieval is an indexing and search-support plane, not truth authority.

Canonical duties:

- embeddings via `Voyage AI` or another configured embedding provider
- vector storage and retrieval via `Qdrant`
- clustering, topic support, semantic recall, and similarity search

Rules:

- `Voyage AI` is embedding generation only
- `Qdrant` is retrieval/search support only
- retrieval score is not truth confidence
- retrieval artifacts may be stale without changing verified truth

---

## 4. Canonical Active Flow

### 4.1 What runs today

1. `DataForSEO` returns SERP candidates.
2. Crawler fetches pages and persists `raw.pages` / `raw.sections`.
3. Truth extraction LLM receives one `raw.section` at a time.
4. LLM returns strict candidate JSON.
5. Deterministic validator checks:
   - role
   - concept key
   - params completeness
   - evidence span integrity
   - freshness/completeness ambiguity
6. Candidate receives:
   - `structured`
   - `needs_hitl`
   - `rejected`
7. Adjudicator groups `structured` candidates by semantic identity.
8. Adjudicator decides:
   - `verified`
   - `needs_hitl`
   - `rejected`
9. Only adjudicated admissible truth reaches `verified.rule_instances`.
10. `truth_admissibility_gate` blocks page drafting if admissible truth is absent.

### 4.2 Who does what

- truth extraction LLM:
  - extractor / normalizer only
- validator:
  - deterministic structural and evidence judge
- adjudicator:
  - deterministic truth verdict gate
- page drafting LLM:
  - editorial writer only

---

## 5. Target Expert Flow

The target expert architecture is richer than the current active path and must be returned on top of the stabilized truth core.

The full executable target sequence is the 56-step flow in [V6_SeoSiteBuildWorkflow_Working_Plan.md](V6_SeoSiteBuildWorkflow_Working_Plan.md).

Owner-level grouped sequence:

1. preflight, verified-support loading, SERP discovery, crawl, and raw evidence registration;
2. whole-page, utility, DOM relevance, sectioning, sectioning-contract, and CAS gates;
3. layer routing, subspan routing, entity spans, canonical mapping, and ontology intake;
4. strict procedural extraction plus operational, editorial, SEO, and commercial extraction;
5. schema validation, candidate validation, triple building, completeness judge, resolution loop, contradiction gate, truth adjudication, and verified truth write;
6. graph/retrieval admissibility, `Neo4j` projection, `Voyage/Qdrant` projection, and projection barriers;
7. SERP normalization, opportunity building, IA, internal linking, and global site reconcile;
8. truth-admissible draft assembly, editorial generation, draft normalization, content contract validation, draft QA, CMS/HITL, publish materialization, preview validation, and final publish;
9. rebuild detection from truth, ontology, graph, retrieval, navigation, or publish changes.

This is not a replacement for truth-core.

It is a richer semantic layer built on top of truth-core.

### 5.1 Coverage boundary

The target flow captures all current-run SEO/truth/site-build product behavior.

It deliberately does not absorb these required support planes:

- Temporal worker build-id, rollout, drain, replay, metrics, and durable-history policy;
- Proto/FBS generation, SQL migrations, SQLx offline metadata, and schema/contract parity checks;
- asynchronous outbox materialization, reconcile workers, stale-outbox reclaim, and projection consumers;
- backup/restore drills, production gates, CI, smoke tests, and immutable run evidence;
- scheduled freshness checks, GSC/analytics ingestion, monitoring, and operator dashboards;
- PII redaction, licensing checks, quality-policy registries, and HITL operator surfaces that gate drafting or publish;
- legacy, test-only, and lab-only workflows unless a future version explicitly promotes them.

Support planes may block release or strict-mode workflow progress through gates and barriers, but they are not truth extraction stages and they must not redefine truth authority.

### 5.2 Activation rule for source files

A source file is not an implemented runtime stage merely because it exists in the repository.

It becomes an active stage only when all of the following are true:

1. the module is exported through the owning crate boundary;
2. the module is reachable from an active use-case, activity, or workflow path;
3. automation treats it as active and verifies the contract;
4. owner docs classify it as active.

If any of those conditions are missing, the file is `code-present, runtime-inactive` and must be documented that way.

---

## 6. Triple Builder

`Triple Builder` is the mandatory deterministic semantic assembly step that returns after truth-core stabilization.

It is:

- deterministic
- code-first
- semantic assembly
- graph-safe

It is not:

- an LLM
- a truth judge
- a direct source of verified status
- a fuzzy string-join shortcut

Its duties:

- join validated/adjudicated extraction outputs to canonical entities
- assemble graph-safe semantic objects and relations
- require deterministic anchors such as `mention_id` or evidence spans
- collapse duplicates
- reject graph writes that lack required bindings
- prepare projection-safe payloads for `Neo4j`
- prepare retrieval-safe payloads for `Voyage/Qdrant`

Authority remains upstream in the Truth Plane.

---

## 7. Status Map

### 7.1 Active

- `SeoSiteBuildWorkflow`
- truth extraction candidate path
- validator foundation
- adjudication writer path
- admissible-only drafting gate
- run-mode/scenario/policy execution branching
- basic planning path:
  - `serp_normalize`
  - `opportunity_build`
  - `ia_build`
  - `link_recommend`
  - `global_site_reconcile`

### 7.2 Partial

- `Neo4j` projection surface
- `Qdrant` projection surface
- retrieval support during crawl/runtime
- ontology-backed canonical keys in relational storage

### 7.3 Deferred

- `Layer Router` as mandatory active runtime gate
- `Entity Span Detection` as mandatory active runtime gate
- `Canonical Mapping` as mandatory active runtime gate
- `Triple Builder` as mandatory active runtime gate
- full evidence-grade entity/relation extraction
- graph-first cluster/topic reasoning

### 7.4 Legacy / Quarantined

- `ContentGenerationWorkflow`
- `ExpertExtractionWorkflow`
- `ExpertDecomposedExtractionWorkflow`
- `ExpertProjectionWorkflow`
- `ExpertSemanticSliceWorkflow`
- any direct truth creation through regex extraction
- any source-tier shortcut such as `government|vfs => verified`
- any path where graph or retrieval upgrades truth directly

---

## 8. Execution Program

The return to the full rich expert flow must happen in this order:

1. Close live truth extraction with real Gemini provider.
2. Keep current truth-core as the only authority path.
3. Reactivate rich extraction stages:
   - `Layer Router`
   - `Entity Span Detection`
   - `Canonical Mapping`
4. Make `Triple Builder` mandatory.
5. Reactivate graph projection from semantic assembly outputs.
6. Reactivate retrieval projection from admissible semantic outputs.
7. Strengthen:
   - cluster/topic understanding
   - entity/relation graph
   - cross-page reasoning
   - navigation/global reconcile quality

The return path must be additive over truth-core, not a rollback to pre-adjudication ambiguity.

---

## 9. Acceptance Gates

### 9.1 Live truth extraction

Phase exit requires:

- configured Gemini provider
- live smoke no longer blocked on provider absence
- real candidates persisted in `extracted.rule_candidates`

### 9.2 Validator / adjudication

Phase exit requires:

- `structured`
- `needs_hitl`
- `rejected`
- `verified/admissible`

all proven by runtime evidence, not only static docs.

### 9.3 Triple Builder integration

Phase exit requires:

- deterministic anchors for semantic joins
- no fuzzy graph joins
- graph-safe assembly output for verified truth

### 9.4 Graph sync

Phase exit requires:

- `Neo4j` receives projection-safe semantic outputs only
- graph sync cannot happen from raw or unadjudicated extraction

### 9.5 Retrieval sync

Phase exit requires:

- `Voyage/Qdrant` receive indexing-safe outputs only
- retrieval indexing cannot mutate truth status

---

## 10. Versioned Change Log

### V6.0

- Consolidated runtime owner-document
- Superseded `V5_Ultimate_Extraction_Protocol.md`
- Superseded `SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md`
- Fixed one honest model for:
  - active runtime
  - target expert flow
  - truth/graph/retrieval authority boundaries
  - legacy and deferred surfaces

### V6.1

- aligned the owner-document summary with the real active runtime surface
- added explicit note that execution is branch-sensitive to run mode, scenario, and publish policy
- clarified that current truth-core validator/adjudication remain active even while richer semantic stages stay deferred

### V6.2

- pointed the owner-level target expert flow at the detailed 56-step working plan;
- clarified that the 56 steps cover the canonical current-run SEO/truth/site-build value stream, while runtime substrate, contracts, outbox workers, release gates, backup/restore, monitoring, analytics, privacy/licensing/quality gates, and legacy/test/lab surfaces are required support planes outside the step list;
- added an explicit rule that support planes may gate or observe the workflow but may not redefine truth authority.
