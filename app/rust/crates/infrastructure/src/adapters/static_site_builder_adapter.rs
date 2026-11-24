use anyhow::{Context, Result};
use pulldown_cmark::{
    html, CowStr, Event, Options as MarkdownOptions, Parser as MarkdownParser, Tag,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::sqlx_static_site_adapter::{
    load_static_site_candidate_snapshot, load_static_site_snapshot, StaticCmsLinkRow,
    StaticCmsPageRow, StaticNavigationItemRow, StaticSiteSnapshot,
};

// Bump only when published HTML/layout semantics change. The Worker uses this
// to refresh already-approved public pages without new editorial revisions.
pub const STATIC_SITE_BUILDER_VERSION: &str = "alegria_static_site_builder@5";

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

fn safe_markdown_destination(raw: &str) -> bool {
    let destination = raw.trim();
    if destination.is_empty() {
        return false;
    }
    let lowered = destination.to_ascii_lowercase();
    if lowered.starts_with("//") {
        return false;
    }
    if lowered.starts_with("https://")
        || lowered.starts_with("http://")
        || lowered.starts_with("mailto:")
        || lowered.starts_with('/')
        || lowered.starts_with('#')
        || lowered.starts_with("./")
        || lowered.starts_with("../")
    {
        return true;
    }
    !lowered.contains(':')
}

fn safe_destination<'a>(destination: CowStr<'a>) -> CowStr<'a> {
    if safe_markdown_destination(destination.as_ref()) {
        destination
    } else {
        CowStr::Borrowed("#")
    }
}

