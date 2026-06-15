async fn latest_output_payload_bytes(
    pool: &sqlx::PgPool,
    run_id: &str,
    step_name: &str,
) -> Vec<Vec<u8>> {
    sqlx::query(
        r#"
        SELECT payload_bytes
        FROM pipeline.step_payload_blobs
        WHERE run_id = $1
          AND step_name = $2
          AND payload_kind = 'output'
        ORDER BY created_at ASC
        "#,
    )
    .bind(Uuid::parse_str(run_id).unwrap())
    .bind(step_name)
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.get::<Vec<u8>, _>("payload_bytes"))
    .collect()
}

async fn latest_output_payload_json(
    pool: &sqlx::PgPool,
    run_id: &str,
    step_name: &str,
) -> Vec<serde_json::Value> {
    latest_output_payload_bytes(pool, run_id, step_name)
        .await
        .into_iter()
        .map(|payload| serde_json::from_slice::<serde_json::Value>(&payload).unwrap())
        .collect()
}

async fn latest_proto_outputs<T: RuntimeProtoPayload>(
    pool: &sqlx::PgPool,
    run_id: &str,
    step_name: &str,
) -> Vec<T> {
    latest_output_payload_bytes(pool, run_id, step_name)
        .await
        .into_iter()
        .map(|payload| T::decode_payload_bytes(&payload).unwrap())
        .collect()
}

fn stable_rule_key(rule_type_key: &str, concept_key: &str, params: &serde_json::Value) -> String {
    format!(
        "{}|{}|{}",
        rule_type_key,
        concept_key,
        serde_json::to_string(params).unwrap()
    )
}

fn stable_decision_key(concept_key: &str, params: &serde_json::Value, reason: &str) -> String {
    format!(
        "{}|{}|{}",
        concept_key,
        serde_json::to_string(params).unwrap(),
        reason
    )
}

async fn insert_preapproved_decisions_when_ready(
    pool: sqlx::PgPool,
    scope_signature: String,
    deadline: Instant,
) {
    while Instant::now() < deadline {
        let rows = sqlx::query(
            r#"
            SELECT d.page_draft_key, d.draft_revision, n.page_node_key
            FROM site.page_drafts d
            JOIN site.page_nodes n ON n.page_node_key = d.page_node_key
            WHERE n.scope_signature = $1
            "#,
        )
        .bind(&scope_signature)
        .fetch_all(&pool)
        .await
        .unwrap();
        if !rows.is_empty() {
            for row in rows {
                let page_node_key: String = row.get("page_node_key");
                let page_draft_key: String = row.get("page_draft_key");
                let draft_revision: i32 = row.get("draft_revision");
                let revision_id = artifact_key(
                    "cms_revision",
                    &[
                        &page_node_key,
                        &page_draft_key,
                        &draft_revision.to_string(),
                        "seo_cms_publish@1",
                    ],
                );
                let decision_key =
                    artifact_key("cms_approval", &[&page_node_key, &revision_id, "approved"]);
                sqlx::query(
                    r#"
                    INSERT INTO site.cms_approval_decisions
                        (decision_key, page_node_key, revision_id, actor_role, decision, reason,
                         decided_at, decision_payload)
                    VALUES ($1, $2, $3, 'seo_reviewer', 'approved', 'truth_certification_preapproval',
                            now(), '{"actor_role":"seo_reviewer","reason":"truth_certification_preapproval"}'::jsonb)
                    ON CONFLICT (decision_key) DO NOTHING
                    "#,
                )
                .bind(decision_key)
                .bind(page_node_key)
                .bind(revision_id)
                .execute(&pool)
                .await
                .unwrap();
            }
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
