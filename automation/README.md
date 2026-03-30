# Alegria Automation

Этот каталог — верхнеуровневый контур экспертной верификации системы.
Он является source-of-truth для регламентов и проверок качества
и собирает единый отчёт консистентности.

Канонический вход:

- `automation/00_ENTRYPOINT.md`

Единая карта automation-контуров:

- `automation/PROJECT_NAVIGATION_MAP.md`
- `automation/LAYER_MAP.md`
- `automation/FSM_TYPED_PHASE_IMPERATIVE_PLAN.md`

## Что проверяется

- Плоская документация в `docs/` (без `docs/knowledge`, все active-docs в корне)
- Границы Python/Rust (runtime Python в `app/` отсутствует)
- Послойность Rust adapter-layer (`infrastructure/adapters/*` как единая точка внешних SDK)
- Purity `primitives = pure`
- Purity `services/temporal` (без raw DB logic и ad hoc JSON boundary)
- Layer dependency matrix (`contracts -> primitives/policies/use_cases/infrastructure` запрещено и т.д.)
- Явные семейства runtime-типов `domain / storage / wire`
- JSON boundary policy (deny-by-default: JSON только на boundary/dynamic слоях)
- DomainError / retry taxonomy для Temporal runtime
- Execution ledger для идемпотентности каждого шага
- Metrics/SLO contract и fail-safe reconcile/DLQ
- Инварианты ingest и идемпотентность
- Контрактный контур proto (`contract_generate` / `contract_verify`)
- Базовая покрытость runtime-зависимостей в `requirements.txt` (Rust-first)
- Базовая готовность артефактов данных к ingest
- Прогресс миграции deterministic-слоя (HTML sections, facts extraction, quality gates) в Rust

## Быстрый запуск

```bash
bash automation/ci_verify.sh
```

Отчёт сохраняется в:

- `automation/reports/consistency_report.json`

Регламент и критерии GO/NO-GO:

- `automation/INTEGRITY_RUNBOOK.md`

## Точечный запуск

```bash
python3 automation/check_expert_consistency.py --report-json automation/reports/consistency_report.json
```

С запуском встроенных check'ов:

```bash
python3 automation/check_expert_consistency.py \
  --run-boundary \
  --run-rust-adapters \
  --run-invariants \
  --report-json automation/reports/consistency_report.json
```

## Операционный режим

Быстрый аудит (локально, без тяжёлых прогонов):

```bash
bash automation/audit.sh fast
```

Полный аудит (все проверки + rust check):

```bash
bash automation/audit.sh full
```

## Контроль миграции Python -> Rust

Проверка строгой послойности adapter-layer:

```bash
python3 automation/check_rust_adapter_boundaries.py
python3 automation/check_rust_layer_isolation.py
python3 automation/check_primitives_purity.py
python3 automation/check_temporal_runtime_purity.py
python3 automation/check_layer_dependency_matrix.py
python3 automation/check_runtime_type_families.py
```

Проверка структуры документации:

```bash
python3 automation/check_docs_layout.py
```

Проверка JSON boundary policy:

```bash
python3 automation/check_json_boundary_policy.py
python3 automation/check_allowed_boundary_points.py
python3 automation/check_use_cases_external_sdk_ban.py
python3 automation/check_services_temporal_boundary.py
```

Проверка error/runtime discipline:

```bash
python3 automation/check_domain_error_usage.py
python3 automation/check_step_execution_contract.py
python3 automation/check_step_contract_completeness.py
python3 automation/check_metrics_contract.py
python3 automation/check_reconcile_failsafe.py
python3 automation/smoke_broken_schema_to_dlq.py
python3 automation/smoke_exhausted_retry_to_dlq.py
python3 automation/smoke_stale_outbox_reclaim.py
python3 automation/smoke_pending_hitl_not_failure.py

Поведенческий fail-safe gate:

```bash
bash automation/fail_safe_behavioral_gate.sh
```
```

Проверка proto-контрактов:

```bash
bash automation/contract_generate.sh
bash automation/contract_verify.sh
```

Проверка контракта миграции (progress mode, Rust CLI):

```bash
cargo run -q --manifest-path app/rust/services/cli_tools/Cargo.toml -- \
  check-rust-migration-contract \
  --root . \
  --report-json automation/reports/rust_migration_contract.rust.json
```

Жёсткий gate перед удалением Python реализации (strict mode, Rust CLI):

```bash
cargo run -q --manifest-path app/rust/services/cli_tools/Cargo.toml -- \
  check-rust-migration-contract \
  --root . \
  --strict \
  --report-json automation/reports/rust_migration_contract.strict.rust.json
```

Проверка parity для `kb/fact_verifier.py` (R4 gate, Rust CLI):

```bash
cargo run -q --manifest-path app/rust/services/cli_tools/Cargo.toml -- \
  check-fact-verifier-parity \
  --root . \
  --report-json automation/reports/fact_verifier_parity.rust.json
```
