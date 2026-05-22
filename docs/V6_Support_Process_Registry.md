# V6 Support Process Registry

**Status:** current support-plane registry
**Class:** `current-support-plane-registry`
**Parent owner document:** [V6_Expert_Truth_Graph_Runtime.md](V6_Expert_Truth_Graph_Runtime.md)
**Execution satellite:** [V6_SeoSiteBuildWorkflow_Working_Plan.md](V6_SeoSiteBuildWorkflow_Working_Plan.md)
**Ops satellite:** [OPS_RUNTIME_RUNBOOK.md](OPS_RUNTIME_RUNBOOK.md)

This file owns the named processes that are required for a production-complete system but are not steps inside the canonical 56-step `SeoSiteBuildWorkflow` value stream.

Each support process must have:

- a single purpose;
- a concrete trigger;
- typed inputs and outputs;
- a named owner;
- an explicit blocking relation to the main workflow;
- immutable evidence artifacts;
- automation coverage.

If a required process is not inside the 56 steps and is not listed here, it is undocumented and therefore not production-complete.

## Registry

| Process | Purpose | Trigger | Inputs | Outputs | Owner | Blocking relation to main flow | Required evidence artifact | Automation checks |
|---|---|---|---|---|---|---|---|---|
| `freshness_monitor` | Detect stale truth, stale retrieval, stale projections, and stale planning inputs before they silently degrade the supersite. | Scheduled workflow run, operator-triggered freshness audit, or post-publish freshness sweep. | `verified.rule_instances`, projection timestamps, run metadata, freshness policies, source recrawl policy. | freshness findings, stale-scope records, rebuild trigger candidates, operator alerts. | Temporal freshness workflow + freshness adapters. | Does not block every run by default; blocks only when strict freshness policy or targeted rebuild policy requires fresh truth before draft/publish. | immutable freshness report under `docs/runs/**` or runtime evidence tables for the audit run. | `automation/check_rebuild_queue_execution_path.py`, `automation/smoke_rebuild_queue_execution_path.py`, `automation/check_run_report_completeness.py` |
| `rebuild_dispatcher` | Consume rebuild backlog and start scoped rebuild runs after truth, ontology, navigation, or publish changes. | `rebuild_detect` output, operator replay, or scheduled backlog sweep. | rebuild backlog rows, impacted page keys, scope policy, workflow starter contract. | started rebuild runs, backlog state transitions, replay-safe dispatch audit. | workflow starter + rebuild queue services. | Outside the current run; blocks system convergence if broken, but not the current extraction macro-step. | rebuild queue report and immutable dispatch evidence. | `automation/check_support_surface_execution.py`, `automation/check_rebuild_queue_execution_path.py`, `automation/smoke_dependency_aware_rebuild.py`, `automation/check_run_id_event_stamping.py` |
| `ontology_backfill_reindex` | Re-map accepted ontology changes, repair graph projections, and invalidate/rebuild affected embeddings. | ontology intake acceptance, registry change, or migration/backfill operation. | ontology change set, impacted mentions/entities, verified truth keys, graph/vector projection state. | remapped entities, repaired graph rows, vector invalidation and reindex jobs, backfill report. | ontology governance + projection adapters. | Blocks full convergence after ontology changes; not a step of a single content run. | backfill run report, impact list, projection repair evidence. | `automation/check_support_surface_execution.py`, `automation/check_seo_projection_contract.py`, `automation/check_seo_migration_parity.py`, `automation/check_reconcile_failsafe.py`, `automation/check_voyage_retrieval_surface.py` |
| `projection_reconcile_and_reclaim` | Materialize outbox projection work, reclaim stale records, and surface lag/failures for graph/retrieval consumers. | outbox queue events, scheduled reconcile scan, stale-outbox reclaim sweep. | `system.sync_outbox`, projection payloads, run ids, reconcile policy. | reconcile runs/actions, projection writes, reclaimed events, DLQ records when needed. | outbox worker + reconcile service. | Blocks strict projection barriers when lagged or failed; otherwise asynchronous. | reconcile run records in `pipeline.reconcile_runs` / `pipeline.reconcile_actions`, immutable smoke evidence. | `automation/check_projection_barrier_run_scoped.py`, `automation/check_reconcile_failsafe.py`, `automation/smoke_stale_outbox_reclaim.py` |
| `post_publish_feedback_loop` | Ingest GSC and analytics signals without mutating verified truth, then drive planning downgrade, monitoring, and future rebuilds. | scheduled analytics sync, post-publish measurement window, operator inspection. | page publish state, analytics/GSC connectors, metric contracts, attribution windows. | derived SEO metrics, stale/orphan/cannibalization findings, planning hints, monitoring alerts. | `gsc_sync`, `analytics_svc`, monitoring surfaces. | Never writes truth authority; may downgrade planning confidence or enqueue rebuild/freshness work. | analytics ingestion evidence, metric snapshots, run reports. | `automation/check_support_surface_execution.py`, `automation/check_metrics_contract.py`, `automation/check_global_navigation_policy.py`, `automation/check_run_report_completeness.py` |
| `release_and_restore_gate` | Enforce that rollout, replay, restore, metrics, and CI safety conditions are satisfied before production release. | release candidate, worker build-id rollout, migration cutover, restore drill schedule. | CI results, restore drill output, metrics endpoint, worker build-id policy, replay/compat checks. | release verdict, restore evidence, rollout decision, blocking reasons. | ops runbook + production gate automation. | Blocks production release, never hidden inside the workflow. | production gate log, restore drill artifact, replay/compat evidence. | `automation/check_support_surface_execution.py`, `automation/check_release_restore_gate_surface.py`, `automation/ci_verify.sh`, `automation/check_temporal_build_id_policy.py`, `automation/check_seo_rollout_compat_contract.py`, `automation/check_backup_restore_layout.py` |

