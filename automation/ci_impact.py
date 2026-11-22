#!/usr/bin/env python3
"""Deterministic, fail-closed Rust change-impact report for the canonical workspace.

This report accelerates developer feedback; it NEVER replaces the full mandatory CI gate.
Unknown external inputs and unresolvable changes expand the scope to the full
workspace, rather than silently excluding a consumer from the analysis.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from collections import deque
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
WORKSPACE = ROOT / "app" / "rust"
SCHEMA_VERSION = 3

# Includes changes to generated contracts, integration fixtures, build tooling,
# container contents and runtime configuration; they are NOT Rust crate-local.
GLOBAL_PREFIXES = (
    ".github/",
    "automation/",
    "app/db/",
    "contracts/",
    "analytics_lab/",
    "infra/",
)
GLOBAL_FILES = {
    "docker-compose.yml",
    "app/rust/Cargo.toml",
    "app/rust/Cargo.lock",
    "app/rust/rust-toolchain.toml",
    "rust-toolchain.toml",
}

# These files do not change a Rust compilation unit. Historical run evidence
# can still be checked by the ordinary source-policy CI owners, but must not be
# treated as a live Truth fixture or proof that Rust behavior passed.
DOCUMENTATION_PREFIXES = ("docs/",)
DOCUMENTATION_FILES = {"README.md", "CONTRIBUTING.md"}

# Non-Rust inputs with a known downstream image. Do not make every release
# script change recompile the entire Cargo workspace.
WORKER_PACKAGING_FILES = {
    "automation/deploy_seo_static_cloudflare.py",
    "automation/check_public_static_output.py",
    "automation/check_public_http_readback.py",
    "automation/check_first_site_release.py",
}

TRUTH_FIXTURE_PREFIX = "app/rust/crates/integration_harness/fixtures/visa_truth_certification/"

# Publishing A17 exercises the real PostgreSQL transaction and a replaceable
# controlled HTTP delivery adapter. Reuse this same impact report in local/CI;
# unrelated leaf-crate edits must not pay for a Docker-backed publication test.
PUBLISHING_A17_PATHS = {
    "app/rust/crates/seo_domain/src/publishing.rs",
    "app/rust/crates/seo_steps/src/finalize_publish_step.rs",
    "app/rust/crates/seo_application/src/review_publish.rs",
    "app/rust/crates/seo_ports/src/lib.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/cms_publish.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/finalize_publish.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/publication_release.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/renderer_refresh.rs",
    "app/rust/crates/integration_harness/src/tests/publication_delivery_tests.rs",
}

# A14 exercises one real PostgreSQL lease/outbox and Qdrant materialization.
# The same decision is consumed by local feedback and the canonical CI runner.
OUTBOX_A14_PATHS = {
    "app/rust/crates/infrastructure/src/adapters/verified_rule_projection_adapter.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter/projection_runtime_store.rs",
    "app/rust/services/outbox_worker/src/worker.rs",
    "app/rust/services/outbox_worker/src/materialize.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_outbox_adapter.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_runtime_outbox_adapter.rs",
    "app/rust/crates/infrastructure/src/adapters/projection_materialize_adapter.rs",
    "app/rust/services/temporal/src/activities/operations/runtime_support_ops.rs",
    "app/rust/crates/integration_harness/src/tests/outbox_delivery_tests.rs",
}

# A13 reads only admissible, unscoped verified facts from disposable PostgreSQL
# through the production SQLx reader and the active SEO generation operation.
KNOWLEDGE_A13_PATHS = {
    "app/rust/crates/infrastructure/src/adapters/verified_rule_projection_adapter.rs",
    "app/rust/crates/seo_ports/src/knowledge.rs",
    "app/rust/crates/seo_ports/src/lib.rs",
    "app/rust/crates/seo_application/src/knowledge.rs",
    "app/rust/crates/infrastructure/src/adapters/canonical_vector_search_adapter.rs",
    "app/rust/crates/infrastructure/src/adapters/semantic_neighbor_search_adapter.rs",
    "app/rust/crates/seo_application/src/context_bundle.rs",
    "app/rust/crates/infrastructure/src/adapters/sqlx_context_bundle_adapter.rs",
    "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter/draft_publish_context_ports.rs",
    "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter/types_and_helpers.rs",
    "app/rust/crates/infrastructure/src/adapters/seo_ports_sqlx_adapter/source_context_projection_ports.rs",
    "app/rust/crates/seo_application/src/drafting.rs",
    "app/rust/services/temporal/src/activities/content_generation.rs",
    "app/rust/crates/integration_harness/src/tests/knowledge_read_tests.rs",
    "app/rust/services/temporal/src/activities/operations/truth_extraction_ops/triple_builder.rs",
    "app/rust/services/temporal/src/activities/operations/truth_extraction_ops/completeness_resolution.rs",
    "app/rust/services/temporal/src/activities/operations/truth_extraction_ops/contradiction_adjudication.rs",
}


# Canonical owners of *executable* Rust-backed integration suites. The exact
# commands are consumed by CI, never reconstructed in the runner/YAML. All
# paths here name live production consumers or the suite's actual inputs.
SUITE_OWNERS = {
    "publishing_a17": {
        "paths": PUBLISHING_A17_PATHS,
        "prefixes": (),
        "on_unknown_global_input": True,
        "command": (
            "cargo", "test", "-p", "integration_harness", "--features", "e2e",
            "--lib", "publishing_a17_postgres_outbox_http_faults_and_recovery",
            "--locked", "--", "--nocapture",
        ),
    },
    "outbox_a14": {
        "paths": OUTBOX_A14_PATHS,
        "prefixes": ("app/rust/services/outbox_worker/src/",),
        "on_unknown_global_input": True,
        "command": (
            "cargo", "test", "-p", "integration_harness", "--features", "e2e",
            "--lib", "outbox_a14_", "--locked", "--", "--nocapture",
        ),
    },
    "knowledge_a13": {
        "paths": KNOWLEDGE_A13_PATHS,
        "prefixes": (),
        "on_unknown_global_input": True,
        "command": (
            "cargo", "test", "-p", "integration_harness", "--features", "e2e",
            "--lib", "knowledge_a13_", "--locked", "--", "--nocapture",
        ),
    },
    "truth_certification": {
        "paths": set(),
        "prefixes": (TRUTH_FIXTURE_PREFIX,),
        "on_unknown_global_input": False,
        "command": (
            "cargo", "test", "-p", "integration_harness", "--features", "e2e",
            "--lib", "truth_certification_suite_matches_fixture_expectations",
            "--locked", "--", "--nocapture",
        ),
    },
    "graph_query": {
        "paths": {"app/rust/crates/infrastructure/src/adapters/graph_capability_adapter.rs"},
        "prefixes": ("app/rust/crates/graph_neo4j/",),
        "on_unknown_global_input": False,
        "command": (
            "cargo", "test", "-p", "integration_harness", "--features", "e2e",
            "--lib", "graph_capability_probe_binds_context_key", "--locked", "--", "--nocapture",
        ),
    },
    "vector_projection": {
        "paths": set(),
        "prefixes": ("app/rust/crates/vector_qdrant/",),
        "on_unknown_global_input": False,
        "command": (
            "cargo", "test", "-p", "integration_harness", "--features", "e2e",
            "--lib", "qdrant_rest_outbox_upsert_is_visible_to_grpc_retrieval",
            "--locked", "--", "--nocapture",
        ),
    },
    "temporal_restart": {
        "paths": {"app/rust/services/temporal/Cargo.toml"},
        "prefixes": ("app/rust/services/temporal/src/",),
        "on_unknown_global_input": False,
        # Build the actual processes in the SAME Cargo target as the preceding
        # workspace tests. The harness already accepts these binary paths;
        # otherwise it compiles both workers again in an isolated target.
        "prepare": (
            "cargo", "build", "--locked",
            "-p", "temporal_worker", "-p", "outbox_worker",
            "--bin", "temporal_worker", "--bin", "outbox_worker",
        ),
        "runtime_binaries": {
            "INTEGRATION_TEMPORAL_WORKER_BIN": "temporal_worker",
            "INTEGRATION_OUTBOX_WORKER_BIN": "outbox_worker",
        },
        "command": (
            "cargo", "test", "-p", "integration_harness", "--features", "e2e",
            "--lib", "canonical_temporal_manual_editorial_resumes_after_worker_restart",
            "--locked", "--", "--nocapture",
        ),
    },
}

# Each image is selected from the actual binaries compiled by its Dockerfile
# plus explicitly COPIED delivery inputs. No other Rust service is an implicit
# Worker dependency. Only temporal-worker is currently packaged by
# worker-image.yml; selection for other images is diagnostic until their release
# owners use this same report.
IMAGE_OWNERS = {
    "temporal-worker": {
        "crates": {"temporal_worker", "cli_tools"},
        "paths": WORKER_PACKAGING_FILES | {"app/rust/services/temporal/Dockerfile"},
        "prefixes": ("infra/cloudflare/seo-static/",),
    },
    "outbox-worker": {
        "crates": {"outbox_worker"},
        "paths": {"app/rust/services/outbox_worker/Dockerfile"},
        "prefixes": (),
    },
    "analytics-svc": {
        "crates": {"analytics_svc"},
        "paths": {"app/rust/services/analytics_svc/Dockerfile"},
        "prefixes": (),
    },
    "analytics-lab": {
        "crates": set(),
        "paths": {"infra/analytics_lab/Dockerfile"},
        "prefixes": ("infra/analytics_lab/", "app/analytics_lab/"),
    },
}


def suite_commands_for_report(report: dict[str, Any]) -> list[tuple[str, list[str]]]:
    """Validate and materialize the planner's selected real integration tests."""
    if report.get("schema_version") != SCHEMA_VERSION or report.get("mandatory_full_ci_gate") is not True:
        raise ValueError("invalid impact report: mandatory CI gate missing")
    suites = report.get("selected_suites")
    if not isinstance(suites, list) or len(suites) != len(set(suites)) or any(
        not isinstance(name, str) or name not in SUITE_OWNERS for name in suites
    ):
        raise ValueError("impact report has missing, duplicate or unknown integration suites")
    return [(name, list(SUITE_OWNERS[name]["command"])) for name in suites]


