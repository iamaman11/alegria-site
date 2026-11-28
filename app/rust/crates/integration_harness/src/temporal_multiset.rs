//! Shared live Temporal step-history witness for independent E2E owners.

use sqlx::Row;
use std::collections::BTreeMap;
use uuid::Uuid;

pub async fn temporal_step_multiset(pool: &sqlx::PgPool, run_id: &str) -> BTreeMap<String, usize> {
    let allowed = [
        "serp_ingest",
        "crawl_sources",
        "raw_knowledge_ingestion",
        "serp_normalize",
        "opportunity_build",
        "ia_build",
        "link_recommend",
        "global_site_reconcile",
        "draft_assemble",
        "editorial_draft_generate",
        "draft_normalize",
        "content_contract_validate",
        "draft_qa",
        "rebuild_detect",
    ];
    let rows = sqlx::query(
        r#"
            SELECT step_name
            FROM pipeline.step_executions
            WHERE run_id = $1
              AND status = 'done'
            ORDER BY step_name
            "#,
    )
    .bind(Uuid::parse_str(run_id).unwrap())
    .fetch_all(pool)
    .await
    .unwrap();
    let mut counts = BTreeMap::new();
    for row in rows {
        let step_name: String = row.get("step_name");
        if allowed.contains(&step_name.as_str()) {
            *counts.entry(step_name).or_insert(0) += 1;
        }
    }
    counts
}
