#!/usr/bin/env python3
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1]
REQUIRED = [
    ROOT / "app/rust/crates/integration_harness/Cargo.toml",
    ROOT / "app/rust/crates/integration_harness/src/lib.rs",
    ROOT / "app/rust/crates/integration_harness/src/env_overrides.rs",
    ROOT / "app/rust/crates/integration_harness/src/stub_servers.rs",
    ROOT / "app/rust/crates/integration_harness/src/containers.rs",
]

LIB_REQUIRED_SNIPPETS = [
    "pub fn provider_env(&self)",
    "pub fn apply_provider_env(&self)",
    "DATAFORSEO_ENDPOINT",
    "SEO_LLM_LOCAL_ENDPOINT",
    "InfraHarness",
    "TemporalHarness",
    "synthetic_full_scenario_persists_page_draft_on_full_harness",
]

ENV_REQUIRED_SNIPPETS = [
    "pub struct EnvOverrideGuard",
    "std::env::set_var",
    "std::env::remove_var",
]

CI_REQUIRED_SNIPPETS = [
    "python3 automation/check_integration_harness_present.py",
]

CONTAINERS_REQUIRED_SNIPPETS = [
    "pub struct PostgresHarness",
    "pub struct QdrantHarness",
    "pub struct Neo4jHarness",
    "pub struct TemporalHarness",
    "pub struct InfraHarness",
    "pub fn runtime_env(&self) -> BTreeMap<String, String>",
    "pub fn apply_runtime_env(&self) -> EnvOverrideGuard",
    "\"TEMPORAL_URL\".to_string()",
]


# The harness must use exactly the same real container images as staging.
# Never require stale string literals (e.g. a retired Neo4j 5.26 tag).
IMAGE_CONTRACT = {
    "postgres": ("postgres", "postgres"),
    "qdrant": ("qdrant/qdrant", "qdrant"),
    "neo4j": ("neo4j", "neo4j"),
    "temporal-server": ("temporalio/server", "temporal"),
}


def image_contract_failures(containers: str, compose: str) -> list[str]:
    failures: list[str] = []
    for service, (image_name, _) in IMAGE_CONTRACT.items():
        block = re.search(
            rf"(?ms)^  {re.escape(service)}:\n(.*?)(?=^  [a-zA-Z][\w-]*:|\Z)",
            compose,
        )
        if block is None:
            failures.append(f"compose service missing: {service}")
            continue
        image = re.search(r"(?m)^    image: ([^\s#]+)", block.group(1))
        if image is None:
            failures.append(f"compose image missing: {service}")
            continue
        expected_image = image.group(1)
        if not expected_image.startswith(image_name + ":"):
            failures.append(f"compose image family differs: {service} {expected_image}")
            continue
        expected_tag = expected_image[len(image_name) + 1:]
        snippet = f'GenericImage::new("{image_name}", "{expected_tag}")'
        if snippet not in containers:
            failures.append(f"harness image mismatch: {service} expected {expected_image}")
    return failures


def read_with_includes(path: Path) -> str:
    if not path.exists():
        return ""
    text = path.read_text(encoding="utf-8")
    import re
    parent = path.parent
    for match in re.finditer(r'include!\("([^"]+)"\);', text):
        include_path = parent / match.group(1)
        if include_path.exists():
            text += "\n" + read_with_includes(include_path)
    return text


def main() -> int:
    missing = [str(path.relative_to(ROOT)) for path in REQUIRED if not path.exists()]
    if missing:
        print("FAIL integration_harness missing=" + ",".join(missing))
        return 1

    lib_text = read_with_includes(ROOT / "app/rust/crates/integration_harness/src/lib.rs")
    missing_lib = [snippet for snippet in LIB_REQUIRED_SNIPPETS if snippet not in lib_text]
    if missing_lib:
        print("FAIL integration_harness lib_missing=" + ",".join(missing_lib))
        return 1

    env_text = (ROOT / "app/rust/crates/integration_harness/src/env_overrides.rs").read_text()
    missing_env = [snippet for snippet in ENV_REQUIRED_SNIPPETS if snippet not in env_text]
    if missing_env:
        print("FAIL integration_harness env_missing=" + ",".join(missing_env))
        return 1

    containers_text = (ROOT / "app/rust/crates/integration_harness/src/containers.rs").read_text()
    missing_containers = [
        snippet for snippet in CONTAINERS_REQUIRED_SNIPPETS if snippet not in containers_text
    ]
    if missing_containers:
        print(
            "FAIL integration_harness containers_missing="
            + ",".join(missing_containers)
        )
        return 1

    compose_text = (ROOT / "docker-compose.yml").read_text(encoding="utf-8")
    image_failures = image_contract_failures(containers_text, compose_text)
    if image_failures:
        print("FAIL integration_harness images=" + ",".join(image_failures))
        return 1

    ci_text = (ROOT / "automation/ci_verify.sh").read_text(encoding="utf-8")
    missing_ci = [snippet for snippet in CI_REQUIRED_SNIPPETS if snippet not in ci_text]
    if missing_ci:
        print("FAIL integration_harness ci_missing=" + ",".join(missing_ci))
        return 1

    # Test execution moved to the canonical Rust owner in the single CI.
    # Workspace tests cover integration_harness's default suite; the separate
    # e2e-feature compile remains required. Never recreate duplicate Cargo runs.
    workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    if "  architecture-and-rust:\n" not in workflow or "\n  rust-security:\n" not in workflow:
        print("FAIL integration_harness missing Rust owner")
        return 1
    rust_job = workflow.split("  architecture-and-rust:\n", 1)[1].split("\n  rust-security:\n", 1)[0]
    for snippet in (
        "run: cargo test --workspace --locked",
        "run: cargo test -p integration_harness --features e2e --no-run --locked",
    ):
        if snippet not in rust_job:
            print("FAIL integration_harness missing Rust CI coverage: " + snippet)
            return 1

    print("OK integration_harness contract")
    return 0


if __name__ == "__main__":
    sys.exit(main())
