#!/usr/bin/env python3
"""Deterministic, fail-closed Rust and PR-owner impact for the canonical workspace.

The selected job owner set is checked by an exact-SHA mandatory CI gate;
unmapped executable inputs conservatively retain all established owners.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from collections import deque
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
WORKSPACE = ROOT / "app" / "rust"
SCHEMA_VERSION = 5

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
# These Markdown files are consumed as policy inputs by source acceptance
# scripts (including opt-in deep checks); they are not merely explanatory.
SOURCE_POLICY_DOCUMENTS = {
    "docs/INDEX.md",
    "docs/OPS_RUNTIME_RUNBOOK.md",
    "docs/OPS_TEMPORAL_PRODUCTION_GATE.md",
    "docs/OPS_TEMPORAL_BUILD_MODES.md",
    "docs/SUPERSITE_10_10_EXPERT_GAP_CLOSURE_PLAN.md",
    "docs/V6_Support_Process_Registry.md",
    "docs/V6_Expert_Truth_Graph_Runtime.md",
    "docs/V6_SeoSiteBuildWorkflow_Working_Plan.md",
}

# Non-Rust inputs with a known downstream image. Do not make every release
# script change recompile the entire Cargo workspace.
WORKER_PACKAGING_FILES = {
    "automation/deploy_seo_static_cloudflare.py",
    "automation/check_public_static_output.py",
    "automation/check_public_http_readback.py",
    "automation/check_first_site_release.py",
}

TRUTH_FIXTURE_PREFIX = "app/rust/crates/integration_harness/fixtures/visa_truth_certification/"
TRUTH_FIXTURE_ID = re.compile(r"[a-z][a-z0-9_]*\Z")

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


# Both live Temporal restart and Truth certification spawn these actual Rust
# processes. Prepare them once in the ordinary Cargo target, not recursively
# inside the integration harness's isolated nested compilation directory.
WORKER_E2E_PREPARE = (
    "cargo", "build", "--locked",
    "-p", "temporal_worker", "-p", "outbox_worker",
    "--bin", "temporal_worker", "--bin", "outbox_worker",
)
WORKER_E2E_RUNTIME_BINARIES = {
    "INTEGRATION_TEMPORAL_WORKER_BIN": "temporal_worker",
    "INTEGRATION_OUTBOX_WORKER_BIN": "outbox_worker",
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
        "prepare": WORKER_E2E_PREPARE,
        "runtime_binaries": WORKER_E2E_RUNTIME_BINARIES,
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
        # This separate live E2E also opens the canonical happy-path JSON and
        # its HTML pages. Do not let fixture-only narrowing skip its consumer.
        "fixture_ids": {"happy_path_publish_allowed"},
        "on_unknown_global_input": False,
        # Build the actual processes in the SAME Cargo target as the preceding
        # workspace tests. The harness already accepts these binary paths;
        # otherwise it compiles both workers again in an isolated target.
        "prepare": WORKER_E2E_PREPARE,
        "runtime_binaries": WORKER_E2E_RUNTIME_BINARIES,
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

# GitHub job IDs, not display names. The PR gate checks every one of these;
# only this planner can mark an owner not applicable to an exact source diff.
CI_PR_OWNERS = (
    "architecture-and-rust",
    "rust-contract-smokes",
    "product-truth",
    "rust-security",
    "cloudflare-static-build",
    "stack-temporal-postgres18",
    "stack-storage-graph",
    "verify",
    "windows-operator-smoke",
)


def ci_jobs_for_change(
    paths: list[str], scope: str, suites: set[str], images: set[str],
) -> dict[str, list[str]]:
    """One fail-closed reasoned selection for the existing PR job owners.

    Integration suites already run in the Rust owner. The two independent
    Compose boot checks are for *deployment configuration*, not a substitute
    for the disposable PostgreSQL/Qdrant/Neo4j adapters exercised by suites.
    """
    if scope == "full":
        return {name: ["fail-closed: global or unknown impact"] for name in CI_PR_OWNERS}

    changed = set(paths)
    selected: dict[str, list[str]] = {}

    def require(name: str, why: str) -> None:
        selected.setdefault(name, []).append(why)

    if scope == "scoped" or suites:
        for name in ("architecture-and-rust", "rust-contract-smokes", "rust-security", "verify"):
            require(name, "affected Rust workspace or integration suite")

    if changed & SOURCE_POLICY_DOCUMENTS:
        require("product-truth", "Markdown is a source-policy input, not just explanatory documentation")
        require("verify", "source-policy documents are read by acceptance scripts")

    if any(path.startswith("docs/runs/") for path in changed):
        require("product-truth", "historical Truth evidence is consumed by Python policy checks")
        require("verify", "historical run evidence is checked by source contracts")

    truth_prefixes = (
        "app/rust/crates/truth_domain/", "app/rust/crates/acquisition/",
        "app/rust/services/temporal/", TRUTH_FIXTURE_PREFIX,
    )
    if any(path.startswith(truth_prefixes) for path in changed) or "truth_certification" in suites:
        require("product-truth", "live Acquisition/Truth/Temporal source or fixture")

    if any(path.startswith("infra/cloudflare/seo-static/") or path in WORKER_PACKAGING_FILES for path in changed):
        require("cloudflare-static-build", "Cloudflare static or delivered Python asset")
    if changed & WORKER_PACKAGING_FILES:
        require("product-truth", "first-site delivery and readback scripts feed Product smoke")
        require("verify", "delivered Python assets require source policy and lint")

    if any(path.startswith(("infra/analytics_lab/", "app/analytics_lab/")) for path in changed):
        require("verify", "analytics-lab Python is checked by source policy and Ruff")

    if "temporal-worker" in images:
        require("windows-operator-smoke", "changed Worker delivery artifact")

    if any(path == "docker-compose.yml" or path.startswith("infra/temporal/") for path in changed):
        require("stack-temporal-postgres18", "Temporal/Compose deployment configuration")
    if any(path == "docker-compose.yml" or path.startswith("infra/postgres/") or
           path.startswith("infra/qdrant/") or path.startswith("infra/neo4j/") for path in changed):
        require("stack-storage-graph", "storage/graph Compose deployment configuration")

    if any(path.startswith("automation/") and path.endswith(".ps1") or
           path.startswith("automation/check_windows_") for path in changed):
        require("windows-operator-smoke", "native Windows operator contract")

    # A Dockerfile is an executable delivery input. No narrow PR job currently
    # builds these images, so do not label it equivalent to Markdown or infer
    # release acceptance from the image impact report.
    if any(path.endswith("/Dockerfile") for path in changed):
        return {name: ["fail-closed: Dockerfile delivery owner not isolated"] for name in CI_PR_OWNERS}

    # New narrow allowlists must name the executable owner explicitly. This
    # catches future additions to the non-Rust `scope=none` classification.
    uncovered = changed - SOURCE_POLICY_DOCUMENTS - DOCUMENTATION_FILES
    uncovered = {path for path in uncovered
                 if not (path.startswith(DOCUMENTATION_PREFIXES) and path.endswith(".md"))
                 and not (path.startswith("docs/runs/") and path.endswith(".json"))
                 and not path.startswith(("infra/analytics_lab/", "app/analytics_lab/"))
                 and not path.startswith("infra/cloudflare/seo-static/")
                 and not path.startswith(TRUTH_FIXTURE_PREFIX)
                 and path not in WORKER_PACKAGING_FILES}
    if scope == "none" and uncovered:
        return {name: ["fail-closed: unowned non-Rust input"] for name in CI_PR_OWNERS}

    return {name: selected[name] for name in CI_PR_OWNERS if name in selected}


def truth_fixture_ids_for_paths(
    changed_paths: list[str], *, root: Path = ROOT,
) -> tuple[list[str] | None, list[str]]:
    """Resolve live HTML consumers from the same JSON manifests as Rust.

    `None` means run *all* certification fixtures. Never scope changed
    manifests, deletes/renames, malformed inventories or unowned HTML by guess.
    """
    changed = sorted(path for path in set(changed_paths) if path.startswith(TRUTH_FIXTURE_PREFIX))
    if not changed:
        return None, ["full: no changed live HTML input to resolve"]
    if any(not path.endswith(".html") for path in changed):
        return None, ["full: fixture manifest or non-HTML input changed"]

    folder = root / TRUTH_FIXTURE_PREFIX
    manifests = sorted(folder.glob("*.json"))
    if not manifests:
        return None, ["full: no readable Truth fixture manifests"]

    references: dict[str, set[str]] = {}
    seen_ids: set[str] = set()
    try:
        for manifest in manifests:
            data = json.loads(manifest.read_text(encoding="utf-8"))
            fixture_id = data["fixture_id"]
            pages = data["source_inputs"]["pages"]
            if (not isinstance(fixture_id, str) or not TRUTH_FIXTURE_ID.fullmatch(fixture_id)
                    or fixture_id != manifest.stem or fixture_id in seen_ids
                    or not isinstance(pages, list) or not pages):
                raise ValueError("invalid or duplicate fixture ID/pages")
            seen_ids.add(fixture_id)
            for page in pages:
                html_file = page["html_file"]
                if (not isinstance(html_file, str) or Path(html_file).name != html_file
                        or not html_file.endswith(".html")
                        or not (folder / html_file).is_file()):
                    raise ValueError("unsafe or missing referenced HTML input")
                references.setdefault(TRUTH_FIXTURE_PREFIX + html_file, set()).add(fixture_id)
    except (OSError, ValueError, TypeError, KeyError) as error:
        return None, [f"full: incomplete Truth fixture inventory ({type(error).__name__})"]

    unmatched = sorted(set(changed) - set(references))
    if unmatched:
        return None, ["full: changed HTML lacks a live JSON fixture consumer: " + ", ".join(unmatched)]

    fixture_ids = sorted(set().union(*(references[path] for path in changed)))
    return fixture_ids, ["changed live HTML is read by: " + ", ".join(fixture_ids)]


def suite_environment_for_report(report: dict[str, Any], name: str) -> dict[str, str]:
    """Validate test filters before passing them to the real Cargo process."""
    scoped = report.get("suite_fixture_ids")
    selected = report.get("selected_suites")
    if not isinstance(scoped, dict) or not isinstance(selected, list) or any(
        suite != "truth_certification" or suite not in selected for suite in scoped
    ):
        raise ValueError("invalid integration fixture owner selection")
    if name != "truth_certification" or name not in scoped:
        return {}
    ids = scoped[name]
    if (not isinstance(ids, list) or not ids
            or any(not isinstance(value, str) or not TRUTH_FIXTURE_ID.fullmatch(value) for value in ids)
            or len(set(ids)) != len(ids)):
        raise ValueError("invalid exact Truth fixture IDs")
    return {"TRUTH_CERT_FIXTURE_IDS": ",".join(ids)}


def suite_commands_for_report(report: dict[str, Any]) -> list[tuple[str, list[str]]]:
    """Validate and materialize the planner's selected real integration tests."""
    if report.get("schema_version") != SCHEMA_VERSION or report.get("mandatory_full_ci_gate") is not True:
        raise ValueError("invalid impact report: mandatory CI gate missing")
    suites = report.get("selected_suites")
    if not isinstance(suites, list) or len(suites) != len(set(suites)) or any(
        not isinstance(name, str) or name not in SUITE_OWNERS for name in suites
    ):
        raise ValueError("impact report has missing, duplicate or unknown integration suites")
    if not isinstance(report.get("suite_fixture_ids"), dict):
        raise ValueError("impact report lacks typed fixture owner selection")
    for name in suites:
        suite_environment_for_report(report, name)
    if report.get("suite_fixture_ids") and "truth_certification" not in suites:
        raise ValueError("fixture selection without its executable suite")
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
            classified_inputs[path] = ("source-policy Markdown: no Rust compile or image input"
                                       if path in SOURCE_POLICY_DOCUMENTS
                                       else "documentation: no executable Rust or image input")
            continue
        if path.startswith("docs/runs/") and path.endswith(".json"):
            classified_inputs[path] = "historical run evidence: not a live test input or a Rust regression"
            continue
        if path.startswith(TRUTH_FIXTURE_PREFIX):
            # Runtime files are opened by the e2e harness, not compiled via
            # include_str!. The selected real suite compiles the required Rust
            # graph; an extra Cargo check/test is duplicate work.
            classified_inputs[path] = "live Truth runtime fixture: consumed by selected e2e suite"
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

    suite_fixture_ids: dict[str, list[str]] = {}
    suite_fixture_reasons: dict[str, list[str]] = {}
    if "truth_certification" in suite_reasons:
        ids, reasons = truth_fixture_ids_for_paths(paths)
        suite_fixture_reasons["truth_certification"] = reasons
        if ids is not None:
            suite_fixture_ids["truth_certification"] = ids
        for name, rule in SUITE_OWNERS.items():
            consuming_ids = rule.get("fixture_ids", set())
            if consuming_ids and (ids is None or set(ids) & consuming_ids):
                suite_reasons.setdefault(name, []).append(
                    "live Truth input also feeds this E2E: " + ", ".join(sorted(consuming_ids))
                )

    ci_job_reasons = ci_jobs_for_change(paths, scope, set(suite_reasons), set(image_reasons))

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
        "suite_fixture_ids": suite_fixture_ids,
        "suite_fixture_reasons": suite_fixture_reasons,
        "selected_images": sorted(image_reasons),
        "image_status": {
            name: "selected" if name in image_reasons else "not_applicable"
            for name in sorted(IMAGE_OWNERS)
        },
        "image_reasons": image_reasons,
        "selected_ci_jobs": sorted(ci_job_reasons),
        "ci_job_reasons": ci_job_reasons,
        "ci_job_status": {
            name: "required" if name in ci_job_reasons else "not_applicable"
            for name in CI_PR_OWNERS
        },
        # The aggregate must prove the selected-versus-skipped owner statuses.
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
