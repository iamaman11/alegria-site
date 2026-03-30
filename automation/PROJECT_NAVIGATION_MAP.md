# Alegria Automation Navigation Map

Единая точка входа для инженерного контроля качества проекта.

## 1) Start Here
1. `automation/README.md` — базовый контур и команды.
2. `automation/INTEGRITY_RUNBOOK.md` — GO/NO-GO правила перед ingest.
3. `automation/ci_verify.sh` — основной fail-fast gate.

## 2) Contract Layer
- `automation/contract_generate.sh` — генерация `PROTO_CONTRACTS.md`.
- `automation/contract_verify.sh` — проверка drift + compile-путь proto-контрактов.
- `automation/PROTO_CONTRACTS.md` — generated инвентарь и хэши proto.

## 3) Boundary & Architecture Gates
- `automation/check_docs_layout.py` — плоская структура `docs/` и отсутствие drift по `docs/knowledge`.
- `automation/check_python_rust_boundary.sh` — запрет Python runtime-path.
- `automation/check_rust_adapter_boundaries.py` — строгая послойность Rust adapter-layer.
- `automation/check_rust_layer_isolation.py` — запрет недопустимых зависимостей между слоями.
- `automation/check_json_boundary_policy.py` — JSON deny-by-default boundary policy.
- `automation/check_python_scope.py` — контроль Python scope и запрещённых импортов.

## 4) Data & Invariants
- `automation/check_end_to_end_invariants.py` — schema + JSONL инварианты ingest.
- `automation/check_expert_consistency.py` — агрегированный consistency-report.

## 5) Migration & Parity
- Rust CLI `check-rust-migration-contract` (`app/rust/services/cli_tools`) — миграционный контракт.
- `automation/check_fact_verifier_parity.py` — parity-валидация верификатора.
- Исторические migration-планы перенесены в `docs/_archive/pipeline_reorg_2026-03-27/automation/`.

## 6) Operational Commands
- Быстрый аудит: `bash automation/audit.sh fast`
- Полный аудит: `bash automation/audit.sh full`
- Основной gate: `bash automation/ci_verify.sh`
