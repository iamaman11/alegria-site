use anyhow::{Context, Result};
use pulldown_cmark::{html, Options as MarkdownOptions, Parser as MarkdownParser};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::sqlx_static_site_adapter::{
    load_static_site_snapshot, StaticCmsLinkRow, StaticCmsPageRow,
};

#[derive(Debug, Clone)]
pub struct StaticArtifact {
    pub relative_path: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct RenderPreviewPage {
    pub page_node_key: String,
    pub revision_id: String,
    pub canonical_url_path: String,
    pub rendered_html: String,
    pub has_breadcrumbs: bool,
    pub has_schema_markup: bool,
    pub required_link_count: usize,
    pub rendered_link_count: usize,
}

#[derive(Debug, Clone)]
pub struct StaticBuildResult {
    pub artifacts: Vec<StaticArtifact>,
    pub previews: Vec<RenderPreviewPage>,
    pub output_dir: PathBuf,
}

fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn normalize_url_path(raw: &str) -> Result<String> {
    if raw.trim().is_empty() {
        anyhow::bail!("canonical_url_path is empty");
    }
    if raw.contains('\\') || raw.contains('?') || raw.contains('#') {
        anyhow::bail!("canonical_url_path contains unsupported characters: {raw}");
    }
    let mut path = format!("/{}", raw.trim_start_matches('/'));
    while path.contains("//") {
        path = path.replace("//", "/");
    }
    if path != "/" {
        path = path.trim_end_matches('/').to_string();
    }
    if path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        anyhow::bail!("canonical_url_path must not contain traversal segments: {raw}");
    }
    Ok(path)
}

fn output_path_for_url(url_path: &str) -> Result<String> {
    let normalized = normalize_url_path(url_path)?;
    if normalized == "/" {
        return Ok("index.html".to_string());
    }
    Ok(format!("{}/index.html", normalized.trim_start_matches('/')))
}

fn absolute_url(base_url: &str, url_path: &str) -> Result<String> {
    let path = normalize_url_path(url_path)?;
    Ok(format!(
        "{}{}",
        base_url.trim_end_matches('/'),
        if path == "/" { "/".to_string() } else { path }
    ))
}

fn markdown_to_html(markdown: &str) -> String {
    let parser = MarkdownParser::new_ext(markdown, MarkdownOptions::all());
    let mut rendered = String::new();
    html::push_html(&mut rendered, parser);
    rendered
}

