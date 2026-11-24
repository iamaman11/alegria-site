# Alegria: действующие владельцы и границы (#43)

Этот документ описывает **текущий код**, а не обязательную схему из множества
слоёв. История переносов, открытые A1–A18 и критерии приёмки находятся в
[Issue #43](https://github.com/iamaman11/alegria-site/issues/43).
Новые crates, порты, фасады и CI jobs добавляются только для действительной
независимой ответственности; имена технологий сами по себе не границы.

| Владелец | Где правило / действующий вызов | Эффекты и внешние типы |
| --- | --- | --- |
| Общие значения | `primitives`: стабильные идентификаторы и чистые функции | Нет SQL, Temporal и внешних сервисов |
| Acquisition | `acquisition::{html_sections,pdf_parser,source_relevance,crawl_policy}`: парсинг, релевантность, robots и retry | `infrastructure::adapters::raw_crawl_adapter`: сеть, очередь, сохранение |
| Truth | `truth_domain::{candidate_intake,completeness,hitl_decision,persisted_candidates,semantic_truth_write}` и соседние предметные модули | `sqlx_truth_write_adapter` владеет **одной** PostgreSQL-транзакцией: источники, кандидаты, HITL, rule state, registry, Outbox и step journal; Temporal Activity только подготавливает проверенные данные/Voyage до неё, Qdrant — только через доставку Outbox |
| Knowledge | `seo_domain::canonical_mapping`, `seo_application::knowledge`, типизированные порты в `seo_ports::knowledge` | SQLx — авторитетное VERIFIED состояние, Qdrant — восстанавливаемая поисковая проекция; UPSERT и DELETE проходят через типизированный Outbox с проверкой актуального состояния/порядка события, а не прямым вызовом из Truth |
| SEO / editorial | `seo_domain::{editorial,draft_quality,page_readiness,content_contract,link_recommendation,ia_planning,whole_page,rebuild,review_decision,cms_revision,writer}`; один `editorial::ContentBlock` и общая проверка traceability | `seo_steps::{editorial_wire,page_wire,content_contract_validate_step,draft_qa_step,link_recommend_step}` адаптируют действующие Temporal DTO; решения остаются в домене; старый неиспользуемый HTML block-validator удалён |
| Global Site Reconcile | `seo_steps::global_site_reconcile_step` решает, какие ссылки истекли, какие Content Gap закрыты и какие страницы актуальны | `runtime_composition::adapters::seo_ports_sqlx_adapter::planning_ports` вызывает PostgreSQL-owner для сохранения IA, ссылок, gaps, retirement и навигации **одной** транзакцией; композиционный адаптер не принимает редакционных решений |
| Плановые проекции | `seo_domain::artifact_identity` и `seo_steps` определяют логический результат; `sqlx_seo_adapter` сохраняет SERP/Opportunity/IA и Outbox атомарно | Qdrant/Neo4j — проекции SQL-истины; один стабильный ключ Qdrant retry в `sqlx_seo_adapter/shared_helpers.rs`, без отдельных переопределений по шагам |
| Application | `seo_application::{drafting,scenario,review_publish,knowledge,rebuild_detect,cms_review}` | Координирует typed ports; единый `GraphCapabilityPort::require_graph_contract`, без дублирования допуска Graph в трёх фазах |
| Publishing | `seo_domain::{publishing,render_preview,cms_revision,review_decision}`; `seo_application::review_publish` | `infrastructure::adapters::sqlx_seo_cms_adapter` и `seo_ports::PublicReleaseDelivery`: PostgreSQL, Outbox, HTTP/CDN; CMS review требует точного `revision_id`. Публичные обязательные ссылки берутся из **утверждённой CMS-ревизии** (`required_link_payload`), не из меняющихся рекомендаций; HTML связывает их с текущим release snapshot, не выводя служебные ключи |
| Публичная навигация | `seo_domain::navigation` определяет подписи; `sqlx_seo_adapter::navigation_persistence` сохраняет scoped-модель; `sqlx_static_site_adapter` читает её вместе с утверждёнными CMS-ревизиями | `static_site_builder_adapter` отображает **только опубликованные** пункты своей локали; меню не пересчитывает самостоятельно из полного каталога страниц, планирование навигации не становится отдельным источником публичной истины |
| Runtime | `services/temporal`, `services/outbox_worker`, `services/cli_tools`, `temporal_gateway` | Workflow/Activity orchestration, retry, таймеры и HITL — у Worker; независимый Temporal gateway владеет клиентским подключением/сигналами; Outbox доставляет проекции, CLI исполняет операторские операции |
| Runtime composition | `runtime_composition::adapters` соединяет `seo_ports` с PostgreSQL, Neo4j и Qdrant без предметных правил | Единственный смешанный слой с направлением зависимостей `runtime_composition → infrastructure / graph_neo4j / vector_qdrant`. `infrastructure` не зависит от графового/векторного владельца даже транзитивно; это проверяется Cargo manifest-closure guard. Worker, CLI и E2E подключают композицию явно. |
| Analytics operator | `services/cli_tools::analytics_client` | Только операторский gRPC-клиент и свой protobuf build; общий `infrastructure` не зависит от генерации Analytics-контрактов |

`contracts` содержит действующие protobuf wire-формы, не предметные решения.
`runtime_models` содержит общие типизированные данные выполнения (например,
`ExecutionRun` и `RuleRoleType`), но не редакционную политику. Удалённый
`runtime_models::seo_blocks` **не является допустимой точкой расширения**.

Навигационная проекция входит в approved CMS build через
`run_publish_materialize_step`. Navigation-only refresh без новой CMS-ревизии
проверен в реальном PostgreSQL NOTIFY → listener → HTML/HTTP readback тесте
(PR #90). Эта операция не создаёт новую CMS-ревизию и сохраняет публикационный
freshness fence; отдельная интеграция с процессом Worker остаётся частью полной
Temporal-приёмки. Возвращать удалённую фиктивную очередь `navigation_reconciled`
для этого не нужно.

## Композиция функциональных операций (#94)

**Создание сайта — recipe из независимых функциональных операций, а не
неизменяемая схема зависимостей приложения.** Границу операции определяют её
предметный вход/выход и побочные эффекты, а не название стадии сайта, SDK или
отдельного CI job. Новая операция должна иметь одного владельца правил и
явные prerequisites; её можно повторно применить в другом сценарии без
копирования алгоритма из Temporal или SQL-адаптера.

- `seo_domain` и `truth_domain` определяют чистые правила и допустимость
  состояния. `seo_application` собирает типизированные операции в план;
  `seo_steps` переводит wire-контракты. SQLx, граф и векторный поиск исполняют
  эффекты, но не выбирают порядок редакционных шагов.
- `seo_application::execution::recipe` описывает типы входных/выходных
  артефактов и проверку порядка для существующего semi-auto исполнителя.
  **Это не универсальный динамический Temporal workflow**: canonical Worker
  пока использует собственный зафиксированный durable порядок команд.
- `seo_ports::SerpIngestRepository` отвечает только за сохранение выдачи SERP;
  `PlanningRepository` — за последующие результаты планирования. Потребитель
  SERP не обязан реализовывать интерфейсы IA, графа, CMS или публикации.
  Границы проверяются и на минимальных тестовых реализациях портов, а не
  только на общем SQLx-адаптере, реализующем несколько ролей.
- `seo_application::scenario` предоставляет узкие исполнители
  `execute_crawl_ingest_only[_with_recipe]` и
  `execute_planning_only[_with_recipe]`. Первый требует только пять портов
  источников/SERP/Truth/проекций; второй дополняет их планированием и графом.
  Они используют общий обход шагов с проверкой рецепта **до I/O**, но не
  требуют портов генерации, CMS, публикации и rebuild. Полный сценарий
  продолжает использовать тот же обход шагов и более широкий набор портов.
- Повтор/восстановление Temporal не может молча получить новый порядок
  команд из изменившегося default recipe. Для изменяемой durable композиции
  требуется закрепить идентичность/версию принятого плана за run, проверить
  зависимости до side effects и отдельно доказать history replay/restart.
- PostgreSQL владеет атомарными Truth/CMS/Outbox/step-ledger переходами;
  Qdrant/Neo4j — восстанавливаемыми проекциями; Temporal — orchestration,
  task queues, timers, HITL и retries. Разделение модулей не должно превращать
  эти инварианты в межпроцессные distributed transactions.

Не создавать crate для каждого шага, общий строковый registry, плагины или
RPC-обёртки только ради перестановки стадий. Если для реальной независимой
ответственности нужен отдельный crate, его цена подтверждается dependency
graph и замерами компиляции. Цель 1–3 минуты относится к полному обязательному
CI и не объявляется достигнутой по локальным unit-тестам.

## При изменении операции

1. Найти текущего владельца решения и все реальные вызовы (`rg` по функции и типу).
2. Менять предметное правило в одном доменном модуле; `seo_application`
   координирует порты, `seo_steps` и Temporal переводят wire, `infrastructure`
   исполняет собственные PostgreSQL/HTTP-эффекты, а `runtime_composition` соединяет
   их с графовым и векторным владельцами. Исключения требуют конкретного объяснения на примере операции.
3. Сразу перенести настоящих потребителей и удалить прежний путь; не оставлять
   совместимость ради тестового приложения.
4. Проверить unit-поведение и выбранный **рабочий** путь с PostgreSQL/Temporal/
   Outbox, если он действительно затронут. Точные владельцы suites и image
   определяются `automation/ci_impact.py` по base/head SHA.
5. Сохранить существующие persisted IDs и Temporal history/replay-семантику:
   удаление дублирующего Rust API не означает разрешения менять wire без теста.

Удалённые тестовые/legacy `SeoSiteBuildWorkflow` и `ContentGenerationWorkflow`
**больше не регистрируются**: новый путь — только
`SeoSiteBuildCanonicalCutoverWorkflow`. Это безопасно для исходного кода,
но **не доказывает готовность развертывания** в среде с незавершёнными
legacy Temporal histories; перед заменой Worker нужны её собственные
drain/replay-доказательства (см. `docs/OPS_TEMPORAL_BUILD_MODES.md`).

## CI и сборка

- Только `.github/workflows/ci.yml`. PR проходит exact-source `impact`,
  применимые owners и обязательный агрегат `CI`; пропущенный owner не PASS.
- Действующие проверки запускаются командой
  `python3 automation/dev_rust_feedback.py --base <base-ref>` локально и штатным
  PR CI. Для внешних эффектов локальная команда не заменяет реальный CI.
- На доверенном `main` `run_main_cache_prime.py` прогревает только повторно
  используемые Rust libraries; полные тесты и реальные E2E уже принадлежат PR.
  Rust cache и Docker release — разные графы с разными compile profiles.
- `select-image` может пропустить Worker на недоставляемом изменении;
  `package` при необходимости проверяет startup, exact-source SHA, сохраняет
  архив и Sigstore provenance. Это не установка на staging/production.

**Не объявлять #43 выполненным** только потому, что переносы завершены и CI
зелёный. Нужны предусмотренные Issue интеграционные доказательства и реальные
времена каждого класса изменений; прогретое ≤5:00 ещё не доказано устойчиво.
