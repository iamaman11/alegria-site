async fn reset_truth_certification_state(pool: &sqlx::PgPool, context_key: &str) {
    let site_tables = sqlx::query_scalar::<_, String>(
        r#"
        SELECT tablename
        FROM pg_tables
        WHERE schemaname = 'site'
          AND tablename NOT IN ('section_templates', 'page_blueprints')
        ORDER BY tablename
        "#,
    )
    .fetch_all(pool)
    .await
    .unwrap();
    if !site_tables.is_empty() {
        let statement = format!(
            "TRUNCATE TABLE {} RESTART IDENTITY CASCADE",
            site_tables
                .iter()
                .map(|table| format!("site.{table}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        sqlx::query(&statement).execute(pool).await.unwrap();
    }

    sqlx::query("TRUNCATE TABLE monitoring.seo_rebuild_backlog RESTART IDENTITY")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("TRUNCATE TABLE system.sync_outbox RESTART IDENTITY CASCADE")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("TRUNCATE TABLE kb.qdrant_points RESTART IDENTITY CASCADE")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM verified.rule_instances WHERE context_key = $1")
        .bind(context_key)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM extracted.rule_candidates WHERE context_key = $1")
        .bind(context_key)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM raw.section_context_candidates WHERE context_key = $1")
        .bind(context_key)
        .execute(pool)
        .await
        .unwrap();
}

async fn seed_truth_certification_support_bundle(
    pool: &sqlx::PgPool,
    context_key: &str,
    fixture: &TruthCertificationFixture,
) {
    let Some(seed) = fixture.source_inputs.support_bundle_seed.as_ref() else {
        return;
    };

    for concept in &seed.concepts {
        seed_truth_context_and_concept(
            pool,
            context_key,
            &concept.concept_key,
            &concept.concept_type,
            &concept.label_ru,
        )
        .await;
    }

    for rule in &seed.preverified_rules {
        seed_admissible_verified_rule(
            pool,
            context_key,
            &rule.rule_instance_id,
            &rule.rule_type_key,
            &rule.concept_key,
            &rule.role_type,
            rule.params.clone(),
            &rule.source_key,
            &rule.source_url,
            &rule.evidence_quote,
        )
        .await;
    }
}

async fn seed_truth_certification_source_governance(
    pool: &sqlx::PgPool,
    fixture: &TruthCertificationFixture,
    rendered_pages: &[RenderedTruthCertificationPage],
) {
    for page in &fixture.source_inputs.pages {
        let rendered = rendered_pages
            .iter()
            .find(|rendered| rendered.fixture_id == fixture.fixture_id && rendered.route == page.route)
            .unwrap_or_else(|| {
                panic!(
                    "missing rendered page for fixture {} route {}",
                    fixture.fixture_id, page.route
                )
            });
        let domain = page.domain.to_ascii_lowercase();
        let mut source_type = if domain.contains(".gov")
            || domain.starts_with("gov.")
            || domain.contains("mfa.")
            || domain.contains("mid.")
            || domain.contains("embassy")
            || domain.contains("consulate")
        {
            "government".to_string()
        } else if domain.contains("vfs") || domain.contains("vfsglobal") {
            "vfs".to_string()
        } else if domain.contains("agency") {
            "niche_agency".to_string()
        } else if domain.contains("forum")
            || domain.contains("reddit.")
            || domain.contains("quora.")
            || domain.contains("stackexchange")
        {
            "forum".to_string()
        } else if domain.contains("news")
            || domain.contains("blog")
            || domain.contains("media")
            || domain.contains("magazine")
            || domain.contains("medium.")
        {
            "editorial".to_string()
        } else {
            "low_trust".to_string()
        };
        let mut authority_class = match source_type.as_str() {
            "government" => "primary_authority".to_string(),
            "vfs" => "delegated_authority".to_string(),
            "editorial" => "editorial".to_string(),
            "niche_agency" => "agency".to_string(),
            "forum" => "forum".to_string(),
            _ => "unknown".to_string(),
        };
        let mut independence_group_key = page.domain.clone();
        let mut trust_level = match source_type.as_str() {
            "government" => 5,
            "vfs" => 4,
            "editorial" => 3,
            "niche_agency" => 2,
            _ => 1,
        };
        let mut freshness_ttl_days = match source_type.as_str() {
            "government" | "vfs" => 14,
            "editorial" | "niche_agency" => 7,
            "forum" => 3,
            _ => 1,
        };
        let mut override_eligible = false;
        if let Some(governance) = page.governance_override.as_ref() {
            if let Some(value) = governance.source_type.clone() {
                source_type = value;
            }
            if let Some(value) = governance.authority_class.clone() {
                authority_class = value;
            }
            if let Some(value) = governance.independence_group_key.clone() {
                independence_group_key = value;
            }
            if let Some(value) = governance.trust_level {
                trust_level = value;
            }
            if let Some(value) = governance.freshness_ttl_days {
                freshness_ttl_days = value;
            }
            if let Some(value) = governance.override_eligible {
                override_eligible = value;
            }
        }
        let source_label = format!("fixture-source:{}", rendered.page.domain);
        let base_url = rendered.url.split('/').take(3).collect::<Vec<_>>().join("/");
        sqlx::query(
            r#"
            INSERT INTO kb.sources
                (source_key, source_type, authority_class, independence_group_key, source_label, base_url, trust_level, freshness_ttl_days, override_eligible, status)
            VALUES
                ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'active')
            ON CONFLICT (source_key) DO UPDATE
            SET source_type = EXCLUDED.source_type,
                authority_class = EXCLUDED.authority_class,
                independence_group_key = EXCLUDED.independence_group_key,
                source_label = EXCLUDED.source_label,
                base_url = EXCLUDED.base_url,
                trust_level = EXCLUDED.trust_level,
                freshness_ttl_days = EXCLUDED.freshness_ttl_days,
                override_eligible = EXCLUDED.override_eligible,
                status = EXCLUDED.status,
                updated_at = now()
            "#,
        )
        .bind(&rendered.url)
        .bind(source_type)
        .bind(authority_class)
        .bind(independence_group_key)
        .bind(source_label)
        .bind(base_url)
        .bind(trust_level)
        .bind(freshness_ttl_days)
        .bind(override_eligible)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn register_truth_certification_input(
    repo: &SqlxSeoRuntimeRepository<'_>,
    run_id: String,
    query_batch_key: String,
    query: String,
    run_mode: String,
) -> contracts::generated::alegria::temporal::v1::SeoSiteBuildInputPayload {
    register_site_build_input(
        repo,
        &SeoSiteBuildRegistrationRequest {
            run_id,
            context_key: None,
            market: "alegria-site".to_string(),
            locale: "ru-RU".to_string(),
            country_code: "ES".to_string(),
            visa_type: "tourist".to_string(),
            visa_subtype: None,
            applicant_profile: "standard".to_string(),
            citizenship_code: "BY".to_string(),
            bootstrap_context: true,
            queries: vec![query],
            query_batch_key: Some(query_batch_key),
            run_mode: Some(run_mode),
        },
    )
    .await
    .unwrap()
}
