# FSM Typed-Phase Imperative Plan

Цель: перевести workflow-state в Temporal с `phase: String` на закрытые `enum`-состояния и закрепить это как архитектурный императив системы.

## 1. Почему это нужно

Текущее состояние:
- workflow-фазы задаются строками (`"extract_facts"`, `"done"` и т.д.).
- ошибки в названиях фаз и нелегальные переходы ловятся поздно (тесты/рантайм), а не компилятором.

Целевое состояние:
- у каждого workflow закрытый `enum Phase`.
- переходы описаны явно через таблицу/функцию переходов.
- статус наружу отдается типизированно (не ad hoc string как источник истины).

## 2. Архитектурный императив (обязательное правило)

С этого плана и далее:
1. Новые workflow/state-машины не могут использовать `phase: String` как канонический state.
2. Канонический state workflow = `enum` + явные правила переходов.
3. Любой переход должен быть проверяемым (compile-time exhaustive `match` + unit tests).
4. Строка фазы допустима только как compatibility/projection-представление, не как source-of-truth.

## 3. Послойное закрепление

### contracts
- Добавить typed phase enums в proto для query/status response (если статус уходит наружу).
- Для compatibility можно оставить текстовое поле временно, но пометить deprecated migration path.

### runtime_models
- Хранить shared status DTO с enum-полями, если они нужны между сервисами.
- Не хранить произвольные phase strings как primary model.

### use_cases
- Не должны знать о workflow-phase строках вообще.
- Оркестрационные состояния остаются в `services/temporal`.

### infrastructure/adapters
- Если нужно сохранять phase в DB/telemetry:
  - сохранять enum-code или строгое string-представление из enum-конвертера.
  - запретить ручные строковые литералы фаз вне enum-конвертера.

### services/temporal
- Для каждого workflow:
  - `enum <WorkflowName>Phase`
  - `enum <WorkflowName>Event` (или эквивалент)
  - `fn transition(state, event) -> Result<state, DomainError>`
- `ctx.state_mut` обновляет enum state, не строку.

## 4. План миграции (поэтапно)

### Phase A — In-place typed state skeleton
1. Для `FactExtractionWorkflow`, `ContentGenerationWorkflow`, `FreshnessWorkflow`, `TestHitlWorkflow`:
   - заменить `phase: String` на typed enum.
2. Добавить `as_str()`/`from_str()` только как projection/compat слой.
3. Не менять порядок activity-вызовов (чтобы не ломать deterministic replay).

### Phase B — Явные переходы
1. Для каждого workflow добавить transition-функцию.
2. Убрать прямые присваивания фаз, кроме через transition helper.
3. В signal/update handlers использовать те же transition-правила.

### Phase C — Typed status contract
1. Если status/query используется внешне:
   - добавить phase enum в proto (`temporal_payloads.proto` или профильный proto).
2. Вернуть статус как typed enum (+ опционально legacy text на миграционный период).

### Phase D — Cleanup
1. Удалить legacy строковые phase-поля из runtime source-of-truth.
2. Оставить только конвертеры для backward compatibility (с sunset датой).

## 5. Automation (обязательные новые проверки)

Добавить в `automation/`:

1. `check_temporal_phase_enum_policy.py`
- FAIL если в `services/temporal/src/workflows/*.rs` найдено:
  - `phase: String`
  - присваивания вида `phase = "...".to_string()`

2. `check_workflow_transition_guards.py`
- проверяет наличие transition-функций для workflow.
- проверяет отсутствие прямых state-mutation в обход transition helper.

3. `check_workflow_phase_contracts.py`
- если status/query экспортируется наружу, проверяет наличие typed phase в proto/DTO.

4. Включить их в `automation/ci_verify.sh`.

## 6. Determinism и версиярование Temporal (критично)

Переход на typed FSM может быть:
- безопасным in-place, если меняется только representation (string -> enum), а topology команд не меняется;
- requiring new workflow type, если меняется state machine behavior (ветвления/порядок команд/signal semantics).

Правило:
1. Только representation change -> новый `WORKER_BUILD_ID` достаточно.
2. Behavioral change -> новый workflow type + rollout/drain policy.

## 7. Acceptance criteria

Считается завершённым, когда:
1. В `services/temporal/src/workflows/*` нет канонического `phase: String`.
2. Все workflow используют typed enum состояния.
3. Есть transition guard-функции и unit tests на легальные/нелегальные переходы.
4. Новые automation-checks зелёные в `ci_verify.sh`.
5. `temporal_production_gate.sh` и `restore_drill.sh` проходят после миграции.

## 8. Канонический порядок выполнения

1. Кодовые правки workflow-state на enum.
2. Добавление transition guards.
3. Контрактные изменения (если требуется внешний typed status).
4. Добавление automation-checks.
5. Полный прогон:
   - `cargo check --workspace`
   - `bash automation/ci_verify.sh`
   - `docker compose build temporal-worker`
   - `TEMPORAL_GATE_WORKER_MODE=docker bash automation/temporal_production_gate.sh`
   - `bash infra/backups/restore_drill.sh`

---

Этот документ является implementation-plan и architectural imperative одновременно: строковые фазы больше не считаются допустимым source-of-truth для workflow-state.
