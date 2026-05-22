# Step Catalog Contract (Step Ledger And Contracts)

**Статус:** current runtime-spec for step ledger/status semantics; historical framing below predates the accepted canonical cutover workflow.
**Цель:** определить контракт шага, статусную модель и execution-record semantics, even though the canonical orchestration workflow is already assembled.

Owner/runtime note:

- the canonical workflow ordering now lives in [V6_SeoSiteBuildWorkflow_Working_Plan.md](V6_SeoSiteBuildWorkflow_Working_Plan.md);
- this file should be used for step record semantics, not for deciding whether orchestration exists yet.

---

## 1. Что такое статусная модель

Статусная модель — это единый жизненный цикл шага/запуска, чтобы:
- одинаково обрабатывать ретраи и ошибки;
- понимать, где именно execution остановился;
- безопасно возобновлять процесс после HITL или сбоя.

Минимальный обязательный набор статусов шага:
- `pending` — шаг создан, но не выполняется;
- `running` — шаг выполняется сейчас;
- `done` — шаг успешно завершён;
- `failed` — шаг завершился ошибкой, auto-retry исчерпан или запрещён;
- `pending_hitl` — шаг остановлен и ждёт решения человека.

Рекомендуемые дополнительные технические статусы:
- `skipped` — шаг намеренно пропущен по policy;
- `cancelled` — шаг остановлен внешней командой;
- `poisoned` — повторяемая детерминированная ошибка контракта/данных;
- `dead_letter` — инфраструктурная ошибка после retry budget.

---

## 2. Историческая модель “сначала каталог шагов, потом поток”

Обязательная последовательность внедрения:
1. Каждый шаг реализуется как отдельный Rust `use_case` (детерминированная логика).
2. Поверх него создаётся тонкий Temporal `activity` wrapper (I/O + вызов use_case).
3. Шаги запускаются вручную по одному (CLI/API), без “большого” workflow.
4. Только после стабилизации контрактов шагов собирается orchestration-workflow.

Эта модель была полезна на этапе сборки. В текущем состоянии проекта она сохраняется как объяснение происхождения step-ledger discipline, а не как описание отсутствующего orchestration workflow.

---

## 3. Единый контракт входа/выхода шага (обязательный)

Поля из вашего списка обязательны, но их недостаточно.  
Полный обязательный контракт `StepExecutionRecord`:

```json
{
  "run_id": "uuid",
  "step_name": "layer_router",
  "step_version": "1.0.0",
  "attempt": 1,
  "status": "running",
  "input_hash": "blake3_hex",
  "output_hash": "blake3_hex",
  "idempotency_key": "step_name:entity_id:input_hash",
  "entity_type": "section|rule|fact|context|page",
  "entity_id": "sec_123",
  "input_schema": "step.layer_router.input.v1",
  "output_schema": "step.layer_router.output.v1",
  "retry_class": "safe|transient|never|hitl_only",
  "max_retries": 2,
  "requires_hitl": false,
  "hitl_task_id": null,
  "prompt_version": null,
  "model_version": null,
  "registry_version": "v5.1.0",
  "pipeline_version": "v5.1.0",
  "executor": "temporal_activity|manual_cli",
  "executor_version": "git_sha_or_build_id",
  "error_code": null,
  "error_message": null,
  "started_at": "RFC3339",
  "finished_at": null,
  "duration_ms": null
}
```

### Что критично добавить сверх исходного списка

- `attempt` — без него нет управляемого retry.
- `entity_type/entity_id` — без них нельзя идемпотентно адресовать шаг.
- `input_schema/output_schema` — без них нет строгой проверяемости контракта.
- `retry_class/max_retries` — без них непредсказуемое поведение при ошибках.
- `executor/executor_version` — нужен точный audit/replay.
- `error_code/error_message` — обязательны для `failed/poisoned/dead_letter`.
- `started_at/finished_at/duration_ms` — операционный контроль SLA.

---

## 4. BLAKE3 как единый hashing-инвариант

Да, во всём приложении за технические хэши должен отвечать **BLAKE3**:
- `input_hash`
- `output_hash`
- `payload_hash` (outbox dedup)
- `content_hash`
- deterministic IDs, где это зафиксировано контрактом

### Жёсткие правила

1. Один алгоритм: только BLAKE3 hex.
2. Один способ сериализации для hash-able payload:
   - детерминированный JSON dump (stable key order).
3. Hash считается в Rust primitives-слое.
4. Любой не-Rust producer обязан проходить parity-check с Rust эталоном.

---

## 5. Нужны ли Proto-контракты для step I/O

Коротко: **да, нужны**, если шаг пересекает процессную/сервисную границу.

### Где Proto обязателен

- Rust service ↔ Rust service (gRPC).
- Temporal starter/worker внешняя команда, если есть межпроцессный транспорт.
- Внешние runtime API, где нужна строгая эволюция схемы.

### Где допустим JSON

- raw snapshots (immutable archive).
- действительно динамические поля (`condition_json`, diagnostics, weakly-typed metadata).
- временные compatibility-стыки, явно помеченные сроком удаления.

Правило проекта: **typed contracts by default, JSON by exception**.

---

## 6. Каталог шагов (без фиксации порядка)

Шаги, которые можно реализовать как независимые units уже сейчас:
- `layer_router`
- `canonical_mapping`
- `completeness_judge`
- `contradiction_gate`
- `hitl_decision`
- `neo4j_backwrite`

Для каждого шага должны существовать:
1. `use_case` (Rust, deterministic logic).
2. `activity` wrapper (Temporal I/O boundary).
3. `single-step` runner (CLI/API) для ручного выполнения.
4. `contract tests` + idempotency tests + replay tests.

---

## 7. Минимальная таблица учёта шагов (рекомендуется)

`pipeline.step_executions` (или эквивалент) должна хранить контракт выше.

Минимальные ограничения:
- `UNIQUE(run_id, step_name, entity_id, attempt)`
- `UNIQUE(idempotency_key)` для side-effectful шагов
- `CHECK(status IN ('pending','running','done','failed','pending_hitl','skipped','cancelled','poisoned','dead_letter'))`
- индекс по `(run_id, status)` и `(step_name, status)`

---

## 8. Acceptance criteria перед сборкой большого workflow

Нельзя собирать orchestration-flow, пока не выполнено:
1. У каждого шага есть стабильный I/O контракт (schema + hashes + idempotency).
2. У каждого шага есть `single-step` запуск и replay.
3. У каждого шага есть deterministic test suite.
4. HITL-путь покрыт (`pending_hitl` -> resolution -> resume).
5. Ошибки классифицируются в `retry_class` без ручных исключений.

---

## 9. Связанные документы

- [V5_Ultimate_Extraction_Protocol.md](V5_Ultimate_Extraction_Protocol.md)
- [V5_Runtime_Contract.md](V5_Runtime_Contract.md)
- [OPS_RUNTIME_RUNBOOK.md](OPS_RUNTIME_RUNBOOK.md)
- [DOMAIN_MODEL.md](DOMAIN_MODEL.md)
