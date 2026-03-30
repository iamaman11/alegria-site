# V5 Runtime Contract

**Статус:** current runtime source of truth for persistence/retry/replay/HITL semantics  
**Назначение:** зафиксировать текущий production-grade runtime contract для V5 pipeline.

---

## 1. Runtime axioms

1. Каждый step обязан иметь `input_hash`, `output_hash`, `idempotency_key`.
2. Каждый step обязан иметь явный `retry_class` и bounded retry behavior.
3. Каждый runtime payload обязан иметь `payload_type`, `schema_version`, `payload_bytes`, `payload_hash`.
4. `blake3` — единственный канонический hash/idempotency/integrity механизм.
5. Procedural truth публикуется только из verified/runtime-controlled stores, не из LLM output напрямую.
6. `pending_hitl` не трактуется как failure/stuck.
7. `dead_letter` и `poisoned` не могут silently disappear.
8. Runtime wire contract — Proto; legacy JSON runtime path запрещён.

---

## 2. Canonical runtime persistence

Current runtime persistence is:

- coarse run registry:
  - `pipeline.execution_runs`
- run-level blobs:
  - `pipeline.execution_run_blobs`
- step ledger:
  - `pipeline.step_executions`
  - `pipeline.step_attempts`
- step payload blobs:
  - `pipeline.step_payload_blobs`
- HITL:
  - `pipeline.hitl_decisions`
- fail-safe / forensic:
  - `system.dead_letter_queue`
  - `pipeline.reconcile_runs`
  - `pipeline.reconcile_actions`
- side effects / outbox:
  - `system.sync_outbox`

`pipeline.execution_runs` is a run registry only. Runtime payload state does not live in legacy JSON fields.

---

## 3. Canonical payload envelope

Every persisted runtime payload follows the same storage quartet:

- `payload_type`
- `schema_version`
- `payload_bytes`
- `payload_hash`

Hashing rules:
- `payload_hash = blake3(payload_bytes)`
- idempotency keys are deterministic and derived from canonical stable inputs

This contract applies to:
- execution run blobs
- step payload blobs
- dead-letter payloads
- reconcile summaries/details
- outbox payloads
- HITL decision payloads

---

## 4. Step ledger contract

Each step execution must persist at least:

```json
{
  "run_id": "uuid",
  "step_name": "verify_rules",
  "attempt": 1,
  "status": "running|done|failed|pending_hitl",
  "input_hash": "blake3_hex",
  "output_hash": "blake3_hex_or_null",
  "idempotency_key": "deterministic_blake3_key",
  "retry_class": "safe|transient|never|hitl_only",
  "requires_hitl": false,
  "error_class": null,
  "executor_build_id": "WORKER_BUILD_ID"
}
```

Additional runtime metadata may exist, but these fields are non-optional.

---

## 5. Retry classes

| retry_class | Meaning | Automatic retry |
|---|---|---:|
| `never` | deterministic contract/data failure | no |
| `safe` | pure/replay-safe step | yes |
| `transient` | infra/network dependency | yes, bounded |
| `hitl_only` | human decision required | no |

Rules:
- `never` -> terminal failure / DLQ according to policy
- `safe` -> bounded retry
- `transient` -> bounded retry + DLQ after budget
- `hitl_only` -> `pending_hitl`, no auto-retry

---

## 6. HITL contract

Current canonical rules:
- workflow pause state is `pending_hitl`
- `pending_hitl` is excluded from stale-failure classification
- `TestHitlWorkflow.resume` accepts no payload
- `FactExtractionWorkflow.resume` accepts typed `HitlDecision`

HITL persistence lives in:
- `pipeline.hitl_tasks`
- `pipeline.hitl_decisions`

---

## 7. Outbox contract

`system.sync_outbox` is Proto/bytes-based only.

Canonical fields:
- `payload_type`
- `schema_version`
- `idempotency_key`
- `payload_bytes`
- `payload_hash`

Runtime must not read or write legacy `payload_json`.

---

## 8. Replay contract

Replay uses immutable canonical inputs plus version tuple and payload blobs.

Minimal replay basis:
- run id / workflow id
- input blob bytes + hash
- step payload blobs
- schema version(s)
- executor build id
- retry/error classification trail

Replay/read-side logic must never depend on removed legacy JSON runtime columns.

---

## 9. Dead-letter and reconcile contract

Terminal deterministic failures and exhausted transient failures must preserve forensic evidence:
- version tuple
- payload bytes/hash
- error class
- workflow/activity correlation
- attempt trail

Canonical fail-safe stores:
- `system.dead_letter_queue`
- `pipeline.reconcile_runs`
- `pipeline.reconcile_actions`

---

## 10. Related sources of truth

- live schema: `app/db/schema.sql`
- live proto contracts: `app/contracts/proto/**`
- step policy: [STEP_CATALOG_CONTRACT.md](STEP_CATALOG_CONTRACT.md)
- runtime operations: [OPS_RUNTIME_RUNBOOK.md](OPS_RUNTIME_RUNBOOK.md)
- production gate: [OPS_TEMPORAL_PRODUCTION_GATE.md](OPS_TEMPORAL_PRODUCTION_GATE.md)
