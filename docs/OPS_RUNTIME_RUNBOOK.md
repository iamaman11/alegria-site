# Operations Runtime Runbook (V5, Rust-first)

Этот документ владеет runtime substrate, rollout, replay, restore, HITL transport, and production-gate operations. It is not the owner of truth architecture or of the 56-step workflow semantics.

## 1) Runtime architecture

- Temporal server хранит durable workflow history.
- Rust temporal worker исполняет зарегистрированные workflows/activities.
- Rust temporal starter запускает workflow executions и служит операционной точкой входа.
- Workflow layer: только deterministic orchestration.
- Activity layer: весь I/O (DB, HTTP, LLM, graph, vector).

### Runtime code map (актуально)

- Temporal service: `app/rust/services/temporal`
  - worker: `src/main.rs`
  - starter CLI: `src/bin/temporal_starter.rs`
  - workflows: `src/workflows/mod.rs`
  - activities: `src/activities/mod.rs`
- Domain crates: `app/rust/crates/*`
- Runtime services: `app/rust/services/*`
- DB schema: `app/db/schema.sql`
- Baseline DB diagnostics/bootstrap:
  - `automation/check_local_runtime_db_baseline.py`
  - `automation/bootstrap_local_runtime_baseline.py`
- Truth extraction provider preflight:
  - `automation/check_truth_extraction_provider_ready.py`
- Canonical Step 5 live-provider gate:
  - `bash automation/run_live_provider_minimal_scope_gate.sh`
- Business DB container: `alegria_postgres`
- Business DB pool boundary: `alegria_pgbouncer` (`:6432`, session pooling)
- Temporal DB container: `alegria_postgres_temporal`
- Temporal server container: `alegria_temporal`
- Temporal worker container: `alegria_temporal_worker`
- Metrics endpoint: `http://localhost:9464/metrics`
- Observability stack: `alegria_prometheus` (`:9090`), `alegria_grafana` (`:3000`)
- Worker version discipline: every docker worker deployment must set `WORKER_BUILD_ID`
- `WORKER_BUILD_ID` is mandatory runtime identity and traceability metadata.
- Full server-side worker routing/version assignment must only be enabled together with a dedicated rollout procedure and acceptance-tested workflow assignment.
- New `WORKER_BUILD_ID` is sufficient only for activity-only changes or replay-safe internal fixes.
- New workflow type is mandatory when workflow branch/order/signal semantics change or old histories cannot replay safely.
- Rollback/drain policy: old worker build-id stays alive until old histories drain; rollback returns old worker fleet, never mutates workflow history in place.
- Runtime forensic/ledger tables:
  - `pipeline.execution_run_blobs`
  - `pipeline.step_executions`
  - `pipeline.step_attempts`
  - `pipeline.step_payload_blobs`
  - `pipeline.hitl_decisions`
  - `system.dead_letter_queue`
  - `pipeline.reconcile_runs`
  - `pipeline.reconcile_actions`
- `pipeline.execution_runs` is coarse run registry only; runtime payload state lives in `pipeline.execution_run_blobs` and `pipeline.step_payload_blobs`.
- Operational DB rule:
  - `app/db/schema.sql` is fresh-bootstrap snapshot only.
  - `app/db/migrations/*.sql` is the canonical incremental upgrade path.
  - A local volume missing runtime baseline tables is `drifted`, not a valid migration target.
- Versioned outbox contract:
  - `system.sync_outbox.payload_type`
  - `system.sync_outbox.schema_version`
  - `system.sync_outbox.idempotency_key`
- Truth extraction provider contract:
  - acceptable live provider env paths:
    - `SEO_TRUTH_LLM_LOCAL_ENDPOINT` or `SEO_LLM_LOCAL_ENDPOINT`
    - `OPENAI_API_KEY`
    - `ANTHROPIC_API_KEY`
    - `GEMINI_API_KEY` or `GOOGLE_API_KEY`
  - current recommended live path for `Step 5` and immediate `R3.4` work: `GEMINI_API_KEY` with `GEMINI_TRUTH_MODEL` or fallback `GEMINI_SEO_MODEL`
- missing truth extraction provider is a hard blocker for live `raw_knowledge_ingestion`
- missing live-provider credentials must short-circuit the Step 5 gate as `PENDING_CREDENTIALS`, not as a synthetic runtime failure after a long smoke

