# Ops Canonicals

Короткий канонический summary для `alegria-site`: что считается инвариантом, какие команды являются обязательными для ops, и какие запреты нельзя нарушать.

## Инварианты

- `primitives = pure`
  - без `contracts`, SQL, HTTP, external SDK и `serde_json::Value` вне `*_json.rs`
- `seo_steps = pure deterministic step execution`
  - без raw SQL, `JSONB`, `serde_json::Value`, `std::env`, direct SDK imports
- `infrastructure = SQL / IO / boundary`
  - единственное место для `sqlx`, `serde_json::Value`, `JSONB`, `reqwest`, `neo4rs`, `qdrant-client`, `temporalio_*`, `playwright-rs`, `graph-flow`, `rig*`
- Temporal runtime использует `DomainError` как канонический error contract
- Любой side-effectful step проходит через `pipeline.step_executions`
- Канонический runtime persistence:
  - `pipeline.execution_runs` — coarse run registry
  - `pipeline.execution_run_blobs` — run-level payload state
  - `pipeline.step_payload_blobs` — step input/output/error blobs
- Канонический wire/store path:
  - `payload_bytes`
  - `payload_type`
  - `schema_version`
  - `payload_hash`
  - runtime payload schema должен быть специализированным Proto message, а не generic JSON-envelope
- `blake3` — обязательный hash/idempotency/integrity механизм
- `system.sync_outbox`, `system.dead_letter_queue`, `pipeline.hitl_decisions`, `pipeline.reconcile_*` не используют legacy JSON runtime path
- `/metrics`, DLQ, reconcile и backup/restore path обязательны для production verdict

## Канонические команды для ops

Статический и архитектурный контроль:

```bash
bash automation/ci_verify.sh
```

Пересборка production worker:

```bash
docker compose build temporal-worker
```

Container-grade production gate:

```bash
TEMPORAL_GATE_WORKER_MODE=docker bash automation/temporal_production_gate.sh
```

Recoverability drill:

```bash
infra/backups/restore_drill.sh
```

Локальный dev worker:

```bash
set -a
. infra/local/dev_db.env
set +a

cd app/rust
TEMPORAL_URL=http://localhost:7233 \
RUST_LOG=info \
cargo run -p temporal_worker --bin temporal_worker
```

## Три вещи, которые нельзя нарушать

1. Нельзя протаскивать SQL, `JSONB`, `serde_json::Value` или external SDK выше `infrastructure/adapters`.
2. Нельзя возвращать legacy JSON runtime path или generic `JsonPayload` для outbox, step-state, DLQ, HITL и reconcile; только специализированные Proto message + bytes/type/version/hash.
3. Нельзя считать production rollout валидным без тройки:
   - `bash automation/ci_verify.sh`
   - `TEMPORAL_GATE_WORKER_MODE=docker bash automation/temporal_production_gate.sh`
   - `infra/backups/restore_drill.sh`

## Source of truth

- `automation/00_ENTRYPOINT.md`
- `automation/INTEGRITY_RUNBOOK.md`
- `automation/LAYER_MAP.md`
