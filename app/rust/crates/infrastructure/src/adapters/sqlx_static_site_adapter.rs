use primitives::errors::DomainError;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{types::Json, PgPool, Row};
use std::collections::BTreeSet;

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
    pub canonical_url_path: String,
    pub required_flag: bool,
}

#[derive(Debug, Clone)]
pub struct StaticNavigationItemRow {
    pub navigation_tree_key: String,
    pub market: String,
    pub locale: String,
    pub page_node_key: String,
    pub label: String,
    pub group_sort_order: i32,
    pub hierarchy_depth: i32,
    pub sort_order: i32,
}

#[derive(Debug, Clone)]
pub struct StaticSiteSnapshot {
    pub pages: Vec<StaticCmsPageRow>,
    pub links: Vec<StaticCmsLinkRow>,
    pub navigation_items: Vec<StaticNavigationItemRow>,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovedRevisionLink {
    target_page_key: String,
    canonical_url_path: String,
    label: String,
    link_role: String,
}

/// Only a CMS revision can approve public links. Live planning recommendations
/// (`candidate`/`accepted`) must never rewrite a published release's links.
/// Retain the revision's exact target path so an unpublished or changed target
/// fails closed in the static builder instead of silently disappearing.
fn approved_links_from_revision(
    source_page_key: &str,
    payload: Value,
) -> Result<Vec<StaticCmsLinkRow>, DomainError> {
    let links: Vec<ApprovedRevisionLink> =
        serde_json::from_value(payload).map_err(|error| DomainError::ContractViolation {
            message: format!(
                "invalid CMS required_link_payload for page {source_page_key}: {error}"
            ),
        })?;
    let mut seen = BTreeSet::new();
    let mut result = Vec::with_capacity(links.len());
    for link in links {
        if link.target_page_key.trim().is_empty()
            || link.canonical_url_path.trim().is_empty()
            || link.label.trim().is_empty()
            || link.link_role.trim().is_empty()
            || link.target_page_key == source_page_key
            || !seen.insert(link.target_page_key.clone())
        {
            return Err(DomainError::ContractViolation {
                message: format!(
                    "invalid or duplicate approved CMS link for page {source_page_key}"
                ),
            });
        }
        result.push(StaticCmsLinkRow {
            source_page_key: source_page_key.to_string(),
            target_page_key: link.target_page_key,
            canonical_url_path: link.canonical_url_path,
            required_flag: true,
        });
    }
    Ok(result)
}

fn pages_and_approved_links(
    rows: Vec<sqlx::postgres::PgRow>,
) -> Result<(Vec<StaticCmsPageRow>, Vec<StaticCmsLinkRow>), DomainError> {
    let mut pages = Vec::with_capacity(rows.len());
    let mut links = Vec::new();
    for row in rows {
        let source_page_key: String = row.get("page_node_key");
        let payload: Json<Value> = row.get("required_link_payload");
        links.extend(approved_links_from_revision(&source_page_key, payload.0)?);
        pages.push(page_from_row(row));
    }
    Ok((pages, links))
}

/// Public navigation is a versioned planning projection. A rendered release
/// may link to its planned items ONLY when those exact pages are also present
/// in the approved/published CMS snapshot passed to the static builder.
async fn load_navigation_items(
    pool: &PgPool,
    published_page_keys: &[String],
) -> Result<Vec<StaticNavigationItemRow>, DomainError> {
    if published_page_keys.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        r#"
        SELECT t.navigation_tree_key, t.market, t.locale,
               i.page_node_key, i.label,
               COALESCE(g.sort_order, 1000) AS group_sort_order,
               i.hierarchy_depth, i.sort_order
        FROM site.navigation_trees t
        JOIN site.navigation_items i ON i.navigation_tree_key = t.navigation_tree_key
        LEFT JOIN site.silo_groups g ON g.silo_group_key = i.silo_group_key
        JOIN site.cms_pages p ON p.page_node_key = i.page_node_key
        JOIN site.site_scopes s ON s.scope_signature = p.scope_signature
        WHERE t.status = 'active' AND i.status = 'active'
          AND i.item_type = 'page' AND i.page_node_key = ANY($1)
          AND i.scope_signature = p.scope_signature
          AND t.market = s.market AND t.locale = s.locale
          AND (i.silo_group_key IS NULL OR g.status = 'active')
        ORDER BY t.market, t.locale, t.navigation_tree_key,
                 COALESCE(g.sort_order, 1000), i.sort_order,
                 i.hierarchy_depth, i.url_path, i.navigation_item_key
        "#,
    )
    .bind(published_page_keys)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|row| StaticNavigationItemRow {
            navigation_tree_key: row.get("navigation_tree_key"),
            market: row.get("market"),
            locale: row.get("locale"),
            page_node_key: row.get("page_node_key"),
            label: row.get("label"),
            group_sort_order: row.get("group_sort_order"),
            hierarchy_depth: row.get("hierarchy_depth"),
            sort_order: row.get("sort_order"),
        })
        .collect())
}

