#!/usr/bin/env python3
"""Guard the native Worker image cache boundary against unrelated test edits.

Docker automatically uses Dockerfile.dockerignore instead of the root ignore
for this Dockerfile. Keep the original security exclusions intact, and only
omit known test-only sources that cannot affect the release Worker binaries.
"""

from pathlib import Path
import unittest


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


if __name__ == "__main__":
    unittest.main()
