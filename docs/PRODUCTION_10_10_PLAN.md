# Alegria: Production 10/10 supersite execution plan

Status: execution in progress
Owner: repository maintainers
Target: a real public production system that can create, publish, maintain, and improve a high-quality supersite in any selected niche through configuration/domain packs rather than core-code rewrites.

## 0. Mission and audit standard

Alegria is not considered 10/10 merely because the current workflow, contracts, and infrastructure agree with each other.

The audit standard is external to the implementation:

> Given a selected niche, can Alegria discover the domain, build a trustworthy knowledge model, infer a useful information architecture, generate genuinely useful pages, publish them safely, keep them fresh, improve them from real search/user feedback, and repeat the process at large scale without changing the generic core?

Every subsystem must therefore be judged by four questions:

1. Does it materially contribute to the supersite objective?
2. Is its output actually consumed by downstream product decisions?
3. Can its behavior be proven on real data rather than probes/fixtures alone?
4. Can the same core behavior transfer to a materially different niche through a domain configuration contract rather than hard-coded vertical logic?

A subsystem that is installed, health-checked, or projected but does not influence the product is not complete.

## 1. Definition of 10/10

The project is 10/10 only when all of the following are simultaneously true:

1. **Domain universality.** The core is domain-agnostic. Niche-specific entities, relations, fact/rule schemas, applicability dimensions, source-governance rules, page archetypes, content blocks, URL rules, freshness SLAs, graph algorithms, and conversion intents are supplied through a versioned domain pack/registry instead of hard-coded visa concepts.
2. **Real niche bootstrap.** A completely new niche can start with zero pre-existing verified truth, discover authoritative sources, produce admissible knowledge, create an IA, draft pages, pass review, and publish without synthetic truth seeding.
3. **Truth integrity.** Public factual content is backed by admissible evidence, applicability, effective dates, freshness, provenance, and human review where required. Transport-level success with zero usable truth is failure.
4. **Graph/GDS is a real reasoning plane.** Neo4j/GDS is not merely reachable. Required graph algorithms and graph queries materially affect clustering, site topology, hub authority, internal linking, coverage, cannibalization, orphan detection, and rebuild impact. When graph reasoning is declared mandatory for a build, absence or stale/incomplete graph state blocks that build.
5. **Retrieval is a real semantic plane.** Voyage/Qdrant materially support semantic discovery, recall, clustering, cross-page similarity, coverage, and neighborhood expansion while never becoming truth authority.
6. **Public output is launch-safe and useful.** Real HTTPS origin, correct locale, human-readable routes/titles, no internal IDs, valid canonical/sitemap/robots/schema, safe HTML, source citations, useful structured content, accessible responsive UX, and only deployed/live-verified releases are public/indexable.
7. **Publication means live.** `published` means the immutable candidate was validated, deployed, fetched from the public origin, matched to the expected release/content hash, and passed live checks. A local filesystem build is not publication.
8. **Workflow result means product result.** A workflow cannot report normal completion while pages are blocked or zero pages were published in a publish scenario.
9. **Freshness is active.** Expired/stale critical truth cannot silently remain admissible; freshness triggers recrawl, revalidation, rebuild, review, warning/noindex, or withdrawal according to domain policy.
10. **Scales operationally.** Incremental rebuild cost is proportional to impacted pages/dependencies rather than the whole site; navigation and sitemap generation scale to large sites; releases are atomic and rollbackable.
11. **Every merge is governed.** Formatting, linting, architecture, unit/integration, security, dependency, and build gates are enforced by required CI checks.
12. **Production is observable and recoverable.** Classified failures, SLOs, immutable evidence, encrypted off-host backups, restore drills, deployment rollback, and dependency-failure behavior are demonstrated.
13. **Outcome loop is closed.** Indexability, index coverage, search impressions/clicks/CTR/position, content freshness, user engagement, conversion, and content-gap discovery feed back into planning/rebuild decisions.
14. **Cross-niche proof exists.** The same core passes certification in at least three materially different domain packs without modifying generic orchestration/domain-independent crates.

## 2. Critical architectural correction: current system is not yet niche-generic

The current implementation is deeply optimized for the visa vertical. Examples include:

