#!/usr/bin/env python3
"""Guard the native Worker image cache boundary against unrelated test edits.

Docker automatically uses Dockerfile.dockerignore instead of the root ignore
for this Dockerfile. Keep the original security exclusions intact, and only
omit known test-only sources that cannot affect the release Worker binaries.
"""

from pathlib import Path
import hashlib
import tempfile
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from prepare_worker_build_context import (
    IMAGE_CONTEXT_ROOTS, V3_DOCKERFILE, V7_PURE_DOCKERFILE, context_files,
    dependency_skeleton_targets, release_closure, write_dependency_skeleton,
)
from check_worker_dependency_skeleton import graph_signature
from compare_physical_build_options import workspace_graph as manifest_workspace_graph


ROOT = Path(__file__).resolve().parents[1]
DOCKERFILE = ROOT / "app/rust/services/temporal/Dockerfile"
DOCKER_IGNORE = DOCKERFILE.with_name("Dockerfile.dockerignore")
IGNORED_TEST_SOURCES = (
    "app/rust/crates/integration_harness/src/tests/",
    "app/rust/crates/integration_harness/src/containers.rs",
    "app/rust/crates/integration_harness/src/env_overrides.rs",
    "app/rust/crates/integration_harness/src/stub_servers.rs",
)
STATIC_MANIFEST = "infra/cloudflare/seo-static/package.json"
STATIC_DEPENDENCY_INSTALL = (
    "RUN npm install --prefix /app/infra/cloudflare/seo-static --no-audit --no-fund"
)
STATIC_ASSET_COPY = "COPY --link infra/cloudflare/seo-static /app/infra/cloudflare/seo-static"
WORKER_CONTEXT_COPY = "COPY --link worker-context.json /app/worker-context.json"
NATIVE_BINARIES = ("temporal_worker", "temporal_starter", "cli_tools")
DEPLOY_SCRIPTS = (
    "automation/deploy_seo_static_cloudflare.py",
    "automation/check_public_static_output.py",
    "automation/check_public_http_readback.py",
    "automation/check_first_site_release.py",
)


def docker_instructions(dockerfile: str) -> list[str]:
    """Unwrap Dockerfile continuations to inspect actual final-stage operations."""
    instructions: list[str] = []
    pending = ""
    for raw in dockerfile.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        continuation = line.endswith("\\")
        pending += line[:-1].strip() + " " if continuation else line
        if not continuation:
            instructions.append(" ".join(pending.split()))
            pending = ""
    if pending:
        raise ValueError("incomplete Dockerfile continuation")
    return instructions


def runtime_cache_invalidations(dockerfile: str, changed_inputs: set[str]) -> dict[str, bool]:
    """Model native BuildKit COPY --link cache dependence, not elapsed CI time.

    A regular instruction depends on all preceding runtime layers; a linked
    COPY can reuse its immutable source layer even if earlier layers changed.
    Only a change to that COPY's own source should invalidate the linked layer.
    The model assumes the builder supplied the given binary paths and ignores
    independent base-image upgrades (which are tested by the actual CI build).
    """
    operations = docker_instructions(dockerfile)
    runtime = operations[next(i for i, op in enumerate(operations) if op.startswith("FROM node:")) + 1:]
    prior_changed = False
    effects: dict[str, bool] = {}
    for operation in runtime:
        words = operation.split()
        if words[0] == "COPY":
            linked = "--link" in words[1:]
            sources = [word for word in words[1:-1] if not word.startswith("--")]
            source_changed = any(
                changed == source or changed.startswith(source.rstrip("/") + "/")
                for source in sources for changed in changed_inputs
            )
            invalidated = source_changed or (prior_changed and not linked)
            effects[words[-1]] = invalidated
        else:
            invalidated = prior_changed
            if operation == STATIC_DEPENDENCY_INSTALL:
                effects[STATIC_DEPENDENCY_INSTALL] = invalidated
        prior_changed |= invalidated
    return effects


