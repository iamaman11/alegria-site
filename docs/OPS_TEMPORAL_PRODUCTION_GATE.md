# Temporal Production Gate

Этот документ фиксирует **воспроизводимый** способ подтвердить production-grade готовность Temporal-контура в Alegria.

## Каноническая команда

```bash
bash automation/temporal_production_gate.sh
```

Если в конце есть строка `PRODUCTION_GATE: PASS`, gate пройден.

Для baseline-диагностики и безопасного fresh bootstrap локальной business DB используются:

```bash
python3 automation/check_local_runtime_db_baseline.py --database-url "$DATABASE_URL"
python3 automation/bootstrap_local_runtime_baseline.py --database-url "$DATABASE_URL" --recreate
python3 automation/check_truth_extraction_provider_ready.py
bash automation/run_retrieval_contract_gate.sh
bash automation/run_live_provider_minimal_scope_gate.sh
```

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
- `SeoSiteBuildCanonicalCutoverWorkflow`:
  - старт по валидному UUID `workflow_id=run_id`;
  - загружает `verified_support_bundle` из БД по `context_key`;
  - не продолжает draft path при пустом support bundle;
  - создает review-required revision;
  - после approval signal выполняет materialization и render validation;
  - завершение `COMPLETED`;
  - `pipeline.execution_runs.status = done`.
- `SeoSiteBuildWorkflow`:
  - допускается только как legacy compat/drain path и не является forward smoke target для production gate.
- `FreshnessCheckWorkflow`:
  - завершение `COMPLETED`.

5. Перед runtime smoke/prod-gate применяется migration set из `app/db/migrations/*.sql`.
   - Полный replay `app/db/schema.sql` поверх старого volume не считается каноническим способом выравнивания БД.
   - Если baseline-таблицы отсутствуют, local volume считается drifted и должен быть recreated или доведён до baseline отдельно.
   - Canonical preflight verdict задаётся `automation/check_local_runtime_db_baseline.py`; drifted volume не является valid migration target.

5. Проверяет runtime observability:
- `temporal-worker` экспонирует `:9464/metrics`;
- endpoint содержит:
  - `activity_duration_seconds`
  - `workflow_completions_total`
  - `step_execution_reused_total`

6. Проверяет deployment discipline:
- worker стартует с `WORKER_BUILD_ID`;
- docker-mode прогон использует актуальный контейнерный runtime.
- hard-required retrieval contract runs through `bash automation/run_retrieval_contract_gate.sh`.
- if the retrieval contract is blocked, production gate fails before workflow smoke.
- active voyage4 retrieval materialization must prove real Voyage vectors for required Qdrant collections; deterministic fingerprint vectors are rejected as production-quality substitutes.
- draft support retrieval must use the required collection order and `rerank-2.5`; missing rerank capability is a production-gate failure.
- canonical Step 5 live-provider gate runs through `bash automation/run_live_provider_minimal_scope_gate.sh`.
- live truth extraction path не считается ready без одного из configured provider paths:
  - `SEO_TRUTH_LLM_LOCAL_ENDPOINT|SEO_LLM_LOCAL_ENDPOINT`
  - `OPENAI_API_KEY`
  - `ANTHROPIC_API_KEY`
  - `GEMINI_API_KEY|GOOGLE_API_KEY`
- current recommended provider for this gate and for `Step 5` closure: `GEMINI_API_KEY` with `GEMINI_TRUTH_MODEL` or fallback `GEMINI_SEO_MODEL`
- `PENDING_CREDENTIALS` from the canonical Step 5 gate is a production-gate failure, not a pass-with-warning.

## Почему это production-grade

- Проверяется **не только компиляция**, а полный E2E путь на живом Temporal Server и Postgres.
- Проверяется отказоустойчивый контур с HITL pause/resume.
- Проверяются DB-инварианты, которые ранее вызывали зацикливание ретраев.
- Проверяется согласованность workflow-статуса и бизнес-статуса в `execution_runs`.

## Ограничение (честный контракт)

`PASS` означает: текущая версия системы прошла production-gate на текущем стенде и данных.

`PASS` не означает математическую гарантию «никогда не упадёт». Для этого нужен непрерывный soak/load-chaos контур.
