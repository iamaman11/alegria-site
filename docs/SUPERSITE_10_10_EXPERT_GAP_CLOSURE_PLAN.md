# Supersite 10/10 — Release Execution Contract

> Superseded by [V6_Expert_Truth_Graph_Runtime.md](V6_Expert_Truth_Graph_Runtime.md). Retained for historical execution log, migration context, and evidence trail. Current architecture and execution ownership now live in `V6`.

Status: superseded execution reference
Class: `superseded-reference`
Owner: Alegria SEO site build runtime
Anchor docs: [`SEO_SUPERSITE_10_10_EXECUTION_PLAN.md`](SEO_SUPERSITE_10_10_EXECUTION_PLAN.md), [`V5_Runtime_Contract.md`](V5_Runtime_Contract.md), [`OPS_RUNTIME_RUNBOOK.md`](OPS_RUNTIME_RUNBOOK.md), [`OPS_TEMPORAL_BUILD_MODES.md`](OPS_TEMPORAL_BUILD_MODES.md), [`OPS_TEMPORAL_PRODUCTION_GATE.md`](OPS_TEMPORAL_PRODUCTION_GATE.md)
Last updated: 2026-05-15

---

## 0. Historical Status Snapshot

### Реальный статус на 2026-05-15
- `R0` closed for current environment: additive `run_mode`, compatibility default, current-run `system.sync_outbox.run_id` barrier, quarantine для legacy content workflow, replay-evidence artifact и environment probe уже закреплены в коде и automation. Для local business DB введён canonical credential source `infra/local/dev_db.env`; на системном PostgreSQL `5432` provisioned dedicated role/database `alegria_local` / `alegria`, а `automation/capture_seo_legacy_replay_inventory.py` теперь даёт `PASS` с `total_runs=0`.
- `R1` closed for current environment, phase-exit candidate: `integration_harness` already contains Postgres/Qdrant/Neo4j/Temporal container harness, unified runtime env surface с `DATABASE_URL`, `QDRANT_URL`, `NEO4J_*`, `TEMPORAL_URL`, compile gate `cargo test -p integration_harness --features e2e --no-run` зелёный, full runtime `cargo test -p integration_harness --features e2e` зелёный после починки local Docker daemon и WSL credential-helper drift, а canonical synthetic E2E test `synthetic_full_scenario_persists_page_draft_on_full_harness` уже проходит через local HTML crawl source, SERP stub, SQLx runtime repo и persistence в `site.page_drafts`.
- `Step 4` closed in current local environment: `cargo test -p integration_harness --features e2e cli_vs_temporal_semantic_parity_on_full_harness -- --nocapture` зелёный, semantic parity surface нормализован до business outputs и blocking decisions, а non-semantic run-specific refs исключены из comparison surface.
- `Step 5` reopened in current local environment: baseline drift старого docker Postgres volume уже снят через canonical helpers `automation/check_local_runtime_db_baseline.py` и `automation/bootstrap_local_runtime_baseline.py`, fresh baseline-ready DB подтверждена, а truth extraction readiness теперь фиксируется отдельным preflight `automation/check_truth_extraction_provider_ready.py`; текущий live run на fresh baseline DB всё ещё в `FAIL` из-за `blocked:no_truth_extraction_provider`, и это уже чистый внешний blocker на `GEMINI_API_KEY|GOOGLE_API_KEY`, а не DB/runtime drift.
- `R2` closed for current environment, phase-exit candidate: crawler invariants, provenance, dedup alias layer, SERP top10 normalization и trust-tier classification already have green contract/smoke coverage.
- `R3` reopened and marked redesign-required: ранее собранные provenance/publish gates остаются полезными, но больше не считаются достаточным phase exit, пока новый candidate/adjudication path не доведён до полного admissible-only truth contract. Уже добавлены strict extraction contract checks, deterministic validator foundation и explicit adjudication contract checks, но live progression всё ещё блокируется отсутствием configured Gemini truth provider.
- `R4` и `R5` переводятся в provisional state: уже собранные rebuild/observability/certification artifacts сохраняются как foundation, но formal progression дальше `R3` приостановлен до завершения redesign truth pipeline.
- **Execution mode now:** strict sequential execution. Начиная с этого состояния, новые работы берутся только по порядку `R0 -> R1 -> R2 -> R3 -> R4 -> R5`; запуск следующего блока до formal exit предыдущего запрещён, кроме прямо разрешённых stabilization fixes внутри уже открытого блока.

### Как читать дальнейшие стадии
- Если ниже написано `Phase exit`, это целевое состояние стадии, а не утверждение, что оно уже достигнуто.
- Любой частично реализованный пункт остаётся blocker, пока не появился machine-verifiable evidence path.

---

## 1. Purpose And Use

Этот документ не дублирует runtime-spec, domain-spec или code inventory. Его роль — задать **release blockers**, порядок снятия рисков, acceptance gates, compatibility/rollout policy и certification ladder для доведения SEO supersite до статуса `10/10`.

Этот файл используется как **GO/NO-GO contract**. Если реализация, automation или live evidence расходятся с этим документом, приоритет остаётся у live code, live schema, proto contracts и automation согласно `docs/INDEX.md`.

Формат документа — **dependency-driven release program**. Здесь нет ложной календарной точности: перечислены только hard dependencies, critical path и machine-verifiable gates. Потенциальная параллельность не считается разрешением на параллельное исполнение: текущий режим этого документа — serial execution with explicit stop/go boundaries.

---

## 2. Current Baseline That Matters

