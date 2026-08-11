# Alegria Production 10/10 repository classification

Status: architectural audit snapshot
Audit branch: `agent/production-10-10-hardening`
Audit starting head: `1604d7b09d6246a31fbdf0b124fe2ce81f2d2652`
Parent plan: `docs/PRODUCTION_10_10_PLAN.md`

## 1. Audit objective

This map classifies the current repository by its real responsibility relative to the target product: Alegria must compile a newly selected niche into an evidence-backed domain contract and then run a generic supersite runtime without rewriting generic core code.

The classification is based on current code paths, not on the existence of files, health checks, projections, tests, or documentation claims. A capability counts only if its output reaches a real product decision and can be certified on real data.

Classification labels used below:

- **generic core** — reusable domain-independent runtime behavior worth preserving;
- **visa-specific** — correct only as the current vertical implementation;
- **should move to Domain Pack** — declarative vertical semantics that must leave generic code;
- **should belong to Niche Compiler** — semantics that must be inferred/compiled from niche evidence rather than hand-programmed;
- **support-plane** — operations, transport, observability, projection, or tooling that supports but does not itself make the supersite decision;
- **legacy/compat** — retained only for replay/migration/compatibility;
- **duplicated** — useful responsibility implemented on a parallel path that canonical runtime does not consume;
- **unused/decorative** — presence/readiness does not yet prove product effect;
- **wrong abstraction** — responsibility is implemented behind a misleading or overly vertical boundary;
- **missing capability** — required by the target product but absent as a first-class runtime capability.

## 2. Repository purpose map

