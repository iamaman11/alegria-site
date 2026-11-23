#!/usr/bin/env python3
"""Guard the native Worker image cache boundary against unrelated test edits.

Docker automatically uses Dockerfile.dockerignore instead of the root ignore
for this Dockerfile. Keep the original security exclusions intact, and only
omit known test-only sources that cannot affect the release Worker binaries.
"""

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from prepare_worker_build_context import context_files, release_closure


ROOT = Path(__file__).resolve().parents[1]
DOCKERFILE = ROOT / "app/rust/services/temporal/Dockerfile"
DOCKER_IGNORE = DOCKERFILE.with_name("Dockerfile.dockerignore")
IGNORED_TEST_SOURCES = (
    "app/rust/crates/integration_harness/src/tests/",
    "app/rust/crates/integration_harness/src/containers.rs",
    "app/rust/crates/integration_harness/src/env_overrides.rs",
    "app/rust/crates/integration_harness/src/stub_servers.rs",
)


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
    if "COPY worker-context.json /app/worker-context.json" not in dockerfile:
        errors.append("Worker image must preserve exact-source context provenance")
    if "cargo build --release --locked" not in dockerfile:
        errors.append("Docker must retain the locked release build")
    if "-p temporal_worker -p cli_tools" not in dockerfile:
        errors.append("Docker release package selection changed: recheck the test-only exclusions")
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

    def test_missing_manifest_fails_closed(self) -> None:
        with self.assertRaisesRegex(ValueError, "workspace manifest"):
            context_files(set(), {"app/rust/services/temporal": "temporal_worker"}, {"temporal_worker"})

if __name__ == "__main__":
    unittest.main()