fn sanitize_markdown_event(event: Event<'_>) -> Option<Event<'_>> {
    match event {
        Event::Html(_) | Event::InlineHtml(_) => None,
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Some(Event::Start(Tag::Link {
            link_type,
            dest_url: safe_destination(dest_url),
            title,
            id,
        })),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Some(Event::Start(Tag::Image {
            link_type,
            dest_url: safe_destination(dest_url),
            title,
            id,
        })),
        other => Some(other),
    }
}

fn markdown_to_html(markdown: &str) -> String {
    let parser = MarkdownParser::new_ext(markdown, MarkdownOptions::all())
        .filter_map(sanitize_markdown_event);
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

fn is_russian(locale: &str) -> bool {
    locale.split(['-', '_']).next() == Some("ru")
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
        // The current CMS revision's approved link manifest alone owns the
        // visible links in this exact release.
        // Both the former `internal_links` and the canonical draft's
        // `related_pages` blocks are editorial obligations, not public HTML.
        // Rendering either leaks internal page keys and duplicates the real
        // related section below.
        if matches!(section_role, "internal_links" | "related_pages") {
            continue;
        }
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

fn navigation_label(page: &StaticCmsPageRow) -> &str {
    match (is_russian(&page.locale_code), page.page_type_key.as_str()) {
        (true, "requirement_page") => "Документы",
        (true, "fee_page") => "Сборы",
        (true, "timeline_page") => "Сроки",
        (false, "requirement_page") => "Documents",
        (false, "fee_page") => "Fees",
        (false, "timeline_page") => "Processing times",
        _ => page.title.split(':').next().unwrap_or(&page.title).trim(),
    }
}

fn render_navigation(
    current: &StaticCmsPageRow,
    pages_by_key: &HashMap<String, StaticCmsPageRow>,
    navigation_items: &[StaticNavigationItemRow],
) -> Result<String> {
    // No fallback to the full CMS page catalog: it would mix markets/locales,
    // bypass the navigation policy and produce an unbounded public header.
    let own_trees = navigation_items
        .iter()
        .filter(|item| {
            item.page_node_key == current.page_node_key && item.locale == current.locale_code
        })
        .map(|item| item.navigation_tree_key.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if own_trees.len() > 1 {
        anyhow::bail!(
            "ambiguous navigation ownership for published CMS page {}",
            current.page_node_key
        );
    }
    let Some(tree) = own_trees.first().copied() else {
        // A newly approved page can be previewed before its navigation plan
        // becomes active. Never infer market/locale from unrelated CMS pages.
        return Ok(String::new());
    };
    let mut entries = navigation_items
        .iter()
        .filter(|item| item.navigation_tree_key == tree && item.hierarchy_depth <= 5)
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        a.group_sort_order
            .cmp(&b.group_sort_order)
            .then(a.sort_order.cmp(&b.sort_order))
            .then(a.hierarchy_depth.cmp(&b.hierarchy_depth))
            .then(a.page_node_key.cmp(&b.page_node_key))
    });
    let mut items = String::new();
    for item in entries
        .into_iter()
        .filter(|item| pages_by_key.contains_key(&item.page_node_key))
        .take(64)
    {
        // Navigation planning includes drafted/unpublished nodes. Only exact
        // CMS release pages may become public links in this snapshot.
        let Some(page) = pages_by_key.get(&item.page_node_key) else {
            continue;
        };
        let href = normalize_url_path(&page.canonical_url_path)?;
        let depth = menu_depth(&href)?;
        let current_marker = if page.page_node_key == current.page_node_key {
            r#" aria-current="page""#
        } else {
            ""
        };
        items.push_str(&format!(
            r#"<a href="{}" data-depth="{}" title="{}"{}>{}</a>"#,
            escape_html(&href),
            depth,
            escape_html(&page.title),
            current_marker,
            escape_html(if item.label.trim().is_empty() {
                navigation_label(page)
            } else {
                item.label.as_str()
            }),
        ));
    }
    Ok(items)
}

fn breadcrumb_entries(
    page: &StaticCmsPageRow,
    pages: &[StaticCmsPageRow],
    base_url: &str,
) -> Result<Vec<(String, String)>> {
    let current = normalize_url_path(&page.canonical_url_path)?;
    let mut ancestors = Vec::new();
    // Every breadcrumb target must exist in the exact release snapshot.
    for candidate in pages {
        let path = normalize_url_path(&candidate.canonical_url_path)?;
        if path != current && (path == "/" || current.starts_with(&format!("{path}/"))) {
            ancestors.push((path, candidate.title.clone()));
        }
    }
    ancestors.sort_by_key(|(path, _)| path.len());
    let mut entries = Vec::new();
    for (path, title) in ancestors {
        entries.push((title, absolute_url(base_url, &path)?));
    }
    entries.push((page.title.clone(), absolute_url(base_url, &current)?));
    Ok(entries)
}

fn render_breadcrumbs(
    page: &StaticCmsPageRow,
    pages: &[StaticCmsPageRow],
    base_url: &str,
) -> Result<String> {
    let entries = breadcrumb_entries(page, pages, base_url)?;
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
        r#"<nav class="breadcrumbs" aria-label="{}"><ol>{items}</ol></nav>"#,
        if is_russian(&page.locale_code) {
            "Навигационная цепочка"
        } else {
            "Breadcrumb"
        }
    ))
}

fn breadcrumb_list_json(
    page: &StaticCmsPageRow,
    pages: &[StaticCmsPageRow],
    base_url: &str,
) -> Result<Value> {
    let entries = breadcrumb_entries(page, pages, base_url)?;
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

fn push_faq_item(items: &mut Vec<Value>, question: &Option<String>, answer_lines: &[String]) {
    let Some(question_text) = question.as_ref() else {
        return;
    };
    let answer = answer_lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if answer.is_empty() {
        return;
    }
    items.push(json!({
        "@type": "Question",
        "name": question_text,
        "acceptedAnswer": {
            "@type": "Answer",
            "text": answer,
        }
    }));
}

fn faq_entities(markdown: &str) -> Vec<Value> {
    let mut items = Vec::new();
    let mut in_faq = false;
    let mut question: Option<String> = None;
    let mut answer_lines: Vec<String> = Vec::new();

    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("## ") {
            let heading_key = heading.trim().to_lowercase();
            if in_faq {
                push_faq_item(&mut items, &question, &answer_lines);
                question = None;
                answer_lines.clear();
            }
            in_faq = heading_key == "faq"
                || heading_key == "frequently asked questions"
                || heading_key == "вопросы и ответы";
            continue;
        }
        if !in_faq {
            continue;
        }
        let next_question = trimmed
            .strip_prefix("### ")
            .map(|text| (text, ""))
            .or_else(|| {
                trimmed
                    .strip_prefix("**")
                    .and_then(|text| text.split_once("**"))
                    .filter(|(text, _)| text.trim_end().ends_with('?'))
            });
        if let Some((next_question, inline_answer)) = next_question {
            push_faq_item(&mut items, &question, &answer_lines);
            question = Some(next_question.trim().to_string());
            answer_lines.clear();
            if !inline_answer.trim().is_empty() {
                answer_lines.push(inline_answer.trim().to_string());
            }
        } else if question.is_some() {
            answer_lines.push(trimmed.to_string());
        }
    }
    if in_faq {
        push_faq_item(&mut items, &question, &answer_lines);
    }
    items
}

