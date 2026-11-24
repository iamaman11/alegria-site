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
| Truth | `truth_domain::{candidate_intake,completeness,hitl_decision,persisted_candidates,semantic_truth_write}` и соседние предметные модули | SQL/LLM/Temporal остаются у адаптеров и runtime; JSON не определяет решение |
| Knowledge | `seo_domain::canonical_mapping`, `seo_application::knowledge`, типизированные порты в `seo_ports::knowledge` | SQLx — авторитетное VERIFIED состояние, Qdrant — необязательная подсказка, Outbox — доставка проекций |
| SEO / editorial | `seo_domain::{editorial,draft_quality,page_readiness,content_contract,link_recommendation,ia_planning,whole_page,rebuild,review_decision,cms_revision,writer}`; один `editorial::ContentBlock` и общая проверка traceability | `seo_steps::{editorial_wire,page_wire,content_contract_validate_step,draft_qa_step,link_recommend_step}` адаптируют действующие Temporal DTO; решения остаются в домене; старый неиспользуемый HTML block-validator удалён |
| Global Site Reconcile | `seo_steps::global_site_reconcile_step` решает, какие ссылки истекли, какие Content Gap закрыты и какие страницы актуальны | `infrastructure::adapters::seo_ports_sqlx_adapter::planning_ports` сохраняет IA, ссылки, gaps, retirement и навигацию **одной** PostgreSQL-транзакцией; SQL-адаптер не принимает редакционных решений |
| Плановые проекции | `seo_domain::artifact_identity` и `seo_steps` определяют логический результат; `sqlx_seo_adapter` сохраняет SERP/Opportunity/IA и Outbox атомарно | Qdrant/Neo4j — проекции SQL-истины; один стабильный ключ Qdrant retry в `sqlx_seo_adapter/shared_helpers.rs`, без отдельных переопределений по шагам |
| Application | `seo_application::{drafting,scenario,review_publish,knowledge,rebuild_detect,cms_review}` | Координирует typed ports; единый `GraphCapabilityPort::require_graph_contract`, без дублирования допуска Graph в трёх фазах |
| Publishing | `seo_domain::{publishing,render_preview,cms_revision,review_decision}`; `seo_application::review_publish` | `infrastructure::adapters::sqlx_seo_cms_adapter` и `seo_ports::PublicReleaseDelivery`: PostgreSQL, Outbox, HTTP/CDN; CMS human review требует точного обязательного `revision_id`, QA — единственный владелец persisted blockers |
| Runtime | `services/temporal`, `services/outbox_worker`, `services/cli_tools` | Activity/workflow orchestration, регистрацию, retry, логи и вызов адаптеров; не копирует бизнес-условия |

`contracts` содержит действующие protobuf wire-формы, не предметные решения.
`runtime_models` содержит общие типизированные данные выполнения (например,
`ExecutionRun` и `RuleRoleType`), но не редакционную политику. Удалённый
`runtime_models::seo_blocks` **не является допустимой точкой расширения**.

## При изменении операции

1. Найти текущего владельца решения и все реальные вызовы (`rg` по функции и типу).
2. Менять предметное правило в одном доменном модуле; `seo_application`
   координирует порты, `seo_steps` и Temporal переводят wire, `infrastructure`
   исполняет I/O. Исключения требуют конкретного объяснения на примере операции.
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
