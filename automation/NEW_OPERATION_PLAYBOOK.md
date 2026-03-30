# New Operation Playbook (Expert, Layered)

Цель документа: дать канонический процесс добавления новой операции в `alegria-site` без нарушения формулы слоёв:

- `primitives = pure`
- `runtime_models = typed runtime domain model`
- `use_cases = policy + orchestration`
- `infrastructure = SQL / IO / boundary`

Этот документ согласован с:

- `automation/LAYER_MAP.md`
- `automation/ci_verify.sh`
- `automation/check_step_contract_completeness.py`
- `automation/check_layer_dependency_matrix.py`

---

## 0. Сначала классифицируйте тип операции

Перед кодом определите, что вы добавляете:

1. Pure operation (чистый расчёт, без IO).
2. Use-case operation (бизнес-сценарий с adapters).
3. Temporal step/activity (оркестрируемый шаг с retry/idempotency/DLQ).
4. Workflow orchestration (порядок вызова нескольких шагов).

Если операция будет участвовать в runtime-step path, сразу проектируйте её как `typed proto + step ledger`.

---

## 1. Contracts: сначала wire-контракт

Реальный путь proto в проекте:

- `app/contracts/proto/temporal_payloads.proto` (runtime payloads)
- при необходимости `app/contracts/proto/read_api.proto`, `rules.proto`, `sync.proto`

Пример для `AnalyzeKeywordDensity`:

- добавить `AnalyzeKeywordDensityInputPayload`
- добавить `AnalyzeKeywordDensityOutputPayload`
- при необходимости добавить в envelope-паттерн (`StepEnvelope` / `StepContractMeta`-совместимость)

После изменений:

1. `bash automation/contract_generate.sh`
2. `bash automation/contract_verify.sh`

Важно:

- не вводить ad hoc JSON как runtime wire.
- не переиспользовать удалённые теги; использовать `reserved`.

---

## 2. Primitives: чистая детерминированная логика

Путь:

- `app/rust/crates/primitives/src/...`

Для примера:

- `app/rust/crates/primitives/src/seo/analysis.rs`

Правила:

- только pure function (вход -> выход, без внешних эффектов).
- без `sqlx`, `reqwest`, `neo4rs`, `qdrant`, `temporalio_*`.
- без `serde_json::Value` (кроме разрешённых `*_json.rs` boundary wrappers).

Выход примитива должен быть типизированным Rust DTO, а не JSON blob.

---

## 3. Runtime models: typed domain DTO

Путь:

- `app/rust/crates/runtime_models/src/lib.rs`

Если операция нужна в runtime pipeline, добавляйте typed DTO сюда:

- входные/выходные модели шага
- без SQL/IO-кода
- без утечек инфраструктурных типов

Допуск `serde_json::Value` здесь только для truly dynamic полей. Для стабильных полей используйте строгие типы.

---

## 4. Infrastructure adapters: единственная IO boundary

Путь:

- `app/rust/crates/infrastructure/src/adapters/...`

Только здесь разрешены:

- `sqlx`, raw SQL
- внешние SDK/HTTP/gRPC
- JSONB mapping
- Proto bytes <-> storage mapping

Для `AnalyzeKeywordDensity`:

- read adapter: загрузка контента
- write adapter: сохранение результата
- при runtime-step: запись/чтение через typed payload store и step ledger

Use-case слой не должен знать ни SQL-текст, ни формат колонок.

---

## 5. Use-cases: бизнес-оркестрация без SQL/JSON boundary

Путь:

- `app/rust/crates/use_cases/src/...`

Для примера:

- `app/rust/crates/use_cases/src/seo/keyword_analysis.rs`

Use-case делает:

1. Получить входные данные через adapter API.
2. Вызвать primitive для расчёта.
3. Сохранить/опубликовать результат через adapter API.

Запрещено в use_cases:

