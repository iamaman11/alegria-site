#!/usr/bin/env python3
from pathlib import Path
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
    "cargo test -q -p integration_harness",
    "cargo test -q -p integration_harness --features e2e --no-run",
]

CONTAINERS_REQUIRED_SNIPPETS = [
    "pub struct PostgresHarness",
    "pub struct QdrantHarness",
    "pub struct Neo4jHarness",
    "pub struct TemporalHarness",
    "pub struct InfraHarness",
    "pub fn runtime_env(&self) -> BTreeMap<String, String>",
    "pub fn apply_runtime_env(&self) -> EnvOverrideGuard",
    "GenericImage::new(\"postgres\", \"16-alpine\")",
    "GenericImage::new(\"qdrant/qdrant\", \"v1.13.2\")",
    "GenericImage::new(\"neo4j\", \"5.26.0-community\")",
    "GenericImage::new(\"temporalio/auto-setup\", \"1.28\")",
    "\"TEMPORAL_URL\".to_string()",
]


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

    ci_text = (ROOT / "automation/ci_verify.sh").read_text()
    missing_ci = [snippet for snippet in CI_REQUIRED_SNIPPETS if snippet not in ci_text]
    if missing_ci:
        print("FAIL integration_harness ci_missing=" + ",".join(missing_ci))
        return 1

    print("OK integration_harness contract")
    return 0


if __name__ == "__main__":
    sys.exit(main())