| Component | Current responsibility | Intended supersite responsibility | Does current code really perform it / where output goes? | Classification | Main problem | Target place | Action | Priority | Acceptance proof |
|---|---|---|---|---|---|---|---|---|---|
| `crates/primitives` | hashes, keys, common low-level values | stable domain-independent primitives | yes; consumed broadly | generic core | some SEO/context naming can still inherit vertical assumptions through callers | generic core | keep, tighten APIs around generic identities | P1 | same crate works unchanged for three compiled domain packs |
| `crates/contracts` generated Temporal/read APIs | wire contracts for workflows, SEO scope, CMS, graph/planning | pack-neutral workflow/artifact contracts | executes, but current payload family exposes country/visa/profile semantics | wrong abstraction + visa-specific | vertical fields are part of orchestration boundary | generic contracts + versioned Domain Pack references | rewrite incrementally; keep transport mechanics | P0 | canonical workflow accepts `domain_pack_ref` + generic context/applicability payload without visa fields |
| `crates/runtime_models::RuleRoleType/RuleParams` | fixed rule/fact parameter families | generic typed fact/rule representation interpreted by pack schema | real persistence/extraction consumers use it | should move to Domain Pack | fixed `Document/Fee/Timeline/WhereToApply/...` enum makes shared model a visa/procedure schema | Domain Pack fact/rule schema + generic typed value envelope | split/rewrite | P0 | product/comparison pack defines specs/prices/alternatives without adding Rust enum variants |
| `crates/runtime_models::seo_blocks` | fixed section roles, headings, fact-role routing, block types | generic content-plan interpreter | directly drives draft section/block selection | visa-specific + should move to Domain Pack | `documents/fees/timing/where_to_apply` and English headings are hard-coded | Domain Pack content archetypes | move semantics; keep generic planner | P0 | changing pack changes sections/blocks/headings without code change |
| `crates/policies` | condition evaluation, conflict/truth/quality guards | generic policy engine plus pack-defined policy data | core mechanics are reusable | generic core with Domain Pack inputs | vertical schemas/roles can leak into policy callers | generic policy engine + Domain Pack policy declarations | keep engine, move declarative semantics | P0/P1 | same evaluator enforces materially different applicability/risk policies from pack data |
| `crates/seo_domain::identity` | derives truth identity and SEO scope | generic domain/audience/page/presentation identity model | real registration and persistence path uses it | wrong abstraction + visa-specific | fixed `country_code/visa_family/visa_subtype/citizenship_code`, fixed applicant enum, scope omits material truth dimensions | generic identity core + visa Domain Pack identity schema | rewrite | P0 | collision tests prove truth/audience/page/locale identities are explicit and pack-driven |
| `crates/seo_domain::applicability` | applicant-profile inclusion/exclusion/exception resolution | generic N-dimensional applicability resolution | real verified-support read path uses it | wrong abstraction + visa-specific | one fixed profile key cannot model residence/jurisdiction or arbitrary niche dimensions | generic condition/applicability engine + pack dimensions | rewrite | P0 | residence/jurisdiction and non-visa dimensions resolve via the same engine |
| `crates/seo_ports` | application ports | stable domain-independent dependency inversion | real application layer depends on ports | generic skeleton + wrong vertical contracts | registration/support payloads carry visa identity; `GraphReasoningPort` includes SQL/Qdrant behavior under graph naming | generic ports split by reasoning plane | keep skeleton, rewrite contracts and plane boundaries | P0 | type-level boundary prevents retrieval implementation from satisfying graph reasoning |
| `crates/seo_application::execution/scenario/registration` | scenario planning/orchestration semantics | generic run policy and scenario result semantics | used by CLI/Temporal | generic core with vertical input leakage | success model is too coarse; registration accepts visa fields | generic execution core + Domain Pack/context refs | keep/rewrite result and input contracts | P0 | publish scenario cannot return success with zero/partial unintended publication |
| `crates/seo_application::planning` | injects graph context and retrieval enrichment into opportunity/IA/link/reconcile | domain-pack-driven planning reasoning | yes, some graph/retrieval output changes clusters/conflicts/links | generic core candidate + incomplete reasoning | graph context is mostly SQL-derived; retrieval collections hard-coded; final IA remains visa-coded | generic planning interpreter consuming versioned graph/retrieval reason packages | rewrite/extend | P0 | graph/retrieval ablation changes certified IA/link/coverage decisions |
| `crates/seo_steps::opportunity_build_step` | SERP-pattern clustering and graph coverage gaps | generic demand/topic opportunity compiler | yes; graph topic signals can collapse clusters and coverage signals create gaps | generic algorithm candidate | token-overlap heuristic and input graph are not equivalent to required GDS community reasoning | generic planner + Domain Pack clustering policy | keep as fallback/feature, add canonical graph algorithms | P0 | real niche scope stores algorithm/version/reason and ablation changes cluster output |
| `crates/seo_steps::ia_build_step` | builds hubs, leaves, page types, URLs, sections | pack-driven site graph/IA construction | definitely changes final site | visa-specific + should move to Domain Pack | hard-coded `/visa`, country slug map, visa hubs/silos, keyword substring page types, fixed section roles | Domain Pack page/URL/IA archetypes; Niche Compiler derives candidate archetypes | rewrite as pack interpreter | P0 | three different packs create structurally different IA with unchanged Rust core |
| extraction steps (`procedural`, `operational`, `editorial`, canonical mapping, ontology gates) | extract/route evidence into candidates | generic evidence pipeline executing compiled schemas | substantial real pipeline exists | generic framework + vertical schema leakage | ontology/entity/rule schemas are not first-class compiled domain artifacts | generic extraction engine + Domain Pack schemas/prompts/validators | keep engine, externalize schemas | P0 | new niche extraction works from compiled schema without Rust enum/schema edits |
| `draft_assemble_step` / drafting application | builds section templates, claim ledger, content blocks, LLM request/fallback | generic evidence-bound content assembly | real output feeds CMS path | visa-specific semantics inside useful generic machinery | fallback copy says traveler; section defaults/FAQ/schema/content routing are fixed; internal link recommendation keys can enter draft fallback | generic content assembler + Domain Pack block/schema/CTA policy | move/rewrite semantics, keep traceability machinery | P0 | pack-defined blocks and conversion model produce niche-specific useful pages with claim-level provenance |
| truth traceability / claim ledger | associates generated factual fragments with support refs | universal publication evidence contract | real QA/persistence path consumes it | generic core | support refs do not yet expose complete public citation/effective/freshness model | generic truth/public-citation core | keep and extend | P0 | every public critical claim maps to current admissible evidence and public citation metadata |
| `crates/infrastructure` transport adapters | Postgres, Temporal, HTTP, Neo4j, Qdrant, Voyage, providers, rendering storage | replaceable infrastructure adapters | heavily used | support-plane / mostly generic | policy defaults and collection names leak into adapters; some adapters implement product reasoning rather than transport | infrastructure support-plane behind precise ports | keep, remove policy leakage | P0/P1 | adapters can swap without changing product semantics or reasoning-plane ownership |
| canonical Temporal `SeoSiteBuildCanonicalCutoverWorkflow` | durable end-to-end SEO/truth/site flow | generic durable compiler/runtime orchestration | active production profile; reaches real activities/HITL/CMS | generic orchestration skeleton + wrong sequencing/results | initial support blocks zero-truth bootstrap; per-page block can be skipped; vertical payloads; graph/retrieval reasoning not fully certified | generic runtime workflow consuming Domain Pack | rewrite sequencing/result contracts, preserve durable mechanics | P0 | zero-truth real workflow reaches reviewed candidate; partial block produces non-success run state |
| compat `seo_site_build` | previous workflow | replay/compat only | registered only behind compat flag | legacy/compat | some automation still inspects this path and can protect obsolete assumptions | compat quarantine | retain only while replay evidence requires it | P1 | production profile cannot accidentally invoke it |
| `Expert*` workflows | old extraction/projection paths | migration/replay diagnostics | quarantined behind migration flag | legacy/compat | docs/gates can confuse presence with canonical ownership | migration-only | keep quarantined then remove when replay obligations end | P1 | no production call path; replay manifest defines removal criteria |
| Temporal `FreshnessCheckWorkflow` + `sqlx_freshness_adapter` | reports old unfinished execution runs | active knowledge/source freshness loop | current query measures `pipeline.execution_runs` age, not truth/source TTL | wrong abstraction | operational run-lag monitor is named freshness; required knowledge freshness capability is missing | runtime-health support-plane + new DomainPack-driven knowledge freshness workflow | move/rename existing; implement missing loop | P0 | stale source/rule causes recrawl/revalidation/bounded rebuild/review/withdraw behavior |
| `outbox_worker` | durable projection dispatch/retry | reliable projection support | yes | support-plane / generic core support | must not be mistaken for proof that projections affect decisions | support-plane | keep | P1 | projection loss/retry/DLQ tests plus decision-plane freshness barrier |
| `reconcile` | repairs Neo4j/Qdrant outbox backlog and monitors DLQ/stuck runtime | operational fail-safe support | yes | support-plane | health/reconciliation is not graph/retrieval product reasoning | support-plane | keep | P1 | dependency failures recover without unsafe new publication or loss of last-known-good serving |
| `graph_capability_adapter` | Neo4j query + optional `gds.version()` probe | fail-closed validation of required graph reasoning package | only proves availability | unused/decorative as product proof | capability probe is weaker than algorithm/projection freshness/output readiness | graph runtime preflight based on Domain Pack declared algorithms/projections | rewrite | P0 | preflight validates named graph version + required algorithm outputs, not only plugin version |
| `GraphReasoningPort` SQLx implementation | planning context/link enrichment/conflict/rebuild/draft coverage | canonical graph reasoning plane | mixed: one Neo4j link query, SQL conflict reads, Qdrant draft coverage/rebuild semantics | wrong abstraction | graph, relational dependency lookup, and semantic retrieval are conflated | separate GraphReasoning, DependencyGraph, RetrievalReasoning ports | rewrite | P0 | graph ablation independently changes topology/link/coverage/rebuild decisions |
| `analytics_svc` | GDS WCC/PageRank and Context/Concept link planning | reusable canonical graph algorithm executor | real GDS code exists, but no canonical application port/call-site was found in inspected runtime | duplicated / off-path capability | strongest GDS algorithms live beside rather than inside canonical decision stream; projection is `Context-RELATES_TO`, not current site/truth graph contract | canonical DomainPack-driven GraphReasoning service/library or removal | P0 | canonical run persists WCC/community/centrality reason packages and site decisions consume them |
| Neo4j SEO projection | materializes relational artifacts into graph | versioned graph reasoning source | projection machinery exists and some link data is queried | support-plane + incomplete product use | materialization/readiness can pass without GDS-derived site decisions | graph projection + algorithm result store | keep projection, add product algorithms and freshness identity | P0 | stale/incomplete required projection blocks generation while old release still serves |
| Voyage/Qdrant semantic search | embeddings, vector search, rerank | semantic discovery/recall/clustering/dedup/neighborhood plane | yes in planning/rebuild; not truth authority | useful reasoning plane + wrong fixed configuration | hard-coded collections/models and `label_ru` fallback; scope freshness/readiness not intrinsically enforced | pack-declared retrieval surfaces with projection generation identity | keep engine, externalize surfaces/policy | P0 | real scope data, no synthetic probes, selected retrieval traces alter certified decisions |
| `app/db/schema.sql` KB/truth identity | source of record for concepts, visa contexts, verified rules, SEO/CMS state | generic domain/evidence/truth/site source of record | authoritative runtime store | visa-specific + wrong abstraction | `kb.visa_families`, `kb.visa_contexts`, fixed applicant profiles, role enum, `label_ru`, fixed Qdrant collection allowlist embed vertical architecture in DB | generic domain-pack registry/schema + pack-versioned typed domain records | migrate/rewrite with compatibility views where needed | P0 | non-visa pack can persist truth/applicability without new vertical tables/constraints |
| `app/db/migrations` + migration parity gate | evolve schema and assert key structures exist | safe generic schema/domain-pack migrations | migrations exist; parity check is string-presence structural evidence | support-plane with vertical lock-in | current migration acceptance preserves current schema, not portability/data migration proof | generic schema migrations + explicit visa-pack migration | rewrite incrementally | P0 | clean install + upgrade of existing visa data + second pack migration pass with no data loss/collisions |
| `sqlx_static_site_adapter::load_static_site_snapshot` | published CMS snapshot | strictly public last-known-good snapshot | PR now correctly filters published/published for public state | support-plane/public read model, but incomplete candidate model | same loader is incorrectly reused for candidate build, hiding approved target revision | separate public snapshot and explicit candidate snapshot APIs | rewrite boundary | P0 | approved target + published deps builds preview while target remains absent from public snapshot |
| `static_site_builder_adapter` | Markdown/content-block render, nav, related links, sitemap/robots | secure scalable immutable release renderer | real HTML artifacts are produced | generic renderer skeleton + unsafe/incomplete | raw Markdown HTML unsanitized; JSON-LD script boundary unsafe; navigation includes all pages; incremental path renders whole site first | generic secure renderer consuming persisted topology/revision-owned links | rewrite critical surfaces | P0/P1 | security corpus passes; 1-page rebuild cost bounded at 100k metadata scale |
| render preview gate | checks HTML markers/placeholders | semantic/security/live prepublication validation | executes before finalize | support-plane smoke, insufficient | marker presence does not validate canonical value, XSS, links, schema, accessibility, public identity | release validator | rewrite | P0 | malicious HTML/JSON-LD/link fixtures block; exact canonical/hash/live checks pass |
| link recommendation persistence/public loading | stores candidate/accepted/applied planning links | revision-owned applied internal-link graph | renderer loader currently accepts all three states | wrong state boundary | raw candidate recommendation can become public link | planning state + explicit applied-revision binding | rewrite | P1 | public artifact contains only links recorded as applied to that revision |
| `publish_materialize` + `finalize_publish` | local build then DB status transition | immutable build → staging deploy → live verify → atomic promotion → published | current finalize maps `render_ready` directly to `published` | wrong abstraction / missing deployment capability | no staging deployment, live HTTP/hash verification, atomic promotion, release rollback proof | release/deployment subsystem | rewrite | P0 | DB `published` only after expected public hash is fetched from production origin |
| public serving | serve generated static output | continuously serve last-known-good release independent of generation-plane failures | repository demonstrates local build, not production release serving contract | missing capability | generation availability and serving availability are not explicitly separated by release pointer/promotion mechanism | immutable release store + serving pointer/CDN/origin contract | implement | P0 | Neo4j/Qdrant/provider outage blocks new generation but public old release remains reachable |
| `gsc_sync` | GET GSC sites endpoint connectivity | search/index feedback ingestion | connectivity only | support-plane probe + missing capability | no query/page metrics, index status, or planning feedback | analytics ingestion/decision feedback loop | rewrite | P1 | GSC/analytics delta creates inspectable refresh/merge/split/noindex/rebuild decisions |
| `analytics_svc` as product analytics | graph analytics only | combined search/user/business outcome feedback | does not implement web product analytics/conversions | wrong naming boundary + missing capability | graph analytics and product analytics are conflated by service naming | graph-reasoning service plus separate product analytics pipeline | split | P1 | conversion/search metrics map to page/release/domain intents and influence planner without becoming truth authority |
| metrics/observability | workflow/process counters and runtime health | SLOs plus product outcome observability | useful process metrics exist | support-plane | process completion can look healthy while product outcome is blocked/zero | support-plane with structured product outcome metrics | extend | P0/P1 | dashboards distinguish workflow completion, page blockers, deploy/live verification, stale truth, graph/retrieval readiness |
| backup scripts | local Docker `pg_dump` and restore drill | production DR | useful local acceptance only | support-plane | local plaintext artifact path, no off-host encryption/immutability/PITR/RPO/RTO/release recovery | production backup/DR platform | keep local test, add real DR | P1 | clean environment recovery within measured RPO/RTO including release/config state |
| `automation/ci_verify.sh` and architecture checks | extensive architecture/string/schema/smoke gates | merge governance and product certification support | strong coverage of repository shape | support-plane; some gates encode wrong assumptions | gate count can create false confidence; `smoke_support_bundle_required.py` explicitly requires the empty-support failure; graph schema gate validates report shape rather than product effect | layered test pyramid: architecture gates + real canonical E2E + ablations/scale/security/DR | rewrite selected gates, keep useful static checks | P0/P1 | CI fails on zero-truth bootstrap regression, graph/retrieval ablation, false workflow success, unsafe renderer, broken candidate/public separation |
| Python E2E/product verdict checks | validate certification manifests/reports | canonical Temporal production certification | verifies coarse page/published counts from evidence files | support-plane, incomplete product proof | cannot by itself prove durable HumanApprovalWait, deployment, live hash, source/graph/retrieval realness | canonical Temporal E2E harness | rewrite | P0 | start→review_requested→decision→signal→build→deploy→live verify evidence captured in one immutable manifest |
| documentation set | architecture/spec/working plans | precise ownership and executable acceptance contract | valuable but some statements describe projection availability more strongly than product use | support-plane/documentation drift | older docs can call deeper graph influence deferred while 10/10 target requires it; existence/status wording can be confused with product completion | owner docs aligned to product evidence | update/retire stale claims | P0/P1 | docs status generated/verified against canonical call paths and certification evidence |
| Niche Compiler / Domain Discovery | absent as first-class workflow | infer `DomainPackCandidate` from niche/market/locale/goals and evidence | not implemented | missing capability | without it every new niche still needs engineering | pre-runtime compiler + HITL/domain-expert approval | implement | P0 | niche seed only → evidence-backed candidate → approval → immutable pack |
| immutable/versioned Domain Pack runtime | target described in docs | sole vertical semantics boundary for generic runtime | not implemented as runtime owner | missing capability | current visa semantics are distributed through Rust/SQL/URLs/templates | domain registry/artifact store + generic interpreter | implement | P0 | visa becomes pack 1; two unrelated packs certify without generic-core edits |