## Owner-Doc Boundaries

- [V6_Expert_Truth_Graph_Runtime.md](V6_Expert_Truth_Graph_Runtime.md) owns architecture, authority, boundaries, and classification of active/deferred/runtime-inactive surfaces.
- [V6_SeoSiteBuildWorkflow_Working_Plan.md](V6_SeoSiteBuildWorkflow_Working_Plan.md) owns execution shape, activation order, and versioned rollout intent for the 56-step flow.
- [OPS_RUNTIME_RUNBOOK.md](OPS_RUNTIME_RUNBOOK.md) owns runtime substrate, rollout, replay, restore, and production-gate operations.
- This file owns only the support processes outside the 56-step flow.

## Activation Criteria

### Code-Present, Runtime-Inactive

Source files are `code-present, runtime-inactive` only when all of the following are true:

1. the module exists in the repository;
2. it is not exported by its crate boundary or not reachable from an active use-case/runtime path;
3. no automation contract claims it as active;
4. no owner document claims it as active.

### Partial / Deferred Surfaces

A partial or deferred surface becomes active only when:

1. it has a typed input/output contract;
2. it is exported through the owning crate boundary;
3. it is invoked by a live use-case, activity, or workflow path;
4. it is covered by automation and at least one deterministic test or smoke;
5. the owner docs describe the activation and boundary correctly.

## V5 Status

The following `V5` files remain normative satellites under `V6`:

- [V5_Runtime_Contract.md](V5_Runtime_Contract.md)
- [V5_Truth_Extraction_LLM_Contract.md](V5_Truth_Extraction_LLM_Contract.md)
- [V5_SEO_Graph_And_Retrieval_Projection_Spec.md](V5_SEO_Graph_And_Retrieval_Projection_Spec.md)

The following `V5` or legacy planning files are reference-only:

- [V5_Ultimate_Extraction_Protocol.md](V5_Ultimate_Extraction_Protocol.md)
- [SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md](SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md)
- research/backlog/reference snapshots listed in [INDEX.md](INDEX.md)

## Current Executable Surfaces

- `freshness_monitor` currently executes through `FreshnessCheckWorkflow`.
- `projection_reconcile_and_reclaim` currently executes through `ProjectionReconcileWorkflow`, `reconcile` service, and outbox workers.
- `rebuild_dispatcher` currently executes through `temporal_starter RebuildDispatch`.
- `rebuild_dispatcher` currently emits machine-readable evidence through `--report-json`.
- `ontology_backfill_reindex` currently executes through `temporal_starter OntologyBackfillPlan`; Neo4j materialization and Voyage/Qdrant ontology materialization are executable now.
- `ontology_backfill_reindex` currently emits machine-readable evidence through `--report-json`.
- `post_publish_feedback_loop` currently has operator probe coverage through `cli_tools SeoPostPublishFeedbackProbe`, plus service surfaces `gsc_sync` and `analytics_svc`.
- `post_publish_feedback_loop` currently emits machine-readable evidence through `--report-json`.
- `release_and_restore_gate` currently has operator probe/execution coverage through `cli_tools SeoReleaseRestoreGate`.
- `release_and_restore_gate` currently emits machine-readable evidence through `--report-json`.
- `release_and_restore_gate` currently executes heavyweight gates through `automation/ci_verify.sh`, `automation/temporal_production_gate.sh`, and `infra/backups/restore_drill.sh`.
- `SeoSiteBuildCanonicalCutoverWorkflow` is now the active forward workflow for canonical site-build execution; support-plane operators that start new expert rebuild/site-build work should target it by default.
- `SeoSiteBuildWorkflow` remains executable only for explicit compat/drain, replay-safe legacy runs, and controlled comparison.
- `ExpertExtractionWorkflow` remains executable only as a migration-only diagnostic extraction surface; it is not a support-plane process, is not a target for new product logic, and requires `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`.
- `ExpertDecomposedExtractionWorkflow` remains executable only as a migration-only diagnostic decomposition surface; it is not a support-plane process, is not a target for new product logic, and requires `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`.
- `ExpertProjectionWorkflow` remains executable only as a migration-only diagnostic truth-to-projection surface; it is not a support-plane process, is not a target for new product logic, and requires `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`.
- `ExpertSemanticSliceWorkflow` remains executable only as a migration-only diagnostic semantic-step surface; it is not a support-plane process, is not a target for new product logic, and requires `ALLOW_EXPERT_MIGRATION_WORKFLOWS=true`.
