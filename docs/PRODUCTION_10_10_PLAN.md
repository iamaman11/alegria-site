# Alegria: Production 10/10 execution plan

Status: execution in progress
Owner: repository maintainers
Target: real public production use, measurable outcomes, safe repeatable operation

## Definition of 10/10

The project is 10/10 only when all of the following are simultaneously true:

1. Public output is launch-safe: real HTTPS origin, correct locale, human-readable routes/titles, no internal IDs or relation-role labels, valid canonical/sitemap/robots/schema markup, and only `published` content is indexable.
2. Truth pipeline produces admissible verified facts from real sources. A transport-level success with zero admissible facts is not a product success.
3. Retrieval and graph contracts are materialized, fresh and non-empty for the launch scope.
4. A clean staging run passes SERP -> official crawl -> extraction -> verification -> retrieval/graph -> planning -> draft -> validation -> HITL -> publish -> rendered-output validation.
5. Every merge is protected by automated formatting, linting, architecture, unit/integration, security and build gates.
6. Production is observable and recoverable: metrics, classified failures, backup/restore drill, immutable evidence and rollback path.
7. The product closes an outcome loop: freshness, coverage, indexability, search performance and user conversion are measured.

## Execution phases

### P0. Public-output safety

- [x] Make static export publish-only. `approved` is not deployable/indexable.
- [x] Remove internal link-role labels from user-visible related links.
- [x] Add a public static-output contract checker that rejects placeholder origins, `lang=und`, opaque SHA-like public routes/titles, non-published manifest entries, broken canonical/sitemap origin consistency and internal role leakage.
- [ ] Configure the real production origin through `ALEGRIA_SITE_BASE_URL` in the deployment environment.
- [ ] Regenerate public pages with human-readable slugs/titles and correct locale.
- [ ] Run the public-output contract against the regenerated production artifact.

Exit criteria: public artifact contains no placeholder/test identity or internal implementation identifiers.

### P0. Product-level truth certification

- [x] Tighten supersite E2E certification so an `empty:*` result or zero generated pages cannot be called PASS.
- [x] Require published pages when a certification scenario requests publish.
- [ ] Obtain a real launch-scope run with admissible verified rules and complete required facts.
- [ ] Make critical official-source crawl failures blocking for the launch scope.

Exit criteria: at least one real launch scope produces complete verified facts and a usable page without seeded truth.

### P0. Retrieval and graph readiness

- [ ] Materialize every required Qdrant collection for the launch scope.
- [ ] Materialize every required Neo4j projection for the launch scope.
- [ ] Prove freshness/point-count/projection-completeness in immutable staging evidence.

Exit criteria: retrieval and graph contract gates PASS with real non-empty state.

### P0. Clean staging certification

- [x] Preserve the existing three-scenario certification model: fresh scope, scope expansion, factual-change rebuild.
- [ ] Run it on a clean staging environment with real provider credentials and the production-like base URL.
- [ ] Do not use synthetic verified-rule seeds or automatic HITL approval as evidence of public-launch readiness.
- [ ] Validate the rendered artifact with the public-output contract before deployment.

Exit criteria: one immutable certification manifest proves the whole path.

### P1. Merge and supply-chain governance

- [x] Add a GitHub Actions quality gate for PRs and main.
- [x] Enforce Rust formatting, workspace compilation/tests, architecture audit and RustSec audit in CI.
- [ ] Protect `main` with required status checks and required review in repository settings.
- [ ] Add secret scanning / push protection and dependency policy in repository settings.
- [ ] Pin third-party CI actions by immutable commit SHA when introducing any.

Exit criteria: unverified code cannot reach `main`.

### P1. Reliability, security and recovery

- [ ] Separate dev/staging/prod credentials and remove default credentials from any production path.
- [ ] Run services with least privilege and non-root containers where applicable.
- [ ] Produce an SBOM and scan release containers/dependencies.
- [ ] Encrypt and retention-manage backups; record RPO/RTO and restore-drill evidence.
- [ ] Exercise provider/DB/Qdrant/Neo4j/Temporal fault scenarios and verify fail-safe behavior.

Exit criteria: a failed dependency cannot silently publish unsafe content and recovery is demonstrated.

### P1. Product outcomes

- [ ] Define launch KPIs: verified-fact completeness, freshness SLA, crawl success, publish latency, index coverage, impressions, clicks, CTR, average position and user conversion.
- [ ] Connect GSC/analytics feedback to release evidence and dashboards.
- [ ] Add a launch scorecard with blocking thresholds, not vanity metrics.

Exit criteria: the system proves not only that it runs, but that it achieves the intended user/business outcome.

## Required release verdict

A release is `GO` only when all P0 exit criteria are green. A transport smoke PASS, successful process exit, or architecture-only gate is insufficient by itself.

## Current known external blockers

These require deployment/runtime information and cannot be invented in source control:

- real production domain/origin;
- production/staging provider credentials;
- real launch-scope Qdrant/Neo4j materialized state;
- real source data sufficient to produce admissible verified facts;
- GitHub branch-protection/security settings.

The repository must fail closed when these are absent from a real launch certification.
