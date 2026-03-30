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
3. Current domain/protocol docs
   - [DOMAIN_MODEL.md](DOMAIN_MODEL.md)
   - [V5_Ultimate_Extraction_Protocol.md](V5_Ultimate_Extraction_Protocol.md)
4. Research / backlog / reference snapshots
   - everything explicitly marked below as `research` or `reference`
5. Immutable run artifacts
   - machine-generated files under `docs/` root listed below

`V5_Ultimate_Extraction_Protocol.md` is the main domain/protocol specification, but it is not the sole operational source of truth for runtime persistence, worker rollout, gating, or recovery.

## File Map

| File | Class | Role |
|---|---|---|
| [INDEX.md](INDEX.md) | current | documentation map and priority rules |
| [DOMAIN_MODEL.md](DOMAIN_MODEL.md) | current | domain invariants and taxonomy |
| [V5_Ultimate_Extraction_Protocol.md](V5_Ultimate_Extraction_Protocol.md) | current-domain-spec | target protocol and extraction semantics |
| [V5_Runtime_Contract.md](V5_Runtime_Contract.md) | current-runtime-spec | runtime persistence, retry, replay, HITL, DLQ contracts |
| [STEP_CATALOG_CONTRACT.md](STEP_CATALOG_CONTRACT.md) | current-runtime-spec | step ledger and step contract rules |
| [OPS_RUNTIME_RUNBOOK.md](OPS_RUNTIME_RUNBOOK.md) | current-ops | runtime architecture and operational rules |
| [OPS_TEMPORAL_BUILD_MODES.md](OPS_TEMPORAL_BUILD_MODES.md) | current-ops | worker build-id / rollout / drain rules |
| [OPS_TEMPORAL_PRODUCTION_GATE.md](OPS_TEMPORAL_PRODUCTION_GATE.md) | current-ops | canonical production gate |
| [OPS_RELIABILITY_HARDENING_RESEARCH_TZ.md](OPS_RELIABILITY_HARDENING_RESEARCH_TZ.md) | research | research specification, not live runtime truth |
| [OPS_RELIABILITY_EXECUTION_BACKLOG.md](OPS_RELIABILITY_EXECUTION_BACKLOG.md) | backlog-reference | execution backlog and implementation history |
| [SERP_INGEST_PLAYBOOK.md](SERP_INGEST_PLAYBOOK.md) | reference-runbook | ingest guidance for the archived SERP run artifacts |
| [V5_Event_Contracts.json](V5_Event_Contracts.json) | reference-schema | legacy/reference event schema snapshot |
| [V5_Schema_Registry.json](V5_Schema_Registry.json) | reference-schema | legacy/reference registry snapshot |
| [V5_Postgres_DDL.sql](V5_Postgres_DDL.sql) | reference-schema | reference DDL snapshot; live schema is `app/db/schema.sql` |
| [V5_Neo4j_Model.cypher](V5_Neo4j_Model.cypher) | reference-domain-model | graph model reference |
| `run_gemini3_global_186_20260320_top10__*.jsonl/json/csv/sql` | immutable-artifact | run outputs; do not treat as live contracts |

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
