# Layer Map

Каноническая матрица слоёв для `alegria-site`.

## Формула

- `primitives = pure`
- `runtime_models = typed runtime domain model`
- `seo_steps = pure deterministic step execution`
- `infrastructure = SQL / IO / boundary`

## Слои

### `contracts`

Содержит только Proto-generated types и thin wire envelopes.

Разрешено:
- `prost`
- generated modules
- wire DTO

Запрещено:
- зависимости на `primitives`
- зависимости на `policies`
- зависимости на `seo_steps`
- зависимости на `infrastructure`

### `primitives`

Содержит детерминированные value objects, parsing, normalization, validation, hashing.

Разрешено:
- `serde`
- `blake3`
- `regex`
- `scraper`
- pure helper logic

Запрещено:
- `contracts`
- `sqlx`
- `reqwest`
- `neo4rs`
- `qdrant-client`
- `temporalio_*`
- `tokio-postgres`
- `tonic`
- `hyper`
- `tower`
- `rig*`
- `graph-flow`
- `playwright-rs`
- `serde_json::Value` вне `*_json.rs`

### `runtime_models`

Содержит typed runtime domain DTO без SQL/IO/boundary logic.

Разрешено:
- `contracts` только для wire-compatible field types
- `serde`
- `serde_json::Value` только для truly dynamic runtime fields
- typed runtime/domain structs

Запрещено:
- `sqlx`
- инфраструктурные adapters
- HTTP/DB/Temporal SDK
- `seo_steps`
- `policies`
- `primitives`

### `policies`

Содержит typed policy logic поверх `primitives` и, по необходимости, policy-schema поверх `contracts`.

Разрешено:
- `primitives`
- `contracts`

Запрещено:
- `sqlx`
- `serde_json::Value`
- HTTP/DB/Temporal SDK
- инфраструктурные adapters

### `seo_steps`

Содержит orchestration-free business logic и typed coordination между domain types и adapter APIs.

Разрешено:
- `primitives`
- `runtime_models`
- `policies`
- `contracts`
- typed adapter APIs из `infrastructure`

Запрещено:
- `sqlx::query*`
- `sqlx::Row`
- `sqlx::PgPool` напрямую
- `serde_json::Value`
- `json!`
- `serde_json::to_value/from_value`
- `std::env` / `env::var`
- `connect_pg()` / `connect_neo4j()` / `connect_qdrant()` в use-case коде
- raw SDK imports (`neo4rs`, `reqwest`, `qdrant-client`, `temporalio_*`, `tokio-postgres`, `rig*`, `graph-flow`, `playwright-rs`)

### `infrastructure/adapters`

Единственная допустимая boundary-зона для SQL/JSON/IO/external SDK.

Разрешено:
- `sqlx`
- `serde_json::Value`
- `JSONB`
- `neo4rs`
- `qdrant-client`
- `reqwest`
- `temporalio_*`
- `tokio-postgres`
- `tonic`
- `hyper`
- `tower`
- `rig*`
- `graph-flow`
- `playwright-rs`

### `services/*`

Thin runtime assembly:
- worker
- starter
- reconcile
- outbox worker
- metrics endpoints

Запрещено:
- raw DB logic
- `sqlx::query*`
- `serde_json::Value`
- direct external SDK calls вне adapters/seo_steps

## Единственные допустимые JSON-boundary points

- `app/rust/crates/infrastructure/src/adapters/**`
- `app/rust/crates/primitives/src/*_json.rs`
- `app/rust/crates/telemetry/src/**`
- `app/rust/services/outbox_worker/src/**`
- `app/rust/services/gsc_sync/src/**`
- `app/rust/services/cli_tools/src/**`

Все остальные JSON-boundary usage считаются нарушением.