Owner-boundary note:

- truth architecture and active/deferred runtime boundaries are owned by [V6_Expert_Truth_Graph_Runtime.md](V6_Expert_Truth_Graph_Runtime.md);
- workflow shape and activation status are owned by [V6_SeoSiteBuildWorkflow_Working_Plan.md](V6_SeoSiteBuildWorkflow_Working_Plan.md);
- support processes outside the 56-step flow are owned by [V6_Support_Process_Registry.md](V6_Support_Process_Registry.md);
- this runbook owns only substrate and operational enforcement.

### Build hygiene policy (accepted)

Critical verification surfaces must not rely on whatever happens to be left inside the shared Cargo build cache.

The accepted rule is:

- `app/rust/target/` is disposable build residue, not evidence;
- critical gates run through isolated `--target-dir` roots;
- a clean acceptance pass uses dedicated isolated target roots rather than the shared developer cache;
- accepted evidence lives in `docs/runs/**`, not in build artifacts.

Operational entrypoints:

- build residue cleanup:
  - `bash automation/clean_build_residue.sh`
- whole-page semantic gate:
  - `bash automation/run_whole_page_semantic_gate.sh`
- truth certification gate:
  - `bash automation/run_truth_certification_gate.sh`
- Voyage evaluation bundle:
  - `bash automation/run_voyage_evaluation_bundle.sh`
- clean acceptance bundle:
  - `bash automation/run_clean_acceptance_bundle.sh`

Supported overrides:

- `WHOLE_PAGE_SEMANTIC_TARGET_DIR`
- `TRUTH_CERT_TARGET_ROOT`
- `CLEAN_ACCEPTANCE_TARGET_ROOT`
- `CLEAN_ACCEPTANCE_CARGO_HOME`

`CLEAN_ACCEPTANCE_CARGO_HOME` exists for environments where dependency/build-script writes must land in a known writable cargo home. It is an environment contract, not a second runtime path.

## 1a) Voyage retrieval policy

Current-runtime Voyage ownership lives in [V6_Voyage_Retrieval_Policy.md](V6_Voyage_Retrieval_Policy.md).

Operational defaults:

- `VOYAGE_MODEL=voyage-4-large`
- `VOYAGE_CONTEXT_MODEL=voyage-context-3`
- `VOYAGE_RERANK_MODEL=rerank-2.5`

Current active retrieval collections:

- `raw_chunks_4`
- `raw_chunks_ctx`
- `whole_page_advisory_prototypes`
- compatibility surface: `ontology`

Hard rules:

- retrieval indexing uses `input_type=document`;
- runtime search uses `input_type=query`;
- peer clustering omits `input_type`;
- truth- and draft-adjacent retrieval input must not silently truncate;
- retrieval remains non-authoritative.

## 2) Current workflow chains

- `SeoSiteBuildCanonicalCutoverWorkflow`:
  - active forward workflow for new production-style site-build execution
  - `seo_preflight`
  - `load_seo_site_build_input`
  - `load_verified_support_bundle.initial`
  - `serp_ingest`
  - `crawl_sources`
  - `whole_page_semantic_pass`
  - `page_utility_classifier`
  - `dom_block_relevance_filter`
  - `sectioning`
  - `sectioning_contract_gate`
  - `cas_gate`
  - `raw_evidence_register`
  - `projection_barrier(raw_evidence)`
  - `layer_router`
  - `subspan_layer_router`
  - `entity_span_detection`
  - `canonical_mapping`
  - `ontology_intake_gate`
  - `procedural_extraction`
  - `operational_extraction`
  - `editorial_extraction`
  - `seo_signal_extraction`
  - `commercial_signal_extraction`
  - `extraction_schema_validate`
  - `candidate_validation`
  - `triple_builder`
  - `completeness_judge`
  - `resolution_loop`
  - `contradiction_gate`
  - `truth_adjudication`
  - `verified_truth_write`
  - optional `load_verified_support_bundle.refresh`
  - `serp_normalize`
  - `opportunity_build`
  - `ia_build`
  - `link_recommend`
  - `global_site_reconcile`
  - `projection_barrier(global_site_reconcile)`
  - per-page:
    - `draft_assemble`
    - `editorial_draft_generate`
    - `draft_normalize`
    - `content_contract_validate`
    - `draft_qa`
    - conditional publish-control path:
      - `truth_admissibility_gate`
      - `cms_request_review`
      - `human_approval_wait`
      - `cms_publish_approved`
      - `publish_materialize`
      - `render_preview_validate`
      - `finalize_publish`
      - `projection_barrier(publish)`
  - conditional final phase:
    - `rebuild_detect`
  - current execution plan is branch-sensitive to normalized `run_mode`, scenario, and policy:
    - `crawl_only` stops before planning/drafting
    - `draft_only` and `dry_run` persist without publish
    - publish phases run only when publish policy and scenario both allow them
    - workflow may terminate early with `done:no_pages`
