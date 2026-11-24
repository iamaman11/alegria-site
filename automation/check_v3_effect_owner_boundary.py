#!/usr/bin/env python3
"""Source-level Issue #94 Truth-HITL PG owner/reachability regression gate.

No Rust build, database, Temporal server or provider is needed. This protects
the independently reviewable *source boundary*, not runtime effect admission.
Run ``python3 automation/check_v3_effect_owner_boundary.py`` or its unittest.
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / "app/rust"
LEGACY_CLI = RUST / "services/cli_tools/src/cli/main_entry.rs"
CLI_TYPES = RUST / "services/cli_tools/src/cli/types.rs"
TRUTH_WRITE = RUST / "crates/infrastructure/src/adapters/sqlx_truth_write_adapter.rs"
LEGACY_RECONCILE = RUST / "crates/infrastructure/src/adapters/sqlx_hitl_adapter.rs"
PG_RECONCILE = RUST / "crates/postgres_store/src/sqlx_hitl_adapter.rs"
PG_MANIFEST = RUST / "crates/postgres_store/Cargo.toml"
OWNER_SERVICE = RUST / "services/temporal_pg_effects"
WORKSPACE_MANIFEST = RUST / "Cargo.toml"

# An effect-specific PG owner must not acquire network/provider capabilities
# indirectly through the old mixed composition or the broad infrastructure
# crate. Transitive paths, not only text matches in a single Cargo.toml, matter.
FORBIDDEN_OWNER_PACKAGES = frozenset({
    "runtime_composition", "infrastructure", "graph_neo4j", "vector_qdrant",
    "acquisition", "http_client", "neo4rs", "qdrant-client", "reqwest",
    "playwright-rs", "hyper", "hyper-util",
})


def check_disabled_cli(text: str) -> list[str]:
    errors: list[str] = []
    for variant in ("SeoRun", "CrawlPendingSources"):
        pattern = (
            r"Command::" + variant + r"\s*\{\s*\.\.\s*}\s*=>\s*\{\s*"
            r"return Err\(UnsupportedDirectSiteCommand::" + variant + r"\.into\(\)\);"
        )
        if not re.search(pattern, text):
            errors.append(f"direct CLI {variant} must reject before any I/O")
    return errors


def check_disabled_cli_diagnostic(text: str) -> list[str]:
    if "CLI_DIRECT_SITE_COMMAND_DISABLED:" not in text:
        return ["disabled CLI lost CLI_DIRECT_SITE_COMMAND_DISABLED diagnostic"]
    if "canonical Temporal workflow operator" not in text:
        return ["disabled CLI lost canonical operator migration guidance"]
    return []


def check_atomic_v1_truth_write(text: str) -> list[str]:
    # The V1 Truth transaction must include queue reconciliation, not launch
    # an independent Activity after the extracted/verified candidate commit.
    if not re.search(
        r"reconcile_truth_candidate_reviews_in_tx\s*\(\s*&mut\s+tx\s*,\s*"
        r"context_key\s*,\s*run_id\s*\)", text,
    ):
        return ["V1 Truth write lost in-transaction HITL reconciliation"]
    return []


def check_direct_pg_owner_routing(root: Path) -> list[str]:
    """After migration, forbid a compatibility facade that masks SQL ownership."""
    rust = root / "app/rust"
    owner_file = rust / "crates/postgres_store/src/sqlx_hitl_adapter.rs"
    if not owner_file.exists():
        return []  # Legacy cut remains explicitly identified until migration.
    errors: list[str] = []
    legacy = rust / "crates/infrastructure/src/adapters/sqlx_hitl_adapter.rs"
    if legacy.exists():
        errors.append("infrastructure Truth HITL compatibility facade must be deleted")
    infrastructure_mod = rust / "crates/infrastructure/src/adapters/mod.rs"
    if "pub mod sqlx_hitl_adapter;" in infrastructure_mod.read_text(encoding="utf-8"):
        errors.append("infrastructure must not reexport the moved PG Truth HITL owner")
    truth_write = rust / "crates/infrastructure/src/adapters/sqlx_truth_write_adapter.rs"
    if "postgres_store::sqlx_hitl_adapter" not in truth_write.read_text(encoding="utf-8"):
        errors.append("V1 Truth write must import postgres_store owner directly")
    for source in rust.rglob("*.rs"):
        if source == legacy:
            continue
        body = source.read_text(encoding="utf-8")
        if "infrastructure::adapters::sqlx_hitl_adapter" in body or re.search(
            r"use\s+infrastructure::adapters::\{[^}]*\bsqlx_hitl_adapter\b", body, re.DOTALL,
        ):
            errors.append(f"legacy infrastructure HITL facade call: {source.relative_to(root)}")
    return errors


def reconcile_function(source: str) -> str:
    marker = "pub async fn reconcile_truth_candidate_reviews_in_tx"
    start = source.find(marker)
    if start < 0:
        return ""
    # Top-level Rust function's closing brace is at column zero. Any nested
    # SQL closures/branches are indented; no provider calls are executed here.
    closing = re.search(r"(?m)^}\s*$", source[start:])
    return source[start : start + closing.end()] if closing else ""


def check_truth_reconcile_sql(source: str) -> list[str]:
    errors: list[str] = []
    body = reconcile_function(source)
    if not body:
        return ["canonical in-tx Truth review reconcile function missing"]
    expected = (
        "UPDATE site.seo_hitl_tasks", "INSERT INTO site.seo_hitl_tasks",
        "extracted.rule_candidates", "source_snapshot_hash", "needs_hitl",
        "truth_candidate_review@1:", "ON CONFLICT (task_key) DO NOTHING",
    )
    for token in expected:
        if token not in body:
            errors.append(f"Truth review reconcile lost SQL fence: {token}")
    # This operation is queue reconciliation, never independent adjudication.
    for verb in ("INSERT INTO", "UPDATE", "DELETE FROM", "TRUNCATE"):
        if re.search(
            rf"\b{verb}\s+(?:verified\.\w+|extracted\.rule_candidates)\b",
            body, re.IGNORECASE,
        ):
            errors.append(f"Truth review reconcile cannot {verb} canonical Truth")
    if "audit_log_payload" not in body:
        errors.append("Truth review reconcile lost its durable audit append")
    return errors


def cargo_runtime_closure(manifest: Path) -> set[str]:
    """Collect local crate and external runtime package names transitively."""
    seen: set[Path] = set()
    packages: set[str] = set()
    pending = [manifest]
    while pending:
        current = pending.pop().resolve()
        if current in seen:
            continue
        seen.add(current)
        data = tomllib.loads(current.read_text(encoding="utf-8"))
        packages.add(data.get("package", {}).get("name", current.parent.name))
        for section in ("dependencies", "build-dependencies"):
            for alias, spec in data.get(section, {}).items():
                package = spec.get("package", alias) if isinstance(spec, dict) else alias
                packages.add(package)
                if isinstance(spec, dict) and "path" in spec:
                    child = (current.parent / spec["path"] / "Cargo.toml").resolve()
                    if not child.is_file():
                        raise ValueError(f"unresolved local Cargo dependency {alias} at {child}")
                    pending.append(child)
    return packages


def check_pg_owner_dependencies(manifest: Path) -> list[str]:
    try:
        reachable = cargo_runtime_closure(manifest)
    except (OSError, ValueError, tomllib.TOMLDecodeError) as exc:
        return [f"could not resolve PG owner Cargo closure: {exc}"]
    forbidden = sorted(reachable & FORBIDDEN_OWNER_PACKAGES)
    return [f"PG owner must not reach mixed/provider package: {name}" for name in forbidden]


def check_v3_worker(root: Path = ROOT) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    notes: list[str] = []
    rust = root / "app/rust"
    pg = rust / "crates/postgres_store"
    errors.extend(check_pg_owner_dependencies(pg / "Cargo.toml"))
    service = rust / "services/temporal_pg_effects"
    if not service.exists():
        notes.append("V3 PG owner service not in this checkout; physical worker checks pending")
        return errors, notes

    manifest = service / "Cargo.toml"
    if not manifest.exists():
        errors.append("V3 PG owner service exists without Cargo.toml")
        return errors, notes
    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    if data.get("package", {}).get("name") != "temporal_pg_effects":
        errors.append("V3 PG owner must use independent temporal_pg_effects package")
    if "postgres_store" not in data.get("dependencies", {}):
        errors.append("V3 PG owner must call the canonical postgres_store owner")
    errors.extend(check_pg_owner_dependencies(manifest))

    workspace = tomllib.loads((rust / "Cargo.toml").read_text(encoding="utf-8"))
    if "services/temporal_pg_effects" not in workspace.get("workspace", {}).get("members", []):
        errors.append("V3 PG owner must be its own Cargo workspace member")
    owner_sources = "\n".join(
        path.read_text(encoding="utf-8") for path in (service / "src").rglob("*.rs")
    )
    if "alegria-pg-effects-v3" not in owner_sources:
        errors.append("PG owner must use isolated alegria-pg-effects-v3 task queue")
    owner_sql = pg / "src/sqlx_hitl_adapter.rs"
    if not owner_sql.exists():
        errors.append("canonical PG reconciliation implementation not moved into postgres_store")
    else:
        owner_sources += "\n" + owner_sql.read_text(encoding="utf-8")
    typed_contract = pg / "src/truth_review_effect_v3.rs"
    if not typed_contract.exists():
        errors.append("PG owner missing typed Truth review effect contract module")
    else:
        owner_sources += "\n" + typed_contract.read_text(encoding="utf-8")
    for contract_part in ("pg.truth.reconcile_candidate_reviews", "pg.truth.review_queue.reconcile"):
        if contract_part not in owner_sources:
            errors.append(f"PG owner has no declared effect contract identity: {contract_part}")
    for admission_source in (
        "pipeline.pg_effect_v3_admissions", "pipeline.pg_effect_v3_receipts",
        "pipeline.execution_runs", "pipeline.step_executions", "verified_truth_write",
        "newer.created_at >= this_run.created_at", "pg_advisory_xact_lock",
        "hashtextextended", "truth-context|", "has_table_privilege",
    ):
        if admission_source not in owner_sources:
            errors.append(f"PG effect admission lost authoritative stale-run fence: {admission_source}")
    return errors, notes


def audit(root: Path = ROOT) -> tuple[list[str], list[str]]:
    rust = root / "app/rust"
    cli = rust / "services/cli_tools/src/cli/main_entry.rs"
    cli_types = rust / "services/cli_tools/src/cli/types.rs"
    truth = rust / "crates/infrastructure/src/adapters/sqlx_truth_write_adapter.rs"
    pg = rust / "crates/postgres_store/src/sqlx_hitl_adapter.rs"
    legacy = rust / "crates/infrastructure/src/adapters/sqlx_hitl_adapter.rs"
    errors = check_disabled_cli(cli.read_text(encoding="utf-8"))
    errors.extend(check_disabled_cli_diagnostic(cli_types.read_text(encoding="utf-8")))
    errors.extend(check_atomic_v1_truth_write(truth.read_text(encoding="utf-8")))
    source = pg if pg.exists() else legacy
    errors.extend(check_truth_reconcile_sql(source.read_text(encoding="utf-8")))
    errors.extend(check_direct_pg_owner_routing(root))
    owner_errors, notes = check_v3_worker(root)
    return errors + owner_errors, notes


if __name__ == "__main__":
    failures, observations = audit()
    for note in observations:
        print(f"PENDING: {note}")
    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        sys.exit(1)
    print("PASS: Issue #94 PG Truth review owner/reachability source fences")
