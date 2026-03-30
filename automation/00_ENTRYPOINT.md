# Automation Entry Point

Канонический вход в контур проверок Alegria SEO Engine.
Цель: за один проход подтвердить целостность данных, архитектурную чистоту слоёв и GO/NO-GO перед ingest/release.

## 1) Один обязательный запуск

```bash
bash automation/ci_verify.sh
```

Если команда завершилась с `All checks passed.`, базовый контроль пройден.

## 2) Что именно проверяется

- Плоская структура документации: `check_docs_layout.py` (нет `docs/knowledge`, активные файлы только в `docs/`)
- Границы Python/Rust runtime: `check_python_rust_boundary.sh`
- Послойность Rust и adapter-invariant: `check_rust_adapter_boundaries.py`
- Изоляция Rust-слоёв (no cross-layer drift): `check_rust_layer_isolation.py`
- Purity `primitives`: `check_primitives_purity.py`
- Purity `services/temporal`: `check_temporal_runtime_purity.py`
- Layer dependency matrix: `check_layer_dependency_matrix.py`
- Явные семейства runtime-типов `domain / storage / wire`: `check_runtime_type_families.py`
- JSON только на deny-by-default boundary-слоях: `check_json_boundary_policy.py`
- Запрет raw external SDK в `use_cases`: `check_use_cases_external_sdk_ban.py`
- Граница `services/temporal`: `check_services_temporal_boundary.py`
- Proto-контракты и совместимость генерации: `contract_verify.sh`
- Error taxonomy и retry-discipline Temporal: `check_domain_error_usage.py`
- Execution ledger / идемпотентность шагов: `check_step_execution_contract.py`
- Полнота step-контрактов: `check_step_contract_completeness.py`
- Metrics/SLO contract: `check_metrics_contract.py`
- Fail-safe / reconcile / DLQ contract: `check_reconcile_failsafe.py`
- Fail-safe smoke scenarios:
  - `smoke_broken_schema_to_dlq.py`
  - `smoke_exhausted_retry_to_dlq.py`
  - `smoke_stale_outbox_reclaim.py`
  - `smoke_pending_hitl_not_failure.py`
- End-to-end ingest-инварианты (идемпотентность/уникальность): `check_end_to_end_invariants.py`
- Экспертный сводный отчёт: `check_expert_consistency.py`
- Сборка Rust workspace: `cargo check -q`
- Контракт миграции Python->Rust + parity fact verifier: Rust CLI checks

## 3) GO / NO-GO правило

GO только если одновременно:

- `ci_verify.sh` завершился без ошибок.
- `automation/reports/consistency_report.json` не содержит `error`.
- Нет нарушений в boundary/adapter/isolation/json/invariants.
- Нет нарушений в структуре документации (`check_docs_layout.py`).

Любой провал любого пункта выше = NO-GO.

## 4) Инварианты, которые считаются обязательными

- Внешние runtime-SDK используются через `app/rust/crates/infrastructure/src/adapters/*`.
- `primitives` не зависят от `contracts` и внешних SDK.
- `use_cases` не содержат raw SQL/JSON-boundary; доступ к DB/IO идёт только через typed adapter API.
- Прямые зависимости на целевые SDK вне adapter-layer запрещены (кроме зафиксированных технических исключений).
- Ingest ключи и дедуп остаются идемпотентными и воспроизводимыми.
- `raw` слой ingest immutable (`ON CONFLICT DO NOTHING`).
- JSON используется только там, где это разрешено boundary-политикой.
- Temporal runtime шаги используют `DomainError` и не опираются на `anyhow` как на final error contract.
- Любой side-effectful step проходит через `pipeline.step_executions`.
- Metrics endpoint и fail-safe DLQ/reconcile контур обязательны.
- Runtime step-state и outbox используют канонический `bytes + type + schema_version + hash` path, а не legacy JSON-path.
- `blake3` остаётся каноническим hash/idempotency/payload integrity механизмом.

## 5) Канонические ops-команды

Базовая инженерная проверка:

```bash
bash automation/ci_verify.sh
```

Контейнерный production gate:

```bash
TEMPORAL_GATE_WORKER_MODE=docker bash automation/temporal_production_gate.sh
```

Подтверждение recoverability через backup/restore:

```bash
infra/backups/restore_drill.sh
```

Поведенческий fail-safe gate:

```bash
bash automation/fail_safe_behavioral_gate.sh
```

Пересборка production worker-образа:

```bash
docker compose build temporal-worker
```

## 6) Три вещи, которые нельзя нарушать

1. Нельзя возвращать SQL/JSON/external SDK в `primitives`, `policies`, `use_cases` или `services/temporal`; это допустимо только в `infrastructure/adapters` и зафиксированных boundary-точках.
2. Нельзя возвращать legacy JSON runtime path или generic `JsonPayload` для step-state/outbox/DLQ/HITL/reconcile; канонический runtime contract — только специализированные Proto message + `payload_bytes`/`payload_type`/`schema_version`/`payload_hash` + `blake3`.
3. Нельзя выкатывать workflow/runtime изменения в production без `ci_verify.sh`, свежего `temporal_production_gate.sh` и `restore_drill.sh`.

## 7) Где смотреть детали (source-of-truth)

- Регламент целостности и GO/NO-GO: `automation/INTEGRITY_RUNBOOK.md`
- Короткий ops/invariants summary: `automation/OPS_CANONICALS.md`
- Карта automation-контуров: `automation/PROJECT_NAVIGATION_MAP.md`
- Общий обзор и точечные команды: `automation/README.md`
- Последние отчёты: `automation/reports/*.json`

## 8) Быстрый triage при падении

1. Запустить отдельно упавший check из вывода `ci_verify.sh`.
2. Исправить нарушение в целевом слое (не обходить проверку).
3. Повторить `bash automation/ci_verify.sh`.
4. Зафиксировать изменение в `INTEGRITY_RUNBOOK.md`, если правило/исключение изменилось.