- `kb.visa_contexts`, `visa_family`, `visa_subtype`, `citizenship_code` in runtime identity;
- `country_code` + `visa_type` in SEO scope;
- hard-coded `/visa/...` canonical paths and country slug mapping;
- fixed applicant profiles (`standard`, `minor`, `student`, `family`);
- fixed content roles such as `documents`, `fees`, `timing`, `where_to_apply`;
- rule roles such as `document_required`, `fee_item`, `appointment_rule`, `eligibility_rule`;
- Russian-specific concept-label selection (`label_ru`);
- visa-specific page-type and section heuristics.

This is valid vertical specialization but not a universal supersite contract.

### Required target: versioned Domain Pack

Introduce a first-class domain configuration boundary owned outside the generic engine. A domain pack must define at minimum:

- `domain_key` and version;
- entity types and canonical identity rules;
- relation types and ontology constraints;
- fact/rule types and parameter schemas;
- applicability dimensions and exception semantics;
- source authority classes, independence groups, jurisdiction rules, and freshness TTLs;
- extraction schemas/prompts and deterministic validators;
- page archetypes and section/content-block archetypes;
- intent taxonomy and query/SERP interpretation rules;
- URL/canonical/localization strategy;
- graph projection schema and required GDS algorithms;
- retrieval collections/surfaces and embedding/rerank policy;
- completeness rules for each page archetype;
- conversion/CTA goals and analytics events;
- review/HITL policies;
- freshness and withdrawal policies.

The visa implementation becomes the first domain pack, not the implicit schema of the core.

### Cross-niche acceptance test

Before claiming generic supersite capability, prove the same core against three different packs, for example:

1. a high-freshness rules/procedure niche (visa/travel-regulatory);
2. a product/comparison niche with specifications, alternatives, and commercial intent;
3. a knowledge/service niche with entities, local intent, procedures, and editorial depth.

No generic core code may be changed between these certifications. Only domain pack/configuration, credentials, source seeds, and deployment settings may differ.

## 3. P0-A — Domain identity, applicability, and page identity

### Current problems

- Truth identity contains citizenship but SEO `scope_signature` does not.
- Canonical page identity/URL is therefore capable of collapsing materially different truth contexts onto one page identity.
- Residence/jurisdiction is absent even though many regulated/service niches require location-dependent applicability.
- Applicant profiles are a fixed visa-oriented enum and cannot represent arbitrary condition dimensions.
- The generic core assumes country/visa semantics.

### Required work

- [ ] Split generic `DomainContextIdentity` from visa-specific identity.
- [ ] Make applicability dimensions data-driven rather than a hard-coded profile enum.
- [ ] Add explicit distinction between truth identity, audience/applicability identity, SEO page identity, and presentation locale.
- [ ] Define when multiple truth contexts intentionally share one indexed page and when they require separate canonical pages.
- [ ] For visa pack, model citizenship, legal residence/consular jurisdiction, age/status/family conditions, and subtype explicitly.
- [ ] Make context/page-key collisions impossible by construction and migration-test existing data.
- [ ] Move `/visa/...`, country slug tables, and visa page-type rules into the visa domain pack.

Exit criteria: two distinct applicability contexts cannot accidentally overwrite/share a page unless an explicit aggregation policy says they should.

## 4. P0-B — Fresh-scope / fresh-niche bootstrap

### Current blocker

The canonical workflow loads `verified_support_bundle.initial` before SERP/crawl/extraction and the loader fails when the bundle is empty. Therefore a genuinely new context with no seeded truth cannot reach the stages that would create its first verified truth.

### Required flow

- [ ] Make initial support loading optional/read-only: existing truth may enrich discovery but zero support is valid at bootstrap.
- [ ] Run source discovery/crawl/extraction/adjudication/write without requiring pre-existing truth.
- [ ] Reload support after verified truth write.
- [ ] Make the refreshed support gate mandatory before planning/drafting/publishing.
- [ ] Require domain-pack completeness, applicability, authority coverage, and freshness after extraction.
- [ ] Add a fresh-niche certification starting from zero domain truth and zero page rows.
- [ ] Remove synthetic verified-rule seeds from any evidence that claims fresh-scope production readiness.

Exit criteria: a new domain/context can generate its first trustworthy page from real sources without seeded authority truth.

## 5. P0-C — Graph/GDS as a mandatory reasoning plane, not a health check

### Architectural position

For the intended 10/10 supersite engine, graph reasoning is core functionality. If a build depends on graph reasoning, missing/stale/incomplete Neo4j/GDS must fail closed.

