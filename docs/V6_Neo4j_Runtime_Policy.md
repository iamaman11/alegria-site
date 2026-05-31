# V6 Neo4j Runtime Policy

Status: active policy owner for Neo4j runtime contract in canonical cutover mode.

## 1. Contract-required flags

Canonical runtime and production gate must enforce:

- `GRAPH_CAPABILITY_REQUIRED=true`
- `NEO4J_SYNC_REQUIRED=true`
- `GRAPH_QUERY_REQUIRED=true`
- `GRAPH_GDS_REQUIRED=true`

When the graph contract is required, `seo_preflight` must hard-block workflow start unless graph status is `pass`.

## 2. Required graph projections

Per-projection existence, freshness, and completeness are mandatory:

- `keyword_cluster`
- `serp_pattern`
- `page_blueprint`
- `page_node`
- `content_gap`
- `link_recommendation`
- `page_brief`

Projection lag and completeness are evaluated in preflight and production gate evidence.

## 3. Block taxonomy

Allowed graph contract outcomes:

- `pass`
- `warn` (only when graph contract is not required)
- `blocked_provider_capability`
- `blocked_missing_projection`
- `blocked_stale_projection`
- `blocked_projection_incomplete`
- `blocked_graph_contract`

Machine-readable reports must include `graph_contract_status` and `graph_block_reason`.

## 4. Authority boundary

Neo4j is a graph/reasoning support plane and projection plane.

Neo4j must not:

- mint or upgrade verified truth directly;
- bypass `candidate_validation` / `truth_adjudication`;
- mutate publish authority verdicts on its own.

Truth authority remains in deterministic truth-core steps.
