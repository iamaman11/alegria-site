# Reliability Hardening Research Specification

> Status: research-only. This document informs design but does not override live code, schema, proto, or automation.

## Purpose

Этот документ задаёт исследовательское ТЗ на усиление надёжности runtime-инфраструктуры проекта `alegria-site`.

Цель исследования:

- перевести текущий runtime из состояния `works and passes gate` в состояние `predictable, versioned, observable, recoverable`;
- убрать неявные контракты между шагами, сервисами и БД;
- формализовать правила изменений так, чтобы workflow/activity/runtime не деградировали при развитии кода;
- подготовить реализацию без архитектурного шума и без расширения JSON-хаоса внутрь domain-core.

Это не документ на немедленную реализацию. Это документ на инженерное исследование, результатом которого должны стать точные RFC/ADR/DDL/Proto-изменения и порядок внедрения.

## System Context

Текущее состояние стека:

- основной runtime: Rust;
- orchestration: Temporal;
- business DB: PostgreSQL (`alegria`);
- Temporal persistence DB: отдельный PostgreSQL (`postgres-temporal`);
- graph layer: Neo4j;
- vector layer: Qdrant;
- межсервисные и runtime-контракты: Proto/gRPC там, где уже формализовано;
- детерминированные хэши и ключи: `blake3`;
- JSON допустим только на внешних и динамических границах:
  - raw snapshots,
  - внешние источники,
  - variadic conditions,
  - telemetry/diagnostics.

Фактические кодовые корни, которые должны быть включены в исследование:

- `app/rust/services/temporal`
- `app/rust/crates/seo_steps`
- `app/rust/crates/infrastructure`
- `app/rust/crates/primitives`
- `app/db/schema.sql`
- `docs/V5_Runtime_Contract.md`
- `docs/STEP_CATALOG_CONTRACT.md`
- `docs/OPS_TEMPORAL_PRODUCTION_GATE.md`
- `automation/`

## Research Principles

Исследование должно соблюдать следующие инварианты:

- `blake3` остаётся единственным стандартом для детерминированных payload/content/idempotency hashes.
- Proto обязателен на service boundary, где есть устойчивый контракт между процессами.
- Workflow-логика не должна опираться на неявные JSON-поля и неверсируемые структуры.
- Schema/contract errors должны завершаться `NonRetryable`, а не уходить в бесконечный retry storm.
- Нельзя предлагать слияние Temporal DB и business DB.
- Нельзя расширять runtime случайными fallback-механизмами без явной политики.
- Нельзя оставлять временные migration-стыки как постоянную часть runtime-архитектуры.

## Research Output Format

По каждому пункту исследования должны быть выданы:

- краткий вывод `keep / change / reject`;
- описание текущего состояния в коде;
- список точек изменения: файлы, таблицы, proto-контракты, очереди, метрики;
- точный дизайн предлагаемого решения;
- risk register;
- migration order;
- acceptance criteria;
- список тестов и проверок для `automation/`.

## Research Track 1: Hard I/O Contracts

### Objective

Сделать все межпроцессные и межшаговые контракты жёсткими, версионируемыми и проверяемыми.

### Technology Context

- Proto/gRPC;
- Temporal payload contracts;
- outbox envelope contracts;
- `schema_version`;
- backward/forward compatibility rules;
- JSON только для raw and dynamic boundaries.

### Current Context To Verify

- Какие payload уже типизированы Proto, а какие всё ещё передаются как ad hoc JSON.
- Какие activity/workflow входы и выходы живут только как `serde_json::Value`.
- Где `outbox` уже имеет typed builder, но не имеет typed wire contract.

### Research Questions

1. Какие boundary уже должны быть переведены на Proto немедленно:
   - Temporal activity payload,
   - inter-service gRPC,
   - outbox envelope,
   - step result contracts.
2. Где Proto нужен как canonical schema, а где достаточно typed Rust struct с сериализацией?
3. Какой набор полей должен быть обязательным для каждого step contract:
   - `run_id`,
   - `step_name`,
   - `schema_version`,
   - `input_hash`,
   - `output_hash`,
   - `idempotency_key`,
   - `requires_hitl`,
   - `registry_version`,
   - `prompt_version`,
   - `model_version`,
   - `error_class`.
4. Какие compatibility rules нужны:
   - additive fields,
   - deprecated fields,
   - required field freeze,
   - enum evolution.

### Expected Deliverables

- boundary inventory;
- canonical proto map;
- contract versioning policy;
- список JSON boundary, которые остаются допустимыми;
- proposal на обновление `docs/V5_Runtime_Contract.md`.

