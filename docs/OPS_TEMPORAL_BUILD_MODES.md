# Temporal Worker Build Modes (Hybrid)

Лучший режим для текущей архитектуры: гибрид.

## 0. Важное про `cargo run`

- Команда `cargo run -p temporal_worker --b` неверна.
- Нужно использовать `--bin`, потому что у пакета `temporal_worker` два бинаря:
  - `temporal_worker`
  - `temporal_starter`

Примеры:

```bash
cd /home/bose/projects/alegria-site/app/rust

cargo run -p temporal_worker --bin temporal_worker
cargo run -p temporal_worker --bin temporal_starter -- ping
```

## 1. Dev (максимальная скорость)

- Держите infra в Docker: `postgres`, `postgres-temporal`, `temporal-server`, `temporal-ui`.
- `temporal_worker` запускайте локально через Cargo.
- При изменениях шагов: только `cargo run` (или `cargo watch`) + быстрый рестарт процесса, без rebuild image.

```bash
cd /home/bose/projects/alegria-site
docker compose up -d postgres postgres-temporal temporal-server temporal-ui

export TEMPORAL_URL=http://localhost:7233
export DATABASE_URL=postgres://postgres:postgres_password@localhost:5433/alegria
export RUST_LOG=info
export WORKER_BUILD_ID=dev-$(date +%s)
export METRICS_PORT=9464

cd app/rust
cargo check -p temporal_worker
cargo run -p temporal_worker --bin temporal_worker
# опционально:
# cargo watch -x "run -p temporal_worker --bin temporal_worker"
```

## 2. Prod/CI (надежность и воспроизводимость)

- `temporal_worker` только как Docker image.
- На изменение кода worker: `docker compose build temporal-worker && docker compose up -d temporal-worker`.
- `temporal-server` пересобирать не нужно, если менялся только worker-код.
- Отдельный “контейнер сборки Rust” не нужен: в Dockerfile уже настроен multi-stage build:
  - builder-слой компилирует `-p temporal_worker`,
  - runtime-слой запускает только бинарь `temporal_worker`.
- Это собирает `temporal_worker` и его зависимости, а не все сервисы workspace как runtime-контейнеры.

```bash
cd /home/bose/projects/alegria-site
export WORKER_BUILD_ID=$(date +%Y%m%d)-$(hostname)
docker compose build temporal-worker
docker compose up -d temporal-worker
docker compose logs -f temporal-worker
```

## 3. Критично для workflow-изменений

- Если меняете логику уже запущенного workflow-типа, используйте versioning-дисциплину: новый workflow type, новая task queue или отдельный rollout-процесс с worker routing.
- Для changes только в activity-коде обычно достаточно перезапуска worker.
- `WORKER_BUILD_ID` обязателен для Docker worker. Новый release = новый build id.
- В текущем runtime `WORKER_BUILD_ID` используется как обязательная identity/traceability-метка worker-а.
- Полноценный server-side worker version routing нельзя включать "вслепую": его нужно активировать только вместе с отдельной процедурой deployment registration и acceptance-gate на assignment workflow к build id.
- Метрики должны оставаться доступны на `:9464/metrics` после каждого деплоя.

### Когда достаточно нового `WORKER_BUILD_ID`

- меняется только activity implementation;
- workflow replay topology не меняется;
- signal/query contract не меняется;
- старые history не увидят нового command ordering.

### Когда обязателен новый `workflow type`

- меняется порядок workflow commands;
- меняется ветвление state machine;
- меняется signal payload semantics;
- старый history не может безопасно replay на новом коде.

### Rollback / drain policy

- старый build-id нельзя выключать до явного drain старых history;
- rollback = вернуть старый worker fleet/build-id, а не менять workflow history “на месте”;
- новый workflow type вводится параллельно старому, пока старые execution не будут завершены или выведены по отдельной процедуре.

## 4. Фактический статус проверок (2026-03-28)

- `cargo test -p temporal_worker` — успешно (`0 failed`), но unit-тестов пока `0`.
- Temporal connectivity: `cargo run -p temporal_worker --bin temporal_starter -- ping` — `ok`.
- E2E HITL и production gate должны подтверждаться отдельным acceptance-прогоном после каждой существенной правки worker/runtime.

## Итог

Для режима “экспертно и производительно”: локальный worker в dev + Docker worker в prod. Это самый быстрый и безопасный режим.
