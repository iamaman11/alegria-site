# Manual LLM mode — one canonical Temporal workflow

This is a **draft feature**, not a claim of production readiness. In the canonical
`SeoSiteBuildCanonicalCutoverWorkflow`, editorial generation can use this ChatGPT
conversation without an API key. The previous deterministic and API provider
adapters remain available. No extra container, HTTP server or secondary workflow
is introduced.

## Configure

Apply `app/db/migrations/20261109_0001_manual_llm_jobs.sql` once to an
existing PostgreSQL database (the initial `app/db/schema.sql` contains the
same idempotent definition for new databases). Keep the ordinary Temporal worker
and all production gates running.

Set `SEO_LLM_PROVIDER=chat_manual` on the **Temporal worker** (not the CLI).
When the canonical workflow reaches editorial generation, the existing activity
persists an immutable typed input and returns `awaiting_manual`. Temporal
durably waits; it never substitutes generated content or an empty success.

Build and run the **existing** Rust CLI from `app/rust`:

```bash
cargo run -p cli_tools -- llm-next
```

It prints either `NO_PENDING_LLM_JOBS` or a JSON job with
`task_id`, `schema_version`, `input_sha256`, `operation` and `payload`.
Pass `payload.request` and supporting verified source material to the
LLM for editorial section generation. Preserve the job IDs unchanged.

