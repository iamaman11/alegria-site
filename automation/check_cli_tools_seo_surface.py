#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
CLI_MAIN = ROOT / "app" / "rust" / "services" / "cli_tools" / "src" / "main.rs"
CMS_DECISION = ROOT / "app" / "rust" / "crates" / "infrastructure" / "src" / "adapters" / "sqlx_seo_cms_adapter" / "human_review_decision.rs"
WORKER_DOCKERFILE = ROOT / "app" / "rust" / "services" / "temporal" / "Dockerfile"

FORBIDDEN = [
    "async fn signal_seo_workflow_resume(",
    "async fn resolve_seo_workflow_id(",
    "sqlx_seo_cms_adapter::apply_human_review_decision(",
    "connect_client(",
    "UntypedSignal::<UntypedWorkflow>::new(\"resume\")",
]


def main() -> int:
    text = CLI_MAIN.read_text(encoding="utf-8")
    errors = [needle for needle in FORBIDDEN if needle in text]
    # Expose the existing operator CLI without a second service.
    docker = WORKER_DOCKERFILE.read_text(encoding="utf-8")
    if "-p cli_tools --bin cli_tools" not in docker or (
        "COPY --from=builder /app/app/rust/target/release/cli_tools /usr/local/bin/cli_tools"
        not in docker
    ):
        errors.append("Temporal Worker runtime must carry existing cli_tools operator binary")
    adapter = CMS_DECISION.read_text(encoding="utf-8")
    resolver = adapter.find("let workflow_id = resolve_seo_workflow_id(")
    begin = adapter.find("let mut tx = pool.begin()")
    commit = adapter.find("tx.commit().await")
    writes = adapter.count(".execute(&mut *tx)")
    if not (0 <= resolver < begin < commit and writes == 2):
        errors.append("CMS review must resolve canonical workflow before two atomic DB writes")
    if errors:
        print("CLI_TOOLS_SEO_SURFACE: FAILED")
        for needle in errors:
            print(f"- forbidden direct SEO mutation surface in cli_tools: {needle}")
        return 1
    print("CLI_TOOLS_SEO_SURFACE: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