def boundary_errors(root_ignore: str, specific_ignore: str, dockerfile: str) -> list[str]:
    errors: list[str] = []
    root_prefix = root_ignore.rstrip("\n") + "\n"
    if not specific_ignore.startswith(root_prefix):
        errors.append("Dockerfile.dockerignore must preserve the complete root .dockerignore prefix")
        return errors

    tail = specific_ignore[len(root_prefix):]
    excluded = [line.strip() for line in tail.splitlines() if line.strip() and not line.lstrip().startswith("#")]
    if excluded != list(IGNORED_TEST_SOURCES):
        errors.append("Docker context exclusions must contain exactly the four approved test-only paths")

    if "COPY app ./app" not in dockerfile:
        errors.append("Docker build context contract changed: update this check before modifying COPY")
    if "COPY dependency-skeleton/app ./app" not in dockerfile:
        errors.append("Docker must preserve the stable source-free dependency layer")
    deps = dockerfile.find("COPY dependency-skeleton/app ./app")
    real = dockerfile.find("COPY app ./app")
    if not (0 <= deps < dockerfile.find("RUN cargo build --release --locked", deps) < real):
        errors.append("Cargo dependencies must compile before production Rust source is copied")
    if dockerfile.count("cargo build --release --locked") != 2:
        errors.append("Docker requires one dependency compile and one real source compile")
    if WORKER_CONTEXT_COPY not in dockerfile:
        errors.append("Worker image must preserve exact-source context provenance")
    if "cargo build --release --locked" not in dockerfile:
        errors.append("Docker must retain the locked release build")
    if "-p temporal_worker -p cli_tools" not in dockerfile:
        errors.append("Docker release package selection changed: recheck the test-only exclusions")
    # A release-specific binary or provenance COPY must never invalidate npm.
    # The Cloudflare deploy adapter's package.json alone owns this stage.
    node_stage = dockerfile.find("FROM node:")
    manifest = dockerfile.find(
        f"COPY {STATIC_MANIFEST} /app/{STATIC_MANIFEST}", node_stage,
    )
    npm = dockerfile.find(STATIC_DEPENDENCY_INSTALL, node_stage)
    assets = dockerfile.find(STATIC_ASSET_COPY, node_stage)
    envelope = dockerfile.find(WORKER_CONTEXT_COPY, node_stage)
    binary = dockerfile.find(
        "COPY --link --from=builder /app/app/rust/target/release/temporal_worker", node_stage,
    )
    if not (0 <= node_stage < manifest < npm < assets < envelope < binary):
        errors.append(
            "Cloudflare package dependencies must install before website assets, "
            "source provenance and Worker binaries"
        )
    if dockerfile.count(STATIC_DEPENDENCY_INSTALL) != 1:
        errors.append("Cloudflare package dependencies must be installed exactly once")
    operations = docker_instructions(dockerfile)
    runtime = operations[next(i for i, op in enumerate(operations) if op.startswith("FROM node:")) + 1:]
    expected_linked_sources = {
        "infra/cloudflare/seo-static",
        "worker-context.json",
        *DEPLOY_SCRIPTS,
        *(f"/app/app/rust/target/release/{name}" for name in NATIVE_BINARIES),
    }
    for source in expected_linked_sources:
        matches = [line for line in runtime if line.startswith("COPY ") and source in line.split()]
        if len(matches) != 1 or "--link" not in matches[0].split():
            errors.append(f"Runtime source must have one independent linked COPY layer: {source}")
    script_copies = [line for line in runtime if line.startswith("COPY ")
                     and DEPLOY_SCRIPTS[0] in line.split()]
    if len(script_copies) != 1 or script_copies[0].split()[-1] != "/app/automation/":
        errors.append("Cloudflare deployment/check scripts must retain their native runtime directory")
    for name in NATIVE_BINARIES:
        src = f"/app/app/rust/target/release/{name}"
        dest = f"/usr/local/bin/{name}"
        if f"COPY --link --from=builder {src} {dest}" not in runtime:
            errors.append(f"Native Worker executable must retain its exact linked destination: {name}")
    if runtime[-1] != 'ENTRYPOINT ["/usr/local/bin/temporal_worker"]':
        errors.append("Release image must keep the native Worker entrypoint")
    if "WORKDIR /app" not in runtime:
        errors.append("Linked release destinations must remain under /app")
    return errors


class TemporalDockerContextTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.base = (ROOT / ".dockerignore").read_text(encoding="utf-8")
        cls.worker = DOCKER_IGNORE.read_text(encoding="utf-8")
        cls.dockerfile = DOCKERFILE.read_text(encoding="utf-8")

    def test_canonical_context_preserves_prod_sources(self) -> None:
        self.assertEqual(boundary_errors(self.base, self.worker, self.dockerfile), [])
        for path in IGNORED_TEST_SOURCES:
            self.assertTrue((ROOT / path).exists(), path)

    def test_root_security_exclusion_cannot_be_dropped(self) -> None:
        self.assertTrue(boundary_errors(self.base + "\nprivate-secrets/\n", self.worker, self.dockerfile))

    def test_test_only_path_cannot_be_dropped(self) -> None:
        wrong = self.worker.replace(IGNORED_TEST_SOURCES[0] + "\n", "")
        self.assertTrue(boundary_errors(self.base, wrong, self.dockerfile))

    def test_production_crate_cannot_be_excluded(self) -> None:
        wrong = self.worker + "app/rust/services/temporal/src/\n"
        self.assertTrue(boundary_errors(self.base, wrong, self.dockerfile))

    def test_different_release_packages_require_review(self) -> None:
        wrong = self.dockerfile.replace("-p temporal_worker -p cli_tools", "-p integration_harness -p cli_tools")
        self.assertTrue(boundary_errors(self.base, self.worker, wrong))

    def test_native_cloudflare_tools_reuse_cache_after_worker_rebuild(self) -> None:
        wrong = self.dockerfile.replace(STATIC_DEPENDENCY_INSTALL + "\n", "", 1)
        wrong = wrong.replace(
            "COPY --link --from=builder /app/app/rust/target/release/cli_tools /usr/local/bin/cli_tools",
            "COPY --link --from=builder /app/app/rust/target/release/cli_tools /usr/local/bin/cli_tools\n"
            + STATIC_DEPENDENCY_INSTALL,
        )
        self.assertTrue(boundary_errors(self.base, self.worker, wrong))

    def test_asset_changes_cannot_trigger_npm_install(self) -> None:
        copy_assets = STATIC_ASSET_COPY
        wrong = self.dockerfile.replace(STATIC_DEPENDENCY_INSTALL + "\n", "", 1)
        wrong = wrong.replace(copy_assets, copy_assets + "\n" + STATIC_DEPENDENCY_INSTALL)
        self.assertTrue(boundary_errors(self.base, self.worker, wrong))

    def test_only_cloudflare_manifest_may_invalidate_npm_dependencies(self) -> None:
        wrong = self.dockerfile.replace(
            f"COPY {STATIC_MANIFEST} /app/{STATIC_MANIFEST}",
            STATIC_ASSET_COPY,
            1,
        )
        self.assertTrue(boundary_errors(self.base, self.worker, wrong))

    def test_asset_or_provenance_only_reuses_all_native_binary_layers(self) -> None:
        executable_destinations = {f"/usr/local/bin/{name}" for name in NATIVE_BINARIES}
        for changed in (
            "infra/cloudflare/seo-static/vite.config.ts",
            "automation/deploy_seo_static_cloudflare.py",
            "worker-context.json",
        ):
            with self.subTest(changed=changed):
                invalid = runtime_cache_invalidations(self.dockerfile, {changed})
                self.assertTrue(any(invalid.values()), "change must invalidate its own content layer")
                self.assertTrue(executable_destinations <= invalid.keys())
                self.assertTrue(all(not invalid[dest] for dest in executable_destinations),
                                "BuildKit must rebase unchanged native binaries")
                self.assertFalse(invalid["/app/infra/cloudflare/seo-static/package.json"],
                                 "assets or provenance must not reinstall npm")
                self.assertFalse(invalid[STATIC_DEPENDENCY_INSTALL])

    def test_manifest_reinstalls_npm_but_keeps_independent_native_layers(self) -> None:
        invalid = runtime_cache_invalidations(
            self.dockerfile, {"infra/cloudflare/seo-static/package.json"},
        )
        self.assertTrue(invalid["/app/infra/cloudflare/seo-static/package.json"])
        self.assertTrue(invalid["/app/infra/cloudflare/seo-static"])
        self.assertTrue(invalid[STATIC_DEPENDENCY_INSTALL])
        self.assertTrue(all(not invalid[f"/usr/local/bin/{name}"] for name in NATIVE_BINARIES))

    def test_source_binary_change_invalidates_its_own_linked_layer(self) -> None:
        invalid = runtime_cache_invalidations(
            self.dockerfile, {"/app/app/rust/target/release/temporal_worker"},
        )
        self.assertFalse(invalid[STATIC_DEPENDENCY_INSTALL])
        self.assertFalse(invalid["/app/infra/cloudflare/seo-static"])
        self.assertTrue(invalid["/usr/local/bin/temporal_worker"])
        self.assertFalse(invalid["/usr/local/bin/temporal_starter"])
        self.assertFalse(invalid["/usr/local/bin/cli_tools"])

    def test_unlinked_binary_regression_would_reexport_after_asset_only_change(self) -> None:
        for name in NATIVE_BINARIES:
            with self.subTest(executable=name):
                src = f"/app/app/rust/target/release/{name}"
                linked = f"COPY --link --from=builder {src}"
                wrong = self.dockerfile.replace(linked, f"COPY --from=builder {src}")
                invalid = runtime_cache_invalidations(
                    wrong, {"infra/cloudflare/seo-static/vite.config.ts"},
                )
                self.assertTrue(invalid[f"/usr/local/bin/{name}"],
                                "ordinary COPY depends on the changed asset layer")
                self.assertTrue(boundary_errors(self.base, self.worker, wrong))

    def test_worker_native_entrypoint_and_cloudflare_smokes_cannot_be_removed(self) -> None:
        for source in DEPLOY_SCRIPTS:
            with self.subTest(source=source):
                self.assertTrue(boundary_errors(
                    self.base, self.worker, self.dockerfile.replace(source, "", 1),
                ))
        wrong_scripts_dir = self.dockerfile.replace(
            "automation/check_first_site_release.py /app/automation/",
            "automation/check_first_site_release.py /tmp/automation/",
        )
        self.assertTrue(boundary_errors(self.base, self.worker, wrong_scripts_dir))
        wrong_entrypoint = self.dockerfile.replace(
            'ENTRYPOINT ["/usr/local/bin/temporal_worker"]',
            'ENTRYPOINT ["/usr/local/bin/temporal_starter"]',
        )
        self.assertTrue(boundary_errors(self.base, self.worker, wrong_entrypoint))

    def test_nondelivered_readme_cannot_poison_worker_image_context(self) -> None:
        owners = {"app/rust/services/temporal": "temporal_worker"}
        tracked = {
            "app/rust/Cargo.toml", "app/rust/Cargo.lock",
            "app/rust/services/temporal/Cargo.toml",
            "infra/cloudflare/seo-static/README.md",
            "infra/cloudflare/seo-static/vite.config.ts",
            "infra/cloudflare/seo-static/package.json",
            "infra/cloudflare/seo-static/extra-delivered-content.md",
            "app/contracts/proto/read_api.proto", "app/contracts/proto/rules.proto",
            "app/contracts/proto/condition.proto", "app/contracts/proto/sync.proto",
            "app/contracts/proto/temporal_payloads.proto", "app/analytics_lab/proto/analytics.proto",
        }
        copied = context_files(tracked, owners, {"temporal_worker"})
        self.assertNotIn("infra/cloudflare/seo-static/README.md", copied)
        self.assertIn("infra/cloudflare/seo-static/vite.config.ts", copied)
        self.assertIn("infra/cloudflare/seo-static/package.json", copied)
        self.assertIn("infra/cloudflare/seo-static/extra-delivered-content.md", copied,
                      "do not drop future published Markdown content")


    def test_dependency_closure_and_independent_service_exclusion(self) -> None:
        # Same reverse edge representation as ci_impact.py.
        owners = {
            "app/rust/services/temporal": "temporal_worker",
            "app/rust/services/cli_tools": "cli_tools",
            "app/rust/services/outbox_worker": "outbox_worker",
            "app/rust/crates/infrastructure": "infrastructure",
        }
        reverse = {
            "temporal_worker": set(),
            "cli_tools": set(),
            "outbox_worker": set(),
            "infrastructure": {"temporal_worker", "cli_tools", "outbox_worker"},
        }
        release = release_closure(owners, reverse)
        self.assertEqual(release, {"temporal_worker", "cli_tools", "infrastructure"})
        tracked = {
            "app/rust/Cargo.toml", "app/rust/Cargo.lock",
            *(f"{directory}/Cargo.toml" for directory in owners),
            "app/rust/services/temporal/src/main.rs",
            "app/rust/services/outbox_worker/src/main.rs",
            "app/rust/crates/infrastructure/src/lib.rs",
            "app/rust/.sqlx/query.json",
            "automation/deploy_seo_static_cloudflare.py",
            "infra/cloudflare/seo-static/package.json",
            "app/contracts/proto/read_api.proto", "app/contracts/proto/rules.proto",
            "app/contracts/proto/condition.proto", "app/contracts/proto/sync.proto",
            "app/contracts/proto/temporal_payloads.proto",
            "app/analytics_lab/proto/analytics.proto",
        }
        copied = context_files(tracked, owners, release)
        self.assertIn("app/rust/services/temporal/src/main.rs", copied)
        self.assertIn("app/rust/crates/infrastructure/src/lib.rs", copied)
        self.assertIn("app/rust/services/outbox_worker/Cargo.toml", copied)
        self.assertNotIn("app/rust/services/outbox_worker/src/main.rs", copied)
        self.assertIn("automation/deploy_seo_static_cloudflare.py", copied)
        self.assertIn("infra/cloudflare/seo-static/package.json", copied)

    def test_v3_exact_owner_contexts_sever_providers_and_unrelated_image_sources(self) -> None:
        owners = {
            "app/rust/services/temporal_orchestrator": "temporal_orchestrator",
            "app/rust/services/temporal_pg_effects": "temporal_pg_effects",
            "app/rust/services/temporal_acquisition_pure": "temporal_acquisition_pure",
            "app/rust/services/acquisition_source_issuer": "acquisition_source_issuer",
            "app/rust/services/one_step_request_issuer": "one_step_request_issuer",
            "app/rust/services/temporal": "temporal_worker",
            "app/rust/crates/acquisition": "acquisition",
            "app/rust/crates/operation_protocol": "operation_protocol",
            "app/rust/crates/truth_domain": "truth_domain",
            "app/rust/crates/postgres_store": "postgres_store",
            "app/rust/crates/graph_neo4j": "graph_neo4j",
            "app/rust/crates/vector_qdrant": "vector_qdrant",
        }
        reverse = {
            "temporal_orchestrator": set(), "temporal_pg_effects": set(),
            "temporal_acquisition_pure": set(),
            "acquisition_source_issuer": set(),
            "one_step_request_issuer": set(),
            "temporal_worker": set(), "truth_domain": {"temporal_orchestrator", "temporal_pg_effects"},
            "postgres_store": {"temporal_pg_effects"},
            "acquisition": {"temporal_acquisition_pure", "acquisition_source_issuer"},
            "operation_protocol": {
                "temporal_acquisition_pure", "acquisition_source_issuer",
                "one_step_request_issuer", "acquisition",
            },
            "graph_neo4j": {"temporal_worker"},
            "vector_qdrant": {"temporal_worker"},
        }
        tracked = {
            "app/rust/Cargo.toml", "app/rust/Cargo.lock", V3_DOCKERFILE,
            *(f"{directory}/Cargo.toml" for directory in owners),
            *(f"{directory}/src/main.rs" for directory in owners if "/services/" in directory),
            *(f"{directory}/src/lib.rs" for directory in owners if "/crates/" in directory),
            "app/contracts/proto/read_api.proto", "app/contracts/proto/rules.proto",
            "app/contracts/proto/condition.proto", "app/contracts/proto/sync.proto",
            "app/contracts/proto/temporal_payloads.proto", "app/analytics_lab/proto/analytics.proto",
            "infra/cloudflare/seo-static/vite.config.ts",
            "automation/deploy_seo_static_cloudflare.py",
        }
        expected = {
            "temporal-orchestrator-v3": {"temporal_orchestrator", "truth_domain"},
            "temporal-pg-effects-v3": {"temporal_pg_effects", "truth_domain", "postgres_store"},
            "temporal-acquisition-pure-v1": {
                "temporal_acquisition_pure", "acquisition", "operation_protocol",
            },
            "acquisition-source-issuer-v1": {
                "acquisition_source_issuer", "acquisition", "operation_protocol",
            },
            "one-step-request-issuer-v1": {"one_step_request_issuer", "operation_protocol"},
        }
        for image_owner, allowed in expected.items():
            with self.subTest(image_owner=image_owner):
                packages = release_closure(owners, reverse, image_owner=image_owner)
                self.assertEqual(packages, allowed)
                paths = context_files(tracked, owners, packages, image_owner=image_owner)
                self.assertIn(V3_DOCKERFILE, paths)
                self.assertIn("app/rust/services/temporal_orchestrator/Cargo.toml", paths)
                self.assertIn("app/rust/services/temporal_pg_effects/Cargo.toml", paths)
                self.assertNotIn("app/rust/services/temporal/src/main.rs", paths)
                self.assertNotIn("app/rust/crates/graph_neo4j/src/lib.rs", paths)
                self.assertNotIn("app/rust/crates/vector_qdrant/src/lib.rs", paths)
                self.assertNotIn("infra/cloudflare/seo-static/vite.config.ts", paths)
                self.assertNotIn("automation/deploy_seo_static_cloudflare.py", paths)
                for package in allowed:
                    directory = next(directory for directory, name in owners.items() if name == package)
                    self.assertIn(f"{directory}/src/{'main.rs' if '/services/' in directory else 'lib.rs'}", paths)

        self.assertEqual(set(IMAGE_CONTEXT_ROOTS), {
            "temporal-worker", "temporal-orchestrator-v3", "temporal-pg-effects-v3",
            "temporal-acquisition-pure-v1",
            "acquisition-source-issuer-v1",
            "one-step-request-issuer-v1",
        })
        with self.assertRaisesRegex(ValueError, "unknown Cargo image context owner"):
            release_closure(owners, reverse, image_owner="temporal-worker-v4")
        with self.assertRaisesRegex(ValueError, "missing tracked V3 Dockerfile"):
            context_files(tracked - {V3_DOCKERFILE}, owners, expected["temporal-orchestrator-v3"],
                          image_owner="temporal-orchestrator-v3")
        v7_context = context_files(
            tracked | {V7_PURE_DOCKERFILE}, owners, expected["temporal-orchestrator-v3"],
            image_owner="temporal-orchestrator-v3",
        )
        self.assertIn(V7_PURE_DOCKERFILE, v7_context)
        self.assertNotIn("app/rust/crates/graph_neo4j/src/lib.rs", v7_context)
        self.assertNotIn("app/rust/crates/vector_qdrant/src/lib.rs", v7_context)
        self.assertNotIn("app/rust/crates/postgres_store/src/lib.rs", v7_context)

    def test_checked_in_v3_owner_closures_use_real_manifest_graph_without_legacy_providers(self) -> None:
        # Actual local Cargo manifests are enough to prove source exclusion;
        # this is not evidence that rustc/linker or Docker compiled/reused it.
        graph, manifests = manifest_workspace_graph()
        owners = {manifest.parent.relative_to(ROOT).as_posix(): name
                  for name, manifest in manifests.items()}
        reverse = {name: set() for name in graph}
        for consumer, dependencies in graph.items():
            for dependency in dependencies:
                reverse[dependency].add(consumer)
        closures = {
            image_owner: release_closure(owners, reverse, image_owner=image_owner)
            for image_owner in (
                "temporal-orchestrator-v3", "temporal-pg-effects-v3",
                "temporal-acquisition-pure-v1", "acquisition-source-issuer-v1",
                "one-step-request-issuer-v1",
            )
        }
        for image_owner, packages in closures.items():
            with self.subTest(image_owner=image_owner):
                self.assertNotIn("temporal_worker", packages)
                self.assertTrue(packages.isdisjoint({
                    "graph_neo4j", "vector_qdrant", "runtime_composition",
                    "infrastructure", "projection_delivery", "outbox_worker",
                }))
                if image_owner in ("temporal-acquisition-pure-v1", "acquisition-source-issuer-v1"):
                    self.assertIn("acquisition", packages)
                    self.assertIn("operation_protocol", packages)
                    self.assertNotIn("postgres_store", packages)
                    self.assertNotIn("temporal_orchestrator", packages)
                    self.assertNotIn("temporal_pg_effects", packages)
                elif image_owner == "one-step-request-issuer-v1":
                    self.assertIn("operation_protocol", packages)
                    self.assertNotIn("acquisition", packages)
                    self.assertNotIn("postgres_store", packages)
                    self.assertNotIn("temporal_orchestrator", packages)
                    self.assertNotIn("temporal_pg_effects", packages)
                else:
                    self.assertIn("truth_domain", packages)
        self.assertNotIn("postgres_store", closures["temporal-orchestrator-v3"])
        self.assertIn("postgres_store", closures["temporal-pg-effects-v3"])

    def test_opt_in_acquisition_images_have_separate_native_runtime_targets(self) -> None:
        dockerfile = (ROOT / V3_DOCKERFILE).read_text(encoding="utf-8")
        for owner, stage, image_stage in (
            ("temporal_acquisition_pure", "acquisition-builder", "acquisition"),
            ("acquisition_source_issuer", "source-issuer-builder", "source-issuer"),
            ("one_step_request_issuer", "request-issuer-builder", "request-issuer"),
        ):
            with self.subTest(owner=owner):
                self.assertEqual(dockerfile.count(f"{owner}) set -- ;;"), 2)
                self.assertIn(f"FROM build-base AS {stage}", dockerfile)
                self.assertIn(f"FROM runtime AS {image_stage}\n", dockerfile)
                self.assertIn(
                    f"COPY --from={stage} /app/app/rust/target/release/{owner} "
                    f"/usr/local/bin/{owner}", dockerfile,
                )
                self.assertIn(f'ENTRYPOINT ["/usr/local/bin/{owner}"]', dockerfile)

    def test_missing_manifest_fails_closed(self) -> None:
        with self.assertRaisesRegex(ValueError, "workspace manifest"):
            context_files(set(), {"app/rust/services/temporal": "temporal_worker"}, {"temporal_worker"})

    def test_dependency_skeleton_uses_cargo_metadata_targets(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package_dir = root / "app/rust/services/temporal"
            owners = {"app/rust/services/temporal": "temporal_worker"}
            metadata = {"packages": [{"name": "temporal_worker", "targets": [
                {"kind": ["bin"], "src_path": str(package_dir / "src/main.rs")},
                {"kind": ["bin"], "src_path": str(package_dir / "src/bin/temporal_starter.rs")},
                {"kind": ["custom-build"], "src_path": str(package_dir / "build.rs")},
            ]}]}
            expected = {
                "app/rust/services/temporal/src/main.rs": "fn main() {}\n",
                "app/rust/services/temporal/src/bin/temporal_starter.rs": "fn main() {}\n",
                "app/rust/services/temporal/build.rs": "fn main() {}\n",
            }
            self.assertEqual(dependency_skeleton_targets(metadata, owners, {"temporal_worker"}, root=root), expected)
            metadata["packages"][0]["targets"].append({
                "kind": ["bin"], "src_path": str(root.parent / "private.rs"),
            })
            with self.assertRaisesRegex(ValueError, "escaped the repository"):
                dependency_skeleton_targets(metadata, owners, {"temporal_worker"}, root=root)

    def test_native_dependency_layer_is_stable_across_rust_source_changes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "source"
            owner = "app/rust/services/temporal"
            src = root / owner / "src/main.rs"
            metadata = {"packages": [{"name": "temporal_worker", "targets": [
                {"kind": ["bin"], "src_path": str(src)},
                {"kind": ["custom-build"], "src_path": str(root / owner / "build.rs")},
            ]}]}
            context_files = {
                "app/rust/Cargo.toml": "[workspace]\nmembers = [\"services/temporal\"]\n",
                "app/rust/Cargo.lock": "version = 4\n",
                f"{owner}/Cargo.toml": "[package]\nname = \"temporal_worker\"\nversion = \"0.1.0\"\n",
                f"{owner}/build.rs": "fn main() {}\n",
                f"{owner}/src/main.rs": "fn main() { println!(\"first\"); }\n",
                "app/contracts/proto/read_api.proto": "syntax = \"proto3\";\n",
            }
            source = root / owner / "src/main.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(context_files[f"{owner}/src/main.rs"], encoding="utf-8")

            def prepare(dest: Path) -> dict[str, str]:
                for name, content in context_files.items():
                    target = dest / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text(content, encoding="utf-8")
                write_dependency_skeleton(dest, metadata=metadata, owners={owner: "temporal_worker"}, release={"temporal_worker"}, root=root)
                skel = dest / "dependency-skeleton"
                return {
                    path.relative_to(skel).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                    for path in skel.rglob("*") if path.is_file()
                }

            first = prepare(Path(tmp) / "first")
            context_files[f"{owner}/src/main.rs"] = "fn main() { println!(\"changed\"); }\n"
            second = prepare(Path(tmp) / "second")
            self.assertEqual(first, second, "Rust body edits must not invalidate the dependency Docker layer")
            context_files[f"{owner}/build.rs"] = "fn main() { println!(\"new protobuf\"); }\n"
            context_files["app/contracts/proto/read_api.proto"] = "syntax = \"proto3\"; // changed\n"
            proto_only = prepare(Path(tmp) / "proto-only")
            self.assertEqual(second, proto_only, "build.rs and protobuf edits must also reuse third-party objects")
            context_files[f"{owner}/Cargo.toml"] += "edition = \"2021\"\n"
            third = prepare(Path(tmp) / "third")
            self.assertNotEqual(proto_only, third, "Cargo manifest edits must invalidate dependency objects")
            self.assertEqual((Path(tmp) / "second" / owner / "src/main.rs").read_text(encoding="utf-8"),
                             "fn main() { println!(\"changed\"); }\n")

    def test_cargo_graph_parity_ignores_relocated_paths_not_feature_changes(self) -> None:
        package = {
            "name": "temporal_worker", "version": "0.1.0", "features": {"default": []},
            "dependencies": [{"name": "sqlx", "source": "registry+crates.io", "req": "^0.9",
                              "kind": None, "path": "/some/absolute/place"}],
            "targets": [{"name": "temporal_worker", "kind": ["bin"]}],
        }
        real = {"packages": [package]}
        moved = {"packages": [{**package, "dependencies": [
            {**package["dependencies"][0], "path": "/another/absolute/place"}
        ]}]}
        self.assertEqual(graph_signature(real), graph_signature(moved))
        moved["packages"][0]["features"] = {"default": ["sqlx/postgres"]}
        self.assertNotEqual(graph_signature(real), graph_signature(moved))

if __name__ == "__main__":
    unittest.main()