/// Public serving/build snapshot.
///
/// A published page is served only from its explicitly committed current
/// revision, never from timestamps of older revisions. A newer candidate
/// remains separate from `cms_pages.current_revision_id` until HTTPS proof.
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
            r.required_link_payload,
            r.updated_at::text AS updated_at
        FROM site.cms_pages p
        JOIN site.cms_page_revisions r
          ON r.page_node_key = p.page_node_key
         AND r.revision_id = p.current_revision_id
         AND r.revision_status = 'published'
        ORDER BY
            CASE WHEN p.canonical_url_path = '/' THEN 0 ELSE 1 END,
            p.canonical_url_path
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let (pages, links) = pages_and_approved_links(rows)?;
    let page_keys = pages.iter().map(|page| page.page_node_key.clone()).collect::<Vec<_>>();
    let navigation_items = load_navigation_items(pool, &page_keys).await?;
    Ok(StaticSiteSnapshot { pages, links, navigation_items })
}

/// Candidate build snapshot used before final publication.
///
/// It contains exactly the requested approved revision for the target page plus
/// the committed published revision for every other page. The target candidate is
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
                r.required_link_payload,
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
                r.required_link_payload,
                r.updated_at::text AS updated_at
            FROM site.cms_pages p
            JOIN site.cms_page_revisions r
              ON r.page_node_key = p.page_node_key
             AND r.revision_id = p.current_revision_id
             AND r.revision_status = 'published'
            WHERE p.page_node_key <> $1
              AND p.scope_signature = (
                  SELECT target.scope_signature
                  FROM site.cms_pages target
                  WHERE target.page_node_key = $1
              )
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

    let (pages, links) = pages_and_approved_links(rows)?;
    if !pages.iter().any(|page| {
        page.page_node_key == target_page_node_key && page.revision_id == target_revision_id
    }) {
        return Err(DomainError::ValidationFailure {
            message: format!(
                "approved candidate revision not found for static build: page_node_key=`{target_page_node_key}` revision_id=`{target_revision_id}`"
            ),
        });
    }

    let page_keys = pages.iter().map(|page| page.page_node_key.clone()).collect::<Vec<_>>();
    let navigation_items = load_navigation_items(pool, &page_keys).await?;
    Ok(StaticSiteSnapshot { pages, links, navigation_items })
}

#[cfg(test)]
mod approved_revision_link_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn public_links_are_exactly_the_immutable_cms_revision_manifest() {
        let links = approved_links_from_revision(
            "source",
            json!([{
                "target_page_key": "published-target",
                "canonical_url_path": "/ru/published-target/",
                "label": "Published target",
                "link_role": "hub_child"
            }]),
        )
        .unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target_page_key, "published-target");
        assert_eq!(links[0].canonical_url_path, "/ru/published-target/");
        assert!(links[0].required_flag);
    }

    #[test]
    fn malformed_or_duplicate_approved_revision_links_fail_closed() {
        let missing_target = json!([{"target_page_key":"", "canonical_url_path":"/ru/x",
            "label":"X", "link_role":"hub_child"}]);
        assert!(approved_links_from_revision("source", missing_target).is_err());
        let duplicate = json!([
            {"target_page_key":"x", "canonical_url_path":"/ru/x", "label":"X", "link_role":"hub_child"},
            {"target_page_key":"x", "canonical_url_path":"/ru/x", "label":"X", "link_role":"hub_child"}
        ]);
        assert!(approved_links_from_revision("source", duplicate).is_err());
        assert!(approved_links_from_revision("source", json!([{
            "target_page_key": "x", "canonical_url_path": "/ru/x",
            "label": "X", "link_role": ""
        }])).is_err());
        assert!(approved_links_from_revision("source", json!({"not_links":true})).is_err());
    }
}