def parse_name_status(raw: bytes) -> list[str]:
    """Decode a NUL-delimited Git diff, keeping both rename paths and deletes."""
    fields = raw.decode("utf-8", errors="surrogateescape").split("\0")
    paths: set[str] = set()
    i = 0
    while i < len(fields):
        status = fields[i]
        i += 1
        if not status:
            continue
        if status[0] not in "ACDMRTUXB":
            raise ValueError(f"unsupported git diff status: {status!r}")
        count = 2 if status[0] in "RC" else 1
        if i + count > len(fields):
            raise ValueError("incomplete NUL-delimited git diff")
        for path in fields[i : i + count]:
            if not path or path.startswith("/") or ".." in Path(path).parts:
                raise ValueError(f"unsafe changed path: {path!r}")
            paths.add(path)
        i += count
    return sorted(paths)


def changed_paths(base: str, head: str, *, root: Path = ROOT) -> list[str]:
    """Return BOTH names in renames/copies, including deleted paths."""
    raw = subprocess.check_output(
        ["git", "diff", "--name-status", "-z", "--find-renames", base, head, "--"],
        cwd=root,
    )
    return parse_name_status(raw)


def changed_worktree_paths(base: str, *, root: Path = ROOT) -> list[str]:
    """Include staged, unstaged, renamed, deleted and not-yet-tracked local files."""
    raw = subprocess.check_output(
        ["git", "diff", "--name-status", "-z", "--find-renames", base, "--"],
        cwd=root,
    )
    tracked = set(parse_name_status(raw))
    untracked = subprocess.check_output(
        ["git", "ls-files", "--others", "--exclude-standard", "-z", "--"],
        cwd=root,
    )
    for path in untracked.decode("utf-8", errors="surrogateescape").split("\0"):
        if path:
            if path.startswith("/") or ".." in Path(path).parts:
                raise ValueError(f"unsafe untracked path: {path!r}")
            tracked.add(path)
    return sorted(tracked)