### Acceptance Criteria

- Для каждого service boundary есть явное решение `proto / typed rust / allowed json`.
- Для каждого runtime-step определён canonical request/response contract.
- Нет неописанных JSON payloads между сервисами.

## Research Track 2: Error Taxonomy For Activities

### Objective

Сделать retries в Temporal инженерно корректными, а не случайными.

### Technology Context

- Temporal activity retry policy;
- Rust error taxonomy;
- `Retryable` vs `NonRetryable`;
- bounded exponential backoff;
- contract failures vs infrastructure failures.

### Current Context To Verify

- Где activity возвращают ошибки без нормализованной классификации.
- Где schema/DB/validation errors ещё могут попасть в retry loop.
- Где transport/timeout/API errors должны оставаться retryable.

### Research Questions

1. Какой должен быть canonical error model?
2. Какой enum error class нужен:
   - `ContractViolation`,
   - `ValidationFailure`,
   - `ForeignKeyViolation`,
   - `ConflictViolation`,
   - `TransportTimeout`,
   - `RemoteRateLimit`,
   - `Remote5xx`,
   - `InfraUnavailable`,
   - `UnexpectedBug`.
3. Какие ошибки должны завершаться сразу как `NonRetryable`?
4. Какие ошибки допустимо ретраить и с какими лимитами?
5. Как эта классификация должна логироваться и метриковаться?

### Expected Deliverables

- canonical error taxonomy;
- retry policy matrix;
- mapping table `error source -> Temporal behavior`;
- список code points, которые надо исправить.

### Acceptance Criteria

- Для каждой activity-группы есть таблица retry semantics.
- Любой schema/contract mismatch формально не может бесконечно ретраиться.

## Research Track 3: Idempotency By Default

### Objective

Сделать повторный запуск шага безопасным по умолчанию.

### Technology Context

- `blake3`;
- `idempotency_key`;
- `UNIQUE`;
- `ON CONFLICT`;
- deterministic payload hashing;
- outbox dedup.

### Current Context To Verify

- Где idempotency уже обеспечена:
  - `system.sync_outbox`,
  - content hashes,
  - stable IDs,
  - point IDs,
  - run-scoped steps.
- Где она ещё частичная:
  - step execution journal,
  - activity result storage,
  - HITL decisions,
  - Neo4j/Qdrant writes.

### Research Questions

1. Как должен вычисляться canonical `idempotency_key` для каждого step?
2. Достаточно ли `blake3` над canonical serialized input, или нужен составной ключ?
3. Какие таблицы должны иметь `UNIQUE` constraints по step execution?
4. Где нужен отдельный `step_journal`/`execution_attempts` слой?
5. Как предотвратить дубли при replay, retries и manual resume?

### Expected Deliverables

- idempotency matrix по всем step types;
- canonical hash algorithm policy;
- DDL proposal on uniqueness and dedup;
- automation checks на duplicate-safe writes.

### Acceptance Criteria

- Для каждого step описан deterministic idempotency strategy.
- Нет write path без documented dedup behavior.

## Research Track 4: Temporal Versioning Discipline

### Objective

Формализовать безопасные изменения workflow-логики без nondeterminism.

### Technology Context

- Temporal build IDs;
- workflow type versioning;
- task queues;
- replay safety;
- deployment discipline.

### Current Context To Verify

- Как сейчас меняются workflow definitions.
- Есть ли already-enforced build-id discipline.
- Какие parts of logic лежат в workflow, а какие в activities.

### Research Questions

1. Что считается breaking change для workflow?
2. Когда нужен новый `workflow type`, а когда новый `build-id rollout`?
3. Какой deployment protocol нужен для:
   - new activities only,
   - changed activity implementation,
   - changed workflow branch logic,
   - changed signal/query semantics?
4. Какой должен быть rollback path?

### Expected Deliverables

- Temporal versioning policy;
- rollout/rollback runbook;
- matrix `change type -> required rollout discipline`.

### Acceptance Criteria

- Любое изменение workflow-логики можно классифицировать по утверждённой таблице.
- В проекте нет “меняем workflow на месте и надеемся, что replay переживёт”.

## Research Track 5: SLO And Observability

### Objective

Сделать runtime наблюдаемым на уровне операционных сигналов, а не только логов.

### Technology Context

- Prometheus metrics;
- Grafana dashboards;
- Temporal visibility;
- queue backlog;
- retry rate;
- failure by error class;
- stuck-running detection.

### Current Context To Verify

