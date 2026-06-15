pub fn normalize_applicant_profile(value: &str) -> Result<String, DomainError> {
    identity::normalize_applicant_profile(value)
}

pub async fn validate_applicant_profile_reference(
    pool: &PgPool,
    value: &str,
) -> Result<String, DomainError> {
    let normalized = normalize_applicant_profile(value)?;
    let row = sqlx::query(
        r#"
        SELECT status
        FROM kb.applicant_profiles
        WHERE profile_key = $1
        LIMIT 1
        "#,
    )
    .bind(&normalized)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    .ok_or_else(|| {
        validation_failure(format!(
            "applicant_profile `{normalized}` is not registered in kb.applicant_profiles"
        ))
    })?;
    let status: String = row.get("status");
    if status != "active" {
        return Err(validation_failure(format!(
            "applicant_profile `{normalized}` is not active"
        )));
    }
    Ok(normalized)
}

pub async fn bootstrap_seo_scope(
    pool: &PgPool,
    requested_context_key: Option<&str>,
    country_code: &str,
    visa_family: &str,
    visa_subtype: Option<&str>,
    citizenship_code: &str,
) -> Result<SeoScopeBootstrapReport, DomainError> {
    let truth =
        identity::derive_truth_identity(country_code, visa_family, visa_subtype, citizenship_code)?;

    sqlx::query(
        r#"
        INSERT INTO kb.visa_families (key, label_ru)
        VALUES ($1, $2)
        ON CONFLICT (key) DO NOTHING
        "#,
    )
    .bind(&truth.visa_family)
    .bind(truth.visa_family.replace('_', " "))
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    let context_key = requested_context_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&truth.context_key)
        .to_string();

    if let Some(row) = sqlx::query(
        r#"
        SELECT context_key
        FROM kb.visa_contexts
        WHERE country_code = $1
          AND visa_family = $2
          AND COALESCE(visa_subtype, '') = COALESCE($3, '')
          AND citizenship_code = $4
        "#,
    )
    .bind(&truth.country_code)
    .bind(&truth.visa_family)
    .bind(if truth.visa_subtype.is_empty() {
        None
    } else {
        Some(truth.visa_subtype.as_str())
    })
    .bind(&truth.citizenship_code)
    .fetch_optional(pool)
    .await
    .map_err(classify_sqlx)?
    {
        let existing_context_key: String = row.get("context_key");
        if requested_context_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some_and(|requested| requested != existing_context_key)
        {
            return Err(validation_failure(format!(
                "seo scope already exists as context_key `{existing_context_key}`"
            )));
        }
        let seeded_registry_count = seed_runtime_registries(pool).await?;
        return Ok(SeoScopeBootstrapReport {
            context_key: existing_context_key,
            created_context: false,
            seeded_registry_count,
        });
    }

    let result = sqlx::query(
        r#"
        INSERT INTO kb.visa_contexts
            (context_key, country_code, visa_family, visa_subtype, citizenship_code, status)
        VALUES ($1, $2, $3, $4, $5, 'active')
        ON CONFLICT (context_key) DO UPDATE
        SET country_code = EXCLUDED.country_code,
            visa_family = EXCLUDED.visa_family,
            visa_subtype = EXCLUDED.visa_subtype,
            citizenship_code = EXCLUDED.citizenship_code,
            status = 'active',
            updated_at = now()
        "#,
    )
    .bind(&context_key)
    .bind(&truth.country_code)
    .bind(&truth.visa_family)
    .bind(if truth.visa_subtype.is_empty() {
        None
    } else {
        Some(truth.visa_subtype.as_str())
    })
    .bind(&truth.citizenship_code)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    let seeded_registry_count = seed_runtime_registries(pool).await?;
    Ok(SeoScopeBootstrapReport {
        context_key,
        created_context: result.rows_affected() > 0,
        seeded_registry_count,
    })
}

async fn seed_runtime_registries(pool: &PgPool) -> Result<u64, DomainError> {
    let page_types = [
        ("country_hub_page", "Country hub"),
        ("hub_page", "Topic hub"),
        ("detail_page", "Detail guide"),
        ("requirement_page", "Requirements guide"),
        ("fee_page", "Fee guide"),
        ("timeline_page", "Timeline guide"),
        ("faq_page", "FAQ guide"),
        ("checklist_page", "Checklist guide"),
        ("troubleshooting_page", "Troubleshooting guide"),
        ("comparison_page", "Comparison guide"),
        ("supporting_editorial", "Supporting editorial guide"),
    ];
    let mut inserted = 0u64;
    for (page_type, label) in page_types {
        let result = sqlx::query(
            r#"
            INSERT INTO site.registry_page_types (page_type_key, label)
            VALUES ($1, $2)
            ON CONFLICT (page_type_key) DO NOTHING
            "#,
        )
        .bind(page_type)
        .bind(label)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        inserted += result.rows_affected();
    }

    let intents = [
        ("informational", "Informational"),
        ("commercial", "Commercial"),
        ("comparison", "Comparison"),
        ("troubleshooting", "Troubleshooting"),
    ];
    for (intent, label) in intents {
        let result = sqlx::query(
            r#"
            INSERT INTO site.registry_intent_types (intent_type_key, label)
            VALUES ($1, $2)
            ON CONFLICT (intent_type_key) DO NOTHING
            "#,
        )
        .bind(intent)
        .bind(label)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        inserted += result.rows_affected();
    }

    let applicant_profiles = [
        ("standard", "other", "Стандартный заявитель"),
        ("minor", "age", "Несовершеннолетний заявитель"),
        ("student", "status", "Студент"),
        ("family", "family", "Семейный заявитель"),
    ];
    for (profile_key, profile_type, label_ru) in applicant_profiles {
        let result = sqlx::query(
            r#"
            INSERT INTO kb.applicant_profiles (profile_key, profile_type, label_ru, status)
            VALUES ($1, $2, $3, 'active')
            ON CONFLICT (profile_key) DO UPDATE
            SET profile_type = EXCLUDED.profile_type,
                label_ru = EXCLUDED.label_ru,
                updated_at = now()
            "#,
        )
        .bind(profile_key)
        .bind(profile_type)
        .bind(label_ru)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        inserted += result.rows_affected();
    }
    Ok(inserted)
}

pub async fn ensure_seo_runtime_registries(pool: &PgPool) -> Result<u64, DomainError> {
    seed_runtime_registries(pool).await
}

