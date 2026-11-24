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

set -a
. infra/local/dev_db.env
set +a

export TEMPORAL_URL=http://localhost:7233
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

- Если меняете логику уже запущенного workflow-типа, используйте versioning-дисциплину: новый workflow type, новая task queue, отдельный rollout с worker routing либо явный SDK patch-marker с сохранением старой ветки и проверкой replay.
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
- меняется ветвление state machine без версионированного SDK patch-marker и подтверждённого replay;
- меняется signal payload semantics;
- старый history не может безопасно replay на новом коде.

### Редакторский возврат `blocked`

- Для новой редакции уже **опубликованной** страницы live CMS pointer остаётся на прежнем релизе. `cli_tools cms-review-list` перечисляет отдельно ожидающую ревизию с её точным `revision_id`; `cli_tools cms-review-show --page-node-key <key> --revision-id <revision>` показывает её неизменяемый текст. Не считать опубликованную текущую ревизию объектом новой проверки.
- Подлинное решение редактора передаётся только с точным `--revision-id <revision>` в `cms-approve-publish` / `cms-block` и с действующим `--temporal-url http://alegria_temporal:7233` при вызове внутри Worker. Приложение проверяет, что ревизия находится в `review_required` и связана с активным каноническим workflow. Отсутствующий или устаревший идентификатор не может одобрить опубликованную старую ревизию. Передавать фактическую роль редактора, не подменять его решение системой или автоматическим тестом.
- `cms-block` сохраняет решение редактора и причину; оно **не** утверждает страницу и не меняет уже сохранённую ревизию.
- В новых Temporal histories marker `seo-editorial-terminal-rework-v1` завершает попытку со статусом `blocked:editorial_rework_required` без публикации; повторный черновик создаётся отдельным новым run после штатной проверки отсутствия активного business run.
- `cms-reopen` оставляет ту же ревизию в HITL-ожидании; `cms-approve-publish` требует отдельного явного решения.
- Для старых histories без patch-marker сохраняется прежняя ветка ожидания. Нельзя переключать их на новое поведение без replay/rollout-проверки; существующий активный workflow не отменять автоматически.

### Rollback / drain policy

- старый build-id нельзя выключать до явного drain старых history;
- rollback = вернуть старый worker fleet/build-id, а не менять workflow history “на месте”;
- новый workflow type вводится параллельно старому, пока старые execution не будут завершены или выведены по отдельной процедуре.

### Retired legacy SEO workflow types and deployment gate

- `SeoSiteBuildWorkflow` и `ContentGenerationWorkflow` удалены из исполняемого Worker: новые SEO-запуски используют только `SeoSiteBuildCanonicalCutoverWorkflow`;
- локальное machine-readable replay evidence показывает `total_runs=0` и `open_runs=0` **только для проверенной локальной среды**, а не для staging/production;
- **до развёртывания Worker** в каждой целевой среде необходимо независимо подтвердить `open_runs=0` для удалённых типов или завершить drain старым Worker/build-id. Если таких доказательств нет, не заменять старый Worker этим образом;
- rollback для всё ещё выполняющихся старых histories — старый Worker/build-id, не переименование и не изменение сохранённой history. Не восстанавливать удалённый runtime ради пустых тестовых сценариев.

### Naming decision after cutover

- `SeoSiteBuildCanonicalCutoverWorkflow` остаётся runtime workflow type и после drain window;
- нельзя вводить новый workflow type только ради косметического rename;
- если в будущем понадобится rename, он допустим только вместе с уже необходимым новым workflow type из-за новой workflow semantics, либо в отдельно одобренное zero-history cleanup окно.

## 4. Фактический статус проверок (2026-03-28)

- `cargo test -p temporal_worker` — успешно (`0 failed`), но unit-тестов пока `0`.
- Temporal connectivity: `cargo run -p temporal_worker --bin temporal_starter -- ping` — `ok`.
- E2E HITL и production gate должны подтверждаться отдельным acceptance-прогоном после каждой существенной правки worker/runtime.

## Итог

Для режима “экспертно и производительно”: локальный worker в dev + Docker worker в prod. Это самый быстрый и безопасный режим.
