# V6 10/10 Completion Plan

**Status:** current execution and closure satellite  
**Class:** `current-execution-satellite`  
**Parent owner document:** [V6_Expert_Truth_Graph_Runtime.md](V6_Expert_Truth_Graph_Runtime.md)  
**Execution satellite:** [V6_SeoSiteBuildWorkflow_Working_Plan.md](V6_SeoSiteBuildWorkflow_Working_Plan.md)

---

## 1. Purpose

This file answers one narrow question:

- what still separates the accepted canonical runtime from a production-closed, 10/10 expert system;
- which gaps are already coded and only need integration;
- which gaps still need fresh implementation;
- which gaps are external environment blockers rather than code gaps.

It does not redefine workflow shape, truth authority, or support-plane boundaries.

The current audit baseline for this file is:

- initial V5 owner intent in `V5_Ultimate_Extraction_Protocol.md` and `V5_Ultimate_Extraction_Protocol_V2.md`;
- current canonical owner/runtime state in `V6_Expert_Truth_Graph_Runtime.md`;
- current execution state in `V6_SeoSiteBuildWorkflow_Working_Plan.md`;
- live code and automation under `app/rust/**`, `app/db/**`, and `automation/**`.

---

## 2. Audit Verdict

### 2.1 Already active and accepted

- `SeoSiteBuildCanonicalCutoverWorkflow` is the single forward workflow.
- canonical `steps 1-56` are wired and accepted.
- truth certification baseline is accepted and enforced.
- truth governance is accepted.
- regex de-authority is accepted.
- planning-only graph reasoning tranche is accepted.
- release path now requires the canonical Step 5 live-provider gate.
- build hygiene for proof surfaces is now formalized:
  - shared `app/rust/target/` is disposable;
  - whole-page and truth-cert gates run through isolated target roots;
  - a dedicated clean acceptance bundle exists for from-scratch operator verification.

### 2.2 Already coded but recently missing or incomplete

- `whole_page_semantic_pass` was active, but until the latest closure slice it emitted only:
  - `page_mode_hint`
  - `dominant_layers`
  - `page_summary`
  - `global_entities`
- current V6 docs already required:
  - `page_context_profile`
  - mixed-section hints
- this gap is now treated as a closure item, not as a future design note.

### 2.3 Not a code gap

- live provider readiness is still blocked by missing runtime credentials, not by missing runtime wiring:
  - `GEMINI_API_KEY`
  - `GOOGLE_API_KEY`
  - or another supported truth-extraction provider path

### 2.4 Still real gaps after this audit

1. real live-provider `PASS`, not `PENDING_CREDENTIALS`
2. env-by-env production-gate and drain evidence, not only local acceptance
3. final decision on physical retirement vs permanent diagnostic retention for legacy `Expert*` surfaces

---

## 3. 10/10 Closure Rule

`10/10` is reached only when all of the following are true together:

1. the canonical 56-step runtime remains the single forward execution path;
2. truth authority remains singular and machine-verifiable;
3. full local/CI certification remains green;
4. the real live-provider gate passes with actual extraction, not credential placeholders;
5. production release cannot bypass truth-provider readiness, replay discipline, restore discipline, or support-plane gates;
6. documentation, automation, and implementation agree on active vs legacy surfaces.

---

## 4. Ordered Completion Program

### Tranche A — Live Provider Closure

**Type:** external blocker + operational proof  
**Status:** blocked on credentials  
**Implementation state:** code-ready

#### Already built

- `automation/run_live_provider_minimal_scope_gate.sh`
- `automation/smoke_real_provider_minimal_scope.py`
- `automation/temporal_production_gate.sh`

#### Remaining work

1. provide a supported truth-extraction provider in runtime env;
2. run canonical Step 5 gate to `PASS`;
3. run production gate to `PASS`;
4. persist immutable evidence for the target environment.

#### Acceptance

- `docs/runs/live_provider_minimal_scope_evidence.json` ends in `PASS`
- `automation/temporal_production_gate.sh` ends in `PRODUCTION_GATE: PASS`

### Tranche B — Whole-Page Semantic Closure

**Type:** code + contract hardening  
**Status:** accepted for current runtime scope
**Implementation state:** deterministic scaffold, weighted page-framing heuristics, optional `voyage-4-large` advisory retrieval, and dedicated fixture/regression gate all integrated

#### Now active

- deterministic `page_mode_hint`
- deterministic `dominant_layers`
- deterministic `page_summary`
- deterministic `global_entities`
- deterministic `page_context_profile`
- deterministic mixed-section hints
- optional `voyage-4-large` page-sketch retrieval against a dedicated whole-page prototype collection
- conservative advisory fusion that can raise uncertainty or widen analysis but cannot create truth authority
- dedicated machine-readable fixture pack and baseline regression gate for whole-page semantics