The correction is not to make Graph/GDS optional. The correction is to ensure that the hard dependency is justified by real product use.

`100% functionality` means 100% of the graph capabilities declared necessary by the architecture are executed, consumed, measured, and regression-tested. It does **not** mean using every algorithm shipped by the GDS product.

### Current gaps

- GDS readiness is primarily proven with `gds.version()` capability checks.
- Planning graph context is largely reconstructed from Postgres/step blobs and only part of link context is read from Neo4j.
- `find_conflict_neighborhood` currently reads relational `site.link_recommendations`, not a graph traversal.
- `evaluate_draft_coverage_neighborhood` currently uses Qdrant retrieval rather than graph topology.
- Graph/GDS is currently declared hard-required more strongly than its actual reasoning contribution warrants.
- The older completion plan classifies graph influence on draft/publish/rebuild as post-10/10 optional; that is inconsistent with the supersite objective and must be retired.

### Required graph product responsibilities

- [ ] Domain-pack-specific graph schema with typed nodes/relations and graph integrity constraints.
- [ ] Build per-scope/projected named graphs with explicit version/freshness identity.
- [ ] Use community detection (for example Leiden/Louvain where appropriate) to inform topic clusters and site silos.
- [ ] Use centrality/PageRank-style signals to select hubs, authority pages, navigation prominence, and internal-link priority where the domain model makes that meaningful.
- [ ] Use connected-component/orphan analysis to detect disconnected content/topic regions.
- [ ] Use topology + semantic similarity to detect cannibalization/near-duplicate intent neighborhoods.
- [ ] Use graph path/neighborhood reasoning for required internal links and user-journey coverage.
- [ ] Use truth/topic/page dependency edges for rebuild impact expansion.
- [ ] Use graph coverage to compare required ontology/topic/rule coverage with page coverage before publish.
- [ ] Persist every graph-derived decision with algorithm/version, graph projection version, input scope, score, and reason package.
- [ ] Add ablation tests proving that graph output changes a real planning/linking/coverage/rebuild decision on known fixtures/real scopes.
- [ ] Fail canonical generation/rebuild when the required graph projection or required GDS algorithm output is absent/stale.
- [ ] Keep serving the last known good public release even when Graph/GDS is unavailable; infrastructure failure must block unsafe new publication, not take the already published site offline.

Exit criteria: graph/GDS contributes reproducible, inspectable decisions across IA, linking, coverage, cannibalization, and rebuild; removing it causes certification to fail for the intended reason rather than only failing a version probe.

## 6. P0-D — Retrieval/Voyage as a mandatory semantic plane

- [ ] Separate provider/collection capability smoke from launch-scope data readiness.
- [ ] Exclude synthetic probe records from readiness counts.
- [ ] Require real scope-bound, current projection generations for required collections.
- [ ] Use retrieval for semantic source recall, topic/entity neighborhood discovery, keyword/intent clustering support, candidate deduplication, draft coverage, and rebuild neighbor expansion.
- [ ] Keep retrieval non-authoritative: it may propose/search/rank but never promote truth.
- [ ] Persist retrieval traces including collection/model/version/query/top-k/rerank and selected evidence.
- [ ] Define domain-pack-specific retrieval surfaces rather than a fixed visa-centric collection contract where appropriate.
- [ ] Add retrieval ablation/evaluation datasets tied to product outcomes, not only connectivity.

Exit criteria: required retrieval capability is both live and demonstrably useful to product decisions for the launch scope.

## 7. P0-E — Truth validity, applicability, freshness, and citations

### Current problems

- Verified-support query does not synchronously exclude expired rules by `effective_from/effective_to`.
- `freshness_class` is loaded but stale truth is not automatically made inadmissible at read/publish time.
- `observed_at` is currently populated from `effective_from`, mixing rule effectiveness with source verification time.
- Public support state loses direct source URL/evidence detail needed for user-verifiable citations.
- Concept label selection is Russian-specific (`label_ru`) regardless of target locale.
- Source authority and source applicability/jurisdiction are distinct but not sufficiently enforced for public facts.

### Required work

