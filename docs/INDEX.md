# Documentation Index

Этот индекс фиксирует, какие файлы в `docs/` являются источником истины, а какие — domain-spec, reference или immutable artifacts.

## Priority Order

При конфликте приоритет такой:

1. Live code + live schema + automation
   - `app/rust/**`
   - `app/db/schema.sql`
   - `app/contracts/proto/**`
   - `automation/**`
2. Current runtime and operations docs
   - [V5_Runtime_Contract.md](V5_Runtime_Contract.md)
   - [STEP_CATALOG_CONTRACT.md](STEP_CATALOG_CONTRACT.md)
   - [OPS_RUNTIME_RUNBOOK.md](OPS_RUNTIME_RUNBOOK.md)
   - [OPS_TEMPORAL_BUILD_MODES.md](OPS_TEMPORAL_BUILD_MODES.md)
   - [OPS_TEMPORAL_PRODUCTION_GATE.md](OPS_TEMPORAL_PRODUCTION_GATE.md)
   - [SEO_SUPERSITE_10_10_EXECUTION_PLAN.md](SEO_SUPERSITE_10_10_EXECUTION_PLAN.md)
3. Current domain/protocol docs
   - [DOMAIN_MODEL.md](DOMAIN_MODEL.md)
   - [V5_Ultimate_Extraction_Protocol.md](V5_Ultimate_Extraction_Protocol.md)
4. Research / backlog / reference snapshots
   - everything explicitly marked below as `research` or `reference`
5. Immutable run artifacts
   - machine-generated files under `docs/` root listed below

`V5_Ultimate_Extraction_Protocol.md` is the main domain/protocol specification, but it is not the sole operational source of truth for runtime persistence, worker rollout, gating, or recovery.

## File Map

| File | Class | Implementation status | Role |
|---|---|---|---|
| [INDEX.md](INDEX.md) | current | live | documentation map and priority rules |
| [DOMAIN_MODEL.md](DOMAIN_MODEL.md) | current | live | domain invariants and taxonomy |
| [V5_Ultimate_Extraction_Protocol.md](V5_Ultimate_Extraction_Protocol.md) | current-domain-spec | partial | target protocol and extraction semantics |
| [V5_Runtime_Contract.md](V5_Runtime_Contract.md) | current-runtime-spec | live | runtime persistence, retry, replay, HITL, DLQ contracts |
| [STEP_CATALOG_CONTRACT.md](STEP_CATALOG_CONTRACT.md) | current-runtime-spec | partial | step ledger and step contract rules |
| [OPS_RUNTIME_RUNBOOK.md](OPS_RUNTIME_RUNBOOK.md) | current-ops | live | runtime architecture and operational rules |
| [OPS_TEMPORAL_BUILD_MODES.md](OPS_TEMPORAL_BUILD_MODES.md) | current-ops | live | worker build-id / rollout / drain rules |
| [OPS_TEMPORAL_PRODUCTION_GATE.md](OPS_TEMPORAL_PRODUCTION_GATE.md) | current-ops | live | canonical production gate |
| [SEO_ARCHITECTURE_FINALIZATION_CHANGE_NOTE.md](SEO_ARCHITECTURE_FINALIZATION_CHANGE_NOTE.md) | current-ops | live | architecture migration summary for team review and ADR follow-up |
| [SEO_SUPERSITE_10_10_EXECUTION_PLAN.md](SEO_SUPERSITE_10_10_EXECUTION_PLAN.md) | current-seo-execution-plan | partial | live-aligned SEO supersite capability, gaps, and execution order |
| [V5_SEO_Identity_And_Applicability_Hardening_Plan.md](V5_SEO_Identity_And_Applicability_Hardening_Plan.md) | current-seo-execution-plan | partial | live-aligned identity, applicability, and scope-hardening plan for SEO runtime |
| [V5_SEO_Live_Schema_Design_Spec.md](V5_SEO_Live_Schema_Design_Spec.md) | current-seo-build-spec | partial | SEO relational source-of-record design + SQLx persistence |
| [V5_SEO_Runtime_Step_Contracts_Spec.md](V5_SEO_Runtime_Step_Contracts_Spec.md) | current-seo-build-spec | partial | SEO Proto, Temporal steps, and workflow skeleton |
| [V5_SEO_Graph_And_Retrieval_Projection_Spec.md](V5_SEO_Graph_And_Retrieval_Projection_Spec.md) | current-seo-build-spec | partial | SEO Neo4j and Qdrant projection design + bootstrap outbox surface |
| [V5_SEO_CMS_And_HITL_Control_Plane_Spec.md](V5_SEO_CMS_And_HITL_Control_Plane_Spec.md) | current-seo-build-spec | partial | SEO CMS publish-control handoff and HITL control-plane design |
| [OPS_RELIABILITY_HARDENING_RESEARCH_TZ.md](OPS_RELIABILITY_HARDENING_RESEARCH_TZ.md) | research | reference | research specification, not live runtime truth |
| [OPS_RELIABILITY_EXECUTION_BACKLOG.md](OPS_RELIABILITY_EXECUTION_BACKLOG.md) | backlog-reference | reference | execution backlog and implementation history |
| [SERP_INGEST_PLAYBOOK.md](SERP_INGEST_PLAYBOOK.md) | reference-runbook | reference | ingest guidance for the archived SERP run artifacts |
| [V5_Event_Contracts.json](V5_Event_Contracts.json) | reference-schema | reference | legacy/reference event schema snapshot |
| [V5_Schema_Registry.json](V5_Schema_Registry.json) | reference-schema | reference | legacy/reference registry snapshot |
| [V5_Postgres_DDL.sql](V5_Postgres_DDL.sql) | reference-schema | reference | reference DDL snapshot; live schema is `app/db/schema.sql` |
| [V5_Neo4j_Model.cypher](V5_Neo4j_Model.cypher) | reference-domain-model | reference | graph model reference |
| `run_gemini3_global_186_20260320_top10__*.jsonl/json/csv/sql` | immutable-artifact | immutable | run outputs; do not treat as live contracts |

## Canonical Live Roots

- Rust runtime root: `app/rust`
- Business schema: `app/db/schema.sql`
- Proto contracts: `app/contracts/proto`
- Enforcement / gates: `automation/`

## Governance Rules

- Активные документы — только в `docs/` root.
- `docs/_archive/` never participates in current truth.
- `reference` and `research` files may inform design, but must not override current code, schema, or automation.
- Immutable run artifacts must not be edited to “match” code.