### 2.1 Что уже считается рабочим baseline
- `SeoSiteBuildWorkflow` является production SEO orchestration path.
- `seo_application::scenario` уже позволяет исполнять ту же бизнес-цепочку вне Temporal, но CLI-path не сертифицирован как production-equivalent.
- DataForSEO ingest, raw crawl persistence, verified rule persistence, Neo4j/Qdrant projection, draft assembly, QA, CMS request/review, static materialization и rebuild detection уже существуют как code path.
- HITL pause/resume уже есть в runtime; SEO-path умеет блокировать publish на review step и candidate/adjudication loop, а legacy standalone extraction workflow больше не является текущим truth-path.
- `seo-preflight` уже проверяет runtime dependencies и projection status.
- `automation/ci_verify.sh` уже держит structural gates, но этого недостаточно как release evidence.

### 2.2 Что остаётся release blocker
- Run modes и rollout semantics не оформлены как совместимый public contract.
- Projection barrier остаётся глобальным вместо current-run scoped behavior.
- Crawler ещё не production-grade по robots, concurrency, retry/backoff, redirect trace и safe dedup.
- SERP intelligence остаётся query-led вместо top10-pattern-led.
- Extraction coverage недостаточна для visa publish safety.
- Competitor-only facts, deterministic editorial fallback, PII redaction и licensing не оформлены как единый publish safety contract.
- Rebuild engine и global navigation policy не доведены до deterministic site evolution.
- Нет регулярного production gate с restore drill и live certification evidence.

### 2.3 Термины этого документа
- `rule_type`: тип factual rule для extraction/verification. Не используется для page templates или QA thresholds.
- `page_type`: тип страницы и её template/section contract.
- `quality policy`: отдельная конфигурация порогов качества по `page_type` и locale.
- `phase exit`: минимальный порог для перехода к следующей стадии.
- `release gate`: порог для допуска в production-like path.
- `10/10 DoD`: финальный сертификационный уровень.

---

## 3. Non-Negotiable Release Blockers

До статуса `10/10` должны одновременно быть закрыты следующие блокеры:

1. **Compatibility first.**
   Любое изменение workflow branch/order/signal semantics, payload contract или outbox filtering не вводится без rollout policy, compatibility window и replay-safe proof.
2. **Real-path evidence before deep hardening.**
   Synthetic green недостаточен. После test harness обязателен минимальный live-provider smoke до дальнейшего масштабного hardening.
3. **Per-source provenance must survive optimization.**
   Dedup, retrieval, page assembly и rebuild не могут уничтожать идентичность исходного URL/source observation.
4. **Publish safety beats throughput.**
   Любой внешний LLM path, deterministic fallback, competitor-only truth, unsupported claim, low-trust evidence, blocked license или unresolved contradiction должны удерживать publish.
5. **Observability is a release feature.**
   Run report, projection barrier status, rebuild queue, HITL backlog и restore drill evidence входят в release contract, а не считаются вторичной ops-задачей.

---

## 4. Release Program

### 4.1 Стадии и зависимости

```text
R0 Safety And Compatibility Foundation
  -> R1 Executable Test And Real-Path Harness
  -> R2 Source Acquisition Hardening
  -> R3 Truth Extraction And Publish Safety
  -> R4 Site Evolution And Runtime Observability
  -> R5 Production Gate And Certification
```

Hard dependencies:
- `R1` не стартует без решений из `R0`.
- `R2` не стартует без исполняемого harness из `R1`.
- `R3` зависит от `R1` и `R2`.
- `R4` зависит от `R3`; отдельные задачи observability могут идти параллельно, если не обходят publish safety.
- `R5` зависит от зелёных release gates из `R1`–`R4`.

Critical path:
- `R0 run_mode + workflow rollout`
- `R1 executable harness + thin live smoke`
- `R2 production crawler + top10 normalization`
- `R3 extraction coverage + publish safety`
- `R4 rebuild engine + deterministic nav policy`
- `R5 production gate + live certification`

### 4.2 Строгая последовательность исполнения от текущего состояния

Ниже зафиксирована **обязательная** последовательность исполнения. Это не recommendation, а execution order. Любая работа вне текущего активного шага считается отклонением от плана.

#### Step 1 — Close `R0` Compatibility Perimeter

**Scope now:**
- Довести replay-safe certification для уже изменённого workflow contract.
- Довести current-run `run_id` stamping coverage для всего publish-critical perimeter.
- Доказать, что legacy payload/default behavior и старые workflow histories не ломаются на текущем contract surface.

**Must finish before moving on:**
- `automation/check_run_modes_contract.py` зелёный.
- `automation/check_seo_site_build_replay_contract.py` зелёный.
- `automation/check_seo_rollout_compat_contract.py` зелёный.
- `automation/check_projection_barrier_run_scoped.py` зелёный.
- `automation/check_run_id_event_stamping.py` зелёный.
- `automation/check_legacy_workflow_quarantine.py` зелёный.
- `automation/check_seo_legacy_replay_evidence_schema.py` зелёный.
- `automation/check_seo_legacy_replay_inventory_contract.py` зелёный.
- `automation/check_seo_legacy_replay_environment_probe.py` зелёный.
- `automation/check_temporal_workflow_execution_plan.py` зелёный.
- Compile/test контур для `seo_application`, `infrastructure`, `temporal_worker`, `cli_tools` зелёный.

**Forbidden before Step 1 exit:**
- Не добавлять новые SEO runtime branches/modes beyond already introduced `run_mode`.
- Не расширять R2/R3 logic.
- Не вводить новые publish-affecting producers без explicit `run_id` contract.

