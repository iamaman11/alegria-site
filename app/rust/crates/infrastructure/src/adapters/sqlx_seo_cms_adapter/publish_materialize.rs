pub async fn persist_publish_materialize_output(
    pool: &PgPool,
    input: &PublishMaterializeInputPayload,
    planned: &PublishMaterializeOutputPayload,
) -> Result<PublishMaterializeOutputPayload, DomainError> {
    let runtime = seo_publish_runtime_config(input);
    let mut artifact = planned.publish_artifact.clone().unwrap_or_default();
    if artifact.artifact_key.trim().is_empty() {
        artifact.artifact_key = primitives::seo::seo_artifact_key(
            "publish_artifact",
            &[
                &input.page_node_key,
                &input.revision_id,
                "headless_snapshot",
            ],
        );
    }
    let mut blocking_reasons = planned.blocking_reasons.clone();
    let (build_scope, build, incremental_error) =
        match static_site_builder_adapter::build_static_site_incremental(
            pool,
            Path::new(&runtime.output_dir),
            &runtime.base_url,
            &[input.page_node_key.clone()],
        )
        .await
        {
            Ok(result) => ("page".to_string(), result, None::<String>),
            Err(incremental_err) if runtime.allow_full_rebuild_fallback => {
                match static_site_builder_adapter::build_static_site(
                    pool,
                    Path::new(&runtime.output_dir),
                    &runtime.base_url,
                )
                .await
                {
                    Ok(result) => (
                        "site".to_string(),
                        result,
                        Some(incremental_err.to_string()),
                    ),
                    Err(full_err) => {
                        blocking_reasons.push("static_build_failed".to_string());
                        artifact.status = "build_failed".to_string();
                        artifact.artifact_uri = runtime.output_dir.clone();
                        artifact.manifest_json = json!({
                            "build_scope": "site",
                            "output_dir": runtime.output_dir,
                            "base_url": runtime.base_url,
                            "incremental_error": incremental_err.to_string(),
                            "full_rebuild_error": full_err.to_string(),
                        })
                        .to_string();
                        sqlx::query(
                            r#"
                            INSERT INTO site.publish_artifacts
                                (artifact_key, page_node_key, revision_id, artifact_type, artifact_uri,
                                 manifest_json, status)
                            VALUES ($1, $2, $3, $4, $5, $6, $7)
                            ON CONFLICT (artifact_key) DO UPDATE
                            SET artifact_uri = EXCLUDED.artifact_uri,
                                manifest_json = EXCLUDED.manifest_json,
                                status = EXCLUDED.status,
                                updated_at = now()
                            "#,
                        )
                        .bind(&artifact.artifact_key)
                        .bind(&input.page_node_key)
                        .bind(&input.revision_id)
                        .bind(if artifact.artifact_type.trim().is_empty() {
                            "headless_snapshot"
                        } else {
                            artifact.artifact_type.as_str()
                        })
                        .bind(&artifact.artifact_uri)
                        .bind(Json(json!({
                            "error": artifact.manifest_json.clone(),
                        })))
                        .bind(&artifact.status)
                        .execute(pool)
                        .await
                        .map_err(classify_sqlx)?;
                        return Ok(PublishMaterializeOutputPayload {
                            publish_artifact: Some(artifact),
                            preview_pages: Vec::new(),
                            materialization_status: "build_failed".to_string(),
                            blocking_reasons,
                        });
                    }
                }
            }
            Err(err) => {
                blocking_reasons.push("static_build_failed".to_string());
                artifact.status = "build_failed".to_string();
                artifact.artifact_uri = runtime.output_dir.clone();
                artifact.manifest_json = json!({
                    "build_scope": "page",
                    "output_dir": runtime.output_dir,
                    "base_url": runtime.base_url,
                    "incremental_error": err.to_string(),
                })
                .to_string();
                sqlx::query(
                    r#"
                    INSERT INTO site.publish_artifacts
                        (artifact_key, page_node_key, revision_id, artifact_type, artifact_uri,
                         manifest_json, status)
                    VALUES ($1, $2, $3, $4, $5, $6, $7)
                    ON CONFLICT (artifact_key) DO UPDATE
                    SET artifact_uri = EXCLUDED.artifact_uri,
                        manifest_json = EXCLUDED.manifest_json,
                        status = EXCLUDED.status,
                        updated_at = now()
                    "#,
                )
                .bind(&artifact.artifact_key)
                .bind(&input.page_node_key)
                .bind(&input.revision_id)
                .bind(if artifact.artifact_type.trim().is_empty() {
                    "headless_snapshot"
                } else {
                    artifact.artifact_type.as_str()
                })
                .bind(&artifact.artifact_uri)
                .bind(Json(json!({
                    "error": artifact.manifest_json.clone(),
                })))
                .bind(&artifact.status)
                .execute(pool)
                .await
                .map_err(classify_sqlx)?;
                return Ok(PublishMaterializeOutputPayload {
                    publish_artifact: Some(artifact),
                    preview_pages: Vec::new(),
                    materialization_status: "build_failed".to_string(),
                    blocking_reasons,
                });
            }
        };

    let preview_pages = build
        .previews
        .into_iter()
        .map(|preview| RenderPreviewPageState {
            page_node_key: preview.page_node_key,
            revision_id: preview.revision_id,
            canonical_url_path: preview.canonical_url_path,
            rendered_html: preview.rendered_html,
            has_breadcrumbs: preview.has_breadcrumbs,
            has_schema_markup: preview.has_schema_markup,
            required_link_count: preview.required_link_count as u32,
            rendered_link_count: preview.rendered_link_count as u32,
        })
        .collect::<Vec<_>>();
    artifact.artifact_uri = build.output_dir.display().to_string();
    artifact.status = "built_pending_validation".to_string();
    artifact.manifest_json = json!({
        "build_scope": build_scope,
        "output_dir": build.output_dir,
        "base_url": runtime.base_url,
        "artifact_count": build.artifacts.len(),
        "page_count": preview_pages.len(),
        "content_contract": "headless_cms_blocks@1",
        "renderer_version": "alegria_static_site_builder@2",
        "incremental_error": incremental_error,
    })
    .to_string();
    sqlx::query(
        r#"
        INSERT INTO site.publish_artifacts
            (artifact_key, page_node_key, revision_id, artifact_type, artifact_uri,
             manifest_json, status)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (artifact_key) DO UPDATE
        SET artifact_uri = EXCLUDED.artifact_uri,
            manifest_json = EXCLUDED.manifest_json,
            status = EXCLUDED.status,
            updated_at = now()
        "#,
    )
    .bind(&artifact.artifact_key)
    .bind(&input.page_node_key)
    .bind(&input.revision_id)
    .bind(if artifact.artifact_type.trim().is_empty() {
        "headless_snapshot"
    } else {
        artifact.artifact_type.as_str()
    })
    .bind(&artifact.artifact_uri)
    .bind(Json(json!({
        "build_scope": build_scope,
        "output_dir": artifact.artifact_uri.clone(),
        "page_count": preview_pages.len(),
        "blocking_reasons": blocking_reasons,
        "incremental_error": incremental_error,
    })))
    .bind(&artifact.status)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query(r#"DELETE FROM site.publish_artifact_entries WHERE artifact_key = $1"#)
        .bind(&artifact.artifact_key)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    for built_artifact in &build.artifacts {
        let entry_type = match built_artifact.relative_path.as_str() {
            "sitemap.xml" => "sitemap",
            "robots.txt" => "robots",
            "alegria-static-manifest.json" => "manifest",
            _ => "page",
        };
        sqlx::query(
            r#"
            INSERT INTO site.publish_artifact_entries
                (artifact_entry_key, artifact_key, relative_path, entry_type, status)
            VALUES ($1, $2, $3, $4, 'built')
            ON CONFLICT (artifact_key, relative_path) DO UPDATE
            SET entry_type = EXCLUDED.entry_type,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(primitives::seo::seo_artifact_key(
            "publish_artifact_entry",
            &[&artifact.artifact_key, &built_artifact.relative_path],
        ))
        .bind(&artifact.artifact_key)
        .bind(&built_artifact.relative_path)
        .bind(entry_type)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
    }

    Ok(PublishMaterializeOutputPayload {
        publish_artifact: Some(artifact),
        preview_pages,
        materialization_status: format!("built_pending_validation:{build_scope}"),
        blocking_reasons: planned.blocking_reasons.clone(),
    })
}

