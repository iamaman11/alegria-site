use contracts::generated::alegria::temporal::v1::{
    RenderPreviewValidateInputPayload, RenderPreviewValidateOutputPayload, SeoPublishBlockerState,
};
use seo_domain::render_preview::{validate_render_preview, RenderedPage};

pub fn execute(input: &RenderPreviewValidateInputPayload) -> RenderPreviewValidateOutputPayload {
    let pages = input
        .preview_pages
        .iter()
        .map(|page| RenderedPage {
            rendered_html: &page.rendered_html,
            has_breadcrumbs: page.has_breadcrumbs,
            has_schema_markup: page.has_schema_markup,
            required_link_count: page.required_link_count,
            rendered_link_count: page.rendered_link_count,
        })
        .collect::<Vec<_>>();
    let findings = validate_render_preview(&pages);
    let blocking_reasons = findings
        .iter()
        .map(|finding| finding.reason_code().to_string())
        .collect();
    let blockers = findings
        .iter()
        .map(|finding| SeoPublishBlockerState {
            reason_code: finding.reason_code().to_string(),
            task_type: "publish_gate_blocker".to_string(),
            required_next_action: finding.required_next_action().to_string(),
            recheck_trigger: "publish_materialize_rerun".to_string(),
        })
        .collect();
    RenderPreviewValidateOutputPayload {
        verdict: if findings.is_empty() {
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

    #[test]
    fn maps_domain_findings_to_stable_temporal_blockers_once() {
        let mut page = valid_page(&valid_html(
            "<script>alert(1)</script><a onclick=\"x\">[[unfinished]]</a>",
        ));
        page.has_breadcrumbs = false;
        let output = execute(&RenderPreviewValidateInputPayload {
            run_id: "run-1".to_string(),
            preview_pages: vec![page.clone(), page],
        });
        assert_eq!(output.verdict, "render_blocked");
        assert_eq!(
            output.blocking_reasons,
            [
                "missing_breadcrumb_render",
                "render_contains_placeholder",
                "unsafe_active_content_marker",
                "unsafe_script_structure",
            ]
        );
        assert_eq!(output.blockers.len(), output.blocking_reasons.len());
        for (blocker, reason) in output.blockers.iter().zip(&output.blocking_reasons) {
            assert_eq!(&blocker.reason_code, reason);
            assert_eq!(blocker.task_type, "publish_gate_blocker");
            assert_eq!(blocker.recheck_trigger, "publish_materialize_rerun");
        }
        assert_eq!(
            output.blockers[0].required_next_action,
            "Render breadcrumb navigation in the static preview."
        );
    }
}