#### Step 2 — Finish `R1` Synthetic Infra Harness

**Scope now:**
- Добавить в harness недостающий `Temporal` component.
- Собрать полный multi-container infra surface `Postgres + Qdrant + Neo4j + Temporal`.
- Закрепить unified runtime env contract для всего synthetic runtime path.

**Must finish before moving on:**
- `integration_harness` содержит container harness для всех four declared runtime dependencies.
- `cargo test -p integration_harness --features e2e --no-run` зелёный.
- `automation/check_integration_harness_present.py` подтверждает full infra set, а не только partial foundation.

**Forbidden before Step 2 exit:**
- Не писать R2 crawler hardening.
- Не писать new extraction coverage logic.
- Не считать `R1` закрытым по одному лишь provider-stub path.

#### Step 3 — Build One End-to-End Synthetic Scenario

**Scope now:**
- На full infra harness поднять один synthetic E2E scenario.
- Scenario должен пройти planning -> acquisition persistence -> projection barrier -> drafting persistence без live providers.
- Вся конфигурация должна подниматься из harness env surface, без ручного локального glue code вне harness.

**Must finish before moving on:**
- Один deterministic synthetic scenario исполняется end-to-end.
- Scenario сохраняет machine-verifiable artifact или test evidence.
- Failure clearly identifies failing layer: provider stub, persistence, projection, workflow, or publish gate.

**Forbidden before Step 3 exit:**
- Не начинать real-provider smoke.
- Не начинать R2 feature expansion.
- Не раздувать suite до multi-scenario набора, пока нет одного зелёного canonical scenario.

#### Step 4 — Certify CLI vs Temporal Semantic Parity

**Scope now:**
- На fixture/synthetic scope доказать semantic parity между CLI-path и Temporal-path.
- Сравнение делать только по business outputs и blocking decisions.

**Must finish before moving on:**
- `docs/runs/cli_temporal_semantic_parity_evidence.json` имеет `PASS`.
- Зафиксированы allowed differences: timestamps, executor ids, step ledger representation, HITL transport.
- Нет unexplained divergence по `verified.rule_instances`, `site.page_drafts`, publish-intent decisions.

**Forbidden before Step 4 exit:**
- Не трактовать CLI-path как operator-safe equivalent.
- Не использовать parity как excuse для byte-for-byte persistence matching.

#### Step 5 — Run Thin Live-Provider Smoke

**Scope now:**
- Minimal live path with real credentials:
  - one real scope;
  - one real SERP fetch;
  - one official crawl;
  - one extraction pass;
  - no publish.

**Must finish before moving on:**
- Существует immutable artifact live smoke.
- Failure mode классифицирован: provider contract, crawl policy, extraction mismatch, or rate/credential issue.
- `R1` после этого можно считать phase-exit candidate.

**Current status:** reopened. Baseline drift закрыт как operational blocker, но current live artifact всё ещё красный и на fresh baseline DB подтверждает тот же внешний blocker: `raw_knowledge_ingestion=blocked:no_truth_extraction_provider`. Canonical preflight теперь отдельный: `automation/check_truth_extraction_provider_ready.py`.

**Forbidden before Step 5 exit:**
- Не стартовать `R2`.
- Не объявлять harness sufficient for production-like work.

#### Step 6 — Start `R2` Only After `R1` Exit

После закрытия Steps 1–5 разрешён переход к `R2`. Внутри `R2` порядок тоже фиксирован:
1. crawler invariants;
2. per-URL provenance safety;
3. dedup as optimization only;
4. SERP top10 normalization;
5. trust-tier classification.

Запрещено начинать extraction/publish-safety work из `R3`, пока `R2` не прошёл phase exit.

#### Step 7 — Start `R3` Only After `R2` Exit

Внутри `R3` порядок фиксирован:
1. provenance invariants;
2. required `rule_type` coverage;
3. contradiction handling;
4. PII redaction pre-pass;
5. licensing gate;
6. deterministic-fallback publish blocking;
7. HITL operator surface.

#### Step 8 — Start `R4` Only After `R3` Exit

Внутри `R4` порядок фиксирован:
1. rebuild dependency graph;
2. rebuild queue and execution path;
3. global navigation policy;
4. quality policy registry and scoring contract;
5. run report and metrics surface.

#### Step 9 — Start `R5` Only After `R4` Exit

Внутри `R5` порядок фиксирован:
1. production gate wiring;
2. restore drill;
3. nightly stability;
4. three live certification scenarios;
5. final `GO/NO-GO`.

### 4.3 Current Active Step

На дату этого обновления активным шагом считается **Step 7 — Start `R3` Only After `R2` Exit**.

`Step 1`, `Step 2`, `Step 3`, `Step 4` и `Step 6` в текущем локальном окружении закрыты. `Step 5` reopened due local DB contract drift. `Step 7` reopened: `R3` должен быть доведён до нового evidence-grade truth contract до любого дальнейшего formal progression в `R4` и `R5`.

---

## 5. Release Stages R0–R5

### R0 — Safety And Compatibility Foundation

**Current status:** in progress. Базовый кодовый контракт уже введён, но automation/evidence layer ещё не доведён до phase-exit уровня.

**Цель:** оформить public execution contract так, чтобы дальнейшие изменения можно было выкатывать без повреждения durable histories, legacy payloads и старых outbox rows.

**Почему это blocker:** без `R0` любой успех следующих стадий может оказаться unreplayable, non-drainable или несовместимым с уже идущими workflow runs.

