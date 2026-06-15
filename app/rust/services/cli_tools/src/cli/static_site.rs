#[derive(Debug, Clone)]
struct StaticArtifact {
    relative_path: String,
    bytes: Vec<u8>,
}

fn default_database_url() -> String {
    env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres_password@localhost:5433/alegria".to_string()
    })
}

fn default_temporal_url() -> String {
    env::var("TEMPORAL_URL").unwrap_or_else(|_| "http://localhost:7233".to_string())
}

fn default_temporal_namespace() -> String {
    env::var("TEMPORAL_NAMESPACE").unwrap_or_else(|_| "default".to_string())
}

fn evaluate_gate_status(report: &Value, blocking_reasons: &mut Vec<String>) {
    let label = report
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("unknown_gate");
    let status = report
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("error");
    if status == "error" {
        let exit_code = report
            .get("exit_code")
            .and_then(Value::as_i64)
            .map(|code| code.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        blocking_reasons.push(format!("{label} failed (exit_code={exit_code})"));
    }
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn normalize_url_path(raw: &str) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        anyhow::bail!("canonical_url_path must not be empty");
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
            let heading_key = heading.trim().to_ascii_lowercase();
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
        if let Some(next_question) = trimmed.strip_prefix("### ") {
            push_faq_item(&mut items, &question, &answer_lines);
            question = Some(next_question.trim().to_string());
            answer_lines.clear();
        } else if question.is_some() {
            answer_lines.push(trimmed.to_string());
        }
    }
    if in_faq {
        push_faq_item(&mut items, &question, &answer_lines);
    }
    items
}

fn fallback_schema_json(page: &StaticCmsPageRow, canonical: &str, base_url: &str) -> Result<Value> {
    let markdown = page_markdown(page);
    let mut graph = vec![
        json!({
            "@type": "Article",
            "headline": page.title,
            "url": canonical,
        }),
        breadcrumb_list_json(page, base_url)?,
    ];
    let faq_items = faq_entities(markdown);
    if !faq_items.is_empty() {
        graph.push(json!({
            "@type": "FAQPage",
            "mainEntity": faq_items,
        }));
    }
    Ok(json!({
        "@context": "https://schema.org",
        "@graph": graph,
    }))
}

fn page_schema_json(page: &StaticCmsPageRow, canonical: &str, base_url: &str) -> Result<Value> {
    let custom_schema = page.schema_markup_payload.is_object()
        && !page
            .schema_markup_payload
            .as_object()
            .map(|o| o.is_empty())
            .unwrap_or(true);
    if !custom_schema {
        return fallback_schema_json(page, canonical, base_url);
    }

    let mut graph = vec![
        page.schema_markup_payload.clone(),
        breadcrumb_list_json(page, base_url)?,
    ];
    let faq_items = faq_entities(page_markdown(page));
    if !faq_items.is_empty() {
        graph.push(json!({
            "@type": "FAQPage",
            "mainEntity": faq_items,
        }));
    }
    Ok(json!({
        "@context": "https://schema.org",
        "@graph": graph,
    }))
}

fn render_related_links(
    page: &StaticCmsPageRow,
    pages_by_key: &HashMap<String, StaticCmsPageRow>,
    links_by_source: &HashMap<String, Vec<StaticCmsLinkRow>>,
) -> Result<String> {
    let Some(links) = links_by_source.get(&page.page_node_key) else {
        return Ok(String::new());
    };
    let mut items = String::new();
    for link in links {
        let Some(target) = pages_by_key.get(&link.target_page_key) else {
            continue;
        };
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
        Ok(String::new())
    } else {
        Ok(format!(
            r#"<section class="related"><h2>Related Pages</h2><ul>{items}</ul></section>"#
        ))
    }
}