## 3. Cross-cutting findings that change priority

### 3.1 Fresh bootstrap is actively prevented by both runtime and CI

The canonical workflow requests initial verified support before discovery/crawl/extraction. The SQL support loader errors when no admissible support exists. In addition, `automation/smoke_support_bundle_required.py` treats the text `verified support bundle is empty` as a required contract. This is not merely missing functionality: the current runtime and a gate jointly preserve the wrong invariant.

Required correction: initial support is optional enrichment; refreshed post-adjudication support is mandatory. CI must certify zero-truth bootstrap rather than the presence of the old error.

### 3.2 Knowledge freshness is not implemented by the workflow named Freshness

`FreshnessCheckWorkflow` delegates to a query over old unfinished `pipeline.execution_runs`. That is runtime-lag monitoring. It does not inspect source TTL, source snapshot age, fact verification timestamps, effective validity, critical truth dependencies, or trigger revalidation/rebuild/withdrawal.

Required correction: move the existing check into runtime-health/reconcile semantics and build a separate DomainPack-driven knowledge freshness loop.

### 3.3 Real GDS code exists, but off the canonical value stream

`analytics_svc` has WCC and PageRank implementations. Canonical planning instead uses the SQLx `GraphReasoningPort`, whose inspected methods mix a limited Neo4j link traversal, relational link/dependency reads, and Qdrant semantic search. Therefore the repository already contains useful graph algorithms, but their existence is currently architectural debt rather than proof of supersite graph reasoning.