A response file has exactly this envelope (replace the placeholders and choose
**actual section roles** listed in the job's editorial brief):

```json
{
  "task_id": "<from llm-next>",
  "schema_version": 1,
  "input_sha256": "<from llm-next>",
  "response": {
    "sections": {
      "<actual-role>": "Markdown grounded in the verified supporting evidence"
    }
  }
}
```

Submit it with:

```bash
cargo run -p cli_tools -- llm-submit --response-file response.json --model-key gpt-6
```

`DATABASE_URL` and `TEMPORAL_URL` use the existing environment variables.
CLI also accepts `--database-url`, `--temporal-url` and
`--temporal-namespace` as needed. Connection requires access to the
same database and Temporal namespace as the canonical worker. ChatGPT cannot
be invoked autonomously as a service through a consumer subscription; this is
a supervised exchange via existing chat tools.

The command validates the envelope and section output using the existing Rust
editorial generator/overlay, persists the result, and sends the dedicated
`llm_result_ready` Temporal signal. If signaling fails, the submitted result
remains committed; re-running the **identical** command safely re-signals it.
A different answer or model for a completed task is rejected.

The signal **never** approves publication. The existing editorial normalization,
content validation, draft QA, CMS review and distinct human approval path remain.

## Manual truth proposals — the same PostgreSQL inbox

For supervised candidate extraction set `SEO_TRUTH_LLM_PROVIDER=chat_manual`
on the canonical Temporal worker. This setting is independent of
`SEO_LLM_PROVIDER=chat_manual` for editorial generation; either or both
operations may use manual mode. There is no new server or workflow.

The existing canonical workflow prepares one `truth_candidates` task for
each admissible persisted raw section **after** semantic/sectioning gates and
**before** candidate validation. `llm-next` returns the oldest pending task
across both operations. For truth tasks the `payload` includes exact
`raw_text`, `raw_section_id`, source URL, context and source snapshot hash.
The ChatGPT response must refer to this exact source, not to memory.

Submit using the same `llm-submit` CLI, envelope and model key. A truth
response has this strictly typed shape:

```json
{
  "task_id": "<unchanged task_id>",
  "schema_version": 1,
  "input_sha256": "<unchanged input_sha256>",
  "response": {
    "candidates": []
  }
}
```

The empty array is only valid when no supported claim can be extracted.
When claims exist, each entry must use the existing
`TruthRuleCandidate` contract: `role`, `concept_canonical_key`,
`raw_mention`, `params`, `scope`, `severity`,
`applies_to_profiles`, `alternatives`, `derivation_type`,
`is_numeric`, `is_range`, `is_incomplete`, `confidence`,
`evidence_section_id`, `evidence_quote`, `span_start`,
`span_end`, `uncertainty_flags` (plus optional
`conditions_raw`, `exceptions_raw` and `modality_raw`).
All spans are **UTF-8 byte offsets** into the provided `raw_text`, not
Unicode character positions. `evidence_quote` must match the exact
substring; `evidence_section_id` must match the original section. Unknown
keys such as `verified` or `approved` are rejected.

Only complete, validated results become candidates in the original
`candidate_validation` stage. The actual source URL/tier and snapshot are
read back from the application's persisted raw evidence and are **never**
taken from ChatGPT. The existing ontology/schema, uncertainty, governance,
corroboration, contradiction/adjudication, verified truth write and release
gates remain mandatory. The Temporal `llm_result_ready` signal is a
**wake-up only**, not an approval.

## Acceptance status and limitations

- PostgreSQL queue semantics (identity, exact evidence, duplicates, wrong
  hash, unknown approval fields, restart/reconnect) and actual Temporal
  restart are tested in the existing integration harness. Do not claim
  current HEAD passes until its exact CI runs finish.
- This is **candidate extraction**, not independent confirmation of facts.
  Empty answers, withheld source claims and unavailable required external
  providers cannot be promoted to verified knowledge.
- Live DataForSEO / Voyage, scope-specific real provenance, completeness of
  actual graph/retrieval projections, human CMS approval, production origin
  and independent HTTPS readback are still required for first-site GO.
- Older in-flight workflow histories are not claimed replay-safe when the
  canonical command history changes. Drain/version workers using the
  existing release controls before deployment.

## Supervised first-site certification (not unattended R5)

The automated three-scenario R5 certificate still **requires an API LLM provider**.
For the **first actual site only**, the existing certification script accepts
an explicitly supervised manual path:

```bash
python3 automation/e2e_supersite_certification.py --first-site --manual --dry-run
python3 automation/e2e_supersite_certification.py --first-site --manual
```

Both `SEO_LLM_PROVIDER` and `SEO_TRUTH_LLM_PROVIDER` must be `chat_manual`
on the operator host **and** in the canonical Temporal worker. Real DataForSEO
and Voyage credentials, the dedicated Cloudflare account, HTTPS origin, public
output path and explicit `ALEGRIA_CERT_PUBLISH=1` remain mandatory; never
insert placeholder credentials. The second command **waits** for the original
Temporal workflow to complete. Keep it in a separate operator terminal while
processing `llm-next` / `llm-submit` tasks. The worker image includes the
existing `cli_tools` binary. The real CMS HITL decision is a separate human action.

A stopped operator terminal does not cancel durable Temporal jobs. Before
starting again, inspect the original run and outstanding manual/HITL tasks;
do not blindly start another first-site run. Only after that workflow has
actually completed does the original certification script verify its immutable
release, all declared HTML files and independently served HTTPS bytes.
A supervised ChatGPT response is **not** a human approval, and no synthetic
facts can satisfy the final public gate.

## Native Windows first-site operator — no WSL

Use the existing Windows PowerShell, Python and Docker Desktop **Hyper-V** backend.
No WSL shell, second Docker project, new server or extra workflow is required.
Run from the exact GitHub checkout whose SHA passed both CI jobs and was
deployed through `infra/local/windows_staging.ps1`.

The certification script uses the current directory's `docker compose exec`,
so it must address the **existing** Compose project rather than silently
looking for another project named after the checkout directory. Resolve
the project label from the running business PostgreSQL container:

```powershell
Set-Location 'B:\Alegria\runtime\alegria-site-<CI-green-full-SHA>'
$env:COMPOSE_PROJECT_NAME = (
  docker inspect alegria_postgres --format '{{index .Config.Labels "com.docker.compose.project"}}'
).Trim()

$env:SEO_LLM_PROVIDER = 'chat_manual'
$env:SEO_TRUTH_LLM_PROVIDER = 'chat_manual'
$env:CLOUDFLARE_ACCOUNT_ID = '78e5fcb3762c1473113d51838556da74'
$env:ALEGRIA_CERT_BASE_URL = 'https://alegria-seo-staging.alegria-site.workers.dev'
$env:SEO_STATIC_BASE_URL = $env:ALEGRIA_CERT_BASE_URL
$env:ALEGRIA_CERT_PUBLIC_OUTPUT_DIR = '/app/app/rust/dist/static-site'
$env:ALEGRIA_CERT_PUBLISH = '1'   # explicit supervised publication request

python .\automation\e2e_supersite_certification.py --first-site --manual --dry-run
```

Use the *actual* installed checkout SHA; the angle-bracket placeholder above
is not a runnable path. The preview is read-only and must report `BLOCKED`
until the operator and running worker both have real DataForSEO credentials,
Voyage credentials, and Cloudflare deployment capability. Provider secrets
must be supplied privately outside GitHub; do not paste them in chat or
write them to tracked source files. The dry-run verifies only configured
presence, not provider authentication or completed publication.

The native read-only doctor can be run **before** credentials exist:

```powershell
python .\automation\first_site_doctor.py --json --expected-sha <installed-40-char-SHA>
```

Its `PROVIDER_CONFIG_BLOCKED` outcome is expected until real keys and the
Cloudflare API token work. Provider presence is **not** proof of authentication,
and even `CONFIG_PRESENT_UNVERIFIED` is never publication acceptance.
The report includes only Boolean secret presence, safe HTTP status codes,
real service state and counts; it does not print provider tokens.

When every prerequisite and independent provider capability is confirmed,
choose one persistent **runtime receipt directory outside the checkout**
(including when moving to a newer GitHub commit):

```powershell
$env:ALEGRIA_FIRST_SITE_SESSION_DIR = 'B:\Alegria\state\first-site-sessions'
python .\automation\e2e_supersite_certification.py --first-site --manual
```

The first-site operator prints `FIRST_SITE_RUN_ID=<uuid>` **before** starting
the long-running Temporal wait. A private receipt with status
`WAITING_FOR_RESULT` survives terminal closure and new exact-commit checkouts.
A second plain start from the same persistent receipt directory is
**rejected**, rather than manufacturing a second site workflow.

If the terminal closes while ChatGPT or the separate human CMS decision is
pending, use the **same** environment and the previously printed run UUID:

```powershell
python .\automation\e2e_supersite_certification.py --first-site --manual --resume-run-id <existing-run-uuid>
```

This uses the existing `temporal_starter workflow-wait --workflow-id` CLI
method to await the old workflow result. It never calls `start_workflow`,
never re-registers a site scope and still runs independent public HTTPS
checks. Do **not** use `start` to recover; validate the source
`WORKER_BUILD_ID` and exact-head CI before deploying the new CLI.
If the private receipt is unavailable, recover the exact workflow UUID
from Temporal/PostgreSQL evidence; never invent or guess one. The default
receipt directory (when the environment override is omitted) is
`~/.alegria/first_site_sessions`; do not change it mid-run.


In a second Windows PowerShell terminal handle pending typed tasks through
the existing running container:

```powershell
docker exec alegria_temporal_worker cli_tools llm-next
docker exec alegria_temporal_worker cli_tools llm-submit --help
```

Submit only real, validated response envelopes through the existing CLI.
The first command can validly return `NO_PENDING_LLM_JOBS`. The original
Temporal workflow must be kept running until the separate CMS human approval,
immutable release check and independent public HTTPS readback complete.
An empty manual queue or green CI is not proof that a site was published.

## Required deployment compatibility checks

Do not upgrade the canonical Temporal worker with pre-existing in-flight
`SeoSiteBuildCanonicalCutoverWorkflow` histories unless workflow-history
compatibility has been demonstrated against the candidate worker version.
The CLI acceptance test proves a **process restart of the same build**,
not a replay across different workflow command histories. First quiesce
and inspect existing runs, confirm no waiting/manual or HITL executions
remain, then deploy the updated worker and apply the existing migration.
Changing `WORKER_BUILD_ID` alone is not proof of safe versioning.

## Scoped fact safety boundary

The canonical truth workflow supports one explicitly typed scoped predicate:
`{"applicant_profile":"student"}` (or another active registered profile key).
Independent source governance and evidence validation remain mandatory.
When verified, the rule is saved under a distinct semantic identity and
bound through the existing `verified.rule_instance_profiles` table. SEO
support is returned only for the exact requested profile; unscoped read APIs
exclude such rules. Until the global `verified_rules_4` vector index has
a certified applicant-profile filter, scoped rules are withheld from that
unfiltered index. Unresolved profile conditions or exceptions block use.

Any extra `scope` key or malformed profile, plus nonempty
`applies_to_profiles`, `conditions_raw`, `exceptions_raw`, `alternatives`
or a nonempty `params.conditions_key`, remains **needs_hitl**. The persisted legacy extraction path still
conservatively blocks nonempty `scope` until its separate semantics are
migrated. General conditional DSL evaluation, overlapping global/profile
truth, independent second-niche certification and physical production
readback remain outstanding; this is not full launch approval.

### Native Temporal pinned Worker versions

The canonical Worker supports Temporal Rust SDK's built-in Worker Deployment
Versioning (no second workflow or service). To enable a **new** pinned
version after the initial migration, configure
`TEMPORAL_WORKER_VERSIONING=true`, `TEMPORAL_DEPLOYMENT_NAME=alegria-site`,
and `WORKER_BUILD_ID=<unique-immutable-build-sha>`. Placeholder Build IDs
(`docker-dev`, `dev-local`) are rejected by the Worker. Its default
workflow behavior is `Pinned`, so executions started on that deployment
version stay assigned to compatible code. Activate/ramp the new deployment
version with **the standard Temporal Worker Deployment API/CLI**; starting a
new poller does *not* by itself certify or switch the current version.
Keep previous pinned workers available until their runs complete or are
migrated through a separately validated Temporal mechanism.

This option does not retroactively pin historical **unversioned** workflow
histories. Before first activation inspect and drain those executions using
Temporal's real workflow list/history; do not infer quiescence merely from
PostgreSQL counts. Development without the versioning flag keeps the existing
unversioned behavior for backward-compatible tests.
