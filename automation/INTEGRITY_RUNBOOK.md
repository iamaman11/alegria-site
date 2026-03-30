# Integrity Runbook

Этот документ фиксирует стандарт поддержки целостности для Alegria SEO Engine.

## Назначение

- Поддерживать инварианты ingest и Rust-first архитектуру.
- Быстро обнаруживать регрессии в границах Python/Rust.
- Давать воспроизводимое решение GO/NO-GO перед ingest.

## Состав automation

- `automation/ci_verify.sh` — единый запуск всех обязательных проверок.
- `automation/check_expert_consistency.py` — агрегированный экспертный аудит и JSON-отчёт.
- `automation/check_docs_layout.py` — контроль плоской структуры `docs/` (без `docs/knowledge`, кроме `docs/_archive`).
- `automation/check_python_rust_boundary.sh` — контроль границ ответственности.
- `automation/check_rust_adapter_boundaries.py` — контроль послойности Rust adapter-layer.
- `automation/check_rust_layer_isolation.py` — контроль межслойных зависимостей crate-уровня.
- `automation/check_primitives_purity.py` — контроль, что `primitives` остаётся pure-слоем.
- `automation/check_temporal_runtime_purity.py` — контроль, что `services/temporal` не содержит raw DB/JSON-boundary.
- `automation/check_layer_dependency_matrix.py` — контроль канонической матрицы слоёв.
- `automation/check_json_boundary_policy.py` — deny-by-default контроль JSON boundary.
- `automation/check_domain_error_usage.py` — контроль, что Temporal runtime использует `DomainError`, а не безликий `anyhow`.
- `automation/check_step_execution_contract.py` — контроль `pipeline.step_executions` и execution-ledger discipline.
- `automation/check_metrics_contract.py` — контроль обязательных runtime-метрик и проверки `/metrics` в gate.
- `automation/check_reconcile_failsafe.py` — контроль DLQ/fail-safe snapshot и reconcile-контракта.
- `automation/check_end_to_end_invariants.py` — контроль ключевых инвариантов ingest.
- `automation/contract_verify.sh` — проверка proto-контрактов и compile-пути.

## Обязательный цикл перед ingest

1. `bash automation/ci_verify.sh`
2. Проверить `automation/reports/consistency_report.json`
3. Убедиться, что статус `ok` и нет `error` findings.

## Канонические ops-команды

- Статический и архитектурный контроль:
  - `bash automation/ci_verify.sh`
- Container-grade production gate:
  - `TEMPORAL_GATE_WORKER_MODE=docker bash automation/temporal_production_gate.sh`
- Recoverability drill:
  - `infra/backups/restore_drill.sh`
- Пересборка production worker:
  - `docker compose build temporal-worker`

## GO / NO-GO

- GO:
  - `ci_verify.sh` завершился без ошибок.
  - `CONSISTENCY: OK`.
  - Нет критических нарушений в boundary/invariants.

- NO-GO:
- Любая ошибка в `check_python_rust_boundary.sh`.
- Любая ошибка в `check_docs_layout.py`.
- Любая ошибка в `check_rust_adapter_boundaries.py`.
- Любая ошибка в `check_rust_layer_isolation.py`.
- Любая ошибка в `check_json_boundary_policy.py`.
- Любая ошибка в `check_end_to_end_invariants.py`.
- Любой `error` в `consistency_report.json`.

## Политика инвариантов

- Внешние runtime-библиотеки используются только через `app/rust/crates/infrastructure/src/adapters/*`.
- `primitives = pure`: без `contracts`, SQL и внешних SDK; JSON допустим только в `*_json.rs`.
- `use_cases = policy + orchestration`: raw SQL/JSON-boundary не допускаются; доступ к DB/IO идёт через typed adapter API.
- `infrastructure = SQL / IO / boundary`: только здесь допустимы `sqlx`, `serde_json::Value`, `JSONB`, external SDK.
- В `use_cases` запрещены прямые импорты `neo4rs`, `qdrant_client`, `reqwest`, `tonic`.
- JSON допустим только в постоянных boundary-зонах:
  - raw snapshots,
  - внешние источники,
  - variadic conditions,
  - telemetry/diagnostics.
- К telemetry/diagnostics boundary относятся `app/rust/crates/telemetry/src/**`.
- deny-by-default boundary points зафиксированы в `automation/LAYER_MAP.md`.
- Временные migration-only JSON пути запрещены.
- `DomainError` обязателен на Temporal runtime-path; error class определяет retryability.
- Каждый side-effectful step обязан проходить через `pipeline.step_executions`.
- Канонический runtime/store contract для step-state, outbox, DLQ, HITL и reconcile:
  - `payload_bytes`
  - `payload_type`
  - `schema_version`
  - `payload_hash`
- Generic `JsonPayload` в runtime-state запрещён; допускаются только специализированные Proto message.
- `blake3` обязателен как hash/idempotency/integrity механизм.
- `WORKER_BUILD_ID` обязателен как runtime identity/traceability marker.
- `system.dead_letter_queue`, reconcile fail-safe snapshot и `/metrics` обязательны для GO.
- Ключи ingest должны оставаться идемпотентными и уникальными.
- `raw` слой ingest — immutable (`ON CONFLICT DO NOTHING`).

## Три запрета

1. Нельзя протаскивать SQL, JSON-boundary и external SDK выше `infrastructure/adapters`.
2. Нельзя возвращать legacy JSON runtime path или generic `JsonPayload` для outbox, step-state, DLQ, HITL и reconcile.
3. Нельзя считать production rollout валидным без `ci_verify.sh`, `temporal_production_gate.sh` и `restore_drill.sh`.

### SDK adapter-invariant (зафиксировано 2026-03-26)

Обязательные библиотеки и их адаптеры:

- `temporalio-sdk` / `temporalio-sdk-core` / `temporalio-client` / `temporalio-common` / `temporalio-macros` → `temporalio_sdk_adapter.rs`
- `reqwest` → `reqwest_adapter.rs`
- `hyper` → `hyper_adapter.rs`
- `tower` → `tower_adapter.rs`
- `playwright-rs` → `playwright_rs_adapter.rs`
- `graph-flow` → `graph_flow_adapter.rs`
- `rig-core` (`rig`) → `rig_core_adapter.rs`
- `rig-vertexai` → `rig_vertexai_adapter.rs`
- `tokio-postgres` → `tokio_postgres_adapter.rs`
- `neo4rs` → `neo4rs_adapter.rs`

Срез версий (latest на дату фиксации):

- `temporalio-sdk = 0.2.0`
- `reqwest = 0.13.2`
- `hyper = 1.8.1`
- `tower = 0.5.3`
- `playwright-rs = 0.8.7`
- `graph-flow = 0.4.0`
- `rig-core = 0.33.0`
- `rig-vertexai = 0.3.2`
- `tokio-postgres = 0.7.16`
- `neo4rs = 0.9.0-rc.9` (latest published pre-release)

Техническое исключение:

- `app/rust/services/temporal/Cargo.toml` хранит прямые `temporalio-*` зависимости только из-за требований proc-macro (`#[activities]`, `#[workflow_methods]`), при этом runtime-код использует adapter-layer.

Проверка:

```bash
python3 automation/check_rust_adapter_boundaries.py
cd app/rust && cargo check -q
```

## Разбор типовых сбоев

- Ошибка boundary:
  - Проверить `automation/check_python_rust_boundary.sh`.
  - Проверить `automation/check_rust_adapter_boundaries.py`.

- Ошибка ingest invariants:
  - Проверить `app/db/schema.sql` и `use_cases::serp_ingest` на уникальность/идемпотентность.