- [ ] Enforce effective-date validity synchronously when assembling admissible support.
- [ ] Enforce source freshness TTL and critical-fact freshness synchronously at draft/publish gates.
- [ ] Add separate `source_observed_at`, `source_fetched_at`, `fact_verified_at`, `last_revalidated_at`, `effective_from`, `effective_to`.
- [ ] Add public citation model: source URL, label, authority/jurisdiction, evidence pointer/quote, verified-at date.
- [ ] Make label/localization selection locale-aware and domain-pack-driven.
- [ ] Model source jurisdiction/applicability separately from source authority/trust.
- [ ] Make failure of a critical authoritative source blocking when required facts cannot be independently supported.
- [ ] Define stale/expired behavior per domain pack: block rebuild, warning, noindex, withdraw, or require review.

Exit criteria: no public critical fact can be generated from expired, stale, inapplicable, or untraceable support.

## 8. P0-F — Workflow result semantics and failure propagation

### Current blocker

Per-page publish failures can set `page_blocked = true`, skip the page, and still allow the page loop to return `Completed(published_pages)`. The parent workflow can then finalize as done even when publication is partial or zero.

### Required work

- [ ] Return structured page outcomes: requested, planned, drafted, QA-blocked, review-blocked, build-blocked, render-blocked, deployed, live-verified, published.
- [ ] Return explicit run states: `success`, `partial_success`, `blocked`, `failed`, never a generic success for zero published pages in a publish scenario.
- [ ] Persist blockers per page and aggregate failure classes.
- [ ] Make scenario-specific success criteria part of the execution contract.
- [ ] Make `done:no_pages` a valid result only for scenarios where no-page is an expected outcome; it is a launch failure for fresh-scope/page-generation certification.
- [ ] Ensure metrics distinguish process completion from product success.

Exit criteria: orchestration status cannot hide page-level product failure.

## 9. P0-G — Correct HITL and production E2E certification

### Current problems

- `e2e_supersite_certification.py` invokes Cargo from repository root while the workspace manifest lives under `app/rust`.
- The CLI certification path cannot naturally prove the real durable `HumanApprovalWait` lifecycle when interaction policy is `FailIfHitlRequired`.
- Pre-approval cannot represent genuine review because revision identity exists only after review request.
- Current scenario verdicts are still too coarse to prove the intended business invariant.

### Required work

- [ ] Fix manifest/cwd reproducibility for all harness commands.
- [ ] Make canonical Temporal workflow the production E2E subject, not a CLI approximation.
- [ ] Start workflow, wait for `review_requested`, obtain revision ID, record real human/operator decision, signal resume, then continue publish.
- [ ] Keep CLI paths as diagnostics, not the canonical publish certification.
- [ ] `fresh_scope` must prove preexisting truth=0, real crawl>0, verified truth>0, completeness pass, real review, deploy, and live verification.
- [ ] `scope_expansion` must prove intentional delta, stable existing canonicals, and no unexpected cannibalization.
- [ ] `factual_change_rebuild` must prove changed truth, expected dependency impact, rebuild, new revision, and no unrelated rebuild fan-out.
- [ ] Certification evidence must identify all provider/model versions, graph/retrieval projection versions, domain pack version, release ID, and public URL/hash.

Exit criteria: one immutable certification manifest proves the actual production path including HITL and live deployment.

## 10. P0-H — Publication state machine and correction to the current PR

### Critical correction

The current branch change that simply makes the static snapshot `published`-only is **not merge-safe** by itself.

The real flow materializes a candidate before `finalize_publish`, while the target revision is still approved. If the same snapshot loader is changed to published-only, the incremental candidate build cannot see the target approved revision.

### Correct model

Separate public state from build-candidate state:

- public snapshot: deployed/live-verified published revisions only;
- candidate build input: one explicit approved target revision plus published dependencies/navigation needed for preview;
- candidate revision must not enter live sitemap/global navigation before promotion.

### Required state machine

- [ ] `draft`
- [ ] `review_required`
- [ ] `approved`
- [ ] `built`
- [ ] `validated`
- [ ] `deployed`
- [ ] `live_verified`
- [ ] `published`
- [ ] `blocked/withdrawn/deprecated` as explicit terminal/side states

### Required work

- [ ] Replace the branch's simple published-only snapshot change with an explicit candidate snapshot/build API.
- [ ] Keep public snapshot strictly live-published.
- [ ] Build immutable release directories/objects keyed by release ID.
- [ ] Deploy candidate release to staging/preview origin.
- [ ] Verify live HTTP status, canonical, expected content/release hash, key links, schema, and security markers.
- [ ] Promote release atomically only after live verification.
- [ ] Keep previous release immediately rollbackable.
- [ ] Only then mark CMS/page state `published`.

