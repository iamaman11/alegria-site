async fn cms_review_decision(
    database_url: Option<String>,
    page_node_key: &str,
    actor_role: &str,
    decision: &str,
    reason: &str,
) -> Result<(String, String, String)> {
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = SqlxSeoRuntimeRepository::new(&pool);
    let workflow = TemporalSeoWorkflowControlAdapter::new(
        default_temporal_url(),
        format!("alegria-cli-tools@{}", std::process::id()),
        default_temporal_namespace(),
    );
    let result = apply_human_review_decision(
        &repo,
        &workflow,
        &ApplyHumanReviewDecisionInput {
            page_node_key: page_node_key.to_string(),
            actor_role: actor_role.to_string(),
            decision: decision.to_string(),
            reason: reason.to_string(),
            source: "cli_tools_headless_cms@1".to_string(),
        },
    )
    .await
    .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    Ok((result.decision_key, result.revision_id, result.workflow_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_path_uses_directory_indexes() {
        assert_eq!(output_path_for_url("/").unwrap(), "index.html");
        assert_eq!(
            output_path_for_url("/guides/poland-visa/").unwrap(),
            "guides/poland-visa/index.html"
        );
    }

    #[test]
    fn output_path_rejects_traversal() {
        assert!(output_path_for_url("/../secret").is_err());
        assert!(output_path_for_url("/safe?x=1").is_err());
    }

    #[test]
    fn markdown_renderer_outputs_html() {
        let html = markdown_to_html("# Title\n\nBody with **strong** text.");
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<strong>strong</strong>"));
    }

    #[test]
    fn static_artifacts_include_sitemap_and_manifest() {
        let pages = vec![StaticCmsPageRow {
            page_node_key: "page_home".to_string(),
            canonical_url_path: "/".to_string(),
            locale_code: "en".to_string(),
            page_type_key: "home".to_string(),
            dominant_intent_key: "overview".to_string(),
            current_status: "approved".to_string(),
            cms_document_id: "doc_home".to_string(),
            revision_id: "rev_home".to_string(),
            title: "Home".to_string(),
            meta_description: "Home page".to_string(),
            h1: "Home".to_string(),
            body_payload: json!({
                "markdown": "Welcome.\n\n## FAQ\n\n### Is this reviewed?\n\nYes, before publish."
            }),
            schema_markup_payload: json!({}),
            updated_at: "2026-05-06T00:00:00Z".to_string(),
        }];

        let artifacts = build_static_artifacts(&pages, &[], "https://alegria.test").unwrap();
        let paths: Vec<&str> = artifacts
            .iter()
            .map(|artifact| artifact.relative_path.as_str())
            .collect();
        assert!(paths.contains(&"index.html"));
        assert!(paths.contains(&"sitemap.xml"));
        assert!(paths.contains(&"robots.txt"));
        assert!(paths.contains(&"alegria-static-manifest.json"));
        let home = artifacts
            .iter()
            .find(|artifact| artifact.relative_path == "index.html")
            .unwrap();
        let html = String::from_utf8(home.bytes.clone()).unwrap();
        assert!(html.contains("BreadcrumbList"));
        assert!(html.contains("FAQPage"));
        assert!(html.contains("class=\"breadcrumbs\""));
    }
}
