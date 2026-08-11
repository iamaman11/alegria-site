use primitives::errors::DomainError;
use serde_json::Value;
use sqlx::{types::Json, PgPool, Row};

use super::proto_runtime_payload_store::classify_sqlx;

#[derive(Debug, Clone)]
pub struct StaticCmsPageRow {
    pub page_node_key: String,
    pub canonical_url_path: String,
    pub locale_code: String,
    pub page_type_key: String,
    pub dominant_intent_key: String,
    pub current_status: String,
    pub cms_document_id: String,
    pub revision_id: String,
    pub title: String,
    pub meta_description: String,
    pub h1: String,
    pub body_payload: Value,
    pub schema_markup_payload: Value,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct StaticCmsLinkRow {
    pub source_page_key: String,
    pub target_page_key: String,
    pub link_role: String,
    pub required_flag: bool,
}

#[derive(Debug, Clone)]
pub struct StaticSiteSnapshot {
    pub pages: Vec<StaticCmsPageRow>,
    pub links: Vec<StaticCmsLinkRow>,
}

pub async fn load_static_site_snapshot(pool: &PgPool) -> Result<StaticSiteSnapshot, DomainError> {
    let rows = sqlx::query(
        r#"
        SELECT
            p.page_node_key,
            p.canonical_url_path,
            p.locale_code,
            p.page_type_key,
            p.dominant_intent_key,
            p.current_status,
            p.cms_document_id,
            r.revision_id,
            r.title,
            r.meta_description,
            r.h1,
            r.body_payload,
            r.schema_markup_payload,
            COALESCE(p.published_at, r.updated_at)::text AS updated_at
        FROM site.cms_pages p
        JOIN site.cms_page_revisions r ON r.revision_id = p.current_revision_id
        WHERE p.current_status = 'published'
          AND r.revision_status = 'published'
        ORDER BY
            CASE WHEN p.canonical_url_path = '/' THEN 0 ELSE 1 END,
            p.canonical_url_path
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let pages: Vec<StaticCmsPageRow> = rows
        .into_iter()
        .map(|row| {
            let body_payload: Json<Value> = row.get("body_payload");
            let schema_markup_payload: Json<Value> = row.get("schema_markup_payload");
            StaticCmsPageRow {
                page_node_key: row.get("page_node_key"),
                canonical_url_path: row.get("canonical_url_path"),
                locale_code: row.get("locale_code"),
                page_type_key: row.get("page_type_key"),
                dominant_intent_key: row.get("dominant_intent_key"),
                current_status: row.get("current_status"),
                cms_document_id: row.get("cms_document_id"),
                revision_id: row.get("revision_id"),
                title: row.get("title"),
                meta_description: row.get("meta_description"),
                h1: row.get("h1"),
                body_payload: body_payload.0,
                schema_markup_payload: schema_markup_payload.0,
                updated_at: row.get("updated_at"),
            }
        })
        .collect();

    if pages.is_empty() {
        return Ok(StaticSiteSnapshot {
            pages,
            links: Vec::new(),
        });
    }

    let page_keys: Vec<String> = pages
        .iter()
        .map(|page| page.page_node_key.clone())
        .collect();
    let link_rows = sqlx::query(
        r#"
        SELECT
            l.source_page_key,
            l.target_page_key,
            l.link_role,
            l.required_flag
        FROM site.link_recommendations l
        JOIN site.cms_pages target_page ON target_page.page_node_key = l.target_page_key
        WHERE l.source_page_key = ANY($1)
          AND l.status IN ('candidate', 'accepted', 'applied')
          AND target_page.current_status = 'published'
        ORDER BY l.required_flag DESC, l.score DESC, l.target_page_key
        "#,
    )
    .bind(&page_keys)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let links = link_rows
        .into_iter()
        .map(|row| StaticCmsLinkRow {
            source_page_key: row.get("source_page_key"),
            target_page_key: row.get("target_page_key"),
            // `link_role` is an internal graph/planning concept. Keep it out of the
            // public rendering surface even though the renderer retains the field.
            link_role: String::new(),
            required_flag: row.get("required_flag"),
        })
        .collect();

    Ok(StaticSiteSnapshot { pages, links })
}