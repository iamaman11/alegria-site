#[async_trait]
impl ProjectionStatusRepository for SqlxSeoPgRepository<'_> {
    async fn load_projection_barrier_status(
        &self,
        run_id: &str,
    ) -> Result<ProjectionBarrierStatus, DomainError> {
        let run_uuid = Uuid::parse_str(run_id).map_err(|err| DomainError::ValidationFailure {
            message: format!("projection barrier run_id must be a UUID: {err}"),
        })?;
        let verified_truth_write_done: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM pipeline.step_executions
                WHERE run_id = $1
                  AND step_name = 'verified_truth_write'
                  AND status = 'done'
            )
            "#,
        )
        .bind(run_uuid)
        .fetch_one(&*self.pool)
        .await
        .map_err(|err| DomainError::InfraUnavailable {
            message: format!("load verified_truth_write barrier state failed: {err}"),
        })?;

        if verified_truth_write_done {
            let site_input = sqlx_seo_adapter::load_seo_site_build_input(self.pool, run_id).await?;
            let scope = site_input.scope.as_ref().ok_or_else(|| DomainError::ValidationFailure {
                message: "post-truth-write support gate requires SeoSiteBuildInputPayload.scope"
                    .to_string(),
            })?;
            let support = sqlx_seo_adapter::load_verified_support_bundle(
                self.pool,
                run_id,
                &site_input.context_key,
                &scope.scope_signature,
                &scope.applicant_profile,
            )
            .await?;
            if support.is_empty() {
                return Err(DomainError::ValidationFailure {
                    message: "post-truth-write verified support bundle is empty; graph projection and planning are blocked"
                        .to_string(),
                });
            }
        }

        let statuses =
            sqlx_seo_adapter::read_projection_sync_status_for_run(self.pool, run_id).await?;
        Ok(ProjectionBarrierStatus {
            blocked_events: statuses
                .iter()
                .map(|status| status.blocking_event_count())
                .sum(),
            max_open_lag_ms: statuses
                .iter()
                .map(|status| status.max_open_lag_ms)
                .max()
                .unwrap_or(0),
        })
    }
}