fn render_page(
    page: &StaticCmsPageRow,
    pages: &[StaticCmsPageRow],
    pages_by_key: &HashMap<String, StaticCmsPageRow>,
    links_by_source: &HashMap<String, Vec<StaticCmsLinkRow>>,
    base_url: &str,
) -> Result<String> {
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
    let related = render_related_links(page, pages_by_key, links_by_source)?;
    let schema_json = serde_json::to_string_pretty(&page_schema_json(page, &canonical, base_url)?)?;

    Ok(format!(
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
    nav {{ max-width: 1120px; margin: 0 auto; padding: 14px 24px; display: flex; gap: 16px; overflow-x: auto; }}
    nav a {{ color: var(--ink); text-decoration: none; white-space: nowrap; font-size: 14px; }}
    nav a[data-depth="0"] {{ font-weight: 700; }}
    nav a[data-depth="1"] {{ font-weight: 600; }}
    nav a[data-depth="2"] {{ color: var(--muted); }}
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
  <header><nav>{nav}</nav></header>
  <main>
    {breadcrumbs}
    <article>
      <h1>{h1}</h1>
      <div class="meta">{page_type} / {intent} / {updated_at}</div>
      {body}
    </article>
    {related}
  </main>
  <footer>Generated by Alegria Static Site Builder</footer>
</body>
</html>
"#,
        locale = escape_html(&page.locale_code),
        title = escape_html(&page.title),
        description = escape_html(&page.meta_description),
        canonical = escape_html(&canonical),
        h1 = escape_html(&page.h1),
        page_type = escape_html(&page.page_type_key),
        intent = escape_html(&page.dominant_intent_key),
        updated_at = escape_html(&page.updated_at),
        breadcrumbs = breadcrumbs,
    ))
}

fn build_static_artifacts(
    pages: &[StaticCmsPageRow],
    links: &[StaticCmsLinkRow],
    base_url: &str,
) -> Result<Vec<StaticArtifact>> {
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
    let mut sitemap_urls = Vec::new();
    let mut manifest_pages = Vec::new();
    for page in pages {
        let html = render_page(page, pages, &pages_by_key, &links_by_source, base_url)?;
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
            "cms_document_id": page.cms_document_id,
            "canonical_url_path": normalize_url_path(&page.canonical_url_path)?,
            "status": page.current_status,
            "title": page.title,
        }));
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
            "builder": "alegria_static_site_builder@1",
            "base_url": base_url,
            "page_count": pages.len(),
            "pages": manifest_pages,
        }))?,
    });

    Ok(artifacts)
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
    fs::write(&marker, b"alegria_static_site_builder@1\n")
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

async fn build_static_site(
    database_url: Option<String>,
    output_dir: &str,
    base_url: &str,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let snapshot = load_static_site_snapshot(&pool).await?;
    let artifacts = build_static_artifacts(&snapshot.pages, &snapshot.links, base_url)?;
    write_static_artifacts(Path::new(output_dir), &artifacts)?;
    println!(
        "STATIC_SITE_BUILD: OK pages={} artifacts={} output={}",
        snapshot.pages.len(),
        artifacts.len(),
        output_dir
    );
    Ok(0)
}

async fn crawl_pending_sources(
    database_url: Option<String>,
    run_id: String,
    query_batch_key: String,
    limit: i64,
    emit_qdrant: bool,
) -> Result<i32> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    let output = run_crawl_sources(
        &repo,
        &contracts::generated::alegria::temporal::v1::CrawlSourcesInputPayload {
            run_id,
            query_batch_key,
            limit: limit as u32,
            emit_qdrant,
        },
    )
    .await?;
    println!(
        "CRAWL_SUMMARY claimed={} crawled={} failed={} raw_pages={} status={}",
        output.claimed_count,
        output.crawled_count,
        output.failed_count,
        output.raw_page_count,
        output.status
    );
    for url in &output.failed_urls {
        eprintln!("CRAWL_FAILED url={url}");
    }
    Ok(if output.failed_count == 0 { 0 } else { 1 })
}
