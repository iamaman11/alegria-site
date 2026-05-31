#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SYNC_PROTO = ROOT / "app" / "contracts" / "proto" / "sync.proto"
SEO_ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_seo_adapter.rs"
NEO4J_ADAPTER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "neo4j_materialization_adapter.rs"
OUTBOX_WORKER = ROOT / "app" / "rust" / "services" / "outbox_worker" / "src" / "materialize.rs"
PROJECTION_MATERIALIZER = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "projection_materialize_adapter.rs"
SEO_DOMAIN = ROOT / "app" / "rust" / "crates" / "seo_domain" / "src"
CI = ROOT / "automation" / "ci_verify.sh"

REQUIRED_GRAPH_ARTIFACTS = [
    "keyword_cluster",
    "serp_pattern",
    "page_blueprint",
    "page_node",
    "content_gap",
    "link_recommendation",
    "page_brief",
]

REQUIRED_NEO4J_LABELS = [
    "KeywordCluster",
    "SERPPattern",
    "PageBlueprint",
    "PageNode",
    "ContentGap",
    "PageBrief",
    "Intent",
]

REQUIRED_RELATIONSHIPS = [
    "TARGETS_INTENT",
    "PROPOSES_PAGE",
    "USES_BLUEPRINT",
    "HAS_CONTENT_GAP",
    "RECOMMENDS_LINK_TO",
]

REQUIRED_QDRANT_COLLECTIONS = [
    "seo_keyword_clusters_4",
    "seo_page_blueprints",
    "seo_serp_patterns",
    "editorial_topics_4",
]

FORBIDDEN_ACTIVE_QDRANT_COLLECTIONS = [
    '"seo_keyword_clusters"',
    '"seo_link_targets"',
    '"seo_draft_support_sections"',
]


def main() -> int:
    sync_proto = SYNC_PROTO.read_text(encoding="utf-8")
    seo_adapter = SEO_ADAPTER.read_text(encoding="utf-8")
    neo4j_adapter = NEO4J_ADAPTER.read_text(encoding="utf-8")
    outbox_worker = OUTBOX_WORKER.read_text(encoding="utf-8")
    projection_materializer = PROJECTION_MATERIALIZER.read_text(encoding="utf-8")
    seo_domain = (SEO_DOMAIN / "rebuild.rs").read_text(encoding="utf-8")
    ci = CI.read_text(encoding="utf-8")
    failures: list[str] = []

    for needle in [
        "message SeoGraphProjectionPayload",
        "map<string, string> metadata",
    ]:
        if needle not in sync_proto:
            failures.append(f"sync.proto missing `{needle}`")

    for artifact in REQUIRED_GRAPH_ARTIFACTS:
        if artifact not in seo_adapter:
            failures.append(f"SEO persistence adapter does not emit `{artifact}` projection events")
        if artifact not in neo4j_adapter:
            failures.append(f"Neo4j materializer does not handle `{artifact}`")

    for label in REQUIRED_NEO4J_LABELS:
        if label not in neo4j_adapter:
            failures.append(f"Neo4j materializer missing label `{label}`")
    for rel in REQUIRED_RELATIONSHIPS:
        if rel not in neo4j_adapter:
            failures.append(f"Neo4j materializer missing relationship `{rel}`")
    for collection in REQUIRED_QDRANT_COLLECTIONS:
        if collection not in seo_adapter:
            failures.append(f"SEO persistence adapter missing Qdrant collection `{collection}`")
    for collection in FORBIDDEN_ACTIVE_QDRANT_COLLECTIONS:
        if collection in seo_adapter:
            failures.append(f"SEO persistence adapter still emits legacy Qdrant collection {collection}")

    if "SeoGraphProjectionUpserted" not in seo_adapter:
        failures.append("SEO persistence adapter does not emit SeoGraphProjectionUpserted")
    if "projection_materialize_adapter::dispatch_event" not in outbox_worker:
        failures.append("outbox worker does not delegate projection dispatch to infrastructure materializer")
    if "SeoGraphProjectionUpserted" not in projection_materializer:
        failures.append("projection materializer does not dispatch SeoGraphProjectionUpserted")
    if "cmd.metadata" not in projection_materializer:
        failures.append("projection materializer does not pass Qdrant metadata payloads")
    if "dispatch_seo_graph_projection" not in projection_materializer:
        failures.append("projection materializer does not own SEO graph projection dispatch")
    if "materialize_seo_artifact" not in projection_materializer:
        failures.append("projection materializer does not materialize SEO graph artifacts through Neo4j adapter")
    if "classify_trigger" not in seo_domain:
        failures.append("seo_domain rebuild module is missing canonical rebuild trigger ownership")
    if "check_seo_projection_contract.py" not in ci:
        failures.append("ci_verify.sh does not run SEO projection contract gate")

    if failures:
        print("SEO_PROJECTION_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("SEO_PROJECTION_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
