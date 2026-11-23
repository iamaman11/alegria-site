//! Pure publishing policy for rendered HTML. Temporal payloads and the
//! presentation of blockers belong to the calling wire adapter.

#[derive(Debug, Clone, Copy)]
pub struct RenderedPage<'a> {
    pub rendered_html: &'a str,
    pub has_breadcrumbs: bool,
    pub has_schema_markup: bool,
    pub required_link_count: u32,
    pub rendered_link_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBlocker {
    EmptyPreview,
    EmptyHtml,
    MissingBreadcrumbs,
    MissingSchema,
    MissingRelatedLinks,
    MissingRequiredMarker,
    UnresolvedPlaceholder,
    UnsafeActiveContent,
    UnsafeScriptStructure,
}

impl RenderBlocker {
    pub fn reason_code(self) -> &'static str {
        match self {
            Self::EmptyPreview => "empty_render_preview",
            Self::EmptyHtml => "empty_rendered_html",
            Self::MissingBreadcrumbs => "missing_breadcrumb_render",
            Self::MissingSchema => "missing_schema_render",
            Self::MissingRelatedLinks => "missing_rendered_related_links",
            Self::MissingRequiredMarker => "missing_required_render_marker",
            Self::UnresolvedPlaceholder => "render_contains_placeholder",
            Self::UnsafeActiveContent => "unsafe_active_content_marker",
            Self::UnsafeScriptStructure => "unsafe_script_structure",
        }
    }

    pub fn required_next_action(self) -> &'static str {
        match self {
            Self::EmptyPreview => {
                "Build at least one rendered preview page before publish finalization."
            }
            Self::EmptyHtml => "Render non-empty HTML before publish finalization.",
            Self::MissingBreadcrumbs => "Render breadcrumb navigation in the static preview.",
            Self::MissingSchema => "Render schema markup in the static preview output.",
            Self::MissingRelatedLinks => {
                "Render every required related link in the static preview."
            }
            Self::MissingRequiredMarker => {
                "Render locale, title, description, canonical, H1, breadcrumbs, and schema markers before publication."
            }
            Self::UnresolvedPlaceholder => {
                "Remove unresolved placeholders or unsupported markers from rendered HTML."
            }
            Self::UnsafeActiveContent => {
                "Remove active-content HTML, event handlers, and unsafe URL schemes from the rendered preview."
            }
            Self::UnsafeScriptStructure => {
                "Allow only balanced application/ld+json script elements in the static preview."
            }
        }
    }
}

pub fn validate_render_preview(pages: &[RenderedPage<'_>]) -> Vec<RenderBlocker> {
    let mut blockers = Vec::new();
    if pages.is_empty() {
        blockers.push(RenderBlocker::EmptyPreview);
    }

    for page in pages {
        if page.rendered_html.trim().is_empty() {
            blockers.push(RenderBlocker::EmptyHtml);
        }
        if !page.has_breadcrumbs {
            blockers.push(RenderBlocker::MissingBreadcrumbs);
        }
        if !page.has_schema_markup {
            blockers.push(RenderBlocker::MissingSchema);
        }
        if page.rendered_link_count < page.required_link_count {
            blockers.push(RenderBlocker::MissingRelatedLinks);
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
                blockers.push(RenderBlocker::MissingRequiredMarker);
            }
        }
        for placeholder in ["{{", "[[", "[unsupported:"] {
            if html.contains(placeholder) {
                blockers.push(RenderBlocker::UnresolvedPlaceholder);
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
                blockers.push(RenderBlocker::UnsafeActiveContent);
            }
        }
        let script_open_count = html.match_indices("<script").count();
        let json_ld_open_count = html
            .match_indices("<script type=\"application/ld+json\">")
            .count();
        let script_close_count = html.match_indices("</script>").count();
        if script_open_count != json_ld_open_count || script_open_count != script_close_count {
            blockers.push(RenderBlocker::UnsafeScriptStructure);
        }
    }

    // Reason-code order is the stable wire order consumed by existing clients.
    blockers.sort_unstable_by_key(|blocker| blocker.reason_code());
    blockers.dedup();
    blockers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(html: &str) -> RenderedPage<'_> {
        RenderedPage {
            rendered_html: html,
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
    fn valid_json_ld_preview_has_no_blockers() {
        let html = valid_html("<p>Safe content</p>");
        assert!(validate_render_preview(&[page(&html)]).is_empty());
    }

    #[test]
    fn missing_preview_and_missing_render_require_different_actions() {
        assert_eq!(
            validate_render_preview(&[]),
            vec![RenderBlocker::EmptyPreview]
        );
        let blockers = validate_render_preview(&[page("")]);
        assert!(blockers.contains(&RenderBlocker::EmptyHtml));
        assert!(blockers.contains(&RenderBlocker::MissingRequiredMarker));
        assert_eq!(
            RenderBlocker::EmptyPreview.reason_code(),
            "empty_render_preview"
        );
        assert_eq!(
            RenderBlocker::EmptyHtml.required_next_action(),
            "Render non-empty HTML before publish finalization."
        );
    }

    #[test]
    fn missing_rendered_requirements_are_deduplicated_and_sorted() {
        let html = valid_html("<p>Safe</p>");
        let mut incomplete = page(&html);
        incomplete.has_breadcrumbs = false;
        incomplete.has_schema_markup = false;
        incomplete.required_link_count = 2;
        let blockers = validate_render_preview(&[incomplete, incomplete]);
        assert_eq!(
            blockers,
            vec![
                RenderBlocker::MissingBreadcrumbs,
                RenderBlocker::MissingRelatedLinks,
                RenderBlocker::MissingSchema,
            ]
        );
    }

    #[test]
    fn unsafe_markup_and_unresolved_placeholders_fail_closed() {
        let html = valid_html(
            "<script>alert(1)</script><a href=\"javascript:x\" onload=\"x\">[[todo]]</a>",
        );
        let blockers = validate_render_preview(&[page(&html)]);
        assert_eq!(
            blockers,
            vec![
                RenderBlocker::UnresolvedPlaceholder,
                RenderBlocker::UnsafeActiveContent,
                RenderBlocker::UnsafeScriptStructure,
            ]
        );
    }
}