Exit criteria: DB `published` means a user can retrieve the verified release from the production origin.

## 11. P0-I — Render/security gate

### Current problems

- Markdown is rendered with raw Markdown/HTML capability without a demonstrated HTML allowlist sanitizer.
- Current PII sanitizer redacts email/phone/passport but is not an HTML/XSS sanitizer.
- JSON-LD is embedded into a script element without a dedicated script-safe serialization boundary.
- Current render validation mostly checks marker presence, not semantic correctness.

### Required work

- [ ] Either disable raw HTML in generated Markdown or sanitize rendered HTML with a strict allowlist.
- [ ] Block scripts, event handlers, `javascript:` URLs, dangerous embeds, CSS injection, and malformed URL schemes.
- [ ] Serialize JSON-LD safely for script embedding and validate it parses as JSON/schema candidates.
- [ ] Validate exact canonical value and production origin.
- [ ] Validate locale, title, H1, meta description, robots/index policy, structured data, breadcrumb targets, and all required link hrefs.
- [ ] Validate same-origin/internal-link policy and detect broken links.
- [ ] Validate no internal IDs/role names/debug metadata leak.
- [ ] Add accessibility and HTML conformance checks for core templates.
- [ ] Add security regression fixtures for `<script>`, `</script>`, `onerror`, `javascript:`, malicious Markdown links/images, and unsafe source/LLM payloads.

Exit criteria: generated untrusted content cannot produce active script/content injection or malformed public SEO identity.

## 12. P0-J — Public content quality and user utility

The system must not optimize for “a page exists.” It must optimize for a user resolving an intent better than competing results.

- [ ] Domain-pack completeness contract defines exact facts/sections required for each page archetype.
- [ ] Critical values are rendered structurally (tables/checklists/timelines/specs/etc.) rather than generic prose.
- [ ] Public citations and last-verified signals are visible where factual trust matters.
- [ ] Applicability/assumptions are visible to the user.
- [ ] Generic filler (“standard conditions”, “official channel”) is a quality blocker when exact supported information exists or is required.
- [ ] Human-readable localized titles/slugs/headings are generated from domain semantics, not internal hashes or English fallback labels.
- [ ] CTA and conversion events are domain-pack-defined and measurable.
- [ ] Responsive/accessibility/design-system output is part of launch certification, not a later cosmetic concern.

Exit criteria: a representative user can complete the target intent from the page with clear provenance and next action.

## 13. P1-A — Real incremental build and scalable site topology

### Current problems

- Incremental build currently loads/renders the whole site and filters artifacts afterward, so cost remains approximately O(total pages).
- Global header navigation loops over all pages rather than consuming the navigation-tree/silo model already persisted in the database.
- Full builds delete/recreate the output directory rather than producing an immutable candidate and atomically promoting it.

### Required work

- [ ] Build only impacted target pages plus explicit dependent navigation/sitemap fragments.
- [ ] Make dependency graph the authoritative rebuild fan-out mechanism.
- [ ] Render navigation from `navigation_trees/navigation_items/silo_groups`, not the full page catalog.
- [ ] Add bounded nav depth/size and domain-pack navigation policy.
- [ ] Partition sitemaps for large sites and update only affected shards/index.
- [ ] Produce immutable releases and atomic promotion/rollback.
- [ ] Add scale tests at 1k, 10k, 100k+ page metadata sizes with bounded per-page build cost.

Exit criteria: updating one fact does not require rendering the entire site and HTML size/navigation remain bounded as the site grows.

## 14. P1-B — Link recommendation state correctness

- [ ] `candidate` remains internal reasoning state.
- [ ] `accepted` represents editorial/system policy approval.
- [ ] `applied` represents a link actually part of the approved/published revision.
- [ ] Public renderer consumes only applied revision-owned links.
- [ ] Required-link QA verifies exact target href in rendered output, not presence of internal keys/role strings.
- [ ] Graph algorithm/version/reason is preserved for diagnostics without leaking to users.

Exit criteria: public links are deliberate revision content, not raw recommendation candidates.

## 15. P1-C — Active freshness and autonomous maintenance loop

