# Issue #94 — source-based candidate decision ledger

Baseline: Draft PR #95 at `851c3fa`, reconciled with canonical [Issue #94 §6.3](https://github.com/iamaman11/alegria-site/issues/94).
The full current runtime Activity inventory is generated **directly from Rust** by
`python3 automation/audit_operation_inventory.py --format json`. The legacy
22 phases are generated separately from `SeoPhaseKey` and never become the
normative operation set.

This is an **audit ledger, not a runtime registry**. Decision = proposed
KEEP/MERGE/SPLIT/INTERNAL classification; it does not mean an independent typed
operation was implemented or accepted. Owner is a rule owner or the specific
remaining mixed boundary. Effects include the current Activity/transport
effects, not an authorization grant. `TBD` remains deliberately unresolved.
Reachability denotes source call-site family, not deployed runtime observation.
Paths are repository-relative and checked by the audit script.

## Приёмка по доказательствам (текущий срез Draft PR #95)

В этой таблице **52 предварительных кандидата** (29 KEEP, 12 SPLIT,
11 INTERNAL), но **не 52 принятые самостоятельные операции**. Полный
машинно-читаемый source-audit, включая отдельную строку и степень проверки
для каждого кандидата:

```bash
python3 automation/audit_operation_inventory.py --format summary
python3 automation/audit_operation_inventory.py --format readiness
python3 automation/audit_operation_inventory.py --format json
```

Нельзя отождествлять число кандидатов с 73 Temporal Activity endpoints,
22 историческими `SeoPhaseKey` или пятью *уже установленными* pure-контрактами
`semantic.*`. Их исходные Rust `Operation`-контракты привязаны к
`seo_application::operation_adapter::semantic_extraction_registry`; пять
отдельных one-node DAG unit tests проверяют прямой typed-вызов каждого
контракта и отказ на чужой scope/неверный payload/schema. Это **проверка
пути по исходникам и unit-тесты**, не подтверждённый production Temporal E2E.

`ACQ-01` и `ACQ-02` дополнительно выделены в `acquisition::operations` как
строгие типизированные *owner-only* pure-контракты: DTO не изменяют
десериализацию исторических Temporal V1 payload. **Они не включены** в
`semantic_extraction_registry`, Site Lab или пользовательский Temporal
исполнитель и потому не прибавляются к пяти установленным операциям.
Для `EX-03` исполнена только pure-часть SPLIT, не внешний Qdrant enrichment.

| Доказательство | Уровень, который оно подтверждает | Текущий статус |
| --- | --- | --- |
| Строка матрицы, исходный owner и классификация | Source inventory | 52/52 записаны; финальные границы не утверждены |
| Rust typed `Input`/`Output`/`Failure`, versioned `Operation::contract` | Source-level owner contract; **компиляция требует CI** | 5 `semantic.*` + 2 owner-only ACQ; не все 52 |
| Доверенная регистрация и однозначный исполнитель | Source-level installed/dispatch | 5 `semantic.*`; ACQ — нет |
| Независимый one-node вызов и отрицательные случаи | Unit tests + после прогона CI проверка на SHA | Новые тесты для пяти pure; не подтверждает эффектные owner-гейты |
| Native Temporal History, PG/Outbox owner receipt, replay, failures, tenant isolation | Physical E2E **на конкретном SHA** | 0 из 52 атрибутированы как fully-accepted |
| Режимы production: RLS, provider budgets, rollback, Build-ID/drain | Production readiness | Не утверждено |

Даже завершение unit-тестов пяти pure-операций **не закрывает** кандидатов
с внешними эффектами, не подтверждает каждый `SPLIT` целиком, не разрешает
использовать fixture-only DataForSEO как live-provider proof. Полных
приёмок по §6.3 — **0**, ожидают окончательной приёмки **52**. Скрипт
сознательно использует `not_verified` там, где нет надёжной положительной
атрибуции, а не утверждает, что нужный код отсутствует.

Выявленные отрицательные критерии на следующую приёмку: в новом срезе
`TR-01` заменяет `unwrap()` при соединении section-id на типизированный
`DomainError::ContractViolation`; положительный и два отрицательных теста
требуют CI-подтверждения на новом SHA. `PL-01` pure-модель выдаёт только *предварительное* число SERP
snapshots, а фактический PG/provider receipt принадлежит application;
`ED-05` PII prepass сейчас встроен в сборку черновика, а `CMS-01a`/`CMS-02`
различаются `publish_mode` одного обработчика. `PUB-01` и `PUB-03` пока
совмещают PG/FS/CDN эффекты; их отдельный эффектный admission не доказан.

