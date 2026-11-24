# Добавление и изменение операции Alegria

Цель — менять один понятный предметный owner и только затронутые потребители.
Не нужно для каждой новой функции заводить proto, crate, отдельный слой, CI
job или универсальный фасад. Сначала изучите
[действующие границы](../docs/ARCHITECTURE_BOUNDARIES.md).

## 1. Определите владельца существующей операции

Найдите принятие решения и его рабочий маршрут:

```bash
rg -n 'имя_операции|публичный_тип|reason_code' app/rust
```

| Изменяется | Правильное место |
| --- | --- |
| HTML/PDF, robots, retry, source relevance | `acquisition` |
| VERIFIED evidence, HITL, Truth intake | `truth_domain` |
| Структура страниц, редакция, QA, ссылки, восстановление, approval | `seo_domain` |
| Композиция нескольких решений, эффекты через typed ports | `seo_application` |
| PostgreSQL, Qdrant, Neo4j, LLM, HTTP, Outbox delivery | `infrastructure::adapters` |
| Внешние protobuf входы/выходы существующего шага | `seo_steps` wire mapper и Temporal activity |
| Длительные вызовы, retry, signal, replay, регистрация | Temporal workflow/worker |
| Общие значения без предметной политики | `primitives` или существующий `runtime_models` |

Примеры: `seo_domain::editorial` владеет правилами блоков;
`seo_domain::content_contract::validate` — единым набором QA blockers;
`seo_steps::{editorial_wire,page_wire}` — преобразованием protobuf.
Не возвращайте редакционные правила в удалённый `runtime_models::seo_blocks`.

## 2. Сделайте вертикальный срез

1. Поменяйте typed вход, выход и решение **одного** существующего owner.
2. Обновите действующие вызовы (Application, SQLx adapter, Temporal, CLI).
3. Удалите прежний дублирующий алгоритм и мёртвый re-export.
4. Если понадобился внешний эффект — определите конкретную операцию порта;
   не создавайте generic storage/provider architecture.
5. Меняйте protobuf только при реальном изменении внешнего Temporal контракта.
   При таком изменении нужны генерация, `automation/PROTO_CONTRACTS.md` и
   проверка существующего wire identity; не переиспользуйте удалённые теги.

Для типового локального изменения внутри существующего шага обычно достаточно
изменить доменный модуль и его реальные вызовы. Никаких обязательных десяти
пустых этапов нет.

## 3. Проверяйте поведение и последствия

```bash
python3 automation/dev_rust_feedback.py --base HEAD
python3 automation/check_ci_impact.py
python3 automation/check_module_boundaries.py
```

`dev_rust_feedback.py` включает локальные правки и даёт Cargo scope; для
committed diff можно указать `--base HEAD~1`. Не путайте это с доказательством
GitHub exact-head CI. Перед PR нужен `cargo fmt --all -- --check`; CI сам
выбирает `cargo test` и реальные suite по `automation/ci_impact.py`.

Минимальная приёмка в PR:

- В новом owner есть тест решения (валидный, отказ, крайние значения).
- Компилируются все активные consumers, без запрещённых Cargo edges.
- Изменение соответствующего исходника выбирает настоящую integration suite,
  когда операция затрагивает внешнюю систему: CMS revision, Draft QA scenario,
  Temporal restart, Knowledge A13, Outbox A14 или Publishing A17.
- Все применимые jobs в **единственном** `.github/workflows/ci.yml` завершились
  `success`; отсутствие suite отражено как `not_applicable`, не как PASS.
- Измерены затраты, не добавлена новая церемония ради одного теста.
- Объединяйте принятый PR через GitHub **Merge pull request** с двумя
  родителями, а не Squash/Rebase. Основной CI проверяет точную связь
  merge-коммита с проверенным PR head и отвергает иной тип слияния.

## 4. Release — отдельное действие

После принятого PR на `main` штатные `select-image` и `package` определяют,
нужен ли Worker artifact. `package` проверяет offline startup, source identity,
архив и подпись. Это не развёртывание на staging или в production.
Сценарии эксплуатации, восстановление и публичная readback-приёмка выполняются
по их настоящим runbook/санкционированному release, а не при каждом переносе
доменных функций.

**Антипаттерны:** дубли блокирующих условий в Draft QA и Content Contract;
правила в SQL/Temporal adapters; универсальный DTO там, где уже есть
доменный тип; сохранение старого runtime API; новый CI job для проверки,
которую уже запускает действующий owner.
