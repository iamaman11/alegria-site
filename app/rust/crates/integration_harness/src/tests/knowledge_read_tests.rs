// The real SQLx adapter is the only evidence source. Neither Qdrant nor
// Neo4j is connected: retrieval projections cannot supply verified rules.
#[cfg(feature = "e2e")]
#[tokio::test]
#[serial]
async fn knowledge_a13_verified_context_reads_only_admissible_global_truth() {
    use primitives::errors::ErrorClass;
    use seo_application::knowledge::generate_verified_blocks;
    use seo_ports::{knowledge::VerifiedRulesQuery, VerifiedRulesReader};

    let pg = PostgresHarness::start().await.unwrap();
    let suffix = Uuid::new_v4().simple().to_string();
    let context_key = format!("knowledge-a13-{suffix}");
    let run_id = Uuid::new_v4().to_string();
    let source_key = format!("https://knowledge-{suffix}.example.gov/fee");
    let country_code = format!("K{}", &suffix[..8]);

    sqlx::query(
        "INSERT INTO kb.visa_contexts (context_key,country_code,visa_family,citizenship_code) \
         VALUES ($1,$2,'tourist','BY')",
    )
    .bind(&context_key)
    .bind(&country_code)
    .execute(&pg.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb.sources (source_key,source_type,source_label,trust_level) \
         VALUES ($1,'government','Official source',5)",
    )
    .bind(&source_key)
    .execute(&pg.pool)
    .await
    .unwrap();

    for (idx, status, admissibility, amount) in [
        (1, "verified", "admissible", 35.0),
        (2, "verified", "not_admissible", 900.0),
        (3, "disputed", "admissible", 800.0),
        (4, "verified", "admissible", 700.0),
    ] {
        sqlx::query(
            "INSERT INTO verified.rule_instances \
             (rule_instance_id,context_key,rule_type_key,concept_key,role_type,params,status,source_key,publish_admissibility) \
             VALUES ($1,$2,'consular_fee','consular_fee','fee_item',$3,$4,$5,$6)",
        )
        .bind(format!("knowledge-{suffix}-{idx:02}"))
        .bind(&context_key)
        .bind(sqlx::types::Json(serde_json::json!({"amount": amount, "currency": "EUR"})))
        .bind(status)
        .bind(&source_key)
        .bind(admissibility)
        .execute(&pg.pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO kb.applicant_profiles (profile_key,profile_type,label_ru) \
         VALUES ('knowledge-a13-student','status','Student') ON CONFLICT DO NOTHING",
    )
    .execute(&pg.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO verified.rule_instance_profiles (rule_instance_id,profile_key) \
         VALUES ($1,'knowledge-a13-student')",
    )
    .bind(format!("knowledge-{suffix}-04"))
    .execute(&pg.pool)
    .await
    .unwrap();

    let query = VerifiedRulesQuery {
        run_id,
        context_key: context_key.clone(),
    };
    let reader = SqlxSeoRuntimeRepository::new(&pg.pool);
    let verified = reader.read_verified_rules(&query).await.unwrap().unwrap();
    assert_eq!(verified.context_key, context_key);
    assert_eq!(verified.rules.len(), 1, "pending, inadmissible or profile-bound rules must not leak");
    assert_eq!(verified.rules[0].role.as_str(), "fee_item");
    assert_eq!(verified.rules[0].source_key.as_deref(), Some(source_key.as_str()));
    assert_eq!(verified.rules[0].params.primary_value_text().as_deref(), Some("35"));

    let first = generate_verified_blocks(&reader, &query).await.unwrap();
    let replay = generate_verified_blocks(&reader, &query).await.unwrap();
    assert_eq!(first, replay, "read-only replay must be deterministic");
    assert!(first["fees_table"].contains("35"));
    for excluded in ["900", "800", "700"] {
        assert!(!first["fees_table"].contains(excluded));
    }

    sqlx::query(
        "UPDATE verified.rule_instances SET publish_admissibility='not_admissible' \
         WHERE context_key=$1 AND rule_instance_id=$2",
    )
    .bind(&context_key)
    .bind(&verified.rules[0].rule_instance_id)
    .execute(&pg.pool)
    .await
    .unwrap();
    let no_truth = generate_verified_blocks(&reader, &query).await.unwrap_err();
    assert_eq!(no_truth.class(), ErrorClass::ValidationFailure);
    assert!(no_truth.to_string().contains("stage=verified_support"));

    sqlx::query("UPDATE kb.visa_contexts SET status='deprecated' WHERE context_key=$1")
        .bind(&context_key)
        .execute(&pg.pool)
        .await
        .unwrap();
    assert!(reader.read_verified_rules(&query).await.unwrap().is_none());
    let inactive = generate_verified_blocks(&reader, &query).await.unwrap_err();
    assert_eq!(inactive.class(), ErrorClass::ValidationFailure);
    assert!(inactive.to_string().contains("stage=active_context"));
}