## Почему Protobuf и JSON существуют одновременно

**Не было миграции «с Proto на JSON».** Legacy-путь хранит/передаёт
`alegria.temporal.v1` protobuf DTO из `app/contracts/proto/temporal_payloads.proto`
через `prost`; рядом уже исторически используются JSON DTO для части
ACQ/EX/TR (в `proto_runtime_payload_store/registered_payload_impls.rs`
статически зарегистрированы **43 protobuf и 24 JSON** типа). Даже старый
`StepEnvelope` имел строковый type tag и непрозрачные `bytes`, а некоторые
протокольные сообщения содержат `sections_json` или `extra_json_utf8`.

Новый динамический DAG использует **один provider-free Rust-протокол**
`operation_protocol::Operation<Input, Output, Failure>` и проверяемую
`OperationContract`/pinned fingerprint; формат JSON применяется для
редактируемого Site Lab authoring и конкретных сериализованных данных.
Проверку конкретных DTO выполняет владелец, а не неограниченный словарь
внутри orchestration. Site Lab remote owner требует отдельного
`ValidateOnly`; JSON schema от автора сама по себе не даёт права запуска.

**Остающийся P0:** `OperationContract::fingerprint()` включает ID/revisions,
порты, владельца и effects, но **не полный структурный descriptor полей
Rust Input/Output/Failure**. Изменение DTO без ревизии может оставить прежний
hash. На текущей V2 History-схеме нельзя добавить descriptor in-place,
поскольку это изменит исторические pinned fingerprints. Требуется новая
версионированная owner schema attestation с проверкой replay в отдельном
cutover. Проверки raw duplicate JSON и typed round-trip в pure-исполнителе
защищают от молча проигнорированных параметров **сейчас**, но не заменяют
полного структурного descriptor. Исходные boundary regressions:
`python3 automation/test_operation_wire_boundary.py`; эти source-checks
не доказывают E2E всех 52 кандидатов.

Verified static reachability at the baseline: 55 Activity methods referenced
from canonical V1, 14 additional expert-only methods, two other operational
workflows, two test-only methods; V2 source-only reuses five of the existing
methods (not five new business operations). New generic pure-DAG Workflow
does not turn any of these technical methods into a required fixed site step.

