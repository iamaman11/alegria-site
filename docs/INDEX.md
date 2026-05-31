# Documentation Index

Этот индекс фиксирует, какие файлы в `docs/` являются источником истины, а какие — domain-spec, reference или immutable artifacts.

## Priority Order

При конфликте приоритет такой:

1. Live code + live schema + automation
   - `app/rust/**`
   - `app/db/schema.sql`
   - `app/contracts/proto/**`
   - `automation/**`
2. Canonical owner document
   - [V6_Expert_Truth_Graph_Runtime.md](V6_Expert_Truth_Graph_Runtime.md)
3. Current runtime and implementation satellites
   - [V5_Runtime_Contract.md](V5_Runtime_Contract.md)
   - [V5_Truth_Extraction_LLM_Contract.md](V5_Truth_Extraction_LLM_Contract.md)
   - [V5_SEO_Graph_And_Retrieval_Projection_Spec.md](V5_SEO_Graph_And_Retrieval_Projection_Spec.md)
   - [V6_Support_Process_Registry.md](V6_Support_Process_Registry.md)
   - [V6_Voyage_Retrieval_Policy.md](V6_Voyage_Retrieval_Policy.md)
   - [V6_Neo4j_Runtime_Policy.md](V6_Neo4j_Runtime_Policy.md)
   - [STEP_CATALOG_CONTRACT.md](STEP_CATALOG_CONTRACT.md)
   - [OPS_RUNTIME_RUNBOOK.md](OPS_RUNTIME_RUNBOOK.md)
   - [OPS_TEMPORAL_BUILD_MODES.md](OPS_TEMPORAL_BUILD_MODES.md)
   - [OPS_TEMPORAL_PRODUCTION_GATE.md](OPS_TEMPORAL_PRODUCTION_GATE.md)
   - [V6_10_10_COMPLETION_PLAN.md](V6_10_10_COMPLETION_PLAN.md)
   - [SEO_SUPERSITE_10_10_EXECUTION_PLAN.md](SEO_SUPERSITE_10_10_EXECUTION_PLAN.md)
   - [V5_SEO_Identity_And_Applicability_Hardening_Plan.md](V5_SEO_Identity_And_Applicability_Hardening_Plan.md)
4. Superseded V5 owner/execution docs
   - [V5_Ultimate_Extraction_Protocol.md](V5_Ultimate_Extraction_Protocol.md)
   - [SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md](SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md)
5. Current domain/reference docs
   - [DOMAIN_MODEL.md](DOMAIN_MODEL.md)
6. Research / backlog / reference snapshots
   - everything explicitly marked below as `research` or `reference`
7. Immutable run artifacts
   - machine-generated files under `docs/` root listed below
   - machine-generated evidence under `docs/runs/**`

`V6_Expert_Truth_Graph_Runtime.md` is the single owner-document for the active runtime shape, the target expert architecture, and the truth/graph/retrieval authority split. `V5_Ultimate_Extraction_Protocol.md` and `SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md` are retained as superseded reference context.

## File Map