**Hard deps:** none.

**Implementation decisions:**
- `SeoSiteBuildInputPayload` получает `run_mode` как additive поле.
- Поддерживаемые значения `run_mode`: `dry_run`, `crawl_only`, `draft_only`, `publish_with_hitl`, `full_auto_after_approval`.
- Legacy payload без `run_mode` интерпретируется как `publish_with_hitl`. Это compatibility default, а не implicit new behavior.
- `run_mode` маппится на orchestration gating внутри workflow/scenario path; hardcoded `Full` удаляется из release path.
- Любое изменение workflow branch/order/signal semantics требует либо нового workflow type, либо формального replay-safe proof с прохождением replay checks. Новый `WORKER_BUILD_ID` сам по себе недостаточен.
- `system.sync_outbox` получает `run_id` как current-run identity. Legacy rows без current-run identity не участвуют в current-run barrier и обрабатываются только legacy/global path.
- Вводится compatibility window для outbox migration: dual-write там, где новый код ещё должен обслуживать старые consumers; dual-read там, где barrier должен различать legacy и current-run semantics.
- Старые worker build-id не снимаются, пока не дренированы старые histories.
- Unsafe switches управляются feature flags/env toggles; включение строгих режимов допускается только после прохождения release gates.

**Phase exit:**
- `automation/check_run_modes_contract.py` подтверждает наличие `run_mode` и supported enum surface.
- `automation/check_seo_site_build_replay_contract.py` подтверждает compatibility default, load-before-branch order и normalized `run_mode` branching для `SeoSiteBuildWorkflow`.
- `automation/check_seo_rollout_compat_contract.py` подтверждает, что legacy default, build-id drain policy, workflow-type discipline и `run_id` barrier semantics одновременно закреплены в code+ops contract.
- `automation/check_legacy_workflow_quarantine.py` подтверждает, что `ContentGenerationWorkflow` остаётся только legacy-only path, gated env-флагом и исключён из production gate.
- `automation/check_seo_legacy_replay_evidence_schema.py` подтверждает существование обязательного replay-evidence artifact schema для `SeoSiteBuildWorkflow`, даже если live replay evidence пока ещё не captured.
- `automation/check_seo_legacy_replay_inventory_contract.py` подтверждает, что replay-evidence artifact уже содержит обязательные `inventory` и `environment_probe` sections для будущего доказательства `runs exist` vs `runs absent` и для фиксации canonical DB access verdict.
- `automation/check_seo_legacy_replay_environment_probe.py` подтверждает, что artifact реально содержит verdict по всем canonical endpoints `5433`, `6432`, `5432`, а listening endpoint не остаётся в состоянии `not_attempted`.
- `automation/check_temporal_workflow_execution_plan.py` и replay checks зелёные для совместимых workflow changes.
- `automation/check_projection_barrier_run_scoped.py` подтверждает, что current-run barrier не смешивает legacy/global rows с rows текущего `run_id`.

**Rollback / compat notes:**
- Rollback возвращает старый worker fleet/build-id; rollback никогда не правит workflow history.
- Любая миграция payload/outbox должна быть additive-first; destructive cleanup допускается только после drain и aftercare verification.
- Уже сделано в коде: additive `run_mode`, compatibility default `publish_with_hitl`, current-run barrier read path, outbox `run_id` schema foundation.
- Ещё не доказано: что все producers, влияющие на SEO publish barrier, stamp `run_id` консистентно за пределами уже проверенного critical path, и что старые workflow histories replay-safe на новом contract surface; текущий evidence path уже доказывает только состояние окружения, но не отсутствие histories и не replay success.

### R1 — Executable Test And Real-Path Harness

**Current status:** phase-exit candidate in current environment. В репозитории уже есть отдельный `integration_harness` crate, synthetic stub path, env-override surface для provider adapters, unified runtime env surface для Postgres/Qdrant/Neo4j/Temporal, adapter-level synthetic tests, multi-container infra path и live-provider smoke evidence; формальный exit остаётся предметом release review, но локальный gating path закрыт.

**Цель:** заменить бумажный green на исполняемую систему доказательств, где отдельно видны synthetic correctness и live-provider viability.

**Почему это blocker:** без `R1` последующие стадии невозможно отличить от документов, которые прошли только grep-based gates.

**Hard deps:** `R0`.

**Implementation decisions:**
- Добавляется integration harness на testcontainers для PostgreSQL, Qdrant, Neo4j и Temporal.
- Boundary stubs для DataForSEO и LLM providers остаются допустимыми только внутри harness; release tests не подменяют ими live-provider smoke.
- После появления harness обязателен **thin real-path smoke**:
  - один реальный scope;
  - один реальный SERP fetch;
  - один crawl официального источника;
  - один extraction pass;
  - без publish.
- Synthetic green и real-provider green учитываются раздельно. Synthetic pass не может заменить live smoke.
- CLI-path и Temporal-path сравниваются по **semantic parity business outputs**, а не по bit-for-bit идентичности ledger/timestamps/executor IDs/HITL mechanics.

**Phase exit:**
- `cargo test -p integration_harness --features e2e` зелёный локально и в CI.
- `automation/check_integration_harness_present.py` подтверждает crate, fixtures, stub servers и отсутствие несанкционированных сетевых вызовов в harness path.
- `automation/smoke_real_provider_minimal_scope.py` проходит на реальных credentials и сохраняет immutable artifact.
- `automation/smoke_cli_temporal_parity.py` подтверждает semantic parity по `verified.rule_instances`, `site.page_drafts` и publish-intent outcomes на fixture scope.

