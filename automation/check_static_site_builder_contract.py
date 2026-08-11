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
    adapter_mod = ROOT / "app/rust/crates/infrastructure/src/adapters/mod.rs"
    cargo = ROOT / "app/rust/services/cli_tools/Cargo.toml"
    compose = ROOT / "docker-compose.yml"

    for path in [cli, adapter, adapter_mod, cargo, compose]:
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
        failures += require(adapter, "p.current_status = 'published'", "CMS page publish gate")
        failures += require(adapter, "r.revision_status = 'published'", "CMS revision publish gate")
        failures += require(adapter, "target_page.current_status = 'published'", "link target publish gate")
        failures += forbid(
            adapter,
            "p.current_status IN ('approved', 'published')",
            "approved page public-export ban",
        )
        failures += forbid(
            adapter,
            "r.revision_status IN ('approved', 'published')",
            "approved revision public-export ban",
        )
        failures += require(adapter, "site.link_recommendations", "internal link source")
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