- `SeoSiteBuildWorkflow`:
  - compat/drain workflow only; retained for replay-safe legacy histories and controlled comparison
  - `load_seo_site_build_input`
  - `load_verified_support_bundle`
  - `serp_ingest`
  - `crawl_sources`
  - `raw_knowledge_ingestion`
  - optional `load_verified_support_bundle.refresh`
  - `serp_normalize`
  - `opportunity_build`
  - `ia_build`
  - `link_recommend`
  - `global_site_reconcile`
  - per-page:
    - `draft_assemble`
    - `editorial_draft_generate`
    - `draft_normalize`
    - `content_contract_validate`
    - `draft_qa`
    - conditional publish-control path:
      - `cms_request_review`
      - `human_approval_wait`
      - `cms_publish_approved`
      - `publish_materialize`
      - `render_preview_validate`
      - `finalize_publish`
  - conditional final phase:
    - `rebuild_detect`
  - current execution plan is branch-sensitive to normalized `run_mode`, scenario, and policy:
    - `crawl_only` stops before planning/drafting
    - `draft_only` and `dry_run` persist without publish
    - publish phases run only when publish policy and scenario both allow them
    - workflow may terminate early with `done:no_pages`
  - workflow checkpoints already observe projection barrier status after:
    - `raw_knowledge_ingestion`
    - `global_site_reconcile`
- `ExpertExtractionWorkflow`:
  - migration-only diagnostic extraction surface
  - disabled by default; requires `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`
  - `load_seo_site_build_input`
  - `load_verified_support_bundle.initial`
  - `serp_ingest`
  - `crawl_sources`
  - `raw_knowledge_ingestion`
  - optional `load_verified_support_bundle.refresh`
- `ExpertDecomposedExtractionWorkflow`:
  - migration-only diagnostic decomposition surface for the legacy extraction macro-step
  - disabled by default; requires `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`
  - `load_seo_site_build_input`
  - `load_verified_support_bundle.initial`
  - `serp_ingest`
  - `crawl_sources`
  - `load_semantic_section_sample`
  - `page_utility_classifier`
  - `dom_block_relevance_filter`
  - `layer_router`
  - `entity_span_detection`
  - `canonical_mapping`
  - `procedural_extraction`
  - `operational_extraction`
  - `editorial_extraction`
  - `completeness_judge`
  - `triple_builder`
  - `contradiction_gate`
  - `hitl_decision`
  - `raw_knowledge_ingestion`
  - optional `load_verified_support_bundle.refresh`
- `ExpertProjectionWorkflow`:
  - migration-only diagnostic extraction-plus-projection surface
  - disabled by default; requires `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`
  - `load_seo_site_build_input`
  - `load_verified_support_bundle.initial`
  - `serp_ingest`
  - `crawl_sources`
  - `raw_knowledge_ingestion`
  - `graph_admissibility_gate`
  - `neo4j_sync`
  - `retrieval_admissibility_gate`
  - `voyage_qdrant_sync`
  - `projection_barrier.post_projection`
  - optional `load_verified_support_bundle.refresh`
- `ExpertSemanticSliceWorkflow`:
  - migration-only diagnostic real-section semantic surface
  - disabled by default; requires `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`
  - `load_seo_site_build_input`
  - `load_verified_support_bundle.initial`
  - `serp_ingest`
  - `crawl_sources`
  - `load_semantic_section_sample`
  - `page_utility_classifier`
  - `dom_block_relevance_filter`
  - `layer_router`
  - `entity_span_detection`
  - `canonical_mapping`
  - `procedural_extraction`
  - `operational_extraction`
  - `editorial_extraction`
  - `completeness_judge`
  - `triple_builder`
  - `contradiction_gate`
  - `hitl_decision`
