# V7 source-scoped pure Temporal image — deploy-only evaluation, NO-GO

The canonical `docker-compose.yml` **has not been changed** by this evaluation
path. Its default `temporal-worker` still receives PostgreSQL, Neo4j, Qdrant
and external provider credentials; it is a **NO-GO** for #94's production
physical isolation. This candidate is not a release, deployment migration,
replacement, or authorization to route production Workflow traffic.

## Two independent opt-ins

1. The operator explicitly loads
   `infra/temporal/compose.v7-isolated.yml` *in addition to* the canonical
   Compose. The overlay is absent from every default `docker compose up`.
2. The candidate requires profile `v7-production-isolation-evaluation`, a
   nonzero exact committed source SHA, the prepared source-scoped pure Cargo
   context and `V7_ENABLE_PURE_CANDIDATE=true`. With its gate unset it refuses
   to start; it does **not** fall back to the provider-linked legacy Worker.

For inspection only, after checking out a **clean exact committed Git SHA**:

```sh
SOURCE_SHA="$(git rev-parse HEAD)"
python3 automation/prepare_worker_build_context.py \
  --source-sha "$SOURCE_SHA" \
  --image-owner temporal-orchestrator-v3 \
  --destination /tmp/alegria-v7-pure-exact-sha

V7_SOURCE_SHA="$SOURCE_SHA" \
V7_PURE_BUILD_CONTEXT=/tmp/alegria-v7-pure-exact-sha \
V7_ENABLE_PURE_CANDIDATE=false \
docker compose -f docker-compose.yml \
  -f infra/temporal/compose.v7-isolated.yml \
  --profile v7-production-isolation-evaluation config
```

Do not use a dirty checkout, a generic `.` build context, a different Git
source SHA, or an ordinary production secret/DSN for this isolated build.
`prepare_worker_build_context.py` refuses a mismatched SHA or changed tracked
files and includes only the native `temporal_orchestrator` dependency closure.
`Dockerfile.v7-pure` can compile/copy **only** that pure executable, not the
legacy `temporal_worker`, PostgreSQL owner, Neo4j or Qdrant owners.

## Process and network boundary

- `temporal-pure-release-candidate`: non-root UID 65532, read-only rootfs,
  dropped Linux capabilities, no-new-privileges, dedicated `/tmp` tmpfs.
- Only `TEMPORAL_URL`, its namespace, **existing V5-only evaluation queue**,
  source SHA, explicit mode gates and logging enter the process environment.
  No mounted secrets, host directories, published ports, application database
  URL, Qdrant/Neo4j address or external provider token are present.
- The only attached network is `temporal_pure_control`, explicitly marked
  Docker-internal and shared with `temporal-server`; PostgreSQL/PgBouncer,
  Qdrant, Neo4j, Outbox and the legacy Worker have no attachment to it.
- The candidate intentionally reuses `alegria-effect-dag-v5`, the **currently
  supported native pure binary mode**. Do not start the separate V5 evaluation
  profile at the same time. A genuine V7 Task Queue requires a new checked
  native workflow registration, real Temporal History and a new attested
  versioned operator admission. This topology is not that proof.
- The Qdrant effect pilot's new dedicated worker, credentials and provider
  network are **not** added here. They require separate source build and real
  Temporal+PostgreSQL+Qdrant acceptance; parent isolation is not a substitute.

Read-only static gate:

```sh
python3 automation/check_production_boundary_v5.py
python3 automation/test_production_boundary_v5.py
python3 automation/check_temporal_docker_context.py
```

Expected: `v7_pure_isolation_candidate.boundary=PASS` alongside
`production_static_boundary=NO_GO` and
`production_release_authorized=false`. This proves only static Docker/Compose
shape, not runtime egress, image provenance, routing cutover or history replay.
