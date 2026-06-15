#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SERP_ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_serp_adapter.rs"
DATAFORSEO_ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/dataforseo_serp_adapter.rs"
SERP_NORMALIZE = ROOT / "app/rust/crates/seo_steps/src/serp_normalize_step.rs"
SEO_ADAPTER = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_seo_adapter.rs"
SEO_PORTS = ROOT / "app/rust/crates/seo_ports/src/lib.rs"


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
    serp_adapter = read_with_includes(SERP_ADAPTER)
    dataforseo_adapter = read_with_includes(DATAFORSEO_ADAPTER)
    serp_normalize = read_with_includes(SERP_NORMALIZE)
    seo_adapter = read_with_includes(SEO_ADAPTER)
    seo_ports = read_with_includes(SEO_PORTS)

    failures: list[str] = []

    for needle in [
        "INSERT INTO serp.gemini_top10",
        "(run_id, job_id, rank, title, url, url_norm, domain_norm, source_tier, also_in_sources)",
        "source_domain",
        "query_batch_key",
        "result.domain_norm",
        "source_tier",
    ]:
        if needle not in serp_adapter:
            failures.append(f"serp adapter missing `{needle}`")

    for needle in [
        "classify_domain_tier(",
        'assert_eq!(parsed[0].source_tier, "low_trust")',
        'classify_domain_tier("www.mfa.gov.by"',
        'classify_domain_tier("reddit.com"',
    ]:
        if needle not in dataforseo_adapter:
            failures.append(f"dataforseo adapter missing `{needle}`")

    for needle in [
        "normalized_query_count: serp_patterns.len() as u32",
        "pattern_type: \"query_intent\"",
        "dominant_intent",
        "reliability_score",
        "query_batch_key",
    ]:
        if needle not in serp_normalize:
            failures.append(f"serp normalize step missing `{needle}`")

    for needle in [
        "pub struct OrganicSerpResult",
        "source_tier: String",
    ]:
        if needle not in seo_ports:
            failures.append(f"seo_ports missing `{needle}`")

    for needle in [
        '("government", _) | (_, 5) => "official"',
        '("vfs", _) | (_, 4) => "regulated_partner"',
        '("niche_agency", _) => "industry_reference"',
        '_ => "editorial_reference"',
    ]:
        if needle not in seo_adapter:
            failures.append(f"source tier mapping missing `{needle}`")

    if failures:
        print("SERP_INTELLIGENCE_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("SERP_INTELLIGENCE_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())

