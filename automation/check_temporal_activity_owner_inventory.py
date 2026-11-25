#!/usr/bin/env python3
"""Fail closed when legacy Temporal registers an unowned Activity.

The Rust inventory is deliberately descriptive. This check never changes an
SDK Activity name, a Task Queue, or a historical Workflow command. A new
Activity must declare a typed logical package before it may enter the worker.
"""

from __future__ import annotations

import collections
import hashlib
import re
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "app/rust/services/temporal/src/activities/registry/mod_registry_impl.rs"
INVENTORY = ROOT / "app/rust/services/temporal/src/activities/owner_inventory.rs"
ACTIVITIES_MOD = ROOT / "app/rust/services/temporal/src/activities/mod.rs"
WORKER_MAIN = ROOT / "app/rust/services/temporal/src/main.rs"
WORKFLOWS_MOD = ROOT / "app/rust/services/temporal/src/workflows/mod.rs"
PURE_ORCHESTRATOR_MANIFEST = ROOT / "app/rust/services/temporal_orchestrator/Cargo.toml"

ACTIVITY_METHOD = re.compile(
    r"(?m)^[ \t]*#\[activity(?:\([^\]]*\))?\]\s*"
    r"(?:#\[[^\]]+\]\s*)*pub(?:\(crate\))?\s+async\s+fn\s+(\w+)\s*\("
)
ACTIVITY_ATTRIBUTE = re.compile(r"(?m)^[ \t]*#\[activity\b([^\]]*)\]")
INVENTORY_ENTRY = re.compile(r"(?m)^\s*route!\(\s*(\w+)\s*=>\s*(\w+)\s*\),?\s*$")
OWNER_ENUM = re.compile(r"(?s)enum LogicalOwner\s*\{([^}]+)\}")
LEGACY_QUEUE = "alegria-pipeline"
# Historical Temporal SDK Activity names are persisted in real workflow
# histories. Renaming a method and editing the inventory to match must not
# silently alter those histories. This baseline can change only with an
# explicit versioned workflow/cutover proof and deliberate hash update.
HISTORICAL_ACTIVITY_COUNT = 73
HISTORICAL_NAMES_SHA256 = "8b638a99cb22cfee86b43671a5733f922f5194be646fcf6d4c25f28b8ab3e5a7"

# Historical mixed implementations cannot silently become a template for new
# multi-provider Activities. Changing this list requires explicit review and
# equivalent evidence of ownership, effect fencing, and History compatibility.
GRANDFATHERED_MIXED = frozenset({
    "run_truth_adjudication_step",
    "run_raw_knowledge_ingestion_step",
    "run_link_recommend_step",
    "run_global_site_reconcile_step",
    "run_editorial_draft_generate",
    "run_cms_publish_approved_step",
    "run_publish_materialize_step",
})
GRANDFATHERED_TEST_ONLY = frozenset({"test_step_prepare", "test_step_finalize"})

# The pure Workflow binary must not accidentally compile provider SDKs or
# pull an Activity/process owner into its local Cargo dependency closure.
# This is a build-graph prohibition, not an assertion based on the binary name.
PROVIDER_DEPENDENCY_DENY = frozenset({
    "sqlx", "postgres_store", "pg_runtime_ports", "infrastructure",
    "runtime_composition", "projection_delivery", "graph_neo4j",
    "vector_qdrant", "qdrant_effects_core", "temporal_worker",
    "temporal_pg_effects", "temporal_qdrant_effects", "outbox_worker",
    "neo4rs", "qdrant-client", "tokio-postgres", "reqwest",
    "playwright-rs", "voyage_api",
})


def pure_orchestrator_dependency_violations(
    manifest: Path = PURE_ORCHESTRATOR_MANIFEST,
) -> list[str]:
    """Walk runtime/build/target Cargo edges, detecting transitive SDK ingress.

    Never resolve repository Cargo features by guessing from filenames. This
    conservative union rejects even a dormant optional local provider edge;
    `cargo metadata`/rustc timing remains the authoritative physical proof.
    """
    visited: set[Path] = set()
    pending = [manifest.resolve()]
    errors: list[str] = []
    while pending:
        current = pending.pop()
        if current in visited:
            continue
        visited.add(current)
        package = tomllib.loads(current.read_text(encoding="utf-8"))
        name = package.get("package", {}).get("name")
        if name in PROVIDER_DEPENDENCY_DENY:
            errors.append(f"pure Temporal Cargo closure includes forbidden owner `{name}`")
        tables = [package]
        tables.extend(
            item for item in package.get("target", {}).values()
            if isinstance(item, dict)
        )
        for table in tables:
            for key in ("dependencies", "build-dependencies"):
                for dep_name, dep_value in table.get(key, {}).items():
                    if dep_name in PROVIDER_DEPENDENCY_DENY:
                        errors.append(f"pure Temporal `{name}` imports forbidden provider `{dep_name}`")
                    if isinstance(dep_value, dict) and "path" in dep_value:
                        nested = (current.parent / dep_value["path"] / "Cargo.toml").resolve()
                        if not nested.is_file():
                            errors.append(f"pure Temporal `{name}` has missing local manifest `{nested}`")
                        else:
                            pending.append(nested)
    return sorted(set(errors))


