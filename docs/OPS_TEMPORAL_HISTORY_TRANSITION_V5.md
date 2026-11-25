# V5: read-only historical Temporal drain / tenant-ownership gate

This is the non-destructive prerequisite to the #94 production transition.
It does **not** route traffic, start/cancel/terminate/delete/reset Workflows,
update PostgreSQL, issue admissions, deploy a Worker, or authorize a cutover.
The default production `temporal-worker` remains unchanged.

## Scope and proof boundary

`automation/temporal_history_transition.py` obtains **live** counts from
Temporal Server visibility using the official `temporal workflow count` CLI,
and a global-visibility **read-only** PostgreSQL inventory. It checks all
running and completed Workflows, all 17 recognized registered/retired
Workflow Types (including the four historical V1/V2/SEO names), and compares
their summed counts against an independent **whole-namespace count**. Any
unknown closed Workflow Type, incomplete visibility, or type-count drift
blocks the transition. The local test cross-checks every Rust
`register_workflow` declaration against the explicit inventory.

The PostgreSQL probe checks all nonterminal `pipeline.execution_runs`,
every unresolved `tenant_id` in
`pipeline.execution_runs`, `raw.pages`, `raw.sections`,
`extracted.rule_candidates`, `site.seo_hitl_tasks`,
`pipeline.pg_effect_v3_receipts`, and cross-owner source references.
The #94 `_0007` registry `pipeline.tenant_context_ownership` is authoritative:
the check joins run/candidate/admission/receipt context keys and verifies that
their tenant ownership matches registry entries and the bound execution run.
It also verifies section → page, candidate → source/evidence sections/pages,
and HITL → candidate ownership. Missing linked HITL candidate is blocked.

The audit DB role **must** have namespace-wide SELECT access and be
`BYPASSRLS`/superuser for inventory. Prefer a short-lived independent role
with SELECT grants and no DML. A normal RLS-restricted worker could see zero
rows outside its own tenant and create a dangerously false "drained" report;
the script therefore refuses its evidence. This privilege is for a
read-only DBA-controlled audit only, never the production PG effect Worker.

The actual source SHA must be the accepted exact clean checkout. This is
neither a GitHub green-CI check nor a signature verification. No credentials
are written to evidence: pass the PostgreSQL DSN by environment variable,
not in command-line arguments. Temporal CLI authentication/TLS is configured
by its normal trusted environment/connection settings.

## One typed read-only capture, twice

On a trusted operator host, after deploying the #94 tenant schema migration
and separately quiescing all old business starters through an approved,
observable mechanism:

```bash
export ALEGRIA_HISTORY_AUDIT_DSN='...from secret manager...'
SOURCE_SHA="$(git rev-parse HEAD)"
python3 automation/temporal_history_transition.py capture \
  --environment staging \
  --temporal-address temporal-staging.internal:7233 \
  --temporal-namespace alegria-staging \
  --source-sha "$SOURCE_SHA" \
  --output /secure/evidence/staging-first.json

# Wait at least 30 seconds, capture again with identical environment and SHA.
python3 automation/temporal_history_transition.py capture \
  --environment staging \
  --temporal-address temporal-staging.internal:7233 \
  --temporal-namespace alegria-staging \
  --source-sha "$SOURCE_SHA" \
  --output /secure/evidence/staging-second.json

python3 automation/temporal_history_transition.py compare \
  --earlier /secure/evidence/staging-first.json \
  --later /secure/evidence/staging-second.json \
  --output /secure/evidence/staging-drain-preflight.json
```

Replace names and `/secure/evidence` with an authorized operator-controlled
environment. These examples are **not** commands that were run on staging.
The captures fail closed on missing `tenant_id` schema, missing Temporal
visibility, credential errors, non-global PG audit role, stale or changing
open counts, mismatched tenant-linked raw sources, an unfinished run, or any
historical Workflow whose official SDK replay has not been established.

`DRAIN_CANDIDATE_ONLY` means one read-only snapshot saw no blockers.
`DRAIN_PRECONDITIONS_CAPTURED` means two independently captured, consistent
zero-open snapshots passed a 30-second to 30-minute window and freshness
check. Both always contain `"cutover_authorized": false`.

## Hard production NO-GO after a zero-open snapshot

The CLI deliberately cannot grant release authorization. Before any
production transition, complete **separately** and bind to the same source,
environment, namespace, Run IDs and owner/tenant records:

1. Independently verify old starter admission is frozen and no additional
   old Task Queue poller remains. Visibility counts alone race new starts.
2. Official Temporal SDK history replay for every type with retained history;
   `workflow count` is not a replay and cannot prove determinism.
3. Prove authoritative historical tenant backfill. Never infer a tenant from
   an untrusted request or silently assign a default tenant to `NULL` rows.
   Ambiguous rows remain quarantined and production effects blocked.
4. Prove server-verified parent/child history and exact build/Worker routing;
   verify the old Worker/build ID remains available for rollback and perform
   an independent recovery drill.
5. Verify production mTLS, one owner per Task Queue, signed exact-SHA Worker
   artifacts and operator-issued per-effect admission under RLS.

Cutover and rollback are *not* subcommands of this tool. A rollback means
restoring the previous registered Worker fleet/routing after appropriate
quiescence; editing immutable Temporal event histories or automatically
replaying external effects is prohibited.

Fast local contract suite:

```bash
python3 automation/test_temporal_history_transition.py
```

Official Temporal CLI reference: https://docs.temporal.io/cli/command-reference/workflow
