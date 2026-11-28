# Issue #94: проверенные границы операций и внешних эффектов

Срез исходников: Draft PR #95, `851c3fa54fd80c6ad7eb90c8f24d97568baeb45b` (24.11.2026).
Нормативное ТЗ и полный первоначальный реестр — [Issue #94, §6.1–6.7](https://github.com/iamaman11/alegria-site/issues/94).
Этот документ фиксирует **решения по спорным границам и найденные прямые эффекты**;
он не является вторым runtime-каталогом и не задаёт число, состав или порядок будущих шагов сайта.

Для всей исходной матрицы 52 кандидатов есть
[`ISSUE94_OPERATION_DECISION_MATRIX.md`](ISSUE94_OPERATION_DECISION_MATRIX.md).
Её 52 строки, 73 реальные Temporal Activity сигнатуры/вызовы, 22 legacy фаз
и пять pure-реализаций сверяются командой
`python3 automation/audit_operation_inventory.py --format summary`.
Проверка запрещает молча добавлять/удалять Activity, менять соответствие
`SeoPhaseKey` и терять источники решений; она **не** утверждает полноту будущих
функциональных операций или прохождение effectful production replay.
Текущий статус доказательств для **каждого из 52** кандидатов также доступен
через `python3 automation/audit_operation_inventory.py --format readiness`.
Изолированные owner-контракты `ACQ-01/02` (строгие DTO, без установки в
generic runtime), пять source-registered pure `semantic.*` и отдельные
one-node unit tests не означают, что 52 кандидата прошли приёмку: ни одному
из них пока не атрибутировано независимое физическое Temporal/PG/provider E2E
на текущем SHA. `EX-03` представлен только своей чистой SPLIT-частью.

## Что именно уже посчитано

| Представление в существующем коде | Результат аудита | Чего число не означает |
| --- | --- | --- |
| Temporal Activity registry | 73 endpoints по §6.1; 55 canonical V1, 2 дополнительных штатных, 14 Expert, 2 Test | Не 73 независимые бизнес-операции: часть — transport, journal, gate и projection |
| `SeoPhaseKey` | 22 legacy фаз; §6.2 сопоставляет их с Activity | Не обязательная последовательность и не универсальный operation protocol |
| §6.3 | 52 кандидата с предварительными KEEP/MERGE/SPLIT/INTERNAL | Не 52 утверждённых контракта; несколько строк должны распасться на policy и эффект |
| §6.6 | E01–E18: CLI, gRPC, Worker, recovery, test и analytics entrypoints | Не дополнительные 18 бизнес-шагов; часть работает поверх §6.3 |
| `operation_protocol` | Пять работающих pure-операций в `seo_application::operation_adapter`; generic DAG v2 с invalidation | Не production Temporal DAG и не право на реальные эффекты |

**Правило решения:** самостоятельная операция имеет осмысленные typed Input/Output/Error,
schema/revision, владельца *предметного решения*, эффекты и retry/idempotency.
Произвольный `#[activity]`, SQL query, операторская команда, gRPC transport,
технический журнал или повторный вызов одного правила не создают новой операции.
Порты и контракт операции не зависят от Temporal/SQLx/protobuf; внешний эффект
допускается исполнителем отдельно от `allowed_effects` внутри пользовательского DAG.

## Решения по наиболее спорным строкам §6.3 и §6.6

Слово «решение» здесь означает **проверенную рекомендацию cutover по данному SHA**.
Это не означает, что новый typed contract уже реализован или принят CI.

| ID в #94 | Решение для cutover | Source-level основание и нужная граница |
| --- | --- | --- |
| **EX-07** | **SPLIT на две pure-операции** SEO и commercial signals; общий PG section loader — infrastructure port | [`layered_extraction.rs`](../app/rust/services/temporal/src/activities/operations/truth_extraction_ops/layered_extraction.rs): `seo_signal_extraction_sweep_impl` и `commercial_signal_extraction_sweep_impl` — разные входные/выходные DTO, словари и сигналы; сейчас обе предметные эвристики в Temporal-обёртках после PG read. Не объединять их под названием «извлечение сигналов» и не объявлять SQL fetch предметной операцией |
| **EX-03 + KN-02a** | **SPLIT pure canonical mapping и необязательное vector enrichment**; прежняя гипотеза `MERGE→EX-03` из §6.3 **отклонена** | [`canonical_mapping.rs`](../app/rust/crates/seo_domain/src/canonical_mapping.rs) реализует детерминированный `map_mentions` без I/O, [`knowledge.rs`](../app/rust/crates/seo_application/src/knowledge.rs) делает отдельный `CanonicalVectorSearchPort` lookup только для `MatchingStage::VectorQdrant`, а [`acquisition_section_ops.rs`](../app/rust/services/temporal/src/activities/operations/acquisition_section_ops.rs) смешивает оба в sweep. Qdrant — **неавторитетная подсказка**, не разрешение Truth write; допустим пропуск enrichment с явным результатом unavailable |
| **TR-08 / E06 / E10** | **SPLIT decision / durable manual queue / Temporal await / CLI transport** | [`truth_domain::hitl_decision`](../app/rust/crates/truth_domain/src/hitl_decision.rs) уже владеет чистым решением маршрутизации; [`manual_truth_gate.rs`](../app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover/manual_truth_gate.rs) ждёт durable PG status и только использует signal как wakeup, [`manual_llm.rs`](../app/rust/services/cli_tools/src/cli/manual_llm.rs) сначала сохраняет typed submission, затем посылает сигнал. Ни `LlmNext`, ни `TruthHitlList` — отдельный бизнес-шаг; решение пользователя должно сохранять PG-идемпотентность |
| **ED-05** | **SPLIT PII sanitation и editorial/page admissibility** | [`pii_redaction_prepass_step.rs`](../app/rust/crates/seo_steps/src/pii_redaction_prepass_step.rs) трансформирует строки и protobuf DTO; [`page_readiness.rs`](../app/rust/crates/seo_domain/src/page_readiness.rs) отдельно определяет `PageAdmissibility`, freshness и обязательные роли. PII — отдельный обязательный safety gate перед выходом текста, а не необязательная часть Draft QA; предметные типы надо отвязать от protobuf |
| **E01 ↔ PL-05** | **KEEP как разные назначения, пока не доказан общий contract** | [`analytics_svc::build_link_plan`](../app/rust/services/analytics_svc/src/main.rs) даёт graph-context соседей из `Context/BELONGS_TO/Concept`; [`seo_domain::link_recommendation`](../app/rust/crates/seo_domain/src/link_recommendation.rs) выбирает *page-to-page* scoped links с eligibility/roles/anchor/required. Результат Analytics нельзя автоматически публиковать как `LinkRecommendation`. WCC/PageRank — read analytics, не site planning по умолчанию |
| **E09 BuildStaticSite** | **INTERNAL operator preview/export**, НЕ merge с PUB-03 | [`static_site.rs`](../app/rust/services/cli_tools/src/cli/static_site.rs) вызывает тот же renderer и записывает локальный output dir; он не выполняет CMS approval, release switching и remote delivery. Не использовать этот CLI как эквивалент безопасной финальной публикации |
| **E09 RawReextractEmpty** | **INTERNAL recovery**, не операция проектирования сайта | Тот же `static_site.rs` вызывает `raw_crawl_adapter::recover_empty_raw_sections` для указанной страницы; это реконструкция существующего источника, а не новая семантическая операция |
| **E09 CrawlPendingSources ↔ ACQ-07** | **SPLIT source acquisition business result и прямое legacy runtime исполнение** | `crawl_pending_sources` через `SqlxSeoRuntimeRepository::crawl_sources` выполняет PG/network/возможный Qdrant path и принимает `emit_qdrant`. До cutover требуется доказательство Outbox/fence каждого write; нельзя просто завернуть CLI в generic DAG и считать эффект авторизованным |
| **E13 ↔ CMS-01b** | **MERGE как варианты одной CMS decision policy**, оставить отдельные командные DTO | `approve/block/reopen` — разные решения над одним scoped CMS-review state, а не три новых стадии в site builder. Проверить exact `revision_id` и PG revision fence при переносе |
| **E14 SeoRun** | **INTERNAL legacy composition**, не добавлять в generic operation registry | [`seo_run.rs`](../app/rust/services/cli_tools/src/cli/seo_run.rs) переводит шесть CLI подрежимов в `SeoScenarioKind` и целый `execute_site_build_scenario`; это рецепт над функциями, не отдельная операция |
| **E16 SeoPreflight / RunReconcile / RebuildDispatch** | **INTERNAL capabilities** с явными владельцами проверки состояния, delivery/recovery и scheduling | [`main_dispatch.rs`](../app/rust/services/temporal/src/bin/temporal_starter/main_dispatch.rs) исполняет часть команд до подключения к Temporal, часть — после. Preflight проверяет readiness, а не создаёт содержимое сайта; Reconcile восстанавливает эффекты, RebuildDispatch назначает работу |
| **E16 OntologyBackfillPlan** | **ОТДЕЛЬНЫЙ ОПЕРАТОРСКИЙ backfill; P0 quarantine применяемых write-режимов** | [`ontology_backfill.rs`](../app/rust/services/temporal/src/bin/temporal_starter/ontology_backfill.rs) читает PG, опционально делает прямые Neo4j/Qdrant writes и затем PG `kb.qdrant_points`. Это не PUB-04 и не безвредный dry-run. Ниже обязательный fence |
| **E15 DemoHitl / E17 / E18** | **TEST/LAB**, не принимать в production operation catalog | `temporal_starter DemoHitl` — demo flow; `whole_page_semantic_report` — fixture/report tool; Python analytics lab дублирует функции gRPC сервиса. Удаление/изоляция после доказанной reachability, без изменения replay зарегистрированных live V1 workflows |

## Прямые provider-effects вне обычной Outbox-доставки

Это отдельная **source-call-site проверка**, а не утверждение, что любое использование
Qdrant/Neo4j нарушает Truth-инвариант. Чтение и администрирование имеют другие
полномочия, чем запись восстанавливаемой проекции.

| Точка входа и путь | Наблюдаемый эффект | Классификация / требование |
| --- | --- | --- |
| `runtime_composition::projection_materialize_adapter` через Outbox Worker / leased projection sync | Qdrant `upsert_projection`/`delete_projection`, Neo4j `materialize_*` | Штатная доставка из PG intent; сохранять lease, causal revision, provider ACK и неизвестный результат, не превращать в произвольную Activity |
| `temporal_starter::OntologyBackfillPlan --apply-neo4j` | Прямой `materialize_concept` | **BYPASS** Outbox-пути; операторская пересборка проекции должна иметь versioned PG intent и проверенный допустимый scope/version либо быть временно заблокирована |
| `temporal_starter::OntologyBackfillPlan --apply-qdrant` | Прямой `ensure_default_dense_collection` + `upsert_embedding_points`, затем отдельный `INSERT ... kb.qdrant_points` | **BYPASS** атомарного PG→Outbox, при сбое между RPC и SQL возможна расходимость. Заблокировать write-режим до fence, dry-run оставить read-only; при cutover доказать retry/crash/late-RPC |
| `search_whole_page_prototypes` → `ensure_seeded` | При отсутствии коллекции `whole_page_advisory_prototypes` автоматически создаёт её и делает `upsert_embedding_points` | **Неявный write из read-path**. Это служебный версионированный prototype index, не Verified Truth; вынести init в явный owner bootstrap с idempotent/version schema и диагностикой, поиск оставить read-only. Не объявлять его чистой операцией пока auto-seed включён |
| `analytics_svc::run_wcc/compute_page_rank` → `ensure_graph_projection` | `gds.graph.project` создаёт graph analytics cache при отсутствии | Временный GDS cache-effect, не durable Truth/Neo4j projection. Объявить в operator capability/preflight; не считать WCC/PageRank pure read-only и не смешивать с PL-05 |
| `temporal_starter::Neo4jSchemaInstall` → `graph_neo4j::schema::install_required_constraints` | Schema DDL в Neo4j | Разрешённая *отдельная операторская миграция* только после остановки/дренажа writers; не часть бизнес-плана |
| Qdrant semantic search, canonical vector lookup; graph planning links, provider health/readback | SELECT/SEARCH/COUNT/SHOW | Read-only относительно проекций; возможно внешнее I/O и retry, но нет PG→Outbox write-обязательства. В протоколе всё равно декларировать `read_effect` и provenance |

**Минимальная P0-защита до запуска обновлённых Worker:** закрыть применяющие режимы
`OntologyBackfillPlan` от неконтролируемых provider writes, убрать автоматическую
инициализацию Qdrant из read-only advisory flow (или честно оформить отдельный
необходимый bootstrap), сохранить `Neo4jSchemaInstall` только явной operator action.
Решение должно быть **кодовым** и тестироваться: dry-run не оставляет внешних
изменений, forbidden direct-effect отказ происходит *до* RPC, допустимый
backfill пишет PG intent и проходит тот же lease/ACK/fence path, crash/late-RPC
не переводит неопределённое состояние в успешное.

## Условия следующего architecture freeze

1. Для всех 52 строк §6.3 и E01–E18 утвердить/отвергнуть классификацию
   по конкретному владельцу правила, самостоятельному typed I/O/Error,
   required/produced artifacts и полномочиям эффекта. Этот документ проверяет
   только перечисленные спорные строки, а не закрывает неизвестные по остальным.
2. После каждого SPLIT отделить чистое решение от I/O и отдельного
   durable/transport механизма; подсчитывать только утверждённые **операции**,
   не их Activity, CLI, PG queries или версии исполняемого сценария.
3. Для каждой реальной операции зарегистрировать owner contract и зафиксировать
   revision/hash; валидировать внешний provenance в авторитетном store,
   не доверять одному строковому полю `provenance` в плане.
4. Одна операция/подмена операции должна добавляться через owner registration,
   без переписывания `operation_protocol`, соседних операций и Temporal
   Workflow-source. Три реальные DAG с разными подмножествами/порядком
   проходят preflight без I/O при недопустимых контрактах.
5. Даже если domain operation чистая, её invocation в Temporal может вести
   journal, ждать HITL или читать PG; это отдельные **runtime effects**,
   требующие owner-routing и тестов replay. Не использовать «pure» на уровне
   Activity только потому, что алгоритм внутри pure.

**Статус:** это проверенная source-инвентаризация спорных границ, **не**
закрытие первой checkbox #94. Полный owner/contract decision ledger,
фактическая независимая компиляция, полноценное Temporal DAG execution и
provider safety ещё не приняты; PR #95 остаётся Draft, main — без merge.

Первый отдельный PG-owned business effect в V3 разбирается по SQL,
admission/retry, security и source-reachability в
[`ISSUE94_TRUTH_REVIEW_EFFECT_OWNER_V3.md`](ISSUE94_TRUTH_REVIEW_EFFECT_OWNER_V3.md).
Он не переопределяет общий счёт candidate operations и не снимает
требование физического compile/runtime доказательства.
