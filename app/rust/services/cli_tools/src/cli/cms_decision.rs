/// The CMS HITL operator only needs PostgreSQL, not graph/vector composition.
/// Keep the same typed application port and transactional SQL implementation
/// used by the full Worker, while excluding provider SDKs from this binary.
struct PgCmsReviewRepository<'a> {
    pool: &'a sqlx::PgPool,
}

#[async_trait::async_trait]
impl CmsReviewDecisionPort for PgCmsReviewRepository<'_> {
    async fn apply_human_review_decision(
        &self,
        request: &CmsReviewDecisionRequest,
    ) -> std::result::Result<CmsReviewDecisionOutcome, primitives::errors::DomainError> {
        infrastructure::adapters::sqlx_seo_cms_adapter::apply_human_review_decision(
            self.pool, request,
        )
        .await
    }
}

async fn cms_review_decision(
    database_url: Option<String>,
    temporal_url: Option<&str>,
    page_node_key: &str,
    revision_id: &str,
    actor_role: &str,
    decision: &str,
    reason: &str,
) -> Result<(String, String, String)> {
    // Parse the external CLI string before opening PostgreSQL or Temporal.
    let decision = seo_ports::HumanReviewDecision::parse(decision)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let database_url = database_url.unwrap_or_else(default_database_url);
    let pool = connect_pg(&database_url).await?;
    let repo = PgCmsReviewRepository { pool: &pool };
    let workflow = TemporalSeoWorkflowControlAdapter::new(
        temporal_url.map(str::to_string).unwrap_or_else(default_temporal_url),
        format!("alegria-cli-tools@{}", std::process::id()),
        default_temporal_namespace(),
    );
    let result = apply_human_review_decision(
        &repo,
        &workflow,
        &ApplyHumanReviewDecisionInput {
            page_node_key: page_node_key.to_string(),
            revision_id: revision_id.to_string(),
            actor_role: actor_role.to_string(),
            decision,
            reason: reason.to_string(),
            source: "cli_tools_headless_cms@1".to_string(),
        },
    )
    .await
    .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    Ok((result.decision_key, result.revision_id, result.workflow_id))
}

#[cfg(test)]
mod cms_review_decision_tests {
    use super::*;

    #[test]
    fn all_editorial_decisions_require_an_exact_revision_id() {
        for command in ["cms-approve-publish", "cms-block", "cms-reopen"] {
            let missing = Cli::try_parse_from([
                "cli_tools",
                command,
                "--page-node-key",
                "page-1",
                "--actor-role",
                "editor",
            ]);
            assert!(missing.is_err(), "{command} accepted missing --revision-id");

            let exact = Cli::try_parse_from([
                "cli_tools",
                command,
                "--page-node-key",
                "page-1",
                "--revision-id",
                "revision-1",
                "--actor-role",
                "editor",
            ]);
            assert!(exact.is_ok(), "{command} rejected exact revision: {exact:?}");
        }
    }

    #[tokio::test]
    async fn unsupported_decision_is_rejected_before_database_connection() {
        let error = cms_review_decision(
            Some("postgres://not-a-real-host.invalid/alegria".to_string()),
            None,
            "page-1",
            "revision-1",
            "editor",
            "published",
            "",
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("unsupported decision: published"));
    }
}