**Rollback / compat notes:**
- Failure live-provider smoke блокирует старт `R2`, но не отменяет synthetic harness.
- Если live-provider smoke нестабилен, сначала фиксируется provider contract или fixture strategy, а не ослабляется release gate.
- Уже сделано в коде: scaffold `integration_harness`, базовый `IntegrationCtx`, JSON stub servers для DataForSEO/OpenAI-compatible path, env-override helpers для existing adapters, synthetic test на реальные DataForSEO/local-compatible editorial contracts, unified runtime env surface для Postgres/Qdrant/Neo4j, `e2e` feature с container harness foundation и schema bootstrap, CI hooks в `automation/ci_verify.sh`, а также canonical Step 5 smoke/evidence pair `automation/smoke_real_provider_minimal_scope.py` + `docs/runs/live_provider_minimal_scope_evidence.json`.

### R2 — Source Acquisition Hardening

**Цель:** сделать acquisition path пригодным для production scoring и extraction без потери provenance.

**Почему это blocker:** все downstream guarantees зависят от качества source acquisition; неверные crawl/SERP assumptions делают extraction, rebuild и publish формально корректными, но фактически ложными.

**Hard deps:** `R1`.

**Current status:** phase-exit candidate in current local environment; crawler invariants, provenance, dedup alias layer, robots-policy smoke и SERP top10 normalization smoke already pass.

**Implementation decisions:**
- Crawler получает robots policy, crawl delay, per-domain concurrency, retry/backoff, redirect chain и canonical trace.
- Каждая URL-fetch observation сохраняется как самостоятельный source observation.
- Dedup допускается только как optimization layer:
  - сохраняется исходный URL, status, timestamp, redirect path, canonical hints;
  - content dedup создаёт linkage/alias record, но не заменяет source identity и не уничтожает provenance.
- Crawler persistence contract обязан хранить:
  - redirect chain;
  - robots decision trace;
  - per-URL observation;
  - dedup linkage без provenance collapse.
- SERP intelligence перестраивается на top10-pattern-led model.
- Domain tier classification отделяет `official`, `vfs`, `agency`, `editorial`, `forum`, `low_trust`.
- Low-trust SERP/domain signals не могут становиться factual support сами по себе.

**Phase exit:**
- `automation/check_crawler_invariants.py` подтверждает robots, retry/backoff, per-domain concurrency и redirect trace.
- `automation/smoke_crawler_robots_policy.py` показывает, что disallow source не попадает в publish-support path.
- `automation/check_serp_intelligence_contract.py` подтверждает top10 normalization, tier classification и evidence links для generated page targets.
- `automation/smoke_serp_top10_pattern_pipeline.py` подтверждает, что IA/page blueprint outputs объясняются SERP patterns, а не только query text.

**Rollback / compat notes:**
- До завершения `R2` нельзя вводить aggressive dedup, который изменяет source-of-record shape.
- JS-render fallback можно временно держать behind feature flag, но robots and provenance contracts не флагуются.

### R3 — Truth Extraction And Publish Safety

**Current status:** redesign required. Предыдущий локальный `phase-exit candidate` больше не считается достаточным, пока candidate storage, adjudication и admissibility gate не стали единственным truth-path в runtime. Текущее состояние лучше, чем на старте redesign: strict LLM extraction contract уже проверяется automation, candidate persistence теперь детерминированно выставляет `epistemic_status` (`structured` / `needs_hitl` / `rejected`), а runtime уже содержит current-run adjudication writer path, который группирует persisted candidates по semantic identity и пишет/демотит `verified.rule_instances`. Следующий live blocker остаётся прежним: нет configured Gemini truth provider для повторного Step 5 smoke.

**Цель:** закрыть factual coverage и собрать единый publish safety contract для verified truth, HITL, LLM use и licensing.

**Почему это blocker:** именно здесь решается, можно ли выпускать visa content без ручного SQL, скрытых fallback paths и нелегального внешнего использования данных.

**Hard deps:** `R1`, `R2`.

**Implementation decisions:**
- `rule_type` registry ограничен factual extraction semantics; thresholds QA туда не складываются.
- Required `rule_type` coverage оформляется как publish contract; `coverage_gap` допустим на phase exit только как диагностический результат, но не как final DoD.
- Каждая verified rule обязана иметь source provenance.
- `source_type` и `trust_level` используются только как adjudication signals. Ни `government`, ни `vfs`, ни любой другой tier не дают shortcut verified verdict.
- `facts_extractor.rs` полностью удаляется из truth-path и из репозитория. Regex/keyword extraction не допускается как способ создания verified truth.
- Внешний LLM path разрешён только как extractor/normalizer, но не как authority. LLM не пишет в `verified.rule_instances` напрямую и не присваивает `verified`.
- Любой claim без `evidence_section_id`, `evidence_quote`, `span_start`, `span_end`, `source_snapshot_hash` и валидного `context_key` не может перейти в `verified`.
- Verified truth присваивается только после schema validation, canonical resolution, cross-source adjudication, contradiction gate, freshness/completeness checks и, где нужно, HITL resolution.
- Publish запрещён для source/license combinations с `redistribution_allowed=false`.
- Если editorial path использовал `DeterministicEditorialClient`, итоговый CMS revision маркируется `editorial_provider="deterministic_fallback"`, а `expert_content` publish блокируется.
- Добавляется operator surface для HITL без прямого SQL: list/show/approve/reject/request-changes/registry-extension.
- Contradiction handling обязана блокировать publish для unresolved fee/timeline/document/location/eligibility conflicts.
- Quality policy хранится в отдельном registry/config по `page_type` и locale. `kb.rule_types` для этого не используется.
- До `draft_assemble` добавляется обязательный `truth_admissibility_gate`: page generation читает только `status='verified'` и `publish_admissibility='admissible'`.