def load_cargo_metadata(*, workspace: Path = WORKSPACE) -> dict[str, Any]:
    output = subprocess.check_output(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--no-deps",
            "--manifest-path",
            str(workspace / "Cargo.toml"),
        ],
        cwd=workspace,
        text=True,
    )
    return json.loads(output)


def workspace_graph(
    metadata: dict[str, Any], *, root: Path = ROOT
) -> tuple[dict[str, str], dict[str, set[str]]]:
    """Map local manifest directories to package names and reverse edges.

    Conservatively include normal, build, dev and optional edges. A workspace
    feature can enable a dependency later; omitting it would be a false-negative.
    """
    members = set(metadata["workspace_members"])
    packages = [pkg for pkg in metadata["packages"] if pkg["id"] in members]
    if not packages or len(packages) != len(members):
        raise ValueError("incomplete cargo metadata workspace member set")

    manifest_owner: dict[str, str] = {}
    by_absolute_manifest: dict[Path, str] = {}
    for pkg in packages:
        manifest = Path(pkg["manifest_path"]).resolve()
        directory = manifest.parent.relative_to(root.resolve()).as_posix()
        if pkg["name"] in manifest_owner.values() or directory in manifest_owner:
            raise ValueError("ambiguous workspace package name or directory")
        manifest_owner[directory] = pkg["name"]
        by_absolute_manifest[manifest] = pkg["name"]

    reverse = {name: set() for name in manifest_owner.values()}
    for pkg in packages:
        for dep in pkg["dependencies"]:
            if not dep.get("path"):
                continue
            dependent = by_absolute_manifest.get(
                (Path(dep["path"]) / "Cargo.toml").resolve()
            )
            if dependent is not None:
                reverse[dependent].add(pkg["name"])
    return manifest_owner, reverse