- Какие метрики уже есть у Temporal.
- Какие custom metrics отсутствуют у runtime.
- Какие operational checks сейчас живут только в ручных CLI-командах и gate scripts.

### Research Questions

1. Какие SLI/SLO нужны для проекта?
2. Какие метрики обязательны для продового контура:
   - activity retry rate,
   - workflow completion latency,
   - stuck-running workflows,
   - pending HITL age,
   - outbox backlog,
   - DLQ growth,
   - error class distribution?
3. Какие алерты обязательны?
4. Где должен быть source of truth:
   - Temporal visibility,
   - Postgres queries,
   - app metrics,
   - combined dashboards?

### Expected Deliverables

- SLI/SLO set;
- metric inventory;
- alerting specification;
- dashboard specification;
- additions to `automation/` for smoke observability checks.

### Acceptance Criteria

- Для каждого критичного failure mode есть detectable metric and alert.
- Нет “silent degradation” path без наблюдаемого сигнала.

## Research Track 6: Fail-Safe Contour

### Objective

Сделать систему восстанавливаемой при невосстановимых и полу-зависших состояниях.

### Technology Context

- DLQ;
- reconcile jobs;
- stale run detection;
- stuck workflow cleanup;
- outbox reconciliation.

### Current Context To Verify

- Что уже делает `services/reconcile`.
- Какие зависшие состояния сейчас требуют ручного вмешательства.
- Как закрываются `pending_hitl`, `failed`, `stuck running`, `lost outbox`, `half-persisted` cases.

### Research Questions

1. Какие типы невосстановимых задач должны попадать в DLQ?
2. Нужна ли одна DLQ или несколько:
   - ingestion,
   - extraction,
   - content generation,
   - sync/backwrite?
3. Как должен работать reconcile:
   - schedule,
   - scope,
   - stale thresholds,
   - auto-close vs manual review?
4. Как не потерять forensic evidence при переносе в DLQ?

### Expected Deliverables

- fail-safe architecture;
- DLQ schema/runbook;
- reconcile job spec;
- failure state transition table.

### Acceptance Criteria

- Для каждого irrecoverable path описано, куда уходит задача и как она потом разбирается.
- Для каждого stale state есть deterministic reconcile behavior.

## Research Track 7: Infrastructure Reliability

### Objective

Формализовать продовую надёжность инфраструктуры, а не только runtime-кода.

### Technology Context

- separate Temporal DB;
- separate business DB;
- backup policy;
- retention policy;
- restore drills;
- connection pools;
- container isolation.

### Current Context To Verify

- Отдельные БД уже есть.
- Нужно проверить:
  - retention,
  - backup cadence,
  - restore playbook,
  - connection pool boundaries,
  - blast radius isolation.

### Research Questions

1. Достаточно ли текущего раздельного DB-дизайна?
2. Какие backup policies нужны для:
   - Temporal persistence DB,
   - business DB?
3. Какой restore drill считать обязательным?
4. Какие RPO/RTO допустимы для проекта?
5. Какие соединения и лимиты надо изолировать между Temporal и business runtime?

### Expected Deliverables

- infra reliability ADR;
- backup/restore matrix;
- restore drill plan;
- connection and isolation recommendations.

### Acceptance Criteria

- Есть отдельные backup/restore правила для обеих БД.
- Есть проверяемый restore scenario, а не только декларация “backup configured”.

## Cross-Track Constraints

Во всех исследованиях отдельно проверить:

- как решения влияют на `automation/`;
- какие новые invariants должны быть добавлены в `cli_tools` и gate scripts;
- какие документы должны быть обновлены:
  - `docs/V5_Runtime_Contract.md`,
  - `docs/STEP_CATALOG_CONTRACT.md`,
  - `docs/OPS_RUNTIME_RUNBOOK.md`,
  - `docs/OPS_TEMPORAL_PRODUCTION_GATE.md`.

## Recommended Research Order

1. Hard I/O contracts
2. Error taxonomy
3. Idempotency
4. Temporal versioning
5. Fail-safe contour
6. SLO/observability
7. Infrastructure reliability

Причина такого порядка:

- сначала фиксируются контракты и semantics;
- затем retry/idempotency;
- затем operational controls;
- затем infra hardening.

## Final Research Exit Criteria

Исследование считается завершённым только если:

- по каждому из 7 треков есть отдельное инженерное решение `keep/change/reject`;
- есть список конкретных файлов, таблиц и контрактов для изменения;
- есть migration order;
- есть acceptance tests;
- есть обновления для `automation/`;
- нет пунктов уровня “надо бы повысить надёжность”, не переведённых в конкретные технические действия.
