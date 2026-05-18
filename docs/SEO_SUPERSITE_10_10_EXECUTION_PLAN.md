# SEO Supersite 10/10 Execution Plan

Status: partial live-aligned execution plan
Parent owner document: `V6_Expert_Truth_Graph_Runtime.md`
Owner: Alegria SEO site build runtime
Last updated: 2026-05-16

## Target

Alegria is 10/10 when a production run can create or update a real SEO supersite from live sources:

1. Resolve or bootstrap truth identity and publishing identity separately:
   - truth identity: `country_code + visa_family + visa_subtype + citizenship_code -> kb.visa_contexts.context_key`
   - publishing identity: `market + locale + country_code + visa_type + applicant_profile -> scope_signature`
2. Discover official and competitor source URLs from real SERP and configured source registries.
3. Crawl sources into `raw.pages` and clean `raw.sections`.
4. Extract structured rules with citations, source trust, conditions, and contradiction handling.
5. Persist verified truth in PostgreSQL, project graph relations into Neo4j, and index search chunks in Qdrant.
6. Build and evolve a multi-page, multi-scope information architecture.
7. Update global navigation, breadcrumbs, sitemap, and internal links when new scopes or facts arrive.
8. Generate drafts only from verified facts plus supplemental retrieved context.
9. Block unsupported factual claims and route uncertain facts/pages to HITL.
10. Publish approved pages as CMS/static artifacts with schema, breadcrumbs, canonical URLs, robots, sitemap, and rebuild metadata.

## Current Capability

Implemented baseline:

- `SeoSiteBuildWorkflow` runs multi-page site build stages.
- DataForSEO results can enqueue scoped crawl jobs.
- `crawl_sources` persists `raw.pages` and `raw.sections`, emits Qdrant outbox, and returns `raw_page_ids`.
- `raw_knowledge_ingestion` persists source registration, raw-section context binding, and `extracted.rule_candidates`; it does not bridge raw pages directly into `verified.rule_instances`.
- `source_type`/`trust_level` are adjudication inputs only. No source tier can shortcut a verified verdict.
- `global_site_reconcile` normalizes current-scope hierarchy and required links.
- Data-derived global navigation storage exists: `site.site_scopes`, `site.silo_groups`, `site.navigation_trees`, `site.navigation_items`, and `site.global_rebuild_plan`.
- `global_site_reconcile` persists a navigation snapshot from active `site.page_nodes`; it does not invent menu items before IA/source-backed pages exist.
- `seo-preflight` reports scope readiness, verified/pending knowledge counts, Qdrant point ledger size, live Qdrant/Neo4j probes, and Neo4j/Qdrant/CMS projection outbox status.
- `seo-preflight --strict-projections` blocks launch when projection events are pending, processing, or failed.
- Workflow checkpoints after `raw_knowledge_ingestion` and `global_site_reconcile` observe projection status; `SEO_STRICT_PROJECTION_BARRIER=true` turns open/failed projection events into a workflow-blocking validation failure.
- Draft/QA/CMS/static publishing gates exist.
- `outbox-worker` is present in Compose for Neo4j/Qdrant/CMS projection materialization.

Critical gaps:

- New scopes have starter-side bootstrap/preflight and direct Neo4j/Qdrant probes, but DataForSEO is still credential-checked rather than queried in preflight.
- `SeoSiteBuildWorkflow` already exposes normalized `run_mode` branching such as `crawl_only`, `draft_only`, `publish_with_hitl`, and `full_auto_after_approval`, but the richer semantic/graph path is still not reactivated as a mandatory runtime contour.
- SERP intelligence is still query-led, not top10-pattern-led.
- Crawler is an MVP fetcher, not a production crawler.
- Extraction is too narrow and currently not enough for real visa rule coverage.
- Competitor-only extracted facts are persisted as pending; they need verification/HITL before they can support publish-ready pages.
- Projection barriers already exist in preflight and workflow checkpoints; current workflow control is present, but broader graph/retrieval projection hardening and certification still remain partial.
- Global navigation storage exists, but full policy for multi-country/multi-visa menu grouping still needs refinement after real data appears.
- Rebuild detection marks impacted pages, but there is no full rebuild scheduler.
- Live E2E certification has not been run against real credentials and real sources.

## Phase 1: Production Preflight And Bootstrap

Status: partially implemented. Scope bootstrap, neutral registry seeding, preflight diagnostics, dependency probes, projection status reporting, and explicit SEO site-build run modes are live. Rich semantic and graph/retrieval reactivation remain planned.