fn fallback_schema_json(
    page: &StaticCmsPageRow,
    pages: &[StaticCmsPageRow],
    canonical: &str,
    base_url: &str,
) -> Result<Value> {
    let mut graph = vec![
        json!({ "@type": "Article", "headline": page.title, "url": canonical }),
        breadcrumb_list_json(page, pages, base_url)?,
    ];
    append_faq_schema(&mut graph, page);
    Ok(json!({ "@context": "https://schema.org", "@graph": graph }))
}

fn append_faq_schema(graph: &mut Vec<Value>, page: &StaticCmsPageRow) {
    let faq_items = faq_entities(page_markdown(page));
    if !faq_items.is_empty() {
        graph.push(json!({ "@type": "FAQPage", "mainEntity": faq_items }));
    }
}

fn page_schema_json(
    page: &StaticCmsPageRow,
    pages: &[StaticCmsPageRow],
    canonical: &str,
    base_url: &str,
) -> Result<Value> {
    let custom_schema = page.schema_markup_payload.is_object()
        && !page
            .schema_markup_payload
            .as_object()
            .map(|value| value.is_empty())
            .unwrap_or(true);
    if !custom_schema {
        return fallback_schema_json(page, pages, canonical, base_url);
    }
    let mut graph = vec![
        page.schema_markup_payload.clone(),
        breadcrumb_list_json(page, pages, base_url)?,
    ];
    append_faq_schema(&mut graph, page);
    Ok(json!({ "@context": "https://schema.org", "@graph": graph }))
}