- `sqlx::query*`, `sqlx::Row`, прямой `PgPool`
- `serde_json::Value`, `json!`, `from_value/to_value`
- raw SDK imports

---

## 6. Temporal activity: adapter между Temporal и use-case

Пути:

- `app/rust/services/temporal/src/activities/mod.rs`
- тематический файл, например `app/rust/services/temporal/src/activities/operations.rs`

Шаблон:

1. Activity принимает typed proto input.
2. Конвертирует в доменную typed модель.
3. Вызывает use-case.
4. Возвращает typed proto output.

Для runtime-step path activity должна идти через общий `execute_step(...)` path (idempotency, reuse, dead-letter, metrics).

---

## 7. Workflow: только оркестрация порядка

Пути:

- `app/rust/services/temporal/src/workflows/*.rs`
- регистрация в `app/rust/services/temporal/src/workflows/mod.rs`

Workflow:

- не содержит бизнес-вычислений
- не содержит SQL/JSON logic
- только последовательность activity вызовов, ветвления и signal/query handling

---

## 8. Регистрация в worker

Пути:

- `app/rust/services/temporal/src/activities/mod.rs`
- `app/rust/services/temporal/src/workflows/mod.rs`
- при необходимости `app/rust/services/temporal/src/main.rs`

Если не зарегистрировать activity/workflow, worker не сможет обработать задачу.

---

## 9. Обязательные runtime-инварианты для нового step

Если операция входит в step-runtime, должны быть соблюдены все пункты:

1. Typed proto contract (input/output/error).
2. `StepContractMeta` присутствует и корректно заполнен.
3. Idempotency key derivation deterministic.
4. Step ledger path (`step_executions` / `step_attempts`) используется.
5. Error mapping в taxonomy и retry semantics.
6. Метрики шага и отказов доступны (`step_execution_reused_total`, `activity_failures_total` и т.д.).

---

## 10. Полный порядок внедрения (без срезов)

1. Обновить proto contracts.
2. Сгенерировать и верифицировать contracts.
3. Добавить/обновить primitives.
4. Добавить/обновить runtime_models.
5. Вынести IO в infrastructure adapters.
6. Собрать use-case.
7. Обернуть в activity.
8. Добавить/обновить workflow.
9. Зарегистрировать в worker.
10. Только после этого запускать проверки и сборку.

---

## 11. Канонический validation pipeline

Минимум:

1. `cd /home/bose/projects/alegria-site/app/rust && cargo check --workspace`
2. `cd /home/bose/projects/alegria-site && bash automation/ci_verify.sh`

Для production-grade подтверждения:

1. `docker compose build temporal-worker`
2. `TEMPORAL_GATE_WORKER_MODE=docker bash automation/temporal_production_gate.sh`
3. `bash infra/backups/restore_drill.sh`

Go-критерий:

- Все команды выше завершаются `PASS/OK` без allowlist-исключений.

---

## 12. Антипаттерны (запрещено)

1. Добавлять SQL в `use_cases` или `services/temporal`.
2. Передавать ad hoc JSON вместо typed proto в runtime path.
3. Добавлять внешние SDK в `primitives`.
4. Пропускать регистрацию activity/workflow.
5. Обходить `ci_verify.sh` и production gate.

---

## 13. Короткий template для AnalyzeKeywordDensity

1. Proto:
   - `AnalyzeKeywordDensityInputPayload`
   - `AnalyzeKeywordDensityOutputPayload`
2. Primitive:
   - `compute_keyword_density(text, keywords) -> TypedDensityReport`
3. Adapter:
   - `load_page_text(...)`
   - `save_keyword_density_report(...)`
4. Use-case:
   - `run_keyword_density_analysis(...)`
5. Activity:
   - `analyze_keyword_density(...)`
6. Workflow:
   - включить шаг в нужный workflow path
7. Registration + full validation pipeline.

Этот шаблон применяется к любой новой операции в проекте.