- `ProjectionReconcileWorkflow`:
  - `projection_reconcile.neo4j`
  - `projection_reconcile.qdrant`
- `ContentGenerationWorkflow`:
  - legacy-only
  - excluded from production SEO launch path
  - keep disabled by default outside explicit legacy cutover testing

- `FreshnessCheckWorkflow`:
  - `check_data_freshness`

## 2.1 Support-plane operator surfaces

- `freshness_monitor`
  - executable surface: `FreshnessCheckWorkflow`
- `projection_reconcile_and_reclaim`
  - executable surfaces: `ProjectionReconcileWorkflow`, `reconcile` service, outbox worker
- `rebuild_dispatcher`
  - executable surface: `temporal_starter RebuildDispatch`
  - responsibility: consume `monitoring.seo_rebuild_backlog`, validate scope completeness, register fresh run input, and start a new scoped workflow without mutating existing workflow history
  - evidence surface: `--report-json`
- `ontology_backfill_reindex`
  - executable surface: `temporal_starter OntologyBackfillPlan`
  - named support capability: `Voyage/Qdrant ontology materialization`
  - current hard guarantee: concept impact planning plus optional Neo4j materialization and optional Voyage/Qdrant ontology compatibility materialization with `kb.qdrant_points` ledger update
  - operator knobs: `--apply-neo4j`, `--apply-qdrant` (`apply_qdrant` retrieval materialization path)
  - evidence surface: `--report-json`
- `post_publish_feedback_loop`
  - operator probe surface: `cli_tools SeoPostPublishFeedbackProbe`
  - connected services: `gsc_sync`, `analytics_svc`
  - authority rule: this loop must never mutate verified truth
  - evidence surface: `--report-json`
- `release_and_restore_gate`
  - operator surface: `cli_tools SeoReleaseRestoreGate`
  - evidence surface: `--report-json`
- heavyweight executable surfaces: `automation/ci_verify.sh`, `automation/temporal_production_gate.sh`, `infra/backups/restore_drill.sh`
- truth certification is a mandatory local/CI gate and writes only evidence under `docs/runs/**`; it never writes authority truth
- accepted local/CI certification baseline artifact: [docs/runs/truth_certification_local_ci_baseline.json](/home/bose/projects/alegria-site/docs/runs/truth_certification_local_ci_baseline.json)
- canonical certification entrypoint: `automation/run_truth_certification_gate.sh`
- the canonical entrypoint runs the suite in an isolated Cargo target dir to avoid shared-target proof instability
- the wrapper's transient current report is local-only and defaults to `/tmp/truth_certification_local_ci_current.json`, not `docs/runs/**`
- truth-governance decision tables and regex authority boundaries are owned by [V6_Truth_Governance_Policy.md](V6_Truth_Governance_Policy.md), not by ad hoc runtime overrides

## 3) HITL contract

### Trigger scenarios
- canonical mapping `0.75–0.87` -> review
- canonical mapping `<0.75` -> new candidate
- numeric conflict
- open blocking conflict case
- unresolved range rule

### Priority model
- `1` blocking (SLA 4h)
- `2` important (SLA 8h)
- `3` background (SLA 24h)

### Resolution outcomes
- `approved`
- `rejected`
- `registry_extension_required`
- `conflict_resolution_required`
- `needs_new_source`

### Orchestration behavior
- blocking HITL policy задана контрактом
- текущий runtime: HITL queue + workflow pause/resume через Temporal signal подключены для Rust workflows
- publish stays blocked while unresolved blocking conflict exists

### Current implementation note

- HITL queue orchestration идет через `seo_application::hitl`, а БД-реализация сидит в `seo_ports`/SQLx adapters.
- В `app/rust/services/temporal/src/workflows/mod.rs` есть signal/query/update handlers:
  `pause`, `resume`, `status`, `set_pause`.
- Truth extraction больше не имеет отдельного standalone workflow. HITL применяется на candidate/adjudication path и publish-review path, а не через legacy standalone extraction workflow.

## 4) Rust-first migration scope

In scope:
- `app/*` runtime
- temporal worker
- `seo_application`, `seo_steps`, adapters, policies

Out of scope:
- `docs/_archive`
- `infra/analytics_lab` (R&D only)