fn json_for_script(value: &Value) -> Result<String> {
    Ok(serde_json::to_string_pretty(value)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e"))
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
            if link.required_flag {
                anyhow::bail!(
                    "required link target absent from release: {}",
                    link.target_page_key
                );
            }
            continue;
        };
        // The CMS revision approved an exact destination. A later route
        // change cannot silently redirect that immutable published link.
        if !link.canonical_url_path.is_empty()
            && normalize_url_path(&link.canonical_url_path)?
                != normalize_url_path(&target.canonical_url_path)?
        {
            anyhow::bail!(
                "approved CMS link target changed after revision: {}",
                link.target_page_key
            );
        }
        rendered_link_count += 1;
        let href = normalize_url_path(&target.canonical_url_path)?;
        let marker = if link.required_flag {
            " data-required=\"true\""
        } else {
            ""
        };
        items.push_str(&format!(
            r#"<li{}><a href="{}">{}</a></li>"#,
            marker,
            escape_html(&href),
            escape_html(&target.title),
        ));
    }
    if items.is_empty() {
        Ok((String::new(), required_link_count, rendered_link_count))
    } else {
        Ok((
            format!(
                r#"<section class="related"><h2>{}</h2><ul>{items}</ul></section>"#,
                if is_russian(&page.locale_code) {
                    "Связанные страницы"
                } else {
                    "Related Pages"
                }
            ),
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
    navigation_items: &[StaticNavigationItemRow],
    base_url: &str,
) -> Result<(String, RenderPreviewPage)> {
    let canonical_path = normalize_url_path(&page.canonical_url_path)?;
    let canonical = absolute_url(base_url, &canonical_path)?;
    let block_body = render_content_blocks(page);
    // Once a typed block manifest exists, do not fall back to its raw draft
    // markdown when every block was intentionally non-renderable (e.g. only
    // `related_pages`). That draft text contains internal target page keys.
    let has_typed_blocks = page
        .body_payload
        .get("content_blocks")
        .and_then(Value::as_array)
        .is_some_and(|blocks| !blocks.is_empty());
    let body = if has_typed_blocks {
        block_body
    } else {
        markdown_to_html(page_markdown(page))
    };
    let nav = render_navigation(page, pages_by_key, navigation_items)?;
    let breadcrumbs = render_breadcrumbs(page, pages, base_url)?;
    let (related, required_link_count, rendered_link_count) =
        render_related_links(page, pages_by_key, links_by_source)?;
    let schema_json = json_for_script(&page_schema_json(page, pages, &canonical, base_url)?)?;
    let navigation_label = if is_russian(&page.locale_code) {
        "Разделы сайта"
    } else {
        "Site sections"
    };

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
  <style>
    :root {{ color-scheme: light; --ink: #182026; --muted: #5a6872; --line: #d9e0e5; --accent: #176b5d; --bg: #fbfcfc; }}
    body {{ margin: 0; font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; color: var(--ink); background: var(--bg); line-height: 1.62; }}
    header {{ border-bottom: 1px solid var(--line); background: #ffffff; }}
    .site-nav {{ max-width: 1120px; margin: 0 auto; padding: 12px 24px; display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }}
    .site-nav a {{ color: var(--ink); text-decoration: none; font-size: 14px; padding: 6px 10px; border-radius: 8px; }}
    .site-nav a:hover, .site-nav a[aria-current="page"] {{ color: var(--accent); background: #e9f2f0; }}
    main {{ max-width: 820px; margin: 0 auto; padding: 48px 24px 72px; }}
    .breadcrumbs {{ max-width: 820px; padding: 0; margin: 0 0 28px; display: block; overflow: visible; }}
    .breadcrumbs ol {{ display: flex; flex-wrap: wrap; gap: 8px; list-style: none; padding: 0; margin: 0; color: var(--muted); font-size: 13px; }}
    .breadcrumbs li:not(:last-child)::after {{ content: "/"; margin-left: 8px; color: var(--muted); }}
    .breadcrumbs a {{ color: var(--muted); font-size: 13px; }}
    article h1 {{ font-size: clamp(32px, 5vw, 48px); line-height: 1.08; margin: 0 0 14px; }}
    .meta {{ color: var(--muted); font-size: 14px; margin-bottom: 34px; }}
    article h2 {{ margin-top: 38px; font-size: 26px; line-height: 1.2; }}
    article a {{ color: var(--accent); }}
    article code {{ background: #eef3f2; padding: 2px 5px; border-radius: 4px; }}
    .related {{ margin-top: 52px; border-top: 1px solid var(--line); padding-top: 26px; }}
    .related ul {{ padding: 0; margin: 0; list-style: none; display: grid; gap: 10px; }}
    .related li {{ display: flex; justify-content: space-between; gap: 16px; border-bottom: 1px solid var(--line); padding-bottom: 10px; }}
    .related span {{ color: var(--muted); font-size: 13px; }}
    footer {{ border-top: 1px solid var(--line); color: var(--muted); font-size: 13px; padding: 22px 24px; text-align: center; }}
  </style>
</head>
<body>
  <header><nav class="site-nav" aria-label="{navigation_label}">{nav}</nav></header>
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
        navigation_label = navigation_label,
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
    navigation_items: &[StaticNavigationItemRow],
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
        let (html, preview) = render_page(
            page, pages, &pages_by_key, &links_by_source, navigation_items, base_url,
        )?;
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
            "builder": STATIC_SITE_BUILDER_VERSION,
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
    fs::write(&marker, format!("{STATIC_SITE_BUILDER_VERSION}\n"))
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
    fs::write(&marker, format!("{STATIC_SITE_BUILDER_VERSION}\n"))
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
    let (artifacts, previews) = build_static_artifacts(
        &snapshot.pages, &snapshot.links, &snapshot.navigation_items, base_url,
    )?;
    write_static_artifacts(output_dir, &artifacts)?;
    Ok(StaticBuildResult {
        artifacts,
        previews,
        output_dir: output_dir.to_path_buf(),
    })
}

fn build_incremental_from_snapshot(
    snapshot: StaticSiteSnapshot,
    output_dir: &Path,
    base_url: &str,
    target_page_keys: &[String],
) -> Result<StaticBuildResult> {
    let target_set = target_page_keys
        .iter()
        .filter(|key| !key.trim().is_empty())
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let (all_artifacts, all_previews) =
        build_static_artifacts(
            &snapshot.pages, &snapshot.links, &snapshot.navigation_items, base_url,
        )?;
    let page_paths = snapshot
        .pages
        .iter()
        .filter(|page| target_set.contains(&page.page_node_key))
        .map(|page| output_path_for_url(&page.canonical_url_path))
        .collect::<Result<std::collections::HashSet<_>>>()?;
    if page_paths.is_empty() {
        anyhow::bail!("incremental build target pages are not present in the selected snapshot");
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
    build_incremental_from_snapshot(snapshot, output_dir, base_url, target_page_keys)
}

pub async fn build_static_site_candidate_incremental(
    pool: &PgPool,
    output_dir: &Path,
    base_url: &str,
    target_page_node_key: &str,
    target_revision_id: &str,
) -> Result<StaticBuildResult> {
    let snapshot =
        load_static_site_candidate_snapshot(pool, target_page_node_key, target_revision_id).await?;
    // Each candidate release gets a separate directory, not an existing public
    // tree to patch. Emit every published dependency along with the approved
    // target revision; otherwise the sitemap/manifest would list missing pages.
    // Only the target needs a preview for its per-page editorial gate.
    let (artifacts, all_previews) = build_static_artifacts(
        &snapshot.pages, &snapshot.links, &snapshot.navigation_items, base_url,
    )?;
    write_static_artifacts(output_dir, &artifacts)?;
    let previews = all_previews
        .into_iter()
        .filter(|preview| preview.page_node_key == target_page_node_key)
        .collect();
    Ok(StaticBuildResult {
        artifacts,
        previews,
        output_dir: output_dir.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_release_contains_all_manifest_pages_and_html() {
        let target = StaticCmsPageRow {
            page_node_key: "target".to_string(),
            canonical_url_path: "/".to_string(),
            locale_code: "ru".to_string(),
            page_type_key: "home".to_string(),
            dominant_intent_key: "informational".to_string(),
            current_status: "approved".to_string(),
            cms_document_id: "cms-target".to_string(),
            revision_id: "target-approved".to_string(),
            title: "Home".to_string(),
            meta_description: "Home description".to_string(),
            h1: "Home".to_string(),
            body_payload: json!({"markdown": "Useful home content"}),
            schema_markup_payload: json!({}),
            updated_at: "2026-11-07".to_string(),
        };
        let published_dependency = StaticCmsPageRow {
            page_node_key: "guide".to_string(),
            canonical_url_path: "/guide".to_string(),
            current_status: "published".to_string(),
            cms_document_id: "cms-guide".to_string(),
            revision_id: "guide-published".to_string(),
            title: "Guide".to_string(),
            h1: "Guide".to_string(),
            ..target.clone()
        };
        let (artifacts, previews) = build_static_artifacts(
            &[target, published_dependency],
            &[],
            &[],
            "https://staging.alegria.by",
        )
        .expect("build complete isolated candidate release");
        assert_eq!(previews.len(), 2);

        let output_dir = std::env::temp_dir().join(format!(
            "alegria-candidate-release-{}",
            uuid::Uuid::new_v4()
        ));
        write_static_artifacts(&output_dir, &artifacts).expect("write complete candidate release");
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(output_dir.join("alegria-static-manifest.json")).expect("manifest exists"),
        )
        .expect("valid JSON manifest");
        assert_eq!(manifest["page_count"], 2);
        assert!(output_dir.join("index.html").exists());
        assert!(output_dir.join("guide/index.html").exists());
        let sitemap = fs::read_to_string(output_dir.join("sitemap.xml")).expect("sitemap exists");
        assert!(sitemap.contains("https://staging.alegria.by/guide"));
        assert!(output_dir.join("robots.txt").exists());
        fs::remove_dir_all(&output_dir).expect("remove test release");
    }

    #[test]
    fn public_paths_keep_directory_indexes_and_reject_traversal() {
        assert_eq!(output_path_for_url("/").unwrap(), "index.html");
        assert_eq!(
            output_path_for_url("/guides/poland-visa/").unwrap(),
            "guides/poland-visa/index.html"
        );
        assert!(output_path_for_url("/../secret").is_err());
        assert!(output_path_for_url("/safe?x=1").is_err());
    }

    #[test]
    fn russian_faq_uses_unicode_heading_and_bold_questions() {
        let faq = faq_entities("## ВОПРОСЫ И ОТВЕТЫ\n\n**Как поливать?**\n\nОтвет.");
        assert_eq!(faq.len(), 1);
        assert_eq!(faq[0]["name"], "Как поливать?");
        assert_eq!(faq[0]["acceptedAnswer"]["text"], "Ответ.");
    }

    fn sample_release_page(key: &str, path: &str, title: &str) -> StaticCmsPageRow {
        StaticCmsPageRow {
            page_node_key: key.into(),
            canonical_url_path: path.into(),
            locale_code: "ru".into(),
            page_type_key: "guide".into(),
            dominant_intent_key: "informational".into(),
            current_status: "published".into(),
            cms_document_id: key.into(),
            revision_id: key.into(),
            title: title.into(),
            meta_description: "Описание".into(),
            h1: title.into(),
            body_payload: json!({"markdown":"Текст"}),
            schema_markup_payload: json!({}),
            updated_at: "2026-11-17".into(),
        }
    }

    #[test]
    fn public_menu_uses_only_its_persisted_tree_and_published_pages() {
        let mut a = sample_release_page("a", "/ru/a", "A");
        a.locale_code = "ru-RU".into();
        let mut b = sample_release_page("b", "/ru/b", "B");
        b.locale_code = "ru-RU".into();
        let mut foreign = sample_release_page("foreign", "/en/foreign", "Foreign");
        foreign.locale_code = "en-US".into();
        let published = HashMap::from([
            (a.page_node_key.clone(), a.clone()),
            (b.page_node_key.clone(), b.clone()),
            (foreign.page_node_key.clone(), foreign.clone()),
        ]);
        let nav = vec![
            StaticNavigationItemRow {
                navigation_tree_key: "tree-ru".into(), market: "BY".into(),
                locale: "ru-RU".into(), page_node_key: "a".into(),
                label: "А".into(), group_sort_order: 1000, hierarchy_depth: 1, sort_order: 1,
            },
            StaticNavigationItemRow {
                navigation_tree_key: "tree-ru".into(), market: "BY".into(),
                locale: "ru-RU".into(), page_node_key: "draft-only".into(),
                label: "Draft".into(), group_sort_order: 1000, hierarchy_depth: 1, sort_order: 2,
            },
            StaticNavigationItemRow {
                navigation_tree_key: "tree-ru".into(), market: "BY".into(),
                locale: "ru-RU".into(), page_node_key: "b".into(),
                label: "Б".into(), group_sort_order: 1000, hierarchy_depth: 2, sort_order: 3,
            },
            StaticNavigationItemRow {
                navigation_tree_key: "tree-en".into(), market: "GB".into(),
                locale: "en-US".into(), page_node_key: "foreign".into(),
                label: "Foreign".into(), group_sort_order: 1000, hierarchy_depth: 1, sort_order: 1,
            },
        ];
        let html = render_navigation(&a, &published, &nav).unwrap();
        assert!(html.contains("href=\"/ru/a\""));
        assert!(html.contains("href=\"/ru/b\""));
        assert!(!html.contains("draft-only"));
        assert!(!html.contains("/en/foreign"));
        let foreign_html = render_navigation(&foreign, &published, &nav).unwrap();
        assert!(foreign_html.contains("/en/foreign"));
        assert!(!foreign_html.contains("/ru/a"));
    }

    #[test]
    fn breadcrumbs_include_only_existing_release_routes() {
        let page = sample_release_page("child", "/ru/garden/watering/", "Полив");
        let parent = sample_release_page("parent", "/ru/garden/", "Сад");
        let unrelated = sample_release_page("other", "/ru/gardeners/", "Другой");
        let base = "https://example.test";
        let single = breadcrumb_entries(&page, &[page.clone()], base).unwrap();
        assert_eq!(
            single,
            vec![(
                "Полив".into(),
                "https://example.test/ru/garden/watering".into()
            )]
        );
        let pages = vec![page.clone(), unrelated, parent];
        let result = breadcrumb_entries(&page, &pages, base).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0],
            ("Сад".into(), "https://example.test/ru/garden".into())
        );
        let html = render_breadcrumbs(&page, &pages, base).unwrap();
        assert!(!html.contains("href=\"https://example.test/ru\""));
    }

    #[test]
    fn required_related_links_must_have_real_targets() {
        let page = sample_release_page("child", "/ru/garden/watering/", "Полив");
        let parent = sample_release_page("parent", "/ru/garden/", "Сад");
        let link = StaticCmsLinkRow {
            source_page_key: "child".into(),
            target_page_key: "parent".into(),
            canonical_url_path: "/ru/garden/".into(),
            required_flag: true,
        };
        let links = HashMap::from([("child".to_string(), vec![link])]);
        assert!(render_related_links(&page, &HashMap::new(), &links).is_err());
        let targets = HashMap::from([("parent".to_string(), parent.clone())]);
        let (html, required, rendered) = render_related_links(&page, &targets, &links).unwrap();
        assert_eq!((required, rendered), (1, 1));
        assert!(html.contains("Связанные страницы"));
        assert!(html.contains("href=\"/ru/garden\""));
        let changed = StaticCmsPageRow {
            canonical_url_path: "/ru/new-garden".into(),
            ..parent
        };
        assert!(render_related_links(
            &page,
            &HashMap::from([("parent".to_string(), changed)]),
            &links,
        )
        .is_err());
    }

    #[test]
    fn regional_russian_pages_render_one_related_section_and_compact_navigation() {
        let mut page = sample_release_page(
            "fees",
            "/ru/visa/spain/tourist/fees/",
            "Стоимость туристической визы в Испанию: сборы",
        );
        page.locale_code = "ru-RU".into();
        page.page_type_key = "fee_page".into();
        page.body_payload = json!({
            "markdown": "## Обзор\n\n35 евро.\n\n## Связанные страницы\n\n- Recommended internal link: hub_to_child -> internal-hub-key",
            "content_blocks": [
                {"block_type": "prose", "section_role": "overview", "heading": "Обзор", "markdown": "35 евро."},
                {"block_type": "related_links", "section_role": "internal_links", "heading": "Связанные страницы", "markdown": "- [Устаревшее название](/ru/visa/spain/tourist/)"},
                {"block_type": "related_links", "section_role": "related_pages", "heading": "Связанные страницы", "markdown": "- Recommended internal link: hub_to_child -> internal-hub-key"}
            ]
        });
        let mut hub = sample_release_page(
            "hub",
            "/ru/visa/spain/tourist/",
            "Туристическая виза в Испанию: документы и сроки",
        );
        hub.locale_code = "ru-RU".into();
        hub.page_type_key = "hub_page".into();
        let pages = vec![hub.clone(), page.clone()];
        let targets = HashMap::from([
            (page.page_node_key.clone(), page.clone()),
            (hub.page_node_key.clone(), hub),
        ]);
        let links = HashMap::from([(
            page.page_node_key.clone(),
            vec![StaticCmsLinkRow {
                source_page_key: page.page_node_key.clone(),
                target_page_key: "hub".into(),
                canonical_url_path: "/ru/visa/spain/tourist/".into(),
                required_flag: true,
            }],
        )]);
        let navigation_items = vec![
            StaticNavigationItemRow {
                navigation_tree_key: "ru-navigation".into(),
                market: "BY".into(),
                locale: "ru-RU".into(),
                page_node_key: "hub".into(),
                label: "Туристическая виза".into(),
                group_sort_order: 1000,
                hierarchy_depth: 2,
                sort_order: 1,
            },
            StaticNavigationItemRow {
                navigation_tree_key: "ru-navigation".into(),
                market: "BY".into(),
                locale: "ru-RU".into(),
                page_node_key: "fees".into(),
                label: "Сборы".into(),
                group_sort_order: 1000,
                hierarchy_depth: 3,
                sort_order: 2,
            },
        ];

        let (html, preview) = render_page(
            &page,
            &pages,
            &targets,
            &links,
            &navigation_items,
            "https://staging.example.test",
        )
        .unwrap();
        assert_eq!(preview.required_link_count, 1);
        assert_eq!(preview.rendered_link_count, 1);
        assert_eq!(html.matches("<h2>Связанные страницы</h2>").count(), 1);
        assert!(!html.contains("Устаревшее название"));
        assert!(!html.contains("internal-hub-key"));
        assert!(!html.contains("Recommended internal link"));
        assert!(html.contains("href=\"/ru/visa/spain/tourist\""));
        assert!(!html.contains("Related Pages"));
        assert!(html.contains("aria-label=\"Навигационная цепочка\""));
        assert!(html.contains("aria-label=\"Разделы сайта\""));
        assert!(html.contains("aria-current=\"page\">Сборы</a>"));
        assert!(html.contains("flex-wrap: wrap"));
        assert!(!html.contains("overflow-x: auto"));
    }

    #[test]
    fn only_internal_link_draft_blocks_never_trigger_raw_markdown_fallback() {
        let mut page = sample_release_page("fees", "/ru/visa/spain/tourist/fees", "Сборы");
        page.body_payload = json!({
            "markdown": "- Recommended internal link: hub_to_child -> secret-page-id",
            "content_blocks": [
                {"block_type": "related_links", "section_role": "related_pages", "heading": "Связанные страницы", "markdown": "- Recommended internal link: hub_to_child -> secret-page-id"}
            ]
        });
        let (html, preview) = render_page(
            &page,
            &[page.clone()],
            &HashMap::from([(page.page_node_key.clone(), page.clone())]),
            &HashMap::new(),
            &[],
            "https://staging.example.test",
        )
        .unwrap();
        assert_eq!(preview.required_link_count, 0);
        assert!(!html.contains("secret-page-id"));
        assert!(!html.contains("Recommended internal link"));
    }

    #[test]
    fn faq_preserves_inline_answers_from_existing_markdown() {
        let faq = faq_entities("## Вопросы и ответы\n\n**Как поливать?** Утром.\n\nПо необходимости.\n\n**Когда?** Весной.\n\n## Далее");
        assert_eq!(faq.len(), 2);
        assert_eq!(
            faq[0]["acceptedAnswer"]["text"],
            "Утром.\nПо необходимости."
        );
        assert_eq!(faq[1]["acceptedAnswer"]["text"], "Весной.");
    }

    #[test]
    fn canonical_renderer_keeps_faq_structured_data() {
        let faq = faq_entities("## FAQ\n\n### Is this reviewed?\n\nYes, before publish.");
        assert_eq!(faq.len(), 1);
        assert_eq!(faq[0]["@type"], "Question");
        assert_eq!(faq[0]["name"], "Is this reviewed?");
        let rendered = markdown_to_html("# Title\n\nBody with **strong** text.");
        assert!(rendered.contains("<h1>Title</h1>"));
        assert!(rendered.contains("<strong>strong</strong>"));
    }

    #[test]
    fn markdown_renderer_drops_raw_html_and_neutralizes_dangerous_urls() {
        let rendered = markdown_to_html(
            "before <script>alert(1)</script> [bad](javascript:alert(1)) ![x](data:text/html,boom) after",
        );
        assert!(!rendered.to_ascii_lowercase().contains("<script"));
        assert!(!rendered.to_ascii_lowercase().contains("javascript:"));
        assert!(!rendered.to_ascii_lowercase().contains("data:text/html"));
        assert!(rendered.contains("href=\"#\""));
        assert!(rendered.contains("src=\"#\""));
    }

    #[test]
    fn json_ld_serialization_cannot_close_script_element() {
        let encoded = json_for_script(&json!({"value": "</script><script>alert(1)</script>"}))
            .expect("json serialization should succeed");
        assert!(!encoded.to_ascii_lowercase().contains("</script>"));
        assert!(encoded.contains("\\u003c/script\\u003e"));
    }
}