def analyze(
    paths: list[str],
    owners: dict[str, str],
    reverse: dict[str, set[str]],
    *,
    head: str,
    base: str,
) -> dict[str, Any]:
    """Build an auditable, no-false-negative conservative affected closure."""
    reason: dict[str, set[str]] = {name: set() for name in reverse}
    direct: set[str] = set()
    broad_reasons: list[str] = []
    classified_inputs: dict[str, str] = {}

    for path in sorted(set(paths)):
        if path in DOCUMENTATION_FILES or (path.startswith(DOCUMENTATION_PREFIXES) and path.endswith(".md")):
            classified_inputs[path] = "documentation: no executable Rust or image input"
            continue
        if path.startswith("docs/runs/") and path.endswith(".json"):
            classified_inputs[path] = "historical run evidence: not a live test input or a Rust regression"
            continue
        if path in WORKER_PACKAGING_FILES or path.startswith("infra/cloudflare/seo-static/"):
            classified_inputs[path] = "known non-Rust input: temporal-worker image"
            continue
        if path.startswith(("infra/analytics_lab/", "app/analytics_lab/")):
            classified_inputs[path] = "known non-Rust input: analytics-lab image"
            continue
        if path.endswith("/Dockerfile") and any(path in rule["paths"] for rule in IMAGE_OWNERS.values()):
            classified_inputs[path] = "known image Dockerfile: packaging only"
            continue
        if path in GLOBAL_FILES or path.startswith(GLOBAL_PREFIXES):
            broad_reasons.append(f"global input: {path}")
            continue
        matches = [(prefix, name) for prefix, name in owners.items() if path.startswith(prefix + "/")]
        if len(matches) != 1:
            broad_reasons.append(f"unmapped or ambiguous input: {path}")
            continue
        _, name = matches[0]
        direct.add(name)
        reason[name].add(f"changed: {path}")

    if broad_reasons:
        affected = set(reverse)
        for name in affected:
            reason[name].add("fail-closed: common/unmapped input")
    else:
        affected = set(direct)
        queue = deque(sorted(direct))
        while queue:
            dependency = queue.popleft()
            for dependent in sorted(reverse[dependency]):
                reason[dependent].add(f"depends on: {dependency}")
                if dependent not in affected:
                    affected.add(dependent)
                    queue.append(dependent)

    services = sorted(
        name for directory, name in owners.items()
        if directory.startswith("app/rust/services/") and name in affected
    )
    scope = "full" if broad_reasons else "scoped" if affected else "none"

    suite_reasons: dict[str, list[str]] = {}
    for name, rule in SUITE_OWNERS.items():
        matched = [path for path in sorted(set(paths)) if path in rule["paths"] or path.startswith(rule["prefixes"])]
        # Long service-specific E2E suites follow their known live owners; a
        # planner edit does not silently claim that they were run. The broad
        # Rust gate plus core A13/A14/A17 remain mandatory on unknown inputs.
        if broad_reasons and rule["on_unknown_global_input"]:
            matched = ["fail-closed: common/unmapped input"] + matched
        if matched:
            suite_reasons[name] = matched

    image_reasons: dict[str, list[str]] = {}
    for name, rule in IMAGE_OWNERS.items():
        matched = [f"affected build crate: {crate}" for crate in sorted(rule["crates"] & affected)]
        matched.extend(
            f"packaged input: {path}" for path in sorted(set(paths))
            if path in rule["paths"] or path.startswith(rule["prefixes"])
        )
        if broad_reasons:
            matched.insert(0, "fail-closed: common/unmapped input")
        if matched:
            image_reasons[name] = matched

    return {
        "schema_version": SCHEMA_VERSION,
        "base": base,
        "source_sha": head,
        "base_sha": base,
        "head_sha": head,
        "changed_paths": sorted(set(paths)),
        "scope": scope,
        "analysis": scope,
        "full_scope_reasons": sorted(broad_reasons),
        "classified_inputs": classified_inputs,
        "direct_crates": sorted(direct),
        "affected_crates": sorted(affected),
        "affected_services": services,
        "reasons": {pkg: sorted(reason[pkg]) for pkg in sorted(affected)},
        "selected_suites": sorted(suite_reasons),
        "suite_reasons": suite_reasons,
        "selected_images": sorted(image_reasons),
        "image_status": {
            name: "selected" if name in image_reasons else "not_applicable"
            for name in sorted(IMAGE_OWNERS)
        },
        "image_reasons": image_reasons,
        # Never treat impact analysis as evidence to skip accepted test owners.
        "mandatory_full_ci_gate": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, help="Fetched PR base commit SHA")
    parser.add_argument("--head", required=True, help="Exact CI commit SHA")
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()

    metadata = load_cargo_metadata()
    owners, reverse = workspace_graph(metadata)
    paths = changed_paths(args.base, args.head)
    report = analyze(paths, owners, reverse, base=args.base, head=args.head)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(
        f"RUST_IMPACT={report['analysis']} "
        f"direct={','.join(report['direct_crates']) or '-'} "
        f"affected={','.join(report['affected_crates']) or '-'} "
        f"report={args.report}"
    )
    for why in report["full_scope_reasons"]:
        print(f"  FULL_SCOPE: {why}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