Goal: a new SEO scope can start without manual SQL and fails fast when runtime dependencies are missing.

Work:

- Implemented: starter bootstrap for `kb.visa_contexts`.
- Implemented: seed only neutral runtime registries required for type safety.
- Implemented: do not seed real menus or final section templates before source collection. Menus, page mix, and concrete templates must be derived from scope, SERP evidence, verified facts, and active pages.
- Implemented: preflight checks for DataForSEO credentials, Qdrant, Neo4j, LLM provider, outbox runtime, and static output directory.
- Implemented: report verified/pending rules for the current context and Qdrant point ledger size.
- Implemented: report per-target projection outbox status, lag, failed event details, and strict launch blocking.
- Implemented: SEO site-build run modes: `dry_run`, `crawl_only`, `draft_only`, `publish_with_hitl`, `full_auto_after_approval`.

Gate:

- Starting `SeoSiteBuildWorkflow` for a fresh truth tuple can create/resolve `context_key` without deriving it from `applicant_profile`.
- Starting `SeoSiteBuildWorkflow` for a fresh scope tuple can create/resolve `scope_signature` independently of truth identity.
- Bootstrap does not predeclare final navigation; global navigation is produced only after IA/global reconcile has source-backed page nodes.
- Missing live credentials and blocked projections produce a clear preflight report.
- Empty verified support before crawling does not block the workflow.

## Phase 2: Real SERP Intelligence

Goal: IA and opportunities are derived from real top10 evidence, not only query text.

Work:

- Persist DataForSEO rank/title/snippet/domain/source type/feature metadata.
- Normalize SERP from stored top10 rows.
- Classify official, VFS, agency, editorial, forum, and low-trust sources.
- Build keyword clusters from query intent, title/snippet entities, URL patterns, and repeated competitor sections.

Gate:

- Every generated target page can explain its SERP evidence.
- Low-trust SERP sources cannot become factual support.

## Phase 3: Production Crawler

Goal: crawling is deterministic, scoped, respectful, and useful for extraction.

Work:

- Robots policy, crawl delay, per-domain concurrency, retry/backoff, and max pages per domain.
- Optional rendered fetch for JS-heavy pages.
- Redirect/canonical chain storage.
- Duplicate URL and duplicate content handling.
- Main-content extraction that removes nav/footer/sidebar and preserves tables, lists, FAQs, and citation anchors.

Gate:

- Re-running a scope is idempotent.
- Failed crawls do not poison the run.
- Raw sections are clean enough for extraction and citation.

## Phase 4: Real Knowledge Extraction And Verification

Goal: verified truth is auditable and safe enough for visa content.

Work:

- Extend deterministic extraction for documents, fees, timelines, forms, biometrics, photos, bank statements, invitations, employment proof, accommodation, tickets, minors, appointments, and application locations.
- Add schema-constrained LLM extraction with source spans and confidence.
- Normalize concepts, roles, conditions, applicant profile, nationality, residence country, age, and visa subtype.
- Enforce source trust as adjudication input only: regulated or official sources may increase confidence and corroboration priority, but cannot shortcut a verified verdict; unsupported or conflicting facts remain candidate or HITL.
- Add contradiction gate for fees, timelines, documents, application location, and eligibility.

Gate:

- Every verified rule has source provenance.
- Competitor-only legal facts cannot silently auto-publish.
- Contradictions block publish or route to HITL.

## Phase 5: Qdrant Retrieval As Search, Not Truth

Goal: Qdrant finds relevant chunks; PostgreSQL remains source of truth.

Work:

- Add projection barrier after Qdrant outbox emission.
- Store robust `raw_section -> qdrant_point` hydration keys.
- Return source URL, trust tier, context mapping, and section metadata for each retrieved chunk.
- Keep Qdrant chunks as `supplemental_context_not_fact_support`.
- Add retrieval evaluation for context bleed, source diversity, and recall.

Gate:

- A page can use chunks from multiple sources.
- No Qdrant-only fact can pass QA as verified support.

## Phase 6: Neo4j Projection And Graph Reasoning

Goal: Neo4j validates and recommends site graph changes.

Work:

- Materialize `VisaContext`, `Concept`, `RuleInstance`, `Source`, `PageNode`, `PageBlueprint`, `KeywordCluster`, `ContentGap`, and `LinkRecommendation`.
- Add projection barrier for current run outbox events.
- Detect orphan pages, pages without required links, cannibalization, unsupported pages, and facts not covered by pages.
- Recommend add/split/merge/deprecate/link actions.

