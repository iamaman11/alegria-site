# Reliability Hardening Execution Backlog

> Status: backlog/reference. Use this as implementation history and checklist, not as the sole source of current runtime truth.

Этот документ переводит исследовательское ТЗ из `OPS_RELIABILITY_HARDENING_RESEARCH_TZ.md` в исполнительный backlog `R1..R7`.

## R1. Strict Step Contracts

### Goal
- Убрать неявные step-boundary между runtime-слоями.
- Зафиксировать canonical envelope для runtime-steps.

### Files
- `app/contracts/proto/temporal_payloads.proto`
- `app/rust/crates/contracts/build.rs`
- `app/rust/crates/contracts/src/lib.rs`
- `docs/V5_Runtime_Contract.md`
- `docs/STEP_CATALOG_CONTRACT.md`

### Deliverables
- `StepContractMeta`
- `StepEnvelope`
- `HitlResolutionInput`
- `ValidationInputPayload`
- `ValidationReport`
- `VerifyReport`
- `PersistReport`
- `HitlPauseInfo`
- `FreshnessReport`

### DDL
- нет

### Automation
- `automation/check_reliability_contracts.py`
- интеграция в `automation/ci_verify.sh`

### Status
- `implemented in runtime; remaining cleanup: remove JSON helper usage from temporal activity internals`

## R2. Error Taxonomy

### Goal
- Ввести каноническую domain-level классификацию ошибок для Temporal activity behavior.

### Files
- `app/rust/crates/primitives/src/errors.rs`
- `app/rust/crates/primitives/src/lib.rs`
- `app/rust/services/temporal/src/activities/mod.rs`

### Deliverables
- `DomainError`
- `ErrorClass`
- правила маппинга в `Retryable/NonRetryable`

### DDL
- нет

### Automation
- новая проверка: отсутствие ad hoc string-matching без taxonomy в temporal activity wrappers

### Status
- `implemented at activity boundary; remaining cleanup: propagate DomainError deeper into use_cases`

## R3. Step Idempotency Journal

### Goal
- Сделать step execution replay-safe и audit-safe.

### Files
- `app/db/schema.sql`
- `app/rust/crates/use_cases/src/pipeline_runtime.rs`
- `app/rust/services/temporal/src/activities/mod.rs`

### Deliverables
- `pipeline.execution_run_blobs`
- `pipeline.step_executions`
- `pipeline.step_attempts`
- `pipeline.step_payload_blobs`
- `pipeline.hitl_decisions`
- helper functions:
  - `begin_step_execution`
  - `begin_step_attempt`
  - `finish_step_attempt`
  - `complete_step_execution`
  - `fail_step_execution`
  - `load_completed_step_result`
  - `derive_step_keys`
  - `write_step_payload_blob_typed`
  - `write_hitl_decision_typed`

### DDL
- `pipeline.execution_run_blobs`
- `pipeline.step_executions`
- `pipeline.step_attempts`
- `pipeline.step_payload_blobs`
- `pipeline.hitl_decisions`
- `UNIQUE(step_name, idempotency_key)`

### Automation
- schema check for `pipeline.step_executions`
- replay/idempotency smoke test

### Status
- `implemented in schema + temporal activities + execution_run_blobs + attempt/blob/HITL decision ledger`

## R4. Temporal Versioning Discipline

### Goal
- Убрать неуправляемые workflow-breaking changes.

### Files
- `app/rust/services/temporal/src/main.rs`
- `app/rust/services/temporal/src/bin/temporal_starter.rs`
- `docs/OPS_RUNTIME_RUNBOOK.md`
- `docs/OPS_TEMPORAL_BUILD_MODES.md`

### Deliverables
- `WORKER_BUILD_ID`
- rollout discipline
- matrix `change type -> rollout rule`

### DDL
- нет

### Automation
- build-id presence check
- no workflow-breaking deploy without version marker

### Status
- `build-id discipline implemented; full server-side worker routing deferred until dedicated rollout registration is implemented and acceptance-tested`

## R5. Metrics And SLO

### Goal
- Сделать runtime наблюдаемым.

### Files
- `app/rust/services/temporal/Cargo.toml`
- `app/rust/services/temporal/src/main.rs`
- `docker-compose.yml`
- `docs/OPS_TEMPORAL_PRODUCTION_GATE.md`

### Deliverables
- `/metrics`
- activity latency metrics
- retry metrics
- failure-by-class metrics

### DDL
- нет

### Automation
- `temporal_production_gate.sh` проверяет metrics endpoint

### Status
- `implemented`

## R6. DLQ And Reconcile

### Goal
- Сделать невосстановимые ошибки управляемыми.

### Files
- `app/db/schema.sql`
- `app/rust/services/reconcile/src/main.rs`
- `app/rust/crates/use_cases/src/pipeline_runtime.rs`

### Deliverables
- `system.dead_letter_queue`
- `pipeline.reconcile_runs`
- `pipeline.reconcile_actions`
- reconcile rules for stale workflows / poisoned steps
- runtime DLQ writes for non-retryable contract/validation failures

### DDL
- `system.dead_letter_queue`
- `pipeline.reconcile_runs`
- `pipeline.reconcile_actions`

### Automation
- poisoned message test
- stale workflow reconcile smoke test

### Status
- `implemented for schema + runtime writes + persistent reconcile run/action snapshot`

## R6.1 Versioned Outbox Contract

### Goal
- Сделать outbox wire-contract версионируемым и идемпотентным не только по payload hash.

### Files
- `app/db/schema.sql`
- `app/rust/crates/use_cases/src/outbox_builder.rs`
- `app/rust/crates/infrastructure/src/adapters/sqlx_pipeline_runtime_adapter.rs`

### Deliverables
- `system.sync_outbox.payload_type`
- `system.sync_outbox.schema_version`
- `system.sync_outbox.idempotency_key`
- unique guard `idx_sync_outbox_idempotency`

### Status
- `implemented`

## R7. Infrastructure Reliability

### Goal
- Формализовать backup/restore and connection isolation.

### Files
- `docker-compose.yml`
- `docs/OPS_RUNTIME_RUNBOOK.md`
- `infra/backups/*`

### Deliverables
- backup script
- restore drill script
- optional pooling policy

### DDL
- нет

### Automation
- restore drill verification hook

### Status
- `implemented for backup/restore scripts; pooling policy still explicit-direct until PgBouncer decision`

## Current Implementation Notes

- `R1`: Temporal payload contracts расширены и подключены в runtime.
- `R2`: введён canonical `DomainError/ErrorClass` и activity-level mapping.
- `R3`: execution ledger реально подключён в side-effectful Temporal activities.
- `R4`: `WORKER_BUILD_ID` обязателен и проверяется automation; полноценный server-side worker routing включается только вместе с отдельным rollout-процессом.
- `R5`: metrics endpoint `/metrics` добавлен, compose поднимает Prometheus/Grafana.
- `R6`: DLQ schema и запись non-retryable failures добавлены; reconcile логирует fail-safe snapshot.
- `R7`: backup/restore scripts добавлены, restore drill формализован.