#### Integrated in code now

- canonical activity path:
  - `app/rust/services/temporal/src/activities/operations.rs`
- diagnostic mirror path:
  - `app/rust/crates/infrastructure/src/adapters/expert_extraction_core.rs`

These paths now agree with the documented contract on emitted fields. The remaining gap is output quality hardening, not missing field shape.

#### Accepted evidence

- weighted deterministic country/visa/authority framing now uses page-level inputs (`source_url`, headings, primary content, noise content) instead of flat whole-text substring promotion;
- deterministic fixture coverage now includes:
  - mixed procedural/editorial pages;
  - utility/login pages;
  - menu-directory pages;
  - noisy footer-heavy content pages;
  - country/visa framing retention under footer noise;
  - advisory conflict and advisory-only-hint cases;
- machine-readable fixture baseline:
  - `docs/runs/whole_page_semantic_fixture_baseline.json`
- canonical gate:
  - `automation/run_whole_page_semantic_gate.sh`
- baseline drift check:
  - `automation/check_whole_page_semantic_regression.py`

#### Scope boundary remains

- no verified truth
- no canonical-key assignment
- no publish decision
- no rebuild trigger from advisory retrieval alone

- whole-page outputs are explicitly persisted and tested
- whole-page fixture gate remains green against the accepted baseline
- no truth-certification baseline drift
- docs and runtime contract agree on emitted fields

### Tranche C — Env-by-Env Operational Closure

**Type:** operations + evidence  
**Status:** local accepted, broader environments not yet proven  
**Implementation state:** local env bundle now wired; broader environment evidence incomplete

#### Local env evidence now captured

- canonical bundle:
  - `bash automation/run_local_operational_evidence_bundle.sh`
- accepted local artifact:
  - `docs/runs/local_operational_evidence_bundle.json`

Current local bundle semantics:

- `clean acceptance bundle` must pass;
- `release/restore gate` must return `ok`;
- `legacy replay evidence` must be `PASS`;
- `live provider minimal scope` may still leave the bundle at `BLOCKED_ON_LIVE_PROVIDER` until real credentials exist.

#### Remaining work

1. capture compat/drain evidence per target environment;
2. capture release/restore evidence per target environment;
3. require the same Step 5 and production-gate discipline outside local;
4. use the clean acceptance bundle for from-scratch operator validation where shared developer caches are not trusted.

#### Acceptance

- machine-readable replay/drain evidence per environment
- machine-readable release/restore evidence per environment
- successful `automation/run_clean_acceptance_bundle.sh` in the target environment or equivalent environment-specific wrapper
- env-scoped operational bundle per environment with no unresolved blockers other than explicitly classified external provider readiness

### Tranche D — Legacy Surface Retirement Decision

**Type:** policy + cleanup  
**Status:** decision partially made, physical code retention unresolved  
**Implementation state:** not required for truth correctness, still relevant for repository hygiene

#### Current state

- `SeoSiteBuildWorkflow` is compat/drain only
- `Expert*Workflow` family is diagnostic-only behind explicit opt-in

#### Remaining work

Choose one of:

1. retain `Expert*` surfaces permanently as narrow diagnostics and document them as such;
2. remove them physically after non-local drain evidence exists.

#### Acceptance

- docs, starter, worker registration, and automation all agree on the final policy

### Tranche E — Optional Post-10/10 Enrichment

**Type:** product enrichment, not current blocker  
**Status:** deferred by design

Not required for core 10/10 closure:

- graph influence on `page_brief`
- graph influence on `draft_*`
- graph influence on `publish_*`
- graph influence on `rebuild_detect`

These must stay separate from the core 10/10 closure program.

---

## 5. Code-Ready vs Needs-Build Matrix

| Area | Status | Action |
|---|---|---|
| Canonical 56-step flow | accepted | keep green |
| Truth certification gate | accepted | keep green |
| Truth governance | accepted | keep green |
| Regex de-authority | accepted | keep green |
| Planning-only graph reasoning | accepted | keep green |
| Step 5 live-provider wrapper | code-ready | requires real credentials and live `PASS` |
| Production gate live-provider enforcement | integrated | requires real credentials and live `PASS` |
| Whole-page `page_context_profile` + mixed-section hints | now integrated | strengthen deterministic heuristics, advisory prototype coverage, and tests |
| Legacy workflow quarantine | accepted | final retirement decision later |
| Full non-local operational evidence | incomplete | gather environment evidence |

---

## 6. Non-Negotiable Guardrails

The following are forbidden during closure:

- introducing a second truth authority path;
- allowing graph or retrieval to upgrade truth directly;
- bypassing truth certification to ship a semantic change;
- weakening production gate by treating `PENDING_CREDENTIALS` as operational success;
- hiding doc/runtime mismatches behind reference-doc ambiguity.