Required correction: make the Domain Pack declare required graph algorithms/projections, execute them in the canonical reasoning plane, persist versioned reason packages, and make downstream IA/linking/coverage/rebuild consume them. Add ablation tests.

### 3.4 `GraphReasoningPort` currently violates reasoning-plane separation

A method named `evaluate_draft_coverage_neighborhood` performs Voyage/Qdrant search against `editorial_topics_4`; rebuild semantic widening is also retrieval-driven. These may be useful features, but they are retrieval features and must not be counted as graph usage.

Required correction: separate graph topology reasoning, deterministic dependency lookup, and semantic retrieval interfaces/evidence.

### 3.5 DB verticality is a first-class migration problem

The vertical assumptions are encoded in database primary identities and check constraints, not only UI/templates: `kb.visa_families`, `kb.visa_contexts`, applicant profiles, fixed verified rule roles, Russian labels, and collection allowlists. A Domain Pack cannot be layered over this schema without a migration strategy.

Required correction: introduce pack-neutral domain context/applicability/fact-schema storage and migrate visa data into the first pack with compatibility read models only where necessary during cutover.

### 3.6 Content assembly contains valuable generic machinery wrapped around visa content semantics

Claim ledger, traceability, deterministic blocks, template bindings, QA and HITL are worth preserving. The section ontology and rendering policy are not generic. The correct refactor is not to throw away drafting; it is to turn it into an interpreter of compiled page/content archetypes.

