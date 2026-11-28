# Temporal Service (Rust)

Единый узел Temporal в runtime:

- `temporal_worker` — воркер (workflow + activities).
- `temporal_starter` — CLI для запуска workflow execution и ping.

## Бинарники

- Worker:
  - файл: `src/main.rs`
  - запуск: `cargo run -p temporal_worker`
- Starter:
  - файл: `src/bin/temporal_starter.rs`
  - запуск:
    - `cargo run -p temporal_worker --bin temporal_starter -- ping`
    - `cargo run -p temporal_worker --bin temporal_starter -- start --workflow seo-site-build-canonical-cutover`

В Worker зарегистрирован только один SEO site-build workflow:
`SeoSiteBuildCanonicalCutoverWorkflow`. Удалены прежние
`SeoSiteBuildWorkflow` и `ContentGenerationWorkflow`, а также выбор legacy
workflow в rebuild-dispatch. Перед обновлением уже работающего Temporal Worker
оператор должен подтвердить отсутствие незавершённых историй удалённых типов:
их нельзя безопасно воспроизвести новым Worker.

## Что должно быть в окружении

- Temporal Server (`7233`)
- Temporal persistence DB (отдельная БД/кластер)
- Business Postgres (для activities)

## Task queue

- `alegria-pipeline` (должна совпадать у starter и worker)

## Исполняемая история и изменение порядка шагов

`SeoSiteBuildCanonicalCutoverWorkflow` пока исполняет собственную фиксированную
цепочку Temporal-команд. Новая типизированная композиция шагов
`seo_application::execution::SeoPhaseRecipe` **не является** автоматическим
изменением порядка команд уже работающего Temporal workflow.

Для нового rollout нативного Worker Deployment Versioning необходимо явно
задать `TEMPORAL_WORKER_VERSIONING=true`, `TEMPORAL_DEPLOYMENT_NAME` и
`WORKER_BUILD_ID` в виде полного неизменяемого 40-символьного SHA commit.
Worker отклонит короткие или тестовые Build IDs. Параметр выключен в
локальном Docker Compose: запуск pinned Worker сам по себе не выполняет
Temporal deployment promotion, а старые unversioned histories не получают
пиннинг задним числом. Действующий Worker со старыми histories должен
оставаться доступным до drain.

Перед изменением последовательности реальных workflow-команд необходимы
отдельная версионированная ветка/новый workflow type, проверка реального
исторического replay и выпуск через штатное Temporal Deployment API. Простое
сохранение номера `SeoPhaseRecipe` в данных не обеспечивает replay safety.