| File | Class | Implementation status | Role |
|---|---|---|---|
| [INDEX.md](INDEX.md) | current | live | documentation map and priority rules |
| [V6_Expert_Truth_Graph_Runtime.md](V6_Expert_Truth_Graph_Runtime.md) | current-canonical-owner | live | single owner-document for active runtime, target expert flow, authority boundaries, and execution sequence |
| [V6_SeoSiteBuildWorkflow_Working_Plan.md](V6_SeoSiteBuildWorkflow_Working_Plan.md) | current-execution-satellite | live | versioned detailed working plan for the canonical 56-step site-build flow under `V6`, executed by `SeoSiteBuildCanonicalCutoverWorkflow` |
| [V6_Support_Process_Registry.md](V6_Support_Process_Registry.md) | current-support-plane-registry | live | named support processes outside the 56-step flow, with contracts, owners, evidence, and automation coverage |
| [V6_Truth_Governance_Policy.md](V6_Truth_Governance_Policy.md) | current-runtime-satellite | live | source independence, trust weighting, authority override, freshness, and regex authority-boundary policy |
| [V6_Voyage_Retrieval_Policy.md](V6_Voyage_Retrieval_Policy.md) | current-runtime-satellite | live | current-versus-target Voyage model, collection, parameter, and non-authority policy |
| [V6_Neo4j_Runtime_Policy.md](V6_Neo4j_Runtime_Policy.md) | current-runtime-satellite | live | hard-required Neo4j contract (capabilities, graph projection freshness/completeness, blocked taxonomy) |
| [V6_10_10_COMPLETION_PLAN.md](V6_10_10_COMPLETION_PLAN.md) | current-closure-satellite | live | ordered 10/10 closure backlog distinguishing accepted runtime, code-ready integrations, fresh implementation, and external blockers |
| [DOMAIN_MODEL.md](DOMAIN_MODEL.md) | current | live | domain invariants and taxonomy |
| [V5_Ultimate_Extraction_Protocol.md](V5_Ultimate_Extraction_Protocol.md) | superseded-reference | reference | historical rich extraction and target protocol context; superseded by `V6` |
| [V5_Runtime_Contract.md](V5_Runtime_Contract.md) | current-runtime-satellite | live | runtime persistence, retry, replay, HITL, and DLQ satellite contract under `V6` |
| [V5_Truth_Extraction_LLM_Contract.md](V5_Truth_Extraction_LLM_Contract.md) | current-runtime-satellite | live | exact truth-extraction LLM prompt, wire format, response contract, validation, and adjudication boundaries under `V6` |
| [STEP_CATALOG_CONTRACT.md](STEP_CATALOG_CONTRACT.md) | current-runtime-spec | live | step ledger and step contract rules; historical framing in the intro predates the accepted cutover workflow |
| [OPS_RUNTIME_RUNBOOK.md](OPS_RUNTIME_RUNBOOK.md) | current-ops | live | runtime architecture and operational rules |
| [OPS_TEMPORAL_BUILD_MODES.md](OPS_TEMPORAL_BUILD_MODES.md) | current-ops | live | worker build-id / rollout / drain rules |
| [OPS_TEMPORAL_PRODUCTION_GATE.md](OPS_TEMPORAL_PRODUCTION_GATE.md) | current-ops | live | canonical production gate |
| [SEO_ARCHITECTURE_FINALIZATION_CHANGE_NOTE.md](SEO_ARCHITECTURE_FINALIZATION_CHANGE_NOTE.md) | current-ops | live | architecture migration summary for team review and ADR follow-up |
| [SEO_SUPERSITE_10_10_EXECUTION_PLAN.md](SEO_SUPERSITE_10_10_EXECUTION_PLAN.md) | reference-execution-snapshot | reference | historical SEO supersite capability snapshot; canonical current runtime/execution state lives in `V6` owner docs |
| [SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md](SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md) | superseded-reference | reference | historical code-anchored gap-closure and release evidence context; superseded by `V6` |
| [V5_SEO_Identity_And_Applicability_Hardening_Plan.md](V5_SEO_Identity_And_Applicability_Hardening_Plan.md) | current-seo-execution-plan | partial | live-aligned identity, applicability, and scope-hardening plan for SEO runtime |
| [V5_SEO_Live_Schema_Design_Spec.md](V5_SEO_Live_Schema_Design_Spec.md) | current-seo-build-spec | partial | SEO relational source-of-record design + SQLx persistence |
| [V5_SEO_Runtime_Step_Contracts_Spec.md](V5_SEO_Runtime_Step_Contracts_Spec.md) | current-seo-build-spec | partial | SEO Proto, Temporal steps, and workflow skeleton |
| [V5_SEO_Graph_And_Retrieval_Projection_Spec.md](V5_SEO_Graph_And_Retrieval_Projection_Spec.md) | current-runtime-satellite | partial | exact Neo4j and Qdrant projection satellite contract under `V6` |
| [V5_SEO_CMS_And_HITL_Control_Plane_Spec.md](V5_SEO_CMS_And_HITL_Control_Plane_Spec.md) | current-seo-build-spec | partial | SEO CMS publish-control handoff and HITL control-plane design |
| [OPS_RELIABILITY_HARDENING_RESEARCH_TZ.md](OPS_RELIABILITY_HARDENING_RESEARCH_TZ.md) | research | reference | research specification, not live runtime truth |
| [OPS_RELIABILITY_EXECUTION_BACKLOG.md](OPS_RELIABILITY_EXECUTION_BACKLOG.md) | backlog-reference | reference | execution backlog and implementation history |
| [SERP_INGEST_PLAYBOOK.md](SERP_INGEST_PLAYBOOK.md) | reference-runbook | reference | ingest guidance for the archived SERP run artifacts |
| [V5_Event_Contracts.json](V5_Event_Contracts.json) | reference-schema | reference | legacy/reference event schema snapshot |
| [V5_Schema_Registry.json](V5_Schema_Registry.json) | reference-schema | reference | legacy/reference registry snapshot |
| [V5_Postgres_DDL.sql](V5_Postgres_DDL.sql) | reference-schema | reference | reference DDL snapshot; live schema is `app/db/schema.sql` |
| [V5_Neo4j_Model.cypher](V5_Neo4j_Model.cypher) | reference-domain-model | reference | graph model reference |
| `run_gemini3_global_186_20260320_top10__*.jsonl/json/csv/sql` | immutable-artifact | immutable | run outputs; do not treat as live contracts |
| `docs/runs/**` | immutable-artifact | immutable | smoke, run-report, and certification evidence, including the accepted local/CI baseline `truth_certification_local_ci_baseline.json`; do not treat as live contracts |

## Canonical Live Roots

- Rust runtime root: `app/rust`
- Business schema: `app/db/schema.sql`
- Proto contracts: `app/contracts/proto`
- Enforcement / gates: `automation/`

## Governance Rules

- `V6_Expert_Truth_Graph_Runtime.md` is the single current owner-document for architecture and boundaries.
- `V6_SeoSiteBuildWorkflow_Working_Plan.md` owns execution shape and activation status only.
- `V6_Support_Process_Registry.md` owns required support-plane processes outside the 56-step flow.
- `V6_Truth_Governance_Policy.md` owns truth-governance decision tables and regex authority boundaries.
- `V6_Voyage_Retrieval_Policy.md` owns current-versus-target Voyage model, collection, and parameter policy.
- `V6_Neo4j_Runtime_Policy.md` owns the hard-required Neo4j runtime capability/projection contract.
- `V6_10_10_COMPLETION_PLAN.md` owns the remaining closure backlog and its classification as code-ready, needs-build, or external-blocker work.
- Активные документы — только в `docs/` root.
- `docs/_archive/` never participates in current truth.
- Superseded `V5` owner docs may preserve historical and design context, but must not override `V6`.
- `reference` and `research` files may inform design, but must not override current code, schema, or automation.
- Immutable run artifacts must not be edited to “match” code.
