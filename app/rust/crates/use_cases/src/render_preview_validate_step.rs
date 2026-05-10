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
        if page.required_link_count > 0 && page.rendered_link_count == 0 {
            blockers.push(blocker(
                "missing_rendered_related_links",
                "Render required related links in the static preview.",
            ));
        }
        let html = page.rendered_html.to_ascii_lowercase();
        for needle in [
            "<link rel=\"canonical\"",
            "application/ld+json",
            "<nav class=\"breadcrumbs\"",
        ] {
            if !html.contains(needle) {
                blockers.push(blocker(
                    "missing_required_render_marker",
                    "Render canonical, breadcrumbs, and schema markers before publication.",
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