### 3.7 Publication still confuses a renderer artifact with a live release

The published-only public snapshot is directionally correct, but the candidate build has no explicit approved-target read model. The incremental builder renders the whole public snapshot before filtering. Finalization changes DB state to `published` after local render validation, without deploy/live hash/promotion semantics.

Required correction: explicit candidate input, immutable release object, staging deploy, live verification, atomic promotion, previous-release serving/rollback.

### 3.8 Automation strength must be measured by product invariants, not gate count

The repository has an unusually broad architecture/smoke suite. This is useful support infrastructure, but several checks validate file/string/report shape or current assumptions. 10/10 certification must additionally prove counterfactual product behavior: zero-truth bootstrap, graph/retrieval ablations, actual durable HITL, malicious render payloads, dependency failure with last-known-good serving, and bounded rebuild scale.

## 4. Target ownership after refactor

The intended ownership boundary is:

`Niche seed / market / locale / business goal / risk policy`

→ **Niche Compiler / Domain Discovery**

→ evidence-backed `DomainPackCandidate`

→ **HITL/domain expert approval**

→ immutable/versioned **Domain Pack**

→ **generic truth/extraction/planning/content/publication runtime**

with orthogonal support planes:

- PostgreSQL authoritative state;
- Neo4j/GDS graph reasoning;
- Voyage/Qdrant semantic retrieval;
- Temporal durability/HITL;
- outbox/reconcile/projection reliability;
- release/deploy/serving;
- observability/analytics/freshness/DR.

The generic runtime must not know that the first production pack happens to be visas.

## 5. Immediate acceptance sequence

1. Preserve this classification as the migration map and update the parent production plan with the new blockers.
2. Fix fresh-domain bootstrap invariant and its CI contract.
3. Split truth/audience/page/locale identity behind a generic Domain Pack boundary.
4. Separate Graph, Dependency and Retrieval reasoning planes; promote real GDS algorithms into canonical planning with persisted reason packages and ablation tests.
5. Correct candidate/public snapshot and publication state semantics before merging PR #2.
6. Build the Niche Compiler and immutable Domain Pack lifecycle before claiming cross-niche generality.
7. Certify zero-truth niche → real evidence → verified truth → graph/retrieval decisions → useful page → durable HITL → immutable deploy → live verification.

No item is complete until its acceptance proof demonstrates a changed, justified product outcome.