**Sequential redesign steps:**
1. `R3.1 Delete Regex Extractor`
   - Полностью удалить `app/rust/crates/primitives/src/facts_extractor.rs` и `app/rust/crates/primitives/src/facts_extractor_json.rs`.
   - Удалить их export из `primitives/src/lib.rs`.
   - Удалить все truth-path вызовы из `raw_crawl_adapter.rs`, `fact_extraction.rs`, `proto_runtime_payload_store.rs` и связанных tooling checks.
   - Запретить resurrection этого пути через automation contract.
2. `R3.2 Stop Raw Auto-Verify`
   - `raw_knowledge_ingestion` больше не пишет `verified` и не использует source-tier shortcut verdicts.
   - Raw ingestion заканчивается записью только в `extracted.*` candidate storage.
3. `R3.3 Introduce Candidate Storage`
   - Добавить `extracted.rule_candidates` как canonical pre-verified layer.
   - Минимальные поля: `candidate_id`, `context_key`, `claim_type`, `role_type`, `concept_key_candidate`, `normalized_payload`, `source_key`, `raw_section_id`, `evidence_quote`, `span_start`, `span_end`, `source_snapshot_hash`, `llm_provider`, `llm_model`, `extraction_confidence`, `epistemic_status`, `created_at`.
4. `R3.4 Add LLM Extraction Contract`
   - Новый extraction step вызывает LLM и требует strict JSON-only output.
   - Каждый candidate обязан содержать exact evidence span, quote, modality/conditions/temporality flags и uncertainty markers.
   - LLM output без complete evidence payload считается invalid и не проходит дальше.
   - Текущее состояние: закрыто на code+test foundation; malformed candidate drop подтверждён unit test в `truth_extraction_llm_adapter`, а runtime persistence проверяется synthetic e2e path без direct verified writes.
5. `R3.5 Add Deterministic Validators`
   - Schema validator, canonical resolver, params completeness validator, temporality/freshness validator, evidence-span validator.
   - После этих проверок candidate может стать только `structured`, `needs_hitl` или `rejected`, но не `verified`.
   - Текущее состояние: runtime validator path уже пишет `epistemic_status` в `extracted.rule_candidates`, а synthetic e2e tests подтверждают `structured`, `needs_hitl` и `rejected` outcomes.
6. `R3.6 Add Truth Adjudication`
   - Cross-source adjudicator решает corroboration, contradiction, completeness, source-weighting и stale handling.
   - Выходы: `verified`, `needs_hitl`, `rejected`.
   - Текущее состояние: runtime adjudication writer path уже materializes/demotes `verified.rule_instances`, а synthetic e2e tests подтверждают `corroborated -> verified/admissible` и `conflict -> needs_hitl/no admissible verified row`.
7. `R3.7 Harden Verified Storage`
   - Расширить `verified.rule_instances` полями `evidence_section_id`, `evidence_quote`, `span_start`, `span_end`, `source_snapshot_hash`, `verification_method`, `adjudication_reason`, `publish_admissibility`, `freshness_class`, `completeness_class`, `review_decision_id`.
   - Все pre-storage invariants из `V5_Ultimate_Extraction_Protocol.md` должны быть материализованы в схеме и writer guards, а не только задекларированы в docs.
8. `R3.8 Add Truth Admissibility Gate`
   - `draft_assemble`, `draft_qa` и page-generation path читают только admissible verified truth.
   - Unknown provenance, unresolved contradiction, missing evidence, stale evidence или incomplete params блокируют page generation ещё до draft creation.

**Phase exit:**
- `automation/check_extraction_runtime_contract.py` подтверждает, что `facts_extractor` удалён, regex extraction path отсутствует, а raw ingestion не имеет source-tier shortcut verdicts.
- `automation/check_extracted_rule_candidates_schema.py` подтверждает shape нового `extracted.rule_candidates`.
- `automation/check_llm_extraction_contract.py` подтверждает strict JSON output contract, evidence span requirements и запрет direct verified writes.
- `automation/check_truth_adjudication_contract.py` подтверждает source-tier-is-signal-only policy, contradiction handling, freshness/completeness checks и HITL branching.
- `automation/check_verified_storage_invariants.py` подтверждает наличие evidence-grade полей и pre-storage guards в `verified.rule_instances`.
- `automation/check_truth_admissibility_gate.py` подтверждает, что page generation читает только admissible verified truth.
- `cargo test -p integration_harness --features e2e truth_runtime_promotes_corroborated_candidates_into_admissible_verified_truth`
- `cargo test -p integration_harness --features e2e truth_runtime_marks_incomplete_candidate_as_needs_hitl`
- `cargo test -p integration_harness --features e2e truth_runtime_keeps_conflicting_structured_candidates_out_of_verified_storage`
- `automation/check_pii_redaction_pre_llm.py` подтверждает обязательный pre-pass перед внешним LLM.
- `automation/check_seo_publish_gates.py` подтверждает deterministic-fallback blocking, unsupported-claim blocking и licensing gate.
- `automation/smoke_hitl_resolution_loop.py` подтверждает операторский approve/reject path без прямого SQL.

