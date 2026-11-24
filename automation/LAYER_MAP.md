# Rust layers in Alegria

Карта актуальных зависимостей, а не инструкция создавать новый слой для
каждой операции. Подробные владельцы — в
[ARCHITECTURE_BOUNDARIES.md](../docs/ARCHITECTURE_BOUNDARIES.md).

| Слой / capability | Ответственность | Не владеет |
| --- | --- | --- |
| `primitives` | Общие pure IDs/value objects | SEO, Truth, SQL, Temporal wire |
| `acquisition` | HTML/PDF, source relevance и crawl policy | SQL/network effects, Truth decisions |
| `truth_domain` | Предметная Truth-валидация, evidence, governance, HITL | SQL, LLM/Temporal SDK, protobuf |
| `seo_domain` | Редакция, контентные контракты, IA, ссылки, approval, publishing decisions | `contracts`, `runtime_models`, SQL, transport |
| `seo_application` | Сценарии и комбинация typed operations/ports | SQL/SDK и дублирование domain policy |
| `seo_ports` | Контракты конкретных внешних эффектов/queries | Самостоятельная политика и ненужные Temporal DTO |
| `seo_steps` | Действующие typed step и Temporal wire mappers | SQL/SDK, альтернативный доменный алгоритм |
| `runtime_models` | Общие модели выполнения | Editorial/SEO policy |
| `contracts` | Генерируемые protobuf wire types | Предметные решения и IO |
| `infrastructure::adapters` | SQLx, Qdrant, Neo4j, HTTP, LLM, сериализация/IO | Выбор business verdict при наличии domain owner |
| `services/*` | Compose/runtime, Temporal orchestration, Outbox delivery | Дубли правил из Domain |

Строгая граница направлений: Domain не зависит от adapters, services или
protobuf; Application вызывает предметный API и конкретные typed ports;
инфраструктура реализует порты. Это проверяется Cargo DAG и настоящими
потребителями, а не только текстовыми allowlists.

Исторический протокол в существующем Temporal workflow/step может сохранять
известный wire ID и replay-семантику без переноса прежней бизнес-реализации.
Новый proto добавляется только при действительном внешнем контракте.

Для локальной разработки сначала применяют
`python3 automation/dev_rust_feedback.py --base HEAD` и соответствующий
unit-тест. Точный `ci_impact.py` выбирает реальные E2E для PR; Main CI
производит только необходимый source-bound Worker artifact и прогрев
кэшируемых библиотек. Единственный workflow — `.github/workflows/ci.yml`.
