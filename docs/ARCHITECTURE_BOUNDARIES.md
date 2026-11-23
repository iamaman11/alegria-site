# Alegria: owners and Rust boundaries (#43)

This is the short source-linked map for the ongoing modularization. The five
capabilities below are *responsibilities*, not five mandatory processes or five
mandatory crates. A target marked **planned** is not yet an implemented API.
The current dependency graph is evidence of coupling, not the desired design.

| Owner | Public Rust API and state | Allowed dependencies / current migration |
| --- | --- | --- |
| Core | `primitives::{stable_id,source_key,page_key,concept_key,context_key,hash}`: shared IDs and value objects; no mutable domain state | Shared data types and deterministic libraries only. `contracts`, `scraper`, `regex`, `serde` and editorial writer/validator/SEO-identity dependencies removed from Core. Dormant legacy fact-extraction sources removed. **Planned:** review storage-specific ID utilities. |
| Acquisition | `acquisition::html_sections::{extract_meta_typed,extract_sections_typed,HtmlSection}` and `acquisition::pdf_parser`: parse source documents without judging their truth | `primitives` for stable hashes, parser libraries. Existing fetch/store effects remain in `infrastructure::adapters::raw_crawl_adapter` until their own adapter-boundary slice. No dependency on Truth, SEO or Publishing. |
| Truth / review | **Implemented:** `truth_domain::{truth_candidates,fact_verifier,truth_governance,contradictions,review_decisions,evidence,procedural_candidates,persisted_candidates,semantic_truth_write}` owns validated candidates, governance, contradictions, scoped review veto, evidence, procedural mapping, persisted eligibility/canonical selection and per-semantic-slot Truth write decisions. CLI retains its JSON transport wrapper. **Planned:** remaining review operations | Temporal orchestrates source-registry reads and SQL/projection writes and now consumes a typed Truth write outcome; `infrastructure` retains SQL persistence and JSON conversion. Obsolete regex/keyword fact-extraction implementations are deleted. |
| Knowledge / retrieval | `seo_ports::knowledge` defines typed verified-rule, canonical-vector, diagnostic-neighbor, editorial source-context and verified-rule projection contracts. `seo_application::knowledge` owns their use cases, including `reconcile_verified_rule_projection`. PostgreSQL verified rules alone authorize factual support; Qdrant hits are advisory. | SQLx reads authoritative Truth. `VerifiedRuleProjectionAdapter` owns Voyage embedding, Qdrant deletion and atomic PostgreSQL point-metadata+Outbox staging; the actual Qdrant upsert remains asynchronous Outbox delivery. Temporal only passes Truth's prevalidated global-rule delta. Source-context selection policy stays in Knowledge. Remaining projection operations are planned. |
| SEO / editorial | `seo_domain::{identity,artifact_identity,writer,block_validator,review_decision}` owns pure scope validation, artifact keys, content-gap detector identity, deterministic HTML, block QA and typed human review/approval decisions; `seo_application` owns content operations. **Planned:** remaining draft/revision decisions | `seo_steps::seo_identity` maps Temporal's `SeoScopePayload`; `seo_steps::cms_review_wire` is the one conversion between legacy Temporal `CmsApprovalDecision` and typed `CmsApproval`. `seo_domain` has no `contracts` dependency. `CmsReviewDecisionPort` accepts typed `HumanReviewDecision` and `CmsReviewPort::load_latest_approval_decision` returns typed `CmsApproval`; the SQL adapter alone handles persistence. Remaining `seo_steps`/`seo_ports` transport coupling requires later slices. JSON decoding stays in adapters. |
| Publishing | **Implemented:** `seo_domain::publishing::{ReleaseReadiness,PublicReleaseOutcome,plan_recovery}` and `seo_ports::PublicReleaseDelivery`; `seo_steps::finalize_publish_step` computes readiness, while `infrastructure::adapters::sqlx_seo_cms_adapter` owns external release effects and PostgreSQL/Outbox transaction. **Planned:** remaining editorial revision transitions | Files/SQL/Cloudflare/HTTP are effects, not Truth decisions. Editorial approval, release readiness and verified publication are distinct. Isolated PostgreSQL/HTTP A17 passed; no real public release was performed. |
| Runtime / composition | Existing `services/{temporal,outbox_worker,cli_tools,...}`: wiring, scheduling, retries, telemetry. Outbox's typed `DispatchError` belongs to the concrete projection operation; SQLx claims carry run/lease identifiers | Runtime logs `run_id`, target, event and stable class/stage; status updates are lease-fenced, and terminal failure and dead-letter insertion share one PostgreSQL transaction. A14 live-container verification is required before acceptance. Domain/application must not import runtime or implementation crates. |

