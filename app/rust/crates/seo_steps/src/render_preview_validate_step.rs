use contracts::generated::alegria::temporal::v1::{
    RenderPreviewValidateInputPayload, RenderPreviewValidateOutputPayload, SeoPublishBlockerState,
};

fn blocker(reason_code: &str, required_next_action: &str) -> SeoPublishBlockerState {
    SeoPublishBlockerState {
        reason_code: reason_code.to_string(),
        task_type: "publish_gate_blocker".to_string(),
        required_next_action: required_next_action.to_string(),
        recheck_trigger: "publish_materialize_rerun".to_string(),
    }
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

pub fn execute(input: &RenderPreviewValidateInputPayload) -> RenderPreviewValidateOutputPayload {
    let mut blockers = Vec::new();
    if input.preview_pages.is_empty() {
        blockers.push(blocker(
            "empty_render_preview",
            "Build at least one rendered preview page before publish finalization.",
        ));
    }
    for page in &input.preview_pages {
        if page.rendered_html.trim().is_empty() {
            blockers.push(blocker(
                "empty_rendered_html",
                "Render non-empty HTML before publish finalization.",
            ));
        }
        if !page.has_breadcrumbs {
            blockers.push(blocker(
                "missing_breadcrumb_render",
                "Render breadcrumb navigation in the static preview.",
            ));
        }
        if !page.has_schema_markup {
            blockers.push(blocker(
                "missing_schema_render",
                "Render schema markup in the static preview output.",
            ));
        }
        if page.rendered_link_count < page.required_link_count {
            blockers.push(blocker(
                "missing_rendered_related_links",
                "Render every required related link in the static preview.",
            ));
        }
        let html = page.rendered_html.to_ascii_lowercase();
        for needle in [
            "<html lang=\"",
            "<title>",
            "<meta name=\"description\" content=\"",
            "<link rel=\"canonical\" href=\"",
            "application/ld+json",
            "<nav class=\"breadcrumbs\"",
            "<h1>",
        ] {
            if !html.contains(needle) {
                blockers.push(blocker(
                    "missing_required_render_marker",
                    "Render locale, title, description, canonical, H1, breadcrumbs, and schema markers before publication.",
                ));
            }
        }
        for placeholder in ["{{", "[[", "[unsupported:"] {
            if html.contains(placeholder) {
                blockers.push(blocker(
                    "render_contains_placeholder",
                    "Remove unresolved placeholders or unsupported markers from rendered HTML.",
                ));
            }
        }
        for unsafe_marker in [
            "javascript:",
            "data:text/html",
            "<iframe",
            "<object",
            "<embed",
            "onerror=",
            "onload=",
            "onclick=",
            "onmouseover=",
        ] {
            if html.contains(unsafe_marker) {
                blockers.push(blocker(
                    "unsafe_active_content_marker",
                    "Remove active-content HTML, event handlers, and unsafe URL schemes from the rendered preview.",
                ));
            }
        }
        let script_open_count = count_occurrences(&html, "<script");
        let json_ld_open_count = count_occurrences(&html, "<script type=\"application/ld+json\">");
        let script_close_count = count_occurrences(&html, "</script>");
        if script_open_count != json_ld_open_count || script_open_count != script_close_count {
            blockers.push(blocker(
                "unsafe_script_structure",
                "Allow only balanced application/ld+json script elements in the static preview.",
            ));
        }
    }
    blockers.sort_by(|a, b| a.reason_code.cmp(&b.reason_code));
    blockers.dedup_by(|a, b| a.reason_code == b.reason_code);
    let mut blocking_reasons = blockers
        .iter()
        .map(|blocker| blocker.reason_code.clone())
        .collect::<Vec<_>>();
    blocking_reasons.sort();
    blocking_reasons.dedup();
    RenderPreviewValidateOutputPayload {
        verdict: if blockers.is_empty() {
            "render_ready".to_string()
        } else {
            "render_blocked".to_string()
        },
        blocking_reasons,
        blockers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::generated::alegria::temporal::v1::RenderPreviewPageState;

    fn valid_page(html: &str) -> RenderPreviewPageState {
        RenderPreviewPageState {
            page_node_key: "page-1".to_string(),
            revision_id: "rev-1".to_string(),
            canonical_url_path: "/guide".to_string(),
            rendered_html: html.to_string(),
            has_breadcrumbs: true,
            has_schema_markup: true,
            required_link_count: 1,
            rendered_link_count: 1,
        }
    }

    fn valid_html(body: &str) -> String {
        format!(
            "<!doctype html><html lang=\"en\"><head><title>Guide</title><meta name=\"description\" content=\"Useful guide\"><link rel=\"canonical\" href=\"https://example.com/guide\"><script type=\"application/ld+json\">{{}}</script></head><body><nav class=\"breadcrumbs\"></nav><h1>Guide</h1>{body}</body></html>"
        )
    }

    #[test]
    fn accepts_balanced_safe_preview() {
        let output = execute(&RenderPreviewValidateInputPayload {
            run_id: "run-1".to_string(),
            preview_pages: vec![valid_page(&valid_html("<p>Safe content</p>"))],
        });
        assert_eq!(output.verdict, "render_ready");
    }

    #[test]
    fn rejects_active_content_markers() {
        let output = execute(&RenderPreviewValidateInputPayload {
            run_id: "run-1".to_string(),
            preview_pages: vec![valid_page(&valid_html(
                "<a href=\"javascript:alert(1)\" onclick=\"alert(1)\">bad</a>",
            ))],
        });
        assert_eq!(output.verdict, "render_blocked");
        assert!(output
            .blocking_reasons
            .contains(&"unsafe_active_content_marker".to_string()));
    }

    #[test]
    fn rejects_non_json_ld_script_elements() {
        let output = execute(&RenderPreviewValidateInputPayload {
            run_id: "run-1".to_string(),
            preview_pages: vec![valid_page(&valid_html("<script>alert(1)</script>"))],
        });
        assert_eq!(output.verdict, "render_blocked");
        assert!(output
            .blocking_reasons
            .contains(&"unsafe_script_structure".to_string()));
    }

    #[test]
    fn requires_all_required_links_to_render() {
        let mut page = valid_page(&valid_html("<p>Safe content</p>"));
        page.required_link_count = 2;
        page.rendered_link_count = 1;
        let output = execute(&RenderPreviewValidateInputPayload {
            run_id: "run-1".to_string(),
            preview_pages: vec![page],
        });
        assert_eq!(output.verdict, "render_blocked");
        assert!(output
            .blocking_reasons
            .contains(&"missing_rendered_related_links".to_string()));
    }
}
