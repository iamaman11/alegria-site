pub async fn ensure_source(
    pool: &PgPool,
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
         ON CONFLICT (source_key) DO UPDATE
         SET source_type = EXCLUDED.source_type,
             authority_class = CASE
                 WHEN kb.sources.authority_class IS NULL
                      OR kb.sources.authority_class = ''
                      OR kb.sources.authority_class = 'unknown'
                 THEN EXCLUDED.authority_class
                 ELSE kb.sources.authority_class
             END,
             independence_group_key = CASE
                 WHEN kb.sources.independence_group_key IS NULL
                      OR kb.sources.independence_group_key = ''
                 THEN EXCLUDED.independence_group_key
                 ELSE kb.sources.independence_group_key
             END,
             source_label = EXCLUDED.source_label,
             base_url = EXCLUDED.base_url,
             trust_level = GREATEST(kb.sources.trust_level, EXCLUDED.trust_level),
             freshness_ttl_days = CASE
                 WHEN kb.sources.freshness_ttl_days IS NULL OR kb.sources.freshness_ttl_days <= 0
                 THEN EXCLUDED.freshness_ttl_days
                 ELSE kb.sources.freshness_ttl_days
             END,
             override_eligible = kb.sources.override_eligible OR EXCLUDED.override_eligible,
             status = 'active',
             updated_at = now()",
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
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
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