- [ ] Freshness monitor schedules source revalidation rather than only reporting staleness.
- [ ] Source snapshot change triggers extraction/adjudication only where needed.
- [ ] Critical source failure beyond domain TTL triggers explicit page stale/review/withdraw policy.
- [ ] No-change revalidation advances verification timestamps without unnecessary page regeneration.
- [ ] Changed truth emits precise dependency keys and dispatches bounded rebuilds.
- [ ] Rebuild dispatcher must have a guaranteed complete mapping from page to domain/truth context; remove any ambiguity around `site_scopes.context_key` or equivalent owner mapping.
- [ ] Rebuild completion updates backlog/global plan states and is externally observable.

Exit criteria: published knowledge stays current without manual monitoring and stale critical content fails safe.

## 16. P1-D — GSC/analytics/product feedback loop

### Current problem

`gsc_sync` is currently a connectivity check rather than full Search Console ingestion.

### Required work

- [ ] Ingest page/query/date/device/country GSC performance and index-status data.
- [ ] Ingest analytics/conversion events mapped to page/release/domain intent.
- [ ] Track index coverage, impressions, clicks, CTR, average position, engagement, conversions, freshness, and truth completeness.
- [ ] Feed persistent underperformance into opportunity/content-gap/rebuild planning without allowing analytics to become truth authority.
- [ ] Detect pages that should merge, split, refresh, redirect, or noindex based on combined search/topology/content evidence.
- [ ] Add launch and steady-state scorecards with blocking thresholds where appropriate.

Exit criteria: the system proves not only that it publishes pages but that the supersite improves against explicit search/user/business goals.

## 17. P1-E — Merge, supply-chain, and security governance

- [x] Add a GitHub Actions quality gate for PRs and main.
- [x] Enforce Rust formatting, workspace compilation/tests, architecture audit and RustSec audit in CI.
- [ ] Protect `main` with required status checks and required review.
- [ ] Add secret scanning/push protection.
- [ ] Add dependency/license policy (`cargo deny` or equivalent) and lockfile governance.
- [ ] Add Python type/static checking where Python remains operationally important.
- [ ] Produce SBOMs for release artifacts/containers.
- [ ] Scan containers/dependencies and pin third-party CI actions by immutable SHA.
- [ ] Run production services with least privilege/non-root and explicit secret injection.
- [ ] Remove production fallbacks to default local credentials/hosts.

Exit criteria: unverified code/dependencies/secrets cannot silently enter a production release.

## 18. P1-F — Production configuration fail-closed

- [ ] No production fallback to `https://example.com`.
- [ ] No production fallback to relative static output directories.
- [ ] Require absolute release storage path/URI.
- [ ] Require explicit production DB/Temporal/Qdrant/Neo4j/provider settings.
- [ ] Separate dev/staging/prod credentials and resource namespaces.
- [ ] Validate all required config at worker/service startup and certification preflight.

Exit criteria: a production worker cannot start a publish-capable path with placeholder/default infrastructure identity.

## 19. P1-G — Backup, restore, and disaster recovery

### Current boundary

Local Docker `pg_dump`/restore drills are valuable acceptance tests but not production disaster recovery.

### Required work

- [ ] Encrypted off-host/off-region backups for business and Temporal state.
- [ ] Retention and immutability policy.
- [ ] Point-in-time recovery where supported/required.
- [ ] Backup checksums and restore verification into a clean environment.
- [ ] Document and measure RPO/RTO.
- [ ] Include release artifact/object storage and critical configuration in DR scope.
- [ ] Periodically prove full service recovery plus public-release rollback.

Exit criteria: loss/corruption of a primary environment has a demonstrated recovery path within declared RPO/RTO.

## 20. P1-H — Cost, throughput, and workflow architecture

The 56 logical steps are useful as a semantic/audit model, but not every logical transformation necessarily deserves its own durable Temporal activity.

- [ ] Keep complete logical traceability while grouping pure deterministic transformations where this reduces history/DB overhead without losing replay/audit safety.
- [ ] Keep external I/O, long-running work, retry/idempotency boundaries, projection barriers, and HITL as explicit durable activities.
- [ ] Measure per-page provider calls, tokens, crawl bandwidth, DB I/O, graph queries, vector operations, workflow history size, and build time.
- [ ] Add budgets/guardrails per domain/page/rebuild.
- [ ] Batch operations where safe and prove bounded concurrency/backpressure.

Exit criteria: the supersite can scale economically as well as functionally.

## 21. Required certification matrix

