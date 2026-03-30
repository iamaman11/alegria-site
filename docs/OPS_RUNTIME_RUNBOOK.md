# Operations Runtime Runbook (V5, Rust-first)

Этот документ объединяет orchestration, temporal execution plan, HITL и Rust migration gate.

## 1) Runtime architecture

- Temporal server хранит durable workflow history.
- Rust temporal worker исполняет зарегистрированные workflows/activities.
- Rust temporal starter запускает workflow executions и служит операционной точкой входа.
- Workflow layer: только deterministic orchestration.
- Activity layer: весь I/O (DB, HTTP, LLM, graph, vector).

### Runtime code map (актуально)

- Temporal service: `app/rust/services/temporal`
  - worker: `src/main.rs`
  - starter CLI: `src/bin/temporal_starter.rs`
  - workflows: `src/workflows/mod.rs`
  - activities: `src/activities/mod.rs`
- Domain crates: `app/rust/crates/*`
- Runtime services: `app/rust/services/*`
- DB schema: `app/db/schema.sql`
- Business DB container: `alegria_postgres`
- Business DB pool boundary: `alegria_pgbouncer` (`:6432`, session pooling)
- Temporal DB container: `alegria_postgres_temporal`
- Temporal server container: `alegria_temporal`
- Temporal worker container: `alegria_temporal_worker`
- Metrics endpoint: `http://localhost:9464/metrics`
- Observability stack: `alegria_prometheus` (`:9090`), `alegria_grafana` (`:3000`)
- Worker version discipline: every docker worker deployment must set `WORKER_BUILD_ID`
- `WORKER_BUILD_ID` is mandatory runtime identity and traceability metadata.
- Full server-side worker routing/version assignment must only be enabled together with a dedicated rollout procedure and acceptance-tested workflow assignment.
- New `WORKER_BUILD_ID` is sufficient only for activity-only changes or replay-safe internal fixes.
- New workflow type is mandatory when workflow branch/order/signal semantics change or old histories cannot replay safely.
- Rollback/drain policy: old worker build-id stays alive until old histories drain; rollback returns old worker fleet, never mutates workflow history in place.
- Runtime forensic/ledger tables:
  - `pipeline.execution_run_blobs`
  - `pipeline.step_executions`
  - `pipeline.step_attempts`
  - `pipeline.step_payload_blobs`
  - `pipeline.hitl_decisions`
  - `system.dead_letter_queue`
  - `pipeline.reconcile_runs`
  - `pipeline.reconcile_actions`
- `pipeline.execution_runs` is coarse run registry only; runtime payload state lives in `pipeline.execution_run_blobs` and `pipeline.step_payload_blobs`.
- Versioned outbox contract:
  - `system.sync_outbox.payload_type`
  - `system.sync_outbox.schema_version`
  - `system.sync_outbox.idempotency_key`

## 2) Current workflow chains

- `FactExtractionWorkflow`:
  - `extract_facts`
  - `verify_rules`
  - `prepare_hitl_pause`
  - `wait/resume (signal)`
  - `apply_hitl_resolution`
  - `persist_and_emit`
  - `finalize_run`

- `ContentGenerationWorkflow`:
  - `generate_content`
  - `validate_blocks`
  - `finalize_run`

- `FreshnessCheckWorkflow`:
  - `check_data_freshness`

## 3) HITL contract

### Trigger scenarios
- canonical mapping `0.75–0.87` -> review
- canonical mapping `<0.75` -> new candidate
- numeric conflict
- open blocking conflict case
- unresolved range rule

### Priority model
- `1` blocking (SLA 4h)
- `2` important (SLA 8h)
- `3` background (SLA 24h)

### Resolution outcomes
- `approved`
- `rejected`
- `registry_extension_required`
- `conflict_resolution_required`
- `needs_new_source`

### Orchestration behavior
- blocking HITL policy задана контрактом
- текущий runtime: HITL queue + workflow pause/resume через Temporal signal подключены для Rust workflows
- publish stays blocked while unresolved blocking conflict exists

### Current implementation note

- `use_cases::hitl_queue` существует и работает на уровне БД.
- В `app/rust/services/temporal/src/workflows/mod.rs` есть signal/query/update handlers:
  `pause`, `resume`, `status`, `set_pause`.
- Для `FactExtractionWorkflow` добавлен HITL-gate:
  `prepare_hitl_pause` -> ожидание `resume` -> `apply_hitl_resolution`.

## 4) Rust-first migration scope

In scope:
- `app/*` runtime
- temporal worker
- use_cases, adapters, policies

Out of scope:
- `docs/_archive`
- `infra/analytics_lab` (R&D only)

## 5) Implementation plan (execution order)

### P0 Infrastructure
1. Temporal server + UI healthy
2. Rust worker connected to task queue
3. Temporal starter can start workflows on target queue
4. Temporal persistence separated from business data
5. Docker worker uses PgBouncer, local dev worker may use direct localhost Postgres

### P1 Determinism boundary
1. Workflow contains orchestration only
2. All I/O in activities
3. Non-deterministic functions banned from workflow code

### P2 Run-state and idempotency
1. Every run tracked by `run_id`
2. Side effects idempotent + deduplicated
3. Outbox and publish paths replay-safe
4. Step attempts and payload blobs preserved for replay/forensics

### P3 HITL integration
1. Blocking conflicts pause workflow
2. Human decision resumes workflow
3. Publish gate enforces unresolved-conflict blocking
4. Fail-safe stale scan must exclude `pending_hitl`
5. Every resolution is persisted in `pipeline.hitl_decisions`

### P4 Acceptance
1. E2E run on production-like dataset
2. Metrics/alerts on failures, retry storm, lag
3. Full pass of `automation/ci_verify.sh`

## 6) Backup and restore

- Business DB backup: `infra/backups/backup_business_pg.sh`
- Temporal DB backup: `infra/backups/backup_temporal_pg.sh`
- Business DB restore: `infra/backups/restore_business_pg.sh <dump.sql>`
- Temporal DB restore: `infra/backups/restore_temporal_pg.sh <dump.sql>`
- Full restore drill: `infra/backups/restore_drill.sh`

`restore_drill` is the mandatory resilience exercise after schema/runtime hardening. A release is not considered operationally safe if `restore_drill.sh` or `automation/temporal_production_gate.sh` fails.

## 7) Release gate

Production release is allowed only when:
- `bash automation/ci_verify.sh` passes
- runtime smoke checks pass
- HITL pause/resume verified
- metrics endpoint exposes runtime counters/histograms
- restore drill succeeds
- reconcile writes persistent run/action forensic records

## 8) Archive policy

Любой файл, выводимый из runtime-пути, переносится в `docs/_archive/*` с датой и без удаления истории.
