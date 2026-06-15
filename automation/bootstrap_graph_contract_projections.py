#!/usr/bin/env python3
from __future__ import annotations

import os
import subprocess


def database_url() -> str:
    return os.environ.get(
        "DATABASE_URL", "postgres://postgres:postgres_password@localhost:5433/alegria"
    )


def psql(sql: str) -> None:
    proc = subprocess.run(
        ["psql", database_url(), "-v", "ON_ERROR_STOP=1", "-c", " ".join(sql.split())],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or "psql failed")


def main() -> int:
    scope = "alegria-site|ru-RU|ES|tourist||standard"
    cluster = "graph_contract_probe_cluster"
    blueprint = "graph_contract_probe_blueprint"
    page_a = "graph_contract_probe_page_a"
    page_b = "graph_contract_probe_page_b"
    batch = "graph_contract_probe_batch"
    serp_pattern = "graph_contract_probe_serp_pattern"
    gap = "graph_contract_probe_gap"
    link = "graph_contract_probe_link"
    brief = "graph_contract_probe_brief"
    psql(
        f"""
        INSERT INTO site.keyword_clusters
            (cluster_key, scope_signature, market, locale, country_code, visa_type,
             applicant_profile, seed_keyword, dominant_intent, status, updated_at)
        VALUES
            ('{cluster}', '{scope}', 'alegria-site', 'ru-RU', 'ES', 'tourist',
             'standard', 'graph contract probe', 'informational', 'accepted', now())
        ON CONFLICT (cluster_key) DO UPDATE
        SET status = EXCLUDED.status, updated_at = now();

        INSERT INTO site.page_blueprints
            (blueprint_key, page_type_key, dominant_intent, scope_class, title_pattern,
             section_plan, status, updated_at)
        VALUES
            ('{blueprint}', 'requirement_page', 'informational', 'visa_scope',
             'Graph contract probe', '[]'::jsonb, 'active', now())
        ON CONFLICT (blueprint_key) DO UPDATE
        SET status = EXCLUDED.status, updated_at = now();

        INSERT INTO site.page_nodes
            (page_node_key, scope_signature, keyword_cluster_key, blueprint_key, page_type_key,
             dominant_intent, canonical_slug, canonical_url_path, hierarchy_depth,
             lifecycle_state, updated_at)
        VALUES
            ('{page_a}', '{scope}', '{cluster}', '{blueprint}', 'requirement_page',
             'informational', 'graph-contract-probe-a', '/graph-contract-probe-a',
             0, 'planned', now()),
            ('{page_b}', '{scope}', '{cluster}', '{blueprint}', 'requirement_page',
             'informational', 'graph-contract-probe-b', '/graph-contract-probe-b',
             0, 'planned', now())
        ON CONFLICT (page_node_key) DO UPDATE
        SET lifecycle_state = EXCLUDED.lifecycle_state, updated_at = now();

        INSERT INTO serp.query_batches
            (query_batch_key, scope_signature, market, locale, source_system, status, updated_at)
        VALUES
            ('{batch}', '{scope}', 'alegria-site', 'ru-RU', 'graph_contract_probe', 'done', now())
        ON CONFLICT (query_batch_key) DO UPDATE
        SET status = EXCLUDED.status, updated_at = now();

        INSERT INTO serp.serp_patterns
            (serp_pattern_key, query_batch_key, scope_signature, query, pattern_type,
             dominant_intent, reliability_score, evidence_ref, status, updated_at)
        VALUES
            ('{serp_pattern}', '{batch}', '{scope}', 'graph contract probe',
             'query_intent', 'informational', 1.0, '{batch}', 'active', now())
        ON CONFLICT (serp_pattern_key) DO UPDATE
        SET status = EXCLUDED.status, updated_at = now();

        INSERT INTO site.content_gaps
            (content_gap_key, scope_signature, page_node_key, missing_topic, severity,
             evidence_ref, status, updated_at)
        VALUES
            ('{gap}', '{scope}', '{page_a}', 'graph contract probe gap',
             'low', 'graph_contract_probe', 'accepted', now())
        ON CONFLICT (content_gap_key) DO UPDATE
        SET status = EXCLUDED.status, updated_at = now();

        INSERT INTO site.link_recommendations
            (link_recommendation_key, scope_signature, source_page_key, target_page_key,
             link_role, anchor_strategy, required_flag, score, status, updated_at)
        VALUES
            ('{link}', '{scope}', '{page_a}', '{page_b}',
             'contextual', 'descriptive', false, 1.0, 'accepted', now())
        ON CONFLICT (link_recommendation_key) DO UPDATE
        SET status = EXCLUDED.status, updated_at = now();

        INSERT INTO site.page_briefs
            (page_brief_key, page_node_key, blueprint_key, title, meta_description,
             required_sections, required_links, evidence_manifest, status, updated_at)
        VALUES
            ('{brief}', '{page_a}', '{blueprint}', 'Graph contract probe',
             'Graph contract probe', '[]'::jsonb, '[]'::jsonb, '[]'::jsonb, 'ready', now())
        ON CONFLICT (page_brief_key) DO UPDATE
        SET status = EXCLUDED.status, updated_at = now();
        """
    )
    print("BOOTSTRAP_GRAPH_PROJECTIONS: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
