#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def read_file_resolved(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if path.name == "main.rs" and "cli_tools" in path.parts:
        sub_dir = path.parent / "cli"
        if sub_dir.is_dir():
            for sub_file in sub_dir.glob("*.rs"):
                text += "\n" + sub_file.read_text(encoding="utf-8")
    return text


def require(path: Path, needle: str, label: str) -> list[str]:
    text = read_file_resolved(path)
    if needle not in text:
        return [f"{label}: missing {needle!r} in {path.relative_to(ROOT)}"]
    return []


def forbid(path: Path, needle: str, label: str) -> list[str]:
    text = read_file_resolved(path)
    if needle in text:
        return [f"{label}: forbidden {needle!r} in {path.relative_to(ROOT)}"]
    return []


def main() -> int:
    failures: list[str] = []

    cli = ROOT / "app/rust/services/cli_tools/src/main.rs"
    adapter = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_static_site_adapter.rs"
    builder = ROOT / "app/rust/crates/infrastructure/src/adapters/static_site_builder_adapter.rs"
    publish_materialize = (
        ROOT
        / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_cms_adapter/publish_materialize.rs"
    )
    adapter_mod = ROOT / "app/rust/crates/infrastructure/src/adapters/mod.rs"
    cargo = ROOT / "app/rust/services/cli_tools/Cargo.toml"
    compose = ROOT / "docker-compose.yml"

    for path in [cli, adapter, builder, publish_materialize, adapter_mod, cargo, compose]:
        if not path.exists():
            failures.append(f"missing required file: {path.relative_to(ROOT)}")

    if not failures:
        failures += require(adapter_mod, "pub mod sqlx_static_site_adapter;", "adapter registration")
        failures += require(cargo, "pulldown-cmark", "markdown rendering dependency")
        failures += require(cli, "BuildStaticSite", "CLI command")
        failures += require(cli, "build_static_artifacts", "static artifact builder")
        failures += require(cli, "sitemap.xml", "sitemap artifact")
        failures += require(cli, "robots.txt", "robots artifact")
        failures += require(cli, "render_breadcrumbs", "breadcrumb renderer")
        failures += require(cli, "BreadcrumbList", "breadcrumb JSON-LD")
        failures += require(cli, "FAQPage", "FAQ JSON-LD")
        failures += require(cli, "alegria-static-manifest.json", "build manifest")
        failures += require(cli, "no approved or published CMS pages", "empty publish guard")

        adapter_text = adapter.read_text(encoding="utf-8")
        public_marker = "pub async fn load_static_site_snapshot"
        candidate_marker = "pub async fn load_static_site_candidate_snapshot"
        if public_marker not in adapter_text or candidate_marker not in adapter_text:
            failures.append("static adapter must define separate public and candidate snapshot loaders")
        else:
            public_start = adapter_text.index(public_marker)
            candidate_start = adapter_text.index(candidate_marker)
            public_text = adapter_text[public_start:candidate_start]
            candidate_text = adapter_text[candidate_start:]
            if "revision_status = 'published'" not in public_text:
                failures.append("public snapshot must select only published revisions")
            if "revision_status = 'approved'" in public_text:
                failures.append("approved revision leaked into public snapshot query")
            if "JOIN LATERAL" not in public_text:
                failures.append(
                    "public snapshot must resolve latest published revision independently of current candidate pointer"
                )
            for needle in [
                "target_page_node_key",
                "target_revision_id",
                "r.revision_status = 'approved'",
                "published_dependencies",
            ]:
                if needle not in candidate_text:
                    failures.append(f"candidate snapshot missing `{needle}`")

        failures += require(adapter, "site.link_recommendations", "internal link source")
        failures += require(builder, "build_static_site_candidate_incremental", "candidate builder")
        failures += require(
            builder,
            "load_static_site_candidate_snapshot",
            "candidate snapshot consumption",
        )
        failures += require(
            publish_materialize,
            "build_static_site_candidate_incremental",
            "publish materialization candidate build",
        )
        failures += require(
            publish_materialize,
            '"public_snapshot_fallback_allowed": false',
            "candidate/public fail-closed separation",
        )
        failures += forbid(
            publish_materialize,
            "build_static_site_incremental(",
            "public incremental builder in candidate materialization",
        )
        failures += forbid(
            publish_materialize,
            "static_site_builder_adapter::build_static_site(",
            "public full-build fallback in candidate materialization",
        )
        failures += require(compose, "/dev/tcp/127.0.0.1/6333", "Qdrant healthcheck")

    if failures:
        print("STATIC_SITE_BUILDER_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("STATIC_SITE_BUILDER_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
