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

fn page_from_row(row: sqlx::postgres::PgRow) -> StaticCmsPageRow {
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
}

async fn load_snapshot_links(
    pool: &PgPool,
    page_keys: &[String],
) -> Result<Vec<StaticCmsLinkRow>, DomainError> {
    if page_keys.is_empty() {
        return Ok(Vec::new());
    }
    let link_rows = sqlx::query(
        r#"
        SELECT
            l.source_page_key,
            l.target_page_key,
            l.link_role,
            l.required_flag
        FROM site.link_recommendations l
        WHERE l.source_page_key = ANY($1)
          AND l.target_page_key = ANY($1)
          AND l.status IN ('candidate', 'accepted', 'applied')
        ORDER BY l.required_flag DESC, l.score DESC, l.target_page_key
        "#,
    )
    .bind(page_keys)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    Ok(link_rows
        .into_iter()
        .map(|row| StaticCmsLinkRow {
            source_page_key: row.get("source_page_key"),
            target_page_key: row.get("target_page_key"),
            // `link_role` is internal graph/planning metadata. Keep it out of public HTML.
            link_role: String::new(),
            required_flag: row.get("required_flag"),
        })
        .collect())
}

/// Public serving/build snapshot.
///
/// A page remains public from its latest published revision even while a newer
/// revision is review_required/approved/blocked. `cms_pages.current_revision_id`
/// is the working candidate pointer and therefore must not define public truth.
pub async fn load_static_site_snapshot(pool: &PgPool) -> Result<StaticSiteSnapshot, DomainError> {
    let rows = sqlx::query(
        r#"
        SELECT
            p.page_node_key,
            p.canonical_url_path,
            p.locale_code,
            p.page_type_key,
            p.dominant_intent_key,
            'published'::text AS current_status,
            p.cms_document_id,
            r.revision_id,
            r.title,
            r.meta_description,
            r.h1,
            r.body_payload,
            r.schema_markup_payload,
            r.updated_at::text AS updated_at
        FROM site.cms_pages p
        JOIN LATERAL (
            SELECT
                revision_id,
                title,
                meta_description,
                h1,
                body_payload,
                schema_markup_payload,
                updated_at
            FROM site.cms_page_revisions published_revision
            WHERE published_revision.page_node_key = p.page_node_key
              AND published_revision.revision_status = 'published'
            ORDER BY published_revision.updated_at DESC, published_revision.revision_id DESC
            LIMIT 1
        ) r ON TRUE
        ORDER BY
            CASE WHEN p.canonical_url_path = '/' THEN 0 ELSE 1 END,
            p.canonical_url_path
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let pages = rows.into_iter().map(page_from_row).collect::<Vec<_>>();
    let page_keys = pages
        .iter()
        .map(|page| page.page_node_key.clone())
        .collect::<Vec<_>>();
    let links = load_snapshot_links(pool, &page_keys).await?;
    Ok(StaticSiteSnapshot { pages, links })
}

/// Candidate build snapshot used before final publication.
///
/// It contains exactly the requested approved revision for the target page plus
/// the latest published revision for every other page. The target candidate is
/// therefore previewable without entering the public snapshot prematurely.
pub async fn load_static_site_candidate_snapshot(
    pool: &PgPool,
    target_page_node_key: &str,
    target_revision_id: &str,
) -> Result<StaticSiteSnapshot, DomainError> {
    if target_page_node_key.trim().is_empty() || target_revision_id.trim().is_empty() {
        return Err(DomainError::ValidationFailure {
            message: "candidate static snapshot requires target page_node_key and revision_id"
                .to_string(),
        });
    }

    let rows = sqlx::query(
        r#"
        WITH candidate AS (
            SELECT
                p.page_node_key,
                p.canonical_url_path,
                p.locale_code,
                p.page_type_key,
                p.dominant_intent_key,
                'approved'::text AS current_status,
                p.cms_document_id,
                r.revision_id,
                r.title,
                r.meta_description,
                r.h1,
                r.body_payload,
                r.schema_markup_payload,
                r.updated_at::text AS updated_at
            FROM site.cms_pages p
            JOIN site.cms_page_revisions r
              ON r.page_node_key = p.page_node_key
            WHERE p.page_node_key = $1
              AND r.revision_id = $2
              AND r.revision_status = 'approved'
        ),
        published_dependencies AS (
            SELECT
                p.page_node_key,
                p.canonical_url_path,
                p.locale_code,
                p.page_type_key,
                p.dominant_intent_key,
                'published'::text AS current_status,
                p.cms_document_id,
                r.revision_id,
                r.title,
                r.meta_description,
                r.h1,
                r.body_payload,
                r.schema_markup_payload,
                r.updated_at::text AS updated_at
            FROM site.cms_pages p
            JOIN LATERAL (
                SELECT
                    revision_id,
                    title,
                    meta_description,
                    h1,
                    body_payload,
                    schema_markup_payload,
                    updated_at
                FROM site.cms_page_revisions published_revision
                WHERE published_revision.page_node_key = p.page_node_key
                  AND published_revision.revision_status = 'published'
                ORDER BY published_revision.updated_at DESC, published_revision.revision_id DESC
                LIMIT 1
            ) r ON TRUE
            WHERE p.page_node_key <> $1
        )
        SELECT * FROM candidate
        UNION ALL
        SELECT * FROM published_dependencies
        ORDER BY canonical_url_path
        "#,
    )
    .bind(target_page_node_key)
    .bind(target_revision_id)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let pages = rows.into_iter().map(page_from_row).collect::<Vec<_>>();
    if !pages.iter().any(|page| {
        page.page_node_key == target_page_node_key && page.revision_id == target_revision_id
    }) {
        return Err(DomainError::ValidationFailure {
            message: format!(
                "approved candidate revision not found for static build: page_node_key=`{target_page_node_key}` revision_id=`{target_revision_id}`"
            ),
        });
    }

    let page_keys = pages
        .iter()
        .map(|page| page.page_node_key.clone())
        .collect::<Vec<_>>();
    let links = load_snapshot_links(pool, &page_keys).await?;
    Ok(StaticSiteSnapshot { pages, links })
}