| Candidate | Decision | Owner (actual/proposed) | Effects and boundaries | Reachability | Source |
| --- | --- | --- | --- | --- | --- |
| ACQ-01 | KEEP | acquisition | PURE classifier; Temporal journal | expert+V1 | app/rust/crates/acquisition/src/source_relevance.rs |
| ACQ-02 | KEEP | acquisition | PURE relevance; Temporal journal and PG sweep read | expert+V1 | app/rust/crates/acquisition/src/source_relevance.rs |
| ACQ-03 | INTERNAL | acquisition | PURE extraction prelude, not site step | application-internal | app/rust/crates/acquisition/src/source_relevance.rs |
| ACQ-04a | INTERNAL | acquisition | PURE HTML metadata parsing | application-internal | app/rust/crates/acquisition/src/html_sections.rs |
| ACQ-04b | KEEP | acquisition | PURE sectioning; Activity PG input/journal | V1 | app/rust/crates/acquisition/src/html_sections.rs |
| ACQ-05 | KEEP | acquisition | PURE PDF parser; external file acquisition separate | application-internal | app/rust/crates/acquisition/src/pdf_parser.rs |
| ACQ-06 | INTERNAL | acquisition | PURE robots/retry policy; HTTP execution separate | application-internal | app/rust/crates/acquisition/src/crawl_policy.rs |
| ACQ-07 | SPLIT | acquisition + PG runtime | HTTP crawl, raw PG, optional Qdrant projection; fence TBD | V1+CLI | app/rust/services/cli_tools/src/cli/static_site.rs |
| EX-01 | KEEP | seo_domain extraction | PURE route; PG journal in Activity | expert+V1 | app/rust/crates/seo_domain/src/extraction/layer_routing.rs |
| EX-02 | KEEP | seo_domain extraction | PURE spans; PG journal in Activity | expert+V1 | app/rust/crates/seo_domain/src/extraction/entity_spans.rs |
| EX-03 | SPLIT | seo_domain canonical mapping + knowledge vector port | PURE alias mapping versus Qdrant lookup; optional enrichment not Truth authority | expert+V1+pure-DAG | app/rust/crates/seo_domain/src/canonical_mapping.rs;app/rust/crates/seo_application/src/knowledge.rs |
| EX-04 | KEEP | seo_steps procedural rule | PURE extraction; Activity PG journal | expert+V1 | app/rust/crates/seo_steps/src/procedural_extraction_step.rs |
| EX-05 | KEEP | seo_domain extraction | PURE operational facts; Activity PG journal | expert+V1+pure-DAG | app/rust/crates/seo_domain/src/extraction/operational_entities.rs |
| EX-06 | KEEP | seo_domain extraction | PURE topics; Activity PG journal | expert+V1+pure-DAG | app/rust/crates/seo_domain/src/extraction/editorial_topics.rs |
| EX-07 | SPLIT | seo_domain TBD: SEO signal + commercial signal | Two distinct inline rules, PG section read and Activity journal; owner extraction pending | V1 | app/rust/services/temporal/src/activities/operations/truth_extraction_ops/layered_extraction.rs |
| EX-08a | INTERNAL | seo_steps ontology gate | PURE validation; Activity journal | V1 | app/rust/services/temporal/src/activities/operations/truth_extraction_ops/layered_extraction.rs |
| EX-08b | INTERNAL | seo_steps contract gate | PURE schema check; Activity journal | V1 | app/rust/services/temporal/src/activities/operations/truth_extraction_ops/candidate_validation.rs |
| EX-08c | INTERNAL | acquisition + seo_steps gate | PURE CAS and sectioning checks; PG journal | V1 | app/rust/services/temporal/src/activities/operations/acquisition_section_ops.rs |
| TR-01 | KEEP | truth_domain + seo_steps | PURE triple synthesis; Activity journal | expert+V1 | app/rust/services/temporal/src/activities/operations/truth_extraction_ops/triple_builder.rs |
| TR-02 | KEEP | truth_domain | PURE completeness decision; Activity journal | expert+V1 | app/rust/crates/truth_domain/src/completeness.rs |
| TR-03 | KEEP | truth_domain | PURE contradiction decision; Activity journal | expert+V1 | app/rust/crates/truth_domain/src/review_decisions.rs |
| TR-04a | INTERNAL | truth_domain | PURE HITL routing policy; PG task is separate | expert | app/rust/crates/truth_domain/src/hitl_decision.rs |
| TR-04b | KEEP | truth_domain | PURE conflict resolution; Activity journal | V1 | app/rust/crates/truth_domain/src/review_decisions.rs |
| TR-05 | KEEP | truth_domain + Temporal candidate adapter | PURE candidate/quantity validation; PG manual queue read | V1 | app/rust/services/temporal/src/activities/operations/truth_extraction_ops/candidate_validation.rs |
| TR-06 | KEEP | truth_domain | PURE governance/adjudication; PG journal | V1 | app/rust/crates/truth_domain/src/semantic_truth_write.rs |
| TR-07 | KEEP | truth_domain policy + PG transaction owner | PostgreSQL atomic Truth/HITL/Outbox/step journal; COMMIT fence is mandatory | V1 | app/rust/crates/infrastructure/src/adapters/sqlx_truth_write_adapter.rs |
| TR-08 | SPLIT | truth_domain + PG manual queue + Temporal wait | Manual task persisted in PG, wakeup signal, typed human decision | V1+CLI | app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover/manual_truth_gate.rs;app/rust/services/cli_tools/src/cli/manual_llm.rs |
| TR-09 | INTERNAL | seo_domain page readiness | PURE verified support admissibility; Activity journal | V1 | app/rust/crates/seo_domain/src/page_readiness.rs |
| KN-01a | INTERNAL | seo_application knowledge | PG source-context read; retrieval cache is not Truth | V1+application | app/rust/crates/seo_application/src/knowledge.rs |
| KN-01b | INTERNAL | seo_application knowledge | Qdrant semantic read/rerank; no authoritative mutation | V1+application | app/rust/crates/seo_application/src/knowledge.rs |
| KN-02a | SPLIT | seo_domain mapping + seo_application knowledge | Optional Qdrant read, retains HITL; not merged with pure canonical mapping | V1+application | app/rust/crates/seo_application/src/knowledge.rs |
| KN-02b | INTERNAL | vector/embedding adapter | Voyage embedding preparation; external I/O | V1+application | app/rust/crates/runtime_composition/src/adapters/semantic_search_adapter.rs |
| KN-02c | KEEP | seo_application knowledge | Verified PostgreSQL rule read; PURE verified-block rendering | V1+application | app/rust/crates/seo_application/src/knowledge.rs |
| PL-01 | SPLIT | seo_application SERP + PG owner | External SERP provider read and PG persist separated | V1+semi-auto | app/rust/crates/seo_steps/src/serp_ingest_step.rs |
| PL-02 | KEEP | seo_steps SERP | PURE normalize; PG journal in Activity | V1+semi-auto | app/rust/crates/seo_steps/src/serp_normalize_step.rs |
| PL-03 | KEEP | seo_steps opportunity | PURE discovery; PG persistence separately | V1+semi-auto | app/rust/crates/seo_steps/src/opportunity_build_step.rs |
| PL-04 | KEEP | seo_domain IA + seo_steps | PURE page/blueprint planning; PG persistence separately | V1+semi-auto | app/rust/crates/seo_steps/src/ia_build_step.rs |
| PL-05 | KEEP | seo_domain link policy | PURE scoped page-link rules; PG/Neo4j projection separately | V1+semi-auto | app/rust/crates/seo_domain/src/link_recommendation.rs |
| PL-06 | SPLIT | seo_steps reconcile policy + PG atomic owner | Global site plan and navigation, one PG transaction + Outbox | V1+semi-auto | app/rust/crates/seo_steps/src/global_site_reconcile_step.rs |
| ED-01 | KEEP | seo_steps editorial | PURE draft assembly; input references verified Truth | V1+semi-auto | app/rust/crates/seo_steps/src/draft_assemble_step.rs |
| ED-02 | SPLIT | seo_application editorial + LLM/PG manual queue | External LLM draft and optional manual human submission | V1+CLI | app/rust/services/cli_tools/src/cli/manual_llm.rs |
| ED-03 | KEEP | seo_steps editorial | PURE normalization; PG journal | V1+semi-auto | app/rust/crates/seo_steps/src/draft_normalize_step.rs |
| ED-04a | KEEP | seo_domain content contract | PURE content validation; PG journal | V1+semi-auto | app/rust/crates/seo_domain/src/content_contract.rs |
| ED-04b | KEEP | seo_domain draft QA | PURE QA policy; PG journal | V1+semi-auto | app/rust/crates/seo_domain/src/draft_quality.rs |
| ED-05 | SPLIT | seo_steps PII sanitizer + seo_domain page readiness | PII sanitization and editorial admissibility are distinct safety policies | V1+semi-auto | app/rust/crates/seo_steps/src/pii_redaction_prepass_step.rs;app/rust/crates/seo_domain/src/page_readiness.rs |
| CMS-01a | KEEP | seo_application CMS review | PG task creation, scoped immutable CMS revision | V1+semi-auto | app/rust/crates/seo_application/src/cms_review.rs |
| CMS-01b | SPLIT | seo_domain CMS decision + PG/Temporal HITL | Human approve/block/reopen; PG durable state plus Temporal wake | V1+CLI | app/rust/crates/seo_application/src/cms_review.rs |
| CMS-02 | KEEP | seo_application review publish + PG owner | Exact approved revision selection; PG transaction | V1+semi-auto | app/rust/crates/seo_application/src/review_publish.rs |
| PUB-01 | SPLIT | seo_domain publishing + infrastructure renderer | PG read and candidate static filesystem materialization; no delivery authorization | V1+CLI | app/rust/crates/seo_application/src/review_publish.rs |
| PUB-02 | KEEP | seo_domain render preview | PURE output validation; selected readback separate | V1+semi-auto | app/rust/crates/seo_domain/src/render_preview.rs |
| PUB-03 | SPLIT | seo_domain publish policy + PG/FS/CDN owners | Exact CMS freshness fence then remote delivery/readback/rollback | V1+semi-auto | app/rust/crates/seo_application/src/review_publish.rs |
| PUB-04 | KEEP | seo_domain rebuild | PURE affected-pages decision; PG readiness and scheduling separate | V1+semi-auto | app/rust/crates/seo_domain/src/rebuild.rs |

## Explicitly unresolved; no architecture-freeze claim

- Detailed versioned input/output/error contracts and causal artifact
  schemas for all effectful entries have **not** been extracted from Temporal DTOs.
- The choice to KEEP a candidate is **not** proof that it is the final
  business operation; source facts and new research may merge or split it.
- Non-Temporal entrypoints E01–E18 are a separate technical map in
  [ISSUE94_OPERATION_BOUNDARY_DECISIONS.md](ISSUE94_OPERATION_BOUNDARY_DECISIONS.md),
  particularly the direct ontology Qdrant/Neo4j and advisory auto-seed writes.
- `normalize_run_mode` currently maps unknown values to
  `publish_with_hitl` in legacy application code; **generic admission must
  reject unknown run modes before any effect**.
- Three arbitrary real-effect Temporal DAG compositions, provider-side effect
  admission, historical replay and four owner-only rebuild results remain open.