A 10/10 release program must include all of the following evidence classes.

### A. Fresh-domain bootstrap

- zero pre-existing truth;
- real sources discovered/crawled;
- verified/admissible facts produced;
- graph/retrieval materialized from real state;
- useful IA/pages produced;
- HITL completed;
- candidate deployed and live-verified.

### B. Scope expansion

- intentional new content appears;
- existing canonical identity remains stable;
- topology/nav/link graph updates are bounded and justified;
- no unexpected cannibalization.

### C. Factual/source change

- changed source snapshot produces changed truth where appropriate;
- precise dependency impact is found;
- affected pages rebuild;
- unrelated pages do not rebuild;
- old release remains live until new one passes.

### D. Dependency failure

For DB, Temporal, provider, Qdrant, Neo4j/GDS, storage/deploy, and critical source failure:

- new unsafe publication blocks;
- existing public release remains available where possible;
- failure is observable/actionable;
- retry/recovery does not duplicate/corrupt state.

### E. Security payload

Malicious source/LLM/Markdown/schema inputs cannot execute active content or bypass public-output contracts.

### F. Cross-niche portability

At least three materially different domain packs pass A-E without generic core changes.

## 22. Required release verdict

A release is `GO` only when all P0 exit criteria are green for the target domain pack and environment.

The project is not globally `10/10 supersite-ready` until the cross-niche portability certification passes.

A transport smoke PASS, process exit code 0, available GDS plugin, non-empty synthetic collection, successful local render, or architecture-only gate is insufficient by itself.

## 23. Current known blockers

### Source-code/design blockers

- hard-coded visa domain identity and page/content semantics in generic runtime paths;
- fresh-scope bootstrap blocked by mandatory initial verified support;
- citizenship/truth identity vs SEO scope/page identity mismatch;
- no explicit residence/jurisdiction applicability dimension for visa pack;
- Graph/GDS hard requirement is ahead of actual GDS reasoning usage;
- public-vs-candidate build state is conflated;
- current branch's simple published-only snapshot change breaks approved candidate materialization;
- page-level publish failures can be hidden by workflow completion;
- E2E harness does not prove canonical durable HITL and has a Cargo cwd/manifest reproducibility problem;
- effective/freshness validity is not synchronously enforced when reading support;
- source citation and verification timestamps are insufficient for public trust output;
- locale-specific truth/content labels are not generic;
- raw Markdown/JSON-LD security boundary is insufficiently proven;
- render gate is structural smoke rather than production validation;
- incremental build still performs whole-site rendering work;
- renderer uses all pages as global navigation instead of persisted nav topology;
- publication ends at local artifact/DB state rather than deploy + live verification;
- freshness monitor is passive;
- GSC service is connectivity-only;
- local backup scripts are not production DR.

### External/environment blockers

- real production domain/origin and deployment target;
- production/staging provider credentials;
- real launch-scope graph/retrieval materialized state;
- real authoritative source data sufficient for domain completeness;
- GitHub branch-protection/security settings;
- production backup/storage/monitoring infrastructure.

The repository and release process must fail closed when required P0 capabilities are absent from a real launch certification.

## 24. Immediate execution order

Do not add more decorative infrastructure before closing this vertical slice.

1. Define generic Domain Pack contract and move visa-only identity/content assumptions behind it.
2. Fix fresh-scope bootstrap and applicability/page identity.
3. Implement real Graph/GDS reasoning responsibilities and acceptance/ablation tests.
4. Complete retrieval real-data readiness and product-use contracts.
5. Fix truth effective/freshness/citation semantics.
6. Fix workflow page-failure result propagation.
7. Replace the current public-snapshot fix with explicit approved candidate build + public published snapshot.
8. Add HTML/JSON-LD security and strong render validation.
9. Add immutable staging deployment, live verification, atomic promotion, rollback.
10. Replace CLI approximation with canonical Temporal HITL E2E certification.
11. Produce one complete real domain vertical slice from zero truth to live page.
12. Close scalable incremental/nav/sitemap/rebuild loops.
13. Close active freshness, GSC/analytics, production DR/security.
14. Prove the same core on at least two additional materially different domain packs.

The governing rule is:

> No capability counts as complete because code, a collection, a graph, a model, or a gate exists. It counts only when it changes a justified product decision, is observable in the final supersite, is protected by evidence, and transfers through the generic domain contract where the capability is intended to be generic.