fn page_markdown(page: &StaticCmsPageRow) -> &str {
    page.body_payload
        .get("markdown")
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn render_content_blocks(page: &StaticCmsPageRow) -> String {
    let Some(blocks) = page
        .body_payload
        .get("content_blocks")
        .and_then(Value::as_array)
    else {
        return String::new();
    };
    let mut html = String::new();
    for block in blocks {
        let block_type = block
            .get("block_type")
            .and_then(Value::as_str)
            .unwrap_or("prose");
        let section_role = block
            .get("section_role")
            .and_then(Value::as_str)
            .unwrap_or("section");
        let heading = block.get("heading").and_then(Value::as_str).unwrap_or("");
        let markdown = block.get("markdown").and_then(Value::as_str).unwrap_or("");
        if markdown.trim().is_empty() && heading.trim().is_empty() {
            continue;
        }
        html.push_str(&format!(
            r#"<section class="content-block content-block--{}" data-section-role="{}">"#,
            escape_html(block_type),
            escape_html(section_role)
        ));
        if !heading.trim().is_empty() {
            html.push_str(&format!("<h2>{}</h2>", escape_html(heading)));
        }
        html.push_str(&markdown_to_html(markdown));
        html.push_str("</section>");
    }
    html
}

fn menu_depth(url_path: &str) -> Result<usize> {
    let path = normalize_url_path(url_path)?;
    Ok(path
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .count())
}

fn render_navigation(pages: &[StaticCmsPageRow]) -> Result<String> {
    let mut items = String::new();
    for page in pages {
        let href = normalize_url_path(&page.canonical_url_path)?;
        let depth = menu_depth(&href)?;
        items.push_str(&format!(
            r#"<a href="{}" data-depth="{}">{}</a>"#,
            escape_html(&href),
            depth,
            escape_html(&page.title)
        ));
    }
    Ok(items)
}

fn breadcrumb_label(segment: &str) -> String {
    segment
        .split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn breadcrumb_entries(page: &StaticCmsPageRow, base_url: &str) -> Result<Vec<(String, String)>> {
    let canonical_path = normalize_url_path(&page.canonical_url_path)?;
    let mut entries = vec![("Home".to_string(), absolute_url(base_url, "/")?)];
    if canonical_path == "/" {
        return Ok(entries);
    }
    let segments = canonical_path
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let mut current = String::new();
    for (idx, segment) in segments.iter().enumerate() {
        current.push('/');
        current.push_str(segment);
        current.push('/');
        let label = if idx + 1 == segments.len() {
            page.title.clone()
        } else {
            breadcrumb_label(segment)
        };
        entries.push((label, absolute_url(base_url, &current)?));
    }
    Ok(entries)
}

fn render_breadcrumbs(page: &StaticCmsPageRow, base_url: &str) -> Result<String> {
    let entries = breadcrumb_entries(page, base_url)?;
    let mut items = String::new();
    for (idx, (label, href)) in entries.iter().enumerate() {
        let current = if idx + 1 == entries.len() {
            r#" aria-current="page""#
        } else {
            ""
        };
        items.push_str(&format!(
            r#"<li><a href="{}"{}>{}</a></li>"#,
            escape_html(href),
            current,
            escape_html(label)
        ));
    }
    Ok(format!(
        r#"<nav class="breadcrumbs" aria-label="Breadcrumb"><ol>{items}</ol></nav>"#
    ))
}

fn breadcrumb_list_json(page: &StaticCmsPageRow, base_url: &str) -> Result<Value> {
    let entries = breadcrumb_entries(page, base_url)?;
    Ok(json!({
        "@type": "BreadcrumbList",
        "itemListElement": entries.iter().enumerate().map(|(idx, (label, href))| json!({
            "@type": "ListItem",
            "position": idx + 1,
            "name": label,
            "item": href,
        })).collect::<Vec<_>>()
    }))
}

fn fallback_schema_json(page: &StaticCmsPageRow, canonical: &str, base_url: &str) -> Result<Value> {
    Ok(json!({
        "@context": "https://schema.org",
        "@graph": [
            { "@type": "Article", "headline": page.title, "url": canonical },
            breadcrumb_list_json(page, base_url)?
        ]
    }))
}

fn page_schema_json(page: &StaticCmsPageRow, canonical: &str, base_url: &str) -> Result<Value> {
    let custom_schema = page.schema_markup_payload.is_object()
        && !page
            .schema_markup_payload
            .as_object()
            .map(|value| value.is_empty())
            .unwrap_or(true);
    if !custom_schema {
        return fallback_schema_json(page, canonical, base_url);
    }
    Ok(json!({
        "@context": "https://schema.org",
        "@graph": [
            page.schema_markup_payload.clone(),
            breadcrumb_list_json(page, base_url)?
        ]
    }))
}

fn render_related_links(
    page: &StaticCmsPageRow,
    pages_by_key: &HashMap<String, StaticCmsPageRow>,
    links_by_source: &HashMap<String, Vec<StaticCmsLinkRow>>,
) -> Result<(String, usize, usize)> {
    let Some(links) = links_by_source.get(&page.page_node_key) else {
        return Ok((String::new(), 0, 0));
    };
    let required_link_count = links.iter().filter(|link| link.required_flag).count();
    let mut rendered_link_count = 0usize;
    let mut items = String::new();
    for link in links {
        let Some(target) = pages_by_key.get(&link.target_page_key) else {
            continue;
        };
        rendered_link_count += 1;
        let href = normalize_url_path(&target.canonical_url_path)?;
        let marker = if link.required_flag {
            " data-required=\"true\""
        } else {
            ""
        };
        items.push_str(&format!(
            r#"<li{}><a href="{}">{}</a><span>{}</span></li>"#,
            marker,
            escape_html(&href),
            escape_html(&target.title),
            escape_html(&link.link_role)
        ));
    }
    if items.is_empty() {
        Ok((String::new(), required_link_count, rendered_link_count))
    } else {
        Ok((
            format!(r#"<section class="related"><h2>Related Pages</h2><ul>{items}</ul></section>"#),
            required_link_count,
            rendered_link_count,
        ))
    }
}

fn render_page(
    page: &StaticCmsPageRow,
    pages: &[StaticCmsPageRow],
    pages_by_key: &HashMap<String, StaticCmsPageRow>,
    links_by_source: &HashMap<String, Vec<StaticCmsLinkRow>>,
    base_url: &str,
) -> Result<(String, RenderPreviewPage)> {
    let canonical_path = normalize_url_path(&page.canonical_url_path)?;
    let canonical = absolute_url(base_url, &canonical_path)?;
    let block_body = render_content_blocks(page);
    let body = if block_body.trim().is_empty() {
        markdown_to_html(page_markdown(page))
    } else {
        block_body
    };
    let nav = render_navigation(pages)?;
    let breadcrumbs = render_breadcrumbs(page, base_url)?;
    let (related, required_link_count, rendered_link_count) =
        render_related_links(page, pages_by_key, links_by_source)?;
    let schema_json = serde_json::to_string_pretty(&page_schema_json(page, &canonical, base_url)?)?;

    let html = format!(
        r#"<!doctype html>
<html lang="{locale}">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <meta name="description" content="{description}">
  <link rel="canonical" href="{canonical}">
  <script type="application/ld+json">{schema_json}</script>
</head>
<body>
  <header><nav>{nav}</nav></header>
  <main>
    {breadcrumbs}
    <article>
      <h1>{h1}</h1>
      {body}
    </article>
    {related}
  </main>
</body>
</html>
"#,
        locale = escape_html(&page.locale_code),
        title = escape_html(&page.title),
        description = escape_html(&page.meta_description),
        canonical = escape_html(&canonical),
        schema_json = schema_json,
        nav = nav,
        breadcrumbs = breadcrumbs,
        h1 = escape_html(&page.h1),
        body = body,
        related = related,
    );
    let preview = RenderPreviewPage {
        page_node_key: page.page_node_key.clone(),
        revision_id: page.revision_id.clone(),
        canonical_url_path: canonical_path,
        rendered_html: html.clone(),
        has_breadcrumbs: html.contains("class=\"breadcrumbs\""),
        has_schema_markup: html.contains("application/ld+json"),
        required_link_count,
        rendered_link_count,
    };
    Ok((html, preview))
}

fn build_static_artifacts(
    pages: &[StaticCmsPageRow],
    links: &[StaticCmsLinkRow],
    base_url: &str,
) -> Result<(Vec<StaticArtifact>, Vec<RenderPreviewPage>)> {
    if pages.is_empty() {
        anyhow::bail!("no approved or published CMS pages are available for static build");
    }
    let pages_by_key: HashMap<String, StaticCmsPageRow> = pages
        .iter()
        .map(|page| (page.page_node_key.clone(), page.clone()))
        .collect();
    let mut links_by_source: HashMap<String, Vec<StaticCmsLinkRow>> = HashMap::new();
    for link in links {
        links_by_source
            .entry(link.source_page_key.clone())
            .or_default()
            .push(link.clone());
    }

    let mut artifacts = Vec::new();
    let mut previews = Vec::new();
    let mut sitemap_urls = Vec::new();
    let mut manifest_pages = Vec::new();
    for page in pages {
        let (html, preview) = render_page(page, pages, &pages_by_key, &links_by_source, base_url)?;
        let relative_path = output_path_for_url(&page.canonical_url_path)?;
        artifacts.push(StaticArtifact {
            relative_path,
            bytes: html.into_bytes(),
        });
        let url = absolute_url(base_url, &page.canonical_url_path)?;
        sitemap_urls.push(format!(
            "<url><loc>{}</loc><lastmod>{}</lastmod></url>",
            escape_html(&url),
            escape_html(&page.updated_at)
        ));
        manifest_pages.push(json!({
            "page_node_key": page.page_node_key,
            "revision_id": page.revision_id,
            "canonical_url_path": preview.canonical_url_path,
            "status": page.current_status,
            "title": page.title,
        }));
        previews.push(preview);
    }
    artifacts.push(StaticArtifact {
        relative_path: "sitemap.xml".to_string(),
        bytes: format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">{}</urlset>"#,
            sitemap_urls.join("")
        )
        .into_bytes(),
    });
    artifacts.push(StaticArtifact {
        relative_path: "robots.txt".to_string(),
        bytes: b"User-agent: *\nAllow: /\nSitemap: /sitemap.xml\n".to_vec(),
    });
    artifacts.push(StaticArtifact {
        relative_path: "alegria-static-manifest.json".to_string(),
        bytes: serde_json::to_vec_pretty(&json!({
            "builder": "alegria_static_site_builder@2",
            "base_url": base_url,
            "page_count": pages.len(),
            "pages": manifest_pages,
        }))?,
    });
    Ok((artifacts, previews))
}

fn write_static_artifacts(output_dir: &Path, artifacts: &[StaticArtifact]) -> Result<()> {
    let marker = output_dir.join(".alegria_static_site");
    if output_dir.exists() {
        let mut entries = fs::read_dir(output_dir)
            .with_context(|| format!("read output dir failed: {}", output_dir.display()))?;
        if marker.exists() {
            fs::remove_dir_all(output_dir)
                .with_context(|| format!("clean output dir failed: {}", output_dir.display()))?;
        } else if entries.next().is_some() {
            anyhow::bail!(
                "output dir is not empty and was not created by Alegria: {}",
                output_dir.display()
            );
        }
    }
    fs::create_dir_all(output_dir)
        .with_context(|| format!("create output dir failed: {}", output_dir.display()))?;
    fs::write(&marker, b"alegria_static_site_builder@2\n")
        .with_context(|| format!("write marker failed: {}", marker.display()))?;
    for artifact in artifacts {
        let path = output_dir.join(&artifact.relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create artifact dir failed: {}", parent.display()))?;
        }
        fs::write(&path, &artifact.bytes)
            .with_context(|| format!("write artifact failed: {}", path.display()))?;
    }
    Ok(())
}

fn ensure_output_dir_ready(output_dir: &Path) -> Result<()> {
    let marker = output_dir.join(".alegria_static_site");
    if output_dir.exists() {
        let mut entries = fs::read_dir(output_dir)
            .with_context(|| format!("read output dir failed: {}", output_dir.display()))?;
        if !marker.exists() && entries.next().is_some() {
            anyhow::bail!(
                "output dir is not empty and was not created by Alegria: {}",
                output_dir.display()
            );
        }
    }
    fs::create_dir_all(output_dir)
        .with_context(|| format!("create output dir failed: {}", output_dir.display()))?;
    fs::write(&marker, b"alegria_static_site_builder@2\n")
        .with_context(|| format!("write marker failed: {}", marker.display()))?;
    Ok(())
}

fn write_partial_artifacts(output_dir: &Path, artifacts: &[StaticArtifact]) -> Result<()> {
    ensure_output_dir_ready(output_dir)?;
    for artifact in artifacts {
        let path = output_dir.join(&artifact.relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create artifact dir failed: {}", parent.display()))?;
        }
        fs::write(&path, &artifact.bytes)
            .with_context(|| format!("write artifact failed: {}", path.display()))?;
    }
    Ok(())
}

pub async fn build_static_site(
    pool: &PgPool,
    output_dir: &Path,
    base_url: &str,
) -> Result<StaticBuildResult> {
    let snapshot = load_static_site_snapshot(pool).await?;
    let (artifacts, previews) = build_static_artifacts(&snapshot.pages, &snapshot.links, base_url)?;
    write_static_artifacts(output_dir, &artifacts)?;
    Ok(StaticBuildResult {
        artifacts,
        previews,
        output_dir: output_dir.to_path_buf(),
    })
}

pub async fn build_static_site_incremental(
    pool: &PgPool,
    output_dir: &Path,
    base_url: &str,
    target_page_keys: &[String],
) -> Result<StaticBuildResult> {
    if target_page_keys.is_empty() {
        anyhow::bail!("incremental build requires at least one target page key");
    }
    let snapshot = load_static_site_snapshot(pool).await?;
    let target_set = target_page_keys
        .iter()
        .filter(|key| !key.trim().is_empty())
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let (all_artifacts, all_previews) =
        build_static_artifacts(&snapshot.pages, &snapshot.links, base_url)?;
    let page_paths = snapshot
        .pages
        .iter()
        .filter(|page| target_set.contains(&page.page_node_key))
        .map(|page| output_path_for_url(&page.canonical_url_path))
        .collect::<Result<std::collections::HashSet<_>>>()?;
    if page_paths.is_empty() {
        anyhow::bail!("incremental build target pages are not approved/published");
    }
    let artifacts = all_artifacts
        .into_iter()
        .filter(|artifact| {
            page_paths.contains(&artifact.relative_path)
                || matches!(
                    artifact.relative_path.as_str(),
                    "sitemap.xml" | "robots.txt" | "alegria-static-manifest.json"
                )
        })
        .collect::<Vec<_>>();
    let previews = all_previews
        .into_iter()
        .filter(|preview| target_set.contains(&preview.page_node_key))
        .collect::<Vec<_>>();
    write_partial_artifacts(output_dir, &artifacts)?;
    Ok(StaticBuildResult {
        artifacts,
        previews,
        output_dir: output_dir.to_path_buf(),
    })
}