**Rollback / compat notes:**
- До завершения `R3` publish допускается только в режимах, которые не обходят новые blocking reasons.
- Если quality policy ещё калибруется, thresholds можно держать conservative, но registry shape уже должен быть отделён от `rule_type` registry.
- Legacy standalone extraction workflow уже не считается runtime-опцией и не должен возвращаться ни в worker registration, ни в ops gates, ни в docs canon.

### R4 — Site Evolution And Runtime Observability

**Current status:** provisional only. `automation/temporal_production_gate.sh`, `infra/backups/restore_drill.sh`, `automation/e2e_supersite_certification.sh` and nightly wrapper are wired, but formal progression into `R4` remains blocked until `R3` finishes the truth-pipeline redesign and exposes admissible verified truth as the only page-build input.

**Цель:** сделать развитие сайта и эксплуатацию детерминированными: факт меняется один раз, а последствия для страниц, меню, отчётности и backlog объяснимы машинно.

**Почему это blocker:** без `R4` проект может публиковать отдельные страницы, но не управлять supersite как живой системой.

**Hard deps:** `R3`.

**Implementation decisions:**
- Добавляется `site.rebuild_jobs` и workflow/runtime path, который explainably пересобирает affected pages.
- Rebuild graph должен связывать `page_node_key` как минимум с `rule_instance`, `source_key`, `blueprint`, `link recommendation` и navigation state.
- Navigation policy становится deterministic global contract:
  - multi-country/multi-visa grouping;
  - no menu explosion;
  - long-tail pages через directory path;
  - stable redirect/deprecation behavior.
- Draft quality scoring оформляется как publish gate по `page_type` templates и quality policy registry.
- Run report является обязательным artifact каждого execution run.
- Observability contract включает SQL views, Prometheus metrics и Grafana dashboards, которые позволяют принимать решение без чтения сырых логов.

**Phase exit:**
- `automation/check_rebuild_engine_contract.py` и `automation/smoke_rebuild_propagation.py` подтверждают enqueue и обработку rebuild jobs после factual change.
- `automation/check_global_nav_policy_contract.py` и `automation/smoke_global_nav_expansion.py` подтверждают deterministic nav evolution.
- `automation/check_quality_policy_registry.py` и `automation/smoke_quality_policy_registry.py` подтверждают page-type templates, quality policy lookup и blocking behavior ниже threshold.
- `automation/check_run_report_completeness.py` и `automation/check_metrics_contract.py` подтверждают run report, SQL views и metrics surface.

**Rollback / compat notes:**
- Rebuild и nav policy можно выкатывать incremental mode, но redirect/deprecation contract не должен быть partially enabled.
- Run report schema можно расширять additive-only; удаление полей допустимо только после обновления всех automation consumers.

### R5 — Production Gate And Certification

**Current status:** provisional only. Certification scaffolding already exists, but no final `R5` evidence can be trusted as release-grade until `R3` and then `R4` complete under the redesigned truth contract.

**Цель:** перевести все предыдущие гарантии в регулярный release process с повторяемым live evidence.

**Почему это blocker:** без `R5` проект остаётся набором локально успешных подсистем, а не production-ready supersite runtime.

**Hard deps:** `R1`, `R2`, `R3`, `R4`.

**Implementation decisions:**
- `automation/temporal_production_gate.sh` и `infra/backups/restore_drill.sh` входят в регулярный gate до финальной сертификации, а не только в конец программы.
- Вводится nightly workflow для production gate, artifact upload и restore verification.
- Финальная сертификация фиксируется immutable evidence в `docs/runs/`.
- Финальная сертификация состоит минимум из трёх live scenarios:
  - fresh scope;
  - scope expansion;
  - factual change with rebuild propagation.

**Phase exit:**
- Nightly production gate зелёный семь запусков подряд.
- `automation/e2e_supersite_certification.sh` создаёт три passing certification artifacts.
- `automation/check_end_to_end_invariants.py` зелёный после restore drill и certification runs.

**Rollback / compat notes:**
- Если nightly gate нестабилен, финальный `GO` невозможен даже при passing локальных smoke.
- Certification artifacts immutable; ретроактивное редактирование run evidence запрещено.

---

## 6. Compatibility And Rollout Rules

### 6.1 Workflow and worker rollout
- Новый `WORKER_BUILD_ID` обязателен для каждого worker deployment.
- Изменения activity-only или replay-safe internal fixes могут идти в рамках существующего workflow type.
- Изменения branch/order/signal semantics требуют нового workflow type или формально доказанного replay-safe behavior.
- Старые build-id остаются в обслуживании до drain всех старых histories.

### 6.2 Payload and proto evolution
- Изменения payload contracts — additive-first.
- Legacy payloads без `run_mode` трактуются как `publish_with_hitl`.
- Любое breaking payload change допускается только вместе с compatibility window и явным sunset plan.

### 6.3 Outbox rollout
- `system.sync_outbox.run_id` используется только как current-run barrier key.
- Legacy/global rows не участвуют в current-run barrier и не могут ложно блокировать current-run publish.
- Dual-write/dual-read держатся до завершения migration window и прохождения replay/projection checks.

### 6.4 CLI vs Temporal parity
- Canonical production orchestration path — Temporal.
- CLI-path допускается как operator/batch path только после semantic parity certification.
- Semantic parity измеряется business outputs и blocking decisions; step ledger, timestamps, executor IDs и HITL transport semantics не обязаны совпадать bit-for-bit.

