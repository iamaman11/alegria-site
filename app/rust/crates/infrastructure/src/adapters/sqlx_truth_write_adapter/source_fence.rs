//! Verify prepared extraction provenance under the exact raw-writer advisory
//! locks before the atomic verified Truth/Outbox commit. LLM inference and
//! embeddings happen outside SQL and do not grant the caller a stale source
//! write once a crawler/editor has changed its evidence.

use std::collections::BTreeSet;

use primitives::errors::DomainError;
use sqlx::{Postgres, Row, Transaction};

use super::{raw_crawl_adapter, BTreeMap};

pub(in crate::adapters) async fn lock_and_validate_prepared_sources_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    raw_by_section: &BTreeMap<i64, raw_crawl_adapter::RawSectionRecord>,
) -> Result<(), DomainError> {
    // The caller already holds truth-context. Match raw writer/V3 receipt
    // ordering: page identities first, then section identities, both sorted.
    let pages = raw_by_section
        .values()
        .map(|section| section.page_id)
        .collect::<BTreeSet<_>>();
    for page_id in pages {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("raw-evidence|pages|{page_id}"))
            .execute(&mut **tx)
            .await
            .map_err(super::super::proto_runtime_payload_store::classify_sqlx)?;
    }
    for section_id in raw_by_section.keys() {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("raw-evidence|sections|{section_id}"))
            .execute(&mut **tx)
            .await
            .map_err(super::super::proto_runtime_payload_store::classify_sqlx)?;
    }
    // Re-read *after* all locks, not before. A section->page parent rewrite
    // that won while the writer was waiting must not authorize bytes on a
    // newly-unlocked page. Every source field consumed by the extractor is
    // compared against the actual durable row before canonical Truth changes.
    for (section_id, prepared) in raw_by_section {
        let durable = sqlx::query(
            "SELECT s.page_id, COALESCE(s.heading_path,'') AS heading_path,
                    COALESCE(s.section_type,'') AS section_type,
                    s.content_md, COALESCE(s.content_hash,'') AS content_hash,
                    p.url AS source_url, p.domain AS source_domain,
                    COALESCE(p.dtype,'') AS source_dtype
             FROM raw.sections s JOIN raw.pages p ON p.id=s.page_id
             WHERE s.id=$1",
        )
        .bind(section_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(super::super::proto_runtime_payload_store::classify_sqlx)?;
        let matches_prepared = durable.is_some_and(|row| {
            row.get::<i64, _>("page_id") == prepared.page_id
                && row.get::<String, _>("heading_path") == prepared.heading_path
                && row.get::<String, _>("section_type") == prepared.section_type
                && row.get::<String, _>("content_md") == prepared.content_md
                && row.get::<String, _>("content_hash") == prepared.content_hash
                && row.get::<String, _>("source_url") == prepared.source_url
                && row.get::<String, _>("source_domain") == prepared.source_domain
                && row.get::<String, _>("source_dtype") == prepared.source_dtype
        });
        if !matches_prepared {
            return Err(DomainError::ConflictViolation {
                message: format!(
                    "raw source section {section_id} changed after verified Truth preparation; re-extract and re-adjudicate"
                ),
            });
        }
    }
    Ok(())
}
