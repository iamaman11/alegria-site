pub async fn ensure_source(
    pool: &PgPool,
    section: &RawSectionRecord,
) -> std::result::Result<(), primitives::errors::DomainError> {
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    ensure_source_in_tx(&mut tx, section).await?;
    tx.commit().await.map_err(classify_sqlx)
}

/// Compose source registration with the canonical Truth write transaction.
pub async fn ensure_source_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    section: &RawSectionRecord,
) -> std::result::Result<(), primitives::errors::DomainError> {
    let source_label = if section.source_domain.trim().is_empty() {
        section.source_url.clone()
    } else {
        section.source_domain.clone()
    };
    sqlx::query(
        "INSERT INTO kb.sources
         (source_key, source_type, authority_class, independence_group_key, source_label, base_url, trust_level, freshness_ttl_days, override_eligible, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'active')
         ON CONFLICT (source_key) DO NOTHING",
    )
    .bind(&section.source_url)
    .bind(source_type_for_section(section))
    .bind(authority_class_for_section(section))
    .bind(independence_group_key_for_section(section))
    .bind(source_label)
    .bind(&section.source_url)
    .bind(source_trust_level(section))
    .bind(freshness_ttl_days_for_section(section))
    .bind(override_eligible_for_section(section))
    .execute(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    // A crawler's source classification is only an initial proposal. Once a
    // kb.sources row exists its trust/status/rights belong to independent
    // curatorial authority: later crawls must neither reactivate a deprecated
    // source nor upgrade its trust_level with GREATEST or overwrite rights.
    // Hold a real row SHARE lock until the enclosing Truth/Outbox transaction
    // commits, so a concurrent curator downgrade cannot land between this
    // gate and a verified rule write. The registry trigger independently
    // fences existing candidate contexts with truth-context advisory locks.
    let authority_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM kb.sources WHERE source_key = $1 FOR SHARE",
    )
    .bind(&section.source_url)
    .fetch_optional(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    if authority_status.as_deref() != Some("active") {
        return Err(primitives::errors::DomainError::ConflictViolation {
            message: format!(
                "source registry authority for {} is absent or deprecated; curator re-authorization required",
                section.source_url
            ),
        });
    }
    Ok(())
}

fn source_type_for_section(section: &RawSectionRecord) -> &'static str {
    let dtype = section.source_dtype.to_ascii_lowercase();
    let domain = section.source_domain.to_ascii_lowercase();
    if dtype.contains("government")
        || dtype.contains("official")
        || domain.contains(".gov")
        || domain.starts_with("gov.")
        || domain.contains("mfa.")
        || domain.contains("mid.")
        || domain.contains("embassy")
        || domain.contains("consulate")
    {
        "government"
    } else if dtype.contains("vfs") || domain.contains("vfsglobal") {
        "vfs"
    } else if dtype.contains("agency") {
        "niche_agency"
    } else if dtype.contains("forum")
        || domain.contains("forum")
        || domain.contains("reddit.")
        || domain.contains("quora.")
        || domain.contains("stackexchange")
    {
        "forum"
    } else if dtype.contains("editorial")
        || domain.contains("news")
        || domain.contains("blog")
        || domain.contains("media")
        || domain.contains("magazine")
        || domain.contains("medium.")
    {
        "editorial"
    } else {
        "low_trust"
    }
}

fn source_trust_level(section: &RawSectionRecord) -> i32 {
    match source_type_for_section(section) {
        "government" => 5,
        "vfs" => 4,
        "editorial" => 3,
        "niche_agency" => 2,
        "forum" | "low_trust" => 1,
        _ => 1,
    }
}

fn authority_class_for_section(section: &RawSectionRecord) -> &'static str {
    match source_type_for_section(section) {
        "government" => "primary_authority",
        "vfs" => "delegated_authority",
        "editorial" => "editorial",
        "niche_agency" => "agency",
        "forum" | "low_trust" => "forum",
        _ => "unknown",
    }
}

fn independence_group_key_for_section(section: &RawSectionRecord) -> String {
    let domain = domain_norm(&section.source_domain);
    if domain.is_empty() {
        domain_norm(&section.source_url)
    } else {
        domain
    }
}

fn freshness_ttl_days_for_section(section: &RawSectionRecord) -> i32 {
    match source_type_for_section(section) {
        "government" | "vfs" => 14,
        "editorial" | "niche_agency" => 7,
        "forum" => 3,
        "low_trust" => 1,
        _ => 30,
    }
}

fn override_eligible_for_section(section: &RawSectionRecord) -> bool {
    let _ = section;
    false
}