Gate:

- Neo4j can answer which official requirements are not covered by target pages.
- Cannibalization is detected before publish.

## Phase 7: Global Supersite Model

Goal: adding Poland, Spain, work visas, tourist visas, or new facts updates the whole site, not only one workflow scope.

Work:

- Add `site.site_scopes`, `site.navigation_tree`, `site.navigation_items`, `site.silo_groups`, and `site.global_rebuild_plan`.
- Reconcile all active scopes, not only the current workflow pages.
- Generate global home, country hubs, visa hubs, detail pages, comparison pages, and directory pages.
- Add menu policy that avoids menu explosion while exposing long-tail pages through directories.

Gate:

- Adding `PL work visa` updates Poland hub, work visa index, sitemap, related links, breadcrumbs, and rebuild plan.
- Deprecating a scope removes links and creates redirects/stale flags.

## Phase 8: Draft Generation Quality

Goal: drafts are expert pages with complete traceability.

Work:

- Enforce JSON-only LLM contract with section-level support refs.
- Validate parsed output, section completeness, claim ledger, support refs, and unreferenced numbers/dates/prices.
- Add page-type-specific templates for requirements, fees, timelines, application centers, FAQ, and troubleshooting.
- Score factual coverage, readability, duplicate content risk, internal link coverage, and schema coverage.

Gate:

- Draft passes QA only if every factual claim is supported.
- LLM failure degrades to blocked draft, not hallucinated publish.

## Phase 9: CMS, HITL, And Publication

Goal: humans can review safely, and approved pages publish predictably.

Work:

- Add HITL dashboard/CLI flow for list/show/approve/block/request changes.
- Separate content, factual, legal, and publish approvals.
- Allow automatic publish only for official-source-only, contradiction-free, high-confidence pages.
- Full rebuild after navigation changes; incremental rebuild for content-only changes.
- Redirect handling for canonical path changes and deprecated pages.

Gate:

- Reviewer can approve without direct DB edits.
- Static output includes pages, sitemap, robots, breadcrumbs, schema, and related links.

## Phase 10: Rebuild Engine

Goal: new facts and new scopes trigger explainable regeneration.

Work:

- Add explicit dependency graph from pages to rule instances, blueprints, sources, freshness, links, and nav.
- Add `site.rebuild_jobs`.
- Prioritize urgent published fact changes over draft-only changes.
- Add rebuild workflow to regenerate affected drafts, invalidate CMS revisions, rebuild static output, and emit audit trail.

Gate:

- A changed visa fee marks all affected published pages stale or rebuild-required.
- Rebuild reason is explainable down to the changed rule/source/link.

## Phase 11: Observability And Quality Gates

Goal: every run reports why the supersite is ready or blocked.

Work:

- Metrics: crawled pages, extraction yield, verified/disputed facts, projection lag, blocked pages by reason, publish success, rebuild backlog.
- Run report: sources used, pages created, pages blocked, missing facts, contradictions, HITL tasks.
- SQL views for pages without support, facts without pages, stale sources, pages without links, and low-trust-only facts.

Gate:

- No silent success on empty or weak data.
- Operators can inspect a failed run without reading logs.

## Phase 12: Real E2E Certification

Goal: prove the system on real data.

Scenarios:

1. Fresh scope: `PL tourist visa` from empty verified support to crawl, extraction, draft, HITL, static output.
2. Existing country expansion: add `PL work visa` and verify country hub/menu/sitemap/link updates.
3. Cross-country expansion: add Spain/Italy and verify global country index/menu.
4. Fact change: fee/timeline change triggers impacted page rebuild.
5. Contradiction: official and competitor conflict blocks or routes to HITL.

Gate:

- All factual claims are traceable.
- Global navigation changes when scopes change.
- Neo4j/Qdrant projections are current.
- Rebuild is explainable.
- Unsupported factual pages cannot publish.

## Current Execution Slice

Immediate implementation order:

1. Phase 5/6 current-run scoped projection barriers, building on the implemented preflight and workflow checkpoint barriers.
2. Phase 2 real SERP normalization from stored top10 rows.
3. Phase 4 richer extraction and contradiction routing.
4. Phase 7 multi-country/multi-visa navigation policy refinement.
5. Phase 10 rebuild jobs and rebuild workflow.
