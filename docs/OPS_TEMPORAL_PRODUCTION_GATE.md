# Temporal Production Gate

Этот документ фиксирует **воспроизводимый** способ подтвердить production-grade готовность Temporal-контура в Alegria.

## Каноническая команда

```bash
bash automation/temporal_production_gate.sh
```

Если в конце есть строка `PRODUCTION_GATE: PASS`, gate пройден.

## Что проверяет gate

1. Поднимает продовый стек в Docker:
- `postgres`
- `postgres-temporal`
- `temporal-server`
- `temporal-ui`
- `temporal-worker`
- `prometheus`
- `grafana`

2. Проверяет инварианты БД:
- существует dedup-индекс `system.idx_sync_outbox_dedup`;
- есть активные источники в `kb.sources`;
- присутствуют базовые concepts для smoke-extract:
  - `consular_fee`
  - `passport`
  - `medical_insurance`

3. Проверяет runtime Temporal:
- `temporal_starter ping`;
- `demo-hitl` проходит (`demo_hitl_ok`).

4. Проверяет реальные workflow в боевом контуре:
- `FactExtractionWorkflow`:
  - старт по валидному UUID `workflow_id=run_id`;
  - пауза HITL;
  - `resume` сигнал;
  - завершение `COMPLETED`;
  - `pipeline.execution_runs.status = done`.
- `SeoSiteBuildWorkflow`:
  - старт по валидному UUID `workflow_id=run_id`;
  - загружает `verified_support_bundle` из БД по `context_key`;
  - не продолжает draft path при пустом support bundle;
  - создает review-required revision;
  - после approval signal выполняет materialization и render validation;
  - завершение `COMPLETED`;
  - `pipeline.execution_runs.status = done`.
- `FreshnessCheckWorkflow`:
  - завершение `COMPLETED`.

5. Проверяет runtime observability:
- `temporal-worker` экспонирует `:9464/metrics`;
- endpoint содержит:
  - `activity_duration_seconds`
  - `workflow_completions_total`
  - `step_execution_reused_total`

6. Проверяет deployment discipline:
- worker стартует с `WORKER_BUILD_ID`;
- docker-mode прогон использует актуальный контейнерный runtime.

## Почему это production-grade

- Проверяется **не только компиляция**, а полный E2E путь на живом Temporal Server и Postgres.
- Проверяется отказоустойчивый контур с HITL pause/resume.
- Проверяются DB-инварианты, которые ранее вызывали зацикливание ретраев.
- Проверяется согласованность workflow-статуса и бизнес-статуса в `execution_runs`.

## Ограничение (честный контракт)

`PASS` означает: текущая версия системы прошла production-gate на текущем стенде и данных.

`PASS` не означает математическую гарантию «никогда не упадёт». Для этого нужен непрерывный soak/load-chaos контур.