### Rules for every code slice

- A capability owns its state transitions and output types; another capability
  reads the public result, and requests changes through the owner's operation.
  Cross-capability flows are composed in Application, without cyclic imports.
- Domain decisions are ordinary testable Rust functions; external effects are
  isolated at actual ports. Do not introduce generic storage or plugin layers.
- Move the operation *and all active callers/tests* together. No deprecated
  re-export or parallel implementation. The source-controlled Cargo DAG and
  public module signatures must reflect the actual boundary.
- Preserve persisted identifiers/provenance and active workflow data while
  changing code ownership. A historical identifier is not a Rust API alias.

**Working slices:** HTML/PDF acquisition parsing moved from Core into
`acquisition`, with active crawl consumers and tests. Truth extraction,
governance and write outcomes now belong to `truth_domain`; SQL effects stay
in adapters. Editorial HTML rendering and block validation moved from Core
into `seo_domain`, with the active Temporal caller and Rust migration guard.
Core no longer pulls HTML parser or serialization libraries for editorial QA.
SEO identity no longer imports Temporal payloads or compiles the `contracts`
crate; the existing module-boundary check now guards direct dependency edges,
their transitive heavy SDKs and leaked fully qualified transport imports in
the pure domain modules. This does not yet certify all `seo_ports` contracts.
SEO identity and the first Publishing ownership slice are implemented, with
isolated A17 integration evidence; further editorial transitions remain.
The Outbox A14/A18 slice covers correlation, typed dispatch errors, lease
fencing and atomic dead-letter persistence; its PostgreSQL/Qdrant replay
acceptance passed in the isolated CI. The first Knowledge read boundary now
projects verified rules from PostgreSQL into pure Rust writer data via a typed
port. A13 PostgreSQL acceptance is selected by the shared CI impact plan;
retrieval projections remain non-authoritative.
The A13 Knowledge reader and pure writer projection now reject incomplete
numeric fee/timeline facts with a typed contract violation, including the
affected rule and run identifier; an admissible PostgreSQL record alone must
not cause an omitted amount/currency/duration to become `0` or `EUR`.
Canonical vector resolution and Truth diagnostic neighbors also use typed,
replaceable Knowledge ports, with real Qdrant/A13 evidence. Editorial
source-context retrieval now owns its selection policy in Knowledge rather
than the SQLx-named adapter; the active SEO drafting path converts typed
results to Temporal wire DTOs only after the Knowledge operation returns.
Truth's active verified-rule write now hands a typed delta to
`seo_application::knowledge::reconcile_verified_rule_projection`. The
Voyage/Qdrant/SQLx implementation is replaceable, and Qdrant point metadata
and corresponding Outbox intent stage in one PostgreSQL transaction. The
deduplicated Outbox transport, not an immediate direct write, owns projection
materialization; a demoted point's deletion remains an explicit Qdrant effect.
The active CMS human-review decision now parses the external decision string
once in Application, passes `HumanReviewDecision` through its existing port,
and uses the same `seo_domain::review_decision` policy in the SQL adapter.
The adapter alone checks the exact pending PostgreSQL revision and active
Temporal workflow and commits the decision and HITL queue state together.
The existing disposable PostgreSQL revision-bound E2E is selected by the
shared `cms_review_revision` suite owner for future changes to this path;
this does not imply all editorial approval/revision operations are migrated.
The same PostgreSQL readback now passes through the typed approval port and
Application; `CmsApproval::approves_revision` is the sole human actor/revision
eligibility policy, used by the publish adapter and scenario. The wire mapper
in `seo_steps` is used by both Temporal activities and the existing legacy
scenario input; malformed persisted decision values fail closed. No deprecated
protobuf read port or duplicate approval predicate is retained.
The CLI now parses the external review decision before any SQL/Temporal I/O;
the Application input is a `HumanReviewDecision`, and it returns the port's
typed `CmsReviewDecisionOutcome` directly, without another equivalent result
DTO. Fake-port unit tests prove that invalid human/page requests cause no
storage or workflow calls; the CLI binary test proves unsupported strings are
refused before a database connection.

### Shared change-impact owner registry (in progress)

