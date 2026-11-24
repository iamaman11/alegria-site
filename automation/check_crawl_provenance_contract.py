#!/usr/bin/env python3
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "app/db/schema.sql"
RAW = ROOT / "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter.rs"
PORTS = ROOT / "app/rust/crates/runtime_composition/src/adapters/seo_ports_sqlx_adapter.rs"
SERP = ROOT / "app/rust/crates/infrastructure/src/adapters/sqlx_serp_adapter.rs"
WORKFLOW = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover.rs"
SOURCE_ACQUISITION = ROOT / "app/rust/services/temporal/src/workflows/seo_site_build_canonical_cutover/source_acquisition.rs"
RAW_TYPES = ROOT / "app/rust/crates/infrastructure/src/adapters/raw_crawl_adapter/types.rs"


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
    schema = SCHEMA.read_text(encoding="utf-8")
    raw = read_with_includes(RAW)
    ports = read_with_includes(PORTS)
    workflow = read_with_includes(WORKFLOW)
    source_acquisition = SOURCE_ACQUISITION.read_text(encoding="utf-8")
    raw_types = RAW_TYPES.read_text(encoding="utf-8")
    serp = SERP.read_text(encoding="utf-8")

    failures: list[str] = []

    for needle in [
        "final_url       TEXT",
        "redirect_chain  JSONB",
        "robots_trace    JSONB",
        "source_observation JSONB",
    ]:
        if needle not in schema:
            failures.append(f"schema missing `{needle}`")

    for needle in [
        "pub struct FetchedHtml",
        "pub struct CrawlObservationTrace",
        "source_url: String",
        "final_url: String",
        "redirect_chain: Vec<String>",
        "robots: RobotsDecisionTrace",
        "pub async fn save_crawled_html(",
        "source_url: &str",
        "final_url: &str",
        "redirect_chain: &[String]",
        "robots_trace: &RobotsDecisionTrace",
        "source_observation_json",
        "&fetched.source_url",
        "&fetched.final_url",
        "&fetched.redirect_chain",
        "&robots_trace",
        "forum",
        "low_trust",
    ]:
        if needle not in raw and needle not in ports:
            failures.append(f"provenance contract missing `{needle}`")

    # A recrawl must append a complete, provenance-bearing snapshot, never
    # mutate or discard previously referenced raw evidence sections.
    snapshot_write = raw.split("pub async fn save_crawled_html(", 1)[-1].split(
        "pub async fn record_content_hash_alias_linkage(", 1
    )[0]
    for needle in [
        "INSERT " + "INTO raw.pages",
        "final_url, redirect_chain, robots_trace,",
        "source_observation, title, meta_desc",
        ".bind(redirect_chain_json)",
        ".bind(robots_trace_json)",
        ".bind(source_observation_json)",
        "RETURNING id",
    ]:
        if needle not in snapshot_write:
            failures.append(f"immutable crawl provenance missing `{needle}`")
    if "UPDATE raw.pages" in snapshot_write or (
        "DELETE " + "FROM raw.sections"
    ) in snapshot_write:
        failures.append("recrawl must not modify or remove existing evidence snapshots")

    for needle in ["MAX_RAW_SECTION_BYTES", "MAX_CONTEXTUAL_GROUP_BYTES", "HtmlSection"]:
        if needle not in raw_types:
            failures.append(f"retrieval-safe section contract missing `{needle}`")
    for needle in ["bounded_raw_sections(", "contextual_section_groups(", ".flatten()", "raw section embedding failed:"]:
        if needle not in (raw + ports):
            failures.append(f"oversized contextual embedding handling missing `{needle}`")
    if "limit: 5" not in source_acquisition or source_acquisition.count("db_opts(600)") < 2:
        failures.append("crawl activity must fit a bounded network/embedding timeout")

    # A URL completed by a prior run must be eligible again in a fresh run;
    # otherwise all overlapping top-ten competitors disappear from that run.
    organic_upsert = serp.split("pub async fn save_dataforseo_organic_results_and_enqueue(", 1)[-1].split(
        "pub async fn enqueue_crawl(", 1
    )[0]
    for needle in [
        "serp.crawl_queue.first_seen_run_id = EXCLUDED.first_seen_run_id",
        "serp.crawl_queue.locked_until > now()",
        "locked_until = CASE",
        "crawl_attempt_count = CASE",
        "ELSE 0",
    ]:
        if needle not in organic_upsert:
            failures.append(f"cross-run top-ten recrawl incomplete: missing `{needle}`")
    # Regression: same-domain documents and >25 unique URLs must not disappear.
    crawl_claim = raw.split("pub async fn claim_pending_crawl_batch(", 1)[-1].split(
        "pub async fn crawl_queue_backlog(", 1
    )[0]
    if "DISTINCT ON" in crawl_claim or "per_domain" in crawl_claim:
        failures.append("crawl claim still discards URLs on domain boundaries")
    for needle in [
        "FOR UPDATE SKIP LOCKED",
        "status = 'processing' AND locked_until IS NOT NULL AND locked_until <= now()",
        "pub async fn crawl_queue_backlog(",
        "status IN ('pending', 'processing')",
        '"more"',
        '"pending_retry"',
    ]:
        if needle not in (raw + ports):
            failures.append(f"crawl completeness missing `{needle}`")
    for needle in [
        'while crawled.status == "more" || crawled.status == "pending_retry"',
        "crawled.raw_page_ids.extend(next.raw_page_ids)",
        'ctx.timer(Duration::from_secs(60)).await',
    ]:
        if needle not in source_acquisition:
            failures.append(f"durable crawl drain missing `{needle}`")
    if '"crawl_unresolved_pending_retry"' in workflow:
        failures.append("retry pending must not terminate the canonical workflow")
    if 'failed_count += 1;' not in ports:
        failures.append("terminal crawl failures not counted")
    phases = [
        '"triple_builder".to_string()',
        '"serp_normalize".to_string()',
        '"opportunity_build".to_string()',
        "await_manual_truth_candidates(ctx, &run_id, &validation_input).await?",
        '"truth_adjudication".to_string()',
        '"verified_truth_write".to_string()',
        '"ia_build".to_string()',
        '"link_recommend".to_string()',
        '"global_site_reconcile".to_string()',
        '"graph_admissibility_gate".to_string()',
        '"seo_materialization_gate".to_string()',
    ]
    positions = [workflow.find(phase) for phase in phases]
    if any(index < 0 for index in positions) or positions != sorted(positions):
        failures.append(
            "demand planning must precede fact verification; evidence-aware IA and reconciliation must follow verified truth and precede publication gates"
        )
    # The final structure and coverage must actually consume the verified
    # bundle; changing phase names alone is not sufficient.
    for activity in ("IaBuildInputPayload", "GlobalSiteReconcileInputPayload"):
        request = workflow.split(f"{activity} {{", 1)[-1].split("},", 1)[0]
        if "verified_support: support_bundle.clone()" not in request:
            failures.append(f"{activity} must use admitted verified support")

    if failures:
        print("CRAWL_PROVENANCE_CONTRACT: FAILED")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print("CRAWL_PROVENANCE_CONTRACT: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
