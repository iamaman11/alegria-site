#!/usr/bin/env python3
"""Offline Cargo/source boundary guard for independent services and capabilities."""

from __future__ import annotations

import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / "app" / "rust"
SERVICE_ALLOWED = {
    "gsc_sync": {
        "anyhow", "oauth2", "serde_json", "tokio", "tracing",
        "tracing-subscriber", "http_client",
    },
    "analytics_svc": {
        "anyhow", "tokio", "tonic", "tonic-prost", "prost",
        "tracing", "tracing-subscriber", "graph_neo4j",
    },
}
ADAPTER_ALLOWED = {
    "graph_neo4j": {"anyhow", "neo4rs", "tokio"},
    "http_client": {"anyhow", "reqwest"},
    "vector_qdrant": {"anyhow", "qdrant-client", "reqwest", "serde_json"},
}
CAPABILITY_FORBIDDEN_DIRECT = {
    "acquisition": {"contracts", "infrastructure", "truth_domain", "seo_domain", "seo_steps", "seo_application"},
    "truth_domain": {"contracts", "infrastructure", "policies", "primitives", "seo_domain", "seo_steps", "temporal_worker"},
    "seo_domain": {"contracts", "infrastructure", "runtime_models", "seo_ports", "seo_steps", "temporal_worker"},
    "policies": {"truth_domain"},
}
CAPABILITY_FORBIDDEN_IMPORTS = {
    "acquisition": ("contracts::", "infrastructure::", "truth_domain::", "seo_domain::", "temporalio_sdk::"),
    "truth_domain": ("contracts::", "infrastructure::", "seo_domain::", "sqlx::", "temporalio_sdk::"),
    "seo_domain": ("contracts::", "infrastructure::", "runtime_models::", "sqlx::", "temporalio_sdk::"),
}
FORBIDDEN_TRANSITIVE = {
    "gsc_sync": {
        "infrastructure", "sqlx", "neo4rs", "qdrant-client", "temporalio-sdk",
        "playwright-rs", "rig-core", "rig-vertexai",
    },
    "analytics_svc": {
        "infrastructure", "sqlx", "qdrant-client", "temporalio-sdk",
        "playwright-rs", "rig-core", "rig-vertexai",
    },
    "truth_domain": {"contracts", "sqlx", "neo4rs", "qdrant-client", "temporalio-sdk", "prost", "tonic"},
    "seo_domain": {"contracts", "sqlx", "neo4rs", "qdrant-client", "temporalio-sdk", "prost", "tonic"},
}


def forbidden_source_references(source: str, tokens: tuple[str, ...]) -> list[str]:
    """Detect transport/runtime names even in fully qualified Rust signatures."""
    return sorted(token for token in tokens if token in source)


def lock_dependencies(lock_packages: list[dict], root_name: str) -> set[str]:
    """Conservative closure: include every locked version of a package name."""
    index: dict[str, list[dict]] = {}
    for package in lock_packages:
        index.setdefault(package["name"], []).append(package)
    seen: set[str] = set()
    pending = [root_name]
    while pending:
        name = pending.pop()
        if name in seen:
            continue
        if name not in index:
            raise ValueError(f"missing package in Cargo.lock: {name}")
        seen.add(name)
        for package in index[name]:
            for dependency in package.get("dependencies", []):
                pending.append(dependency.split(" ", 1)[0])
    return seen


def main() -> int:
    manifest = tomllib.loads((RUST / "Cargo.toml").read_text(encoding="utf-8"))
    members = set(manifest["workspace"]["members"])
    # Negative fixture: Cargo edges alone cannot catch a generated payload
    # leaked through a fully qualified domain function signature.
    if forbidden_source_references(
        "pub fn validate(v: contracts::generated::alegria::temporal::v1::SeoScopePayload) {}",
        CAPABILITY_FORBIDDEN_IMPORTS["seo_domain"],
    ) != ["contracts::"]:
        raise ValueError("domain transport-import guard does not detect a wire type")

    for name, forbidden in CAPABILITY_FORBIDDEN_DIRECT.items():
        folder = RUST / "crates" / name
        config = tomllib.loads((folder / "Cargo.toml").read_text(encoding="utf-8"))
        unexpected = set(config.get("dependencies", {})) & forbidden
        if unexpected:
            raise ValueError(f"{name} has forbidden direct dependencies: {sorted(unexpected)}")
        for source in (folder / "src").rglob("*.rs"):
            references = forbidden_source_references(
                source.read_text(encoding="utf-8"), CAPABILITY_FORBIDDEN_IMPORTS.get(name, ()),
            )
            if references:
                raise ValueError(f"{name} has forbidden source references in {source}: {references}")

    # Human-review decisions have one pure owner, not a second free-form
    # decision-to-HITL mapping inside the SQL adapter or Application.
    review_domain = (RUST / "crates/seo_domain/src/review_decision.rs").read_text(encoding="utf-8")
    review_port = (RUST / "crates/seo_ports/src/lib.rs").read_text(encoding="utf-8")
    review_app = (RUST / "crates/seo_application/src/cms_review.rs").read_text(encoding="utf-8")
    review_sql = (RUST / "crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/human_review_decision.rs").read_text(encoding="utf-8")
    approval_sql = (RUST / "crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/types_review_helpers.rs").read_text(encoding="utf-8")
    review_wire = (RUST / "crates/seo_steps/src/cms_review_wire.rs").read_text(encoding="utf-8")
    if ("enum HumanReviewDecision" not in review_domain
            or "pub struct CmsApproval" not in review_domain
            or "pub decision: HumanReviewDecision" not in review_port
            or "Result<Option<CmsApproval>, DomainError>" not in review_port
            or "validate_human_reviewer(" not in review_app
            or "validate_human_reviewer(" not in review_sql
            or "hitl_queue_state()" not in review_sql
            or "match request.decision.as_str()" in review_sql
            or "fn approval_decision_valid(" in approval_sql
            or "pub fn from_wire(" not in review_wire
            or "pub fn to_wire(" not in review_wire):
        raise ValueError("CMS review decision policy must have one typed domain owner")
    for independent, allowed in ADAPTER_ALLOWED.items():
        if f"crates/{independent}" not in members:
            raise ValueError(f"missing workspace member: {independent}")
        config = tomllib.loads(
            (RUST / "crates" / independent / "Cargo.toml").read_text(encoding="utf-8")
        )
        unexpected = set(config["dependencies"]) - allowed
        if unexpected:
            raise ValueError(f"{independent} has unexpected dependencies: {sorted(unexpected)}")

    for name, allowed in SERVICE_ALLOWED.items():
        folder = RUST / "services" / name
        config = tomllib.loads((folder / "Cargo.toml").read_text(encoding="utf-8"))
        dependencies = set(config["dependencies"])
        forbidden = dependencies - allowed
        if forbidden:
            raise ValueError(f"{name} has unexpected heavy dependencies: {sorted(forbidden)}")
        for source in (folder / "src").rglob("*.rs"):
            if "infrastructure::" in source.read_text(encoding="utf-8"):
                raise ValueError(f"{name} still imports infrastructure: {source}")

    lock = tomllib.loads((RUST / "Cargo.lock").read_text(encoding="utf-8"))
    for name, forbidden in FORBIDDEN_TRANSITIVE.items():
        actual = lock_dependencies(lock["package"], name)
        unwanted = actual & forbidden
        if unwanted:
            raise ValueError(f"{name} still transitively compiles heavy SDKs: {sorted(unwanted)}")
    print("INDEPENDENT_RUST_MODULE_BOUNDARIES=PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