def violations(
    registry: str, inventory: str, activities_mod: str, worker_main: str,
    workflows_mod: str,
) -> list[str]:
    errors: list[str] = []
    if not re.search(r"#\[activities\]\s+impl\s+AlegriaActivities\s*\{", registry):
        errors.append("legacy registered #[activities] impl is missing or changed")
    if 'include!("registry/mod_registry_impl.rs")' not in activities_mod:
        errors.append("legacy SDK Activity registration source is not included")
    if "mod owner_inventory;" not in activities_mod:
        errors.append("typed Rust owner inventory is not compiled with legacy Worker")
    if f'build_worker_options("{LEGACY_QUEUE}", acts)' not in worker_main:
        errors.append("legacy Worker Task Queue changed: historical replay/drain required")
    registrations = re.findall(r"\.register_activities\s*\(", workflows_mod)
    if len(registrations) != 1 or ".register_activities(acts)" not in workflows_mod:
        errors.append("legacy Activity registrar changed: expected exactly one unchanged registration")
    if not re.search(rf'const LEGACY_TASK_QUEUE: &str = "{LEGACY_QUEUE}";', inventory):
        errors.append("Rust legacy owner inventory Task Queue differs from deployed queue")

    registered = ACTIVITY_METHOD.findall(registry)
    raw_activity_attrs = ACTIVITY_ATTRIBUTE.findall(registry)
    assigned = INVENTORY_ENTRY.findall(inventory)
    if any(attr != "" for attr in raw_activity_attrs):
        errors.append("parameterized #[activity(...)] changes historical SDK Activity names")
    if len(raw_activity_attrs) != len(registered):
        errors.append(
            f"SDK Activity attribute/parsed method count mismatch: "
            f"{len(raw_activity_attrs)} attributes, {len(registered)} methods"
        )
    if len(registered) != HISTORICAL_ACTIVITY_COUNT:
        errors.append(
            f"historical SDK Activity count changed: "
            f"{len(registered)} != {HISTORICAL_ACTIVITY_COUNT}"
        )
    digest = hashlib.sha256("\n".join(sorted(registered)).encode("utf-8")).hexdigest()
    if digest != HISTORICAL_NAMES_SHA256:
        errors.append("historical SDK Activity names changed without versioned history cutover")
    if not registered:
        errors.append("no SDK-registered legacy Activity methods found")
    if not assigned:
        errors.append("no typed legacy Activity owner assignments found")

    for name, count in sorted(collections.Counter(registered).items()):
        if count != 1:
            errors.append(f"duplicate registered Activity method: {name} ({count})")
    for name, count in sorted(collections.Counter(name for name, _ in assigned).items()):
        if count != 1:
            errors.append(f"duplicate Activity owner assignment: {name} ({count})")

    method_names = set(registered)
    route_names = {name for name, _ in assigned}
    for name in sorted(method_names - route_names):
        errors.append(f"unowned registered legacy Activity: {name}")
    for name in sorted(route_names - method_names):
        errors.append(f"stale legacy owner assignment (no registered Activity): {name}")

    enum_match = OWNER_ENUM.search(inventory)
    owners = set(re.findall(r"\b([A-Z][A-Za-z0-9_]*)\s*,", enum_match.group(1))) if enum_match else set()
    if not owners:
        errors.append("typed LogicalOwner enum is missing")
    for name, owner in assigned:
        if owner not in owners:
            errors.append(f"{name}: unknown typed LogicalOwner::{owner}")
        if owner == "LegacyMixedRuntime" and name not in GRANDFATHERED_MIXED:
            errors.append(f"{name}: new Activity cannot enter legacy mixed-provider ownership")
        if name in GRANDFATHERED_MIXED and owner != "LegacyMixedRuntime":
            errors.append(f"{name}: mixed legacy owner reclassified without explicit extraction/drain")
        if owner == "LegacyTestRuntime" and name not in GRANDFATHERED_TEST_ONLY:
            errors.append(f"{name}: new Activity cannot use legacy test-only ownership")
        if name in GRANDFATHERED_TEST_ONLY and owner != "LegacyTestRuntime":
            errors.append(f"{name}: historical test Activity owner changed")

    # A deleted mixed provider Activity must disappear from the explicit debt
    # register only after its original SDK history is drained/replay-accepted.
    for name in sorted(GRANDFATHERED_MIXED - route_names):
        errors.append(f"historical mixed Activity removed without drain inventory: {name}")
    return errors


def check_repository() -> list[str]:
    errors = violations(
        REGISTRY.read_text(encoding="utf-8"),
        INVENTORY.read_text(encoding="utf-8"),
        ACTIVITIES_MOD.read_text(encoding="utf-8"),
        WORKER_MAIN.read_text(encoding="utf-8"),
        WORKFLOWS_MOD.read_text(encoding="utf-8"),
    )
    errors.extend(pure_orchestrator_dependency_violations())
    return errors


def main() -> int:
    errors = check_repository()
    if errors:
        print("TEMPORAL_LEGACY_ACTIVITY_OWNER_INVENTORY: FAIL")
        for error in errors:
            print(f"- {error}")
        return 1
    count = len(ACTIVITY_METHOD.findall(REGISTRY.read_text(encoding="utf-8")))
    print(f"TEMPORAL_LEGACY_ACTIVITY_OWNER_INVENTORY: PASS ({count}/{count} registered Activities)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
