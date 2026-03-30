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
    - `cargo run -p temporal_worker --bin temporal_starter -- start --workflow fact-extraction`

## Что должно быть в окружении

- Temporal Server (`7233`)
- Temporal persistence DB (отдельная БД/кластер)
- Business Postgres (для activities)

## Task queue

- `alegria-pipeline` (должна совпадать у starter и worker)