## 5) Implementation plan (execution order)

### P0 Infrastructure
1. Temporal server + UI healthy
2. Rust worker connected to task queue
3. Temporal starter can start workflows on target queue
4. Temporal persistence separated from business data
5. Docker worker uses PgBouncer, local dev worker may use direct localhost Postgres

### P1 Determinism boundary
1. Workflow contains orchestration only
2. All I/O in activities
3. Non-deterministic functions banned from workflow code

### P2 Run-state and idempotency
1. Every run tracked by `run_id`
2. Side effects idempotent + deduplicated
3. Outbox and publish paths replay-safe
4. Step attempts and payload blobs preserved for replay/forensics

### P3 HITL integration
1. Blocking conflicts pause workflow
2. Human decision resumes workflow
3. Publish gate enforces unresolved-conflict blocking
4. Fail-safe stale scan must exclude `pending_hitl`
5. Every resolution is persisted in `pipeline.hitl_decisions`

### P4 Acceptance
1. E2E run on production-like dataset
2. Metrics/alerts on failures, retry storm, lag
3. Full pass of `automation/ci_verify.sh`
4. SEO launch path uses `SeoSiteBuildCanonicalCutoverWorkflow` with runtime-loaded verified support bundle
5. SEO publish path materializes artifact through workflow-owned incremental build with validated fallback

## 6) Backup and restore

- Business DB backup: `infra/backups/backup_business_pg.sh`
- Temporal DB backup: `infra/backups/backup_temporal_pg.sh`
- Business DB restore: `infra/backups/restore_business_pg.sh <dump.sql>`
- Temporal DB restore: `infra/backups/restore_temporal_pg.sh <dump.sql>`
- Full restore drill: `infra/backups/restore_drill.sh`

`restore_drill` is the mandatory resilience exercise after schema/runtime hardening. A release is not considered operationally safe if `restore_drill.sh` or `automation/temporal_production_gate.sh` fails.

## 7) Release gate

Production release is allowed only when:
- `bash automation/ci_verify.sh` passes
- runtime smoke checks pass
- HITL pause/resume verified
- metrics endpoint exposes runtime counters/histograms
- restore drill succeeds
- reconcile writes persistent run/action forensic records

The canonical operator probe for this support process is:

- `cargo run -p cli_tools -- seo-release-restore-gate --report-json automation/reports/release_restore_gate.json`

Heavyweight execution can be requested explicitly:

- `cargo run -p cli_tools -- seo-release-restore-gate --run-ci-verify --run-temporal-gate --run-restore-drill --report-json automation/reports/release_restore_gate.full.json`

Canonical local env evidence bundling for this support plane is:

- `bash automation/run_local_operational_evidence_bundle.sh`

This wrapper does not create a new runtime gate. It bundles existing evidence surfaces:

- isolated clean acceptance output;
- release/restore gate report;
- legacy replay evidence;
- live-provider minimal-scope evidence.

Its output is an env-scoped machine-readable artifact. `BLOCKED_ON_LIVE_PROVIDER` is a valid and expected local verdict when all local operational surfaces are green but the real live provider still lacks credentials.

## 7.1 Compat / drain closure and naming

- `SeoSiteBuildWorkflow` remains in code only as compat/drain and controlled replay surface.
- For the current local environment, the compat/drain window is considered closed because [docs/runs/seo_site_build_legacy_replay_evidence.json](/home/bose/projects/alegria-site/docs/runs/seo_site_build_legacy_replay_evidence.json) records `total_runs=0` and `open_runs=0`.
- Local env bundle evidence is captured in [docs/runs/local_operational_evidence_bundle.json](/home/bose/projects/alegria-site/docs/runs/local_operational_evidence_bundle.json).
- For any other environment, the window closes only with equivalent machine-readable legacy replay evidence or explicit production drain signoff.
- `Expert*Workflow` surfaces remain permanent narrow diagnostic surfaces, require `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`, and must never become a hidden forward-path fallback for product execution.
- `SeoSiteBuildCanonicalCutoverWorkflow` remains the runtime workflow type after drain; there is no follow-up workflow-type rename just for naming cosmetics.

## 8) Archive policy

Любой файл, выводимый из runtime-пути, переносится в `docs/_archive/*` с датой и без удаления истории.