`automation/ci_impact.py` is the common local/CI planner. It derives Cargo
reverse dependencies from `cargo metadata --locked --no-deps` and reports
`base_sha`, `head_sha`, `scope` (`none`, `scoped`, `full`), affected crates,
selected real E2E suites, selected images, selected PR job owners and an
auditable reason for each. Its `ci_job_status` records every owner as
`required` or `not_applicable` for the exact evaluated source SHA.
The Rust-backed suite runner consumes the named suite commands from that same
module; it does not maintain an independent list of A13/A14/A17 test filters.
Focused Temporal worker restart, Neo4j query and Qdrant materialization suites
are also selected for their actual source owners. These longer suites are not
silently counted as run on unrelated changes.
For the selected Temporal restart suite the same registry first builds real
`temporal_worker`/`outbox_worker` debug binaries into the **already warmed**
Cargo target, then passes their absolute paths through the harness's existing
`INTEGRATION_*_WORKER_BIN` variables. The test still starts, interrupts and
resumes real processes; it no longer launches a second Cargo compilation tree.

Non-executable Markdown and historical `docs/runs` reports do not select a Rust
compile or image. Policy Markdown directly read by acceptance scripts still
selects Product/Truth and source verification; explanatory Markdown needs no
owner beyond the mandatory impact selector and aggregate CI gate. A live
`integration_harness/fixtures/visa_truth_certification`
input selects the actual Temporal/Truth certification E2E, not an archived PASS.
The fixture-only fast path resolves modified `.html` names through every live
JSON manifest's `source_inputs.pages[].html_file`, selects all consuming
`fixture_id`s and passes those IDs via the harness's existing
`TRUTH_CERT_FIXTURE_IDS` input. A manifest change, unreferenced/renamed HTML or
incomplete mapping keeps the **entire** 16-fixture certification suite. The
separate Temporal restart/manual-editorial E2E reads the canonical
`happy_path_publish_allowed` fixture too; that ID (or an unresolved fixture
mapping) also selects its real worker-restart test and prebuilt binary preparation.
Other HTML fixtures do not trigger the restart test without a live consumer.
The runner rejects externally supplied `TRUTH_CERT_LIMIT` or fixture IDs so that
an accidental environment filter cannot weaken canonical CI. Runtime fixture
files do not modify a compiled Rust unit: the selected E2E compiles the
required harness, without a preceding redundant scoped Cargo check/test.
The Truth and Temporal restart E2Es both use the same prebuilt real
`temporal_worker`/`outbox_worker` binaries in the existing Cargo target; the
runner prepares that shared pair once even when both suites are selected.
This removes the measured 188.7s nested-worker compilation from the prior
full Truth canary without replacing its actual PostgreSQL/Temporal processes.
This optimization is **not** evidence that the full 16-fixture suite ran when
only one ID is selected; the exact scope is reported in `suite_fixture_ids`
and printed as `CI_SUITE_FIXTURE_IDS` in the Rust job.
The Cloudflare static tree and the four Python scripts COPY'd into the Temporal
image select its packaging even if no Rust crate changed. The `worker-image`
release workflow now checks this report against its exact source commit and
records `not_applicable` for unrelated changes instead of building that image.
Local feedback shows the same selections, but defers external Docker/provider
E2E and all release packaging to canonical CI; it cannot be accepted as CI PASS.

The PR workflow now runs the source-bound `impact` job first, then schedules
only the named owners. Every skipped owner is visible as `skipped`, never as a
test PASS. The `CI` aggregate requires all nine known owner results: `success`
for selected jobs and `skipped` for explicitly non-applicable jobs, plus an
accepted impact plan tied to the exact PR workflow SHA. Missing jobs, missing
plans, a wrong SHA, unexpected execution/skips or a failed selector all fail
the aggregate. The `main` merge provenance check still requires an accepted
exact-source PR `impact` and `CI` result and the complete owner result set.
The mandatory selector also runs the existing documentation layout contract,
so a docs-only PR cannot hide an invalid nested docs tree by skipping `verify`.
Unknown/global impacts and Dockerfiles without a narrow verified build owner
retain all nine owners. This is source-policy acceptance, not certification
that every selected image was built, deployed or smoke-tested.

**Remaining:** known Cargo.lock/protobuf inputs have scoped exact-SHA canary
evidence (Issue #43 A6), while unknown schemas still fail closed to `full`.
Full editorial decision ownership, broader Temporal/graph coverage and
complete signed/no-build release provenance still require separate evidence.
PR #44 was merged at `d95927e`; follow-up regression PR #56 was merged at
`498a866`. Keep Issue #43 open until the remaining criteria are proven.