### 6.5 Safety defaults
- Unknown or missing trust/provenance/license state блокирует publish.
- Deterministic editorial fallback никогда не повышается до expert publish.
- Unknown quality policy для `page_type`/locale блокирует publish, а не использует silent fallback.

---

## 7. Acceptance Model

### 7.1 Phase Exit

Phase exit означает только, что стадия достаточно закрыта для старта следующей:
- разрешены явно задокументированные diagnostic gaps;
- все gates machine-verifiable;
- rollback/compat path определён;
- результаты стадии закреплены automation или immutable evidence.

### 7.2 Release Gate

Release gate для production-like path считается зелёным только если одновременно:
- `automation/ci_verify.sh` проходит;
- integration harness зелёный;
- thin real-provider smoke зелёный;
- current-run projection barrier проходит;
- publish safety gates проходят;
- run report и metrics surface доступны;
- `automation/temporal_production_gate.sh` и `restore_drill.sh` проходят на актуальном runtime.

### 7.3 Final 10/10 DoD

`10/10` считается достигнутым только если одновременно выполнены все условия:

| ID | Условие | Доказательство |
|---|---|---|
| D1 | Fresh truth tuple создаёт `kb.visa_contexts.context_key` без ручного SQL | `seo-preflight --bootstrap-truth-identity`; smoke `smoke_fresh_scope_bootstrap.py` |
| D2 | Fresh publishing tuple создаёт `scope_signature` независимо от truth identity | smoke + `seo_flow_tests` assertion |
| D3 | SERP top10 нормализован в pattern-led model с trust tier classification | `check_serp_intelligence_contract.py`; `smoke_serp_top10_pattern_pipeline.py` |
| D4 | Crawler соблюдает robots/concurrency/retry/redirect trace и не теряет per-URL provenance | `check_crawler_invariants.py`; `smoke_crawler_robots_policy.py` |
| D5 | Required `rule_type` coverage закрыта без blocking gaps | `check_extraction_coverage.py`; per-type smoke suite |
| D6 | Каждая verified rule имеет provenance; competitor-only facts не auto-publish | `check_seo_provenance_invariants.py` |
| D7 | Projection barrier scoped по `run_id` и не смешивает legacy/global semantics | `check_projection_barrier_run_scoped.py`; behavioral smoke |
| D8 | Global navigation policy deterministic при расширении scope | `check_global_nav_policy_contract.py`; `smoke_global_nav_expansion.py` |
| D9 | Publish safety contract блокирует unsupported claims, blocked licenses и deterministic fallback expert publish | `check_seo_publish_gates.py`; `smoke_publish_blocked_without_llm.py`; `smoke_draft_qa_scoring.py` |
| D10 | Factual/source change создаёт explainable rebuild job и приводит к новой revision | `smoke_rebuild_propagation.py` |
| D11 | Run report выдаёт decision-grade summary без чтения логов | `check_run_report_completeness.py` |
| D12 | Live certification scenarios выполнены и сохранены как immutable evidence | `automation/e2e_supersite_certification.sh` |

Любой `D# = fail` означает `NO-GO 10/10`.

---

## 8. Certification Scenarios

Минимальный certification set:

1. **S1 Fresh Scope**
   - Fresh scope `PL tourist`.
   - Реальные DataForSEO credentials.
   - Один production LLM provider.
   - Артефакт: `docs/runs/supersite_e2e_PL_tourist_<date>.md`.

2. **S2 Scope Expansion**
   - Расширение `PL work` после S1.
   - Проверяются navigation diff, sitemap diff и affected rebuilds.
   - Артефакт: `docs/runs/supersite_e2e_PL_work_expansion_<date>.md`.

3. **S3 Factual Change And Rebuild**
   - Инъекция change event для fee или другого blocking factual field.
   - Проверяются rebuild enqueue, rebuild explainability и новая CMS revision.
   - Артефакт: `docs/runs/supersite_e2e_rebuild_<date>.md`.

### 8.1 Certification artifact contract
- Артефакты в `docs/runs/` являются immutable evidence.
- Каждый artifact обязан содержать:
  - exact scenario id;
  - run ids / workflow ids;
  - worker build ids;
  - provider set;
  - summary run report;
  - publish/rebuild outcomes;
  - explicit PASS/FAIL per `D1`–`D12`;
  - ссылки на attached logs/histories/dumps при падении.
- Redacted secrets only; provider keys и raw sensitive inputs запрещены.

---

## 9. Documentation Governance

- Этот документ не ведёт open-ended backlog. Если решение обязательно для реализации, оно должно быть зафиксировано здесь как project decision, а не оставлено в `Open questions`.
- Cross-cutting safety rules не выносятся в “фоновый” раздел, если они могут блокировать legal, compliant или replay-safe execution.
- Любая новая automation/test/smoke, на которую ссылается этот документ, должна быть либо реализована, либо явно отмечена как release blocker до следующего phase exit.
- `docs/INDEX.md` должен ссылаться на этот файл только как superseded historical reference.
- Архивирование superseded drafts допускается только в `docs/_archive/` и никогда не заменяет immutable certification artifacts.

---

## 10. Final GO/NO-GO Rule

Финальный `GO` возможен только если одновременно:
- `R0`–`R5` закрыты по phase exit;
- release gate зелёный;
- `D1`–`D12` passing;
- nightly production gate стабилен;
- certification artifacts сохранены как immutable evidence;
- нет открытых compatibility, provenance, compliance, publish safety или rebuild blockers.

Во всех остальных случаях решение — `NO-GO`.
