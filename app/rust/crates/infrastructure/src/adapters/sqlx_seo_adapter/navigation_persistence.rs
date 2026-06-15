pub async fn persist_global_navigation_from_active_pages(
    pool: &PgPool,
    market: &str,
    locale: &str,
    reason: &str,
) -> Result<GlobalNavigationReport, DomainError> {
    let market = if market.trim().is_empty() {
        "global"
    } else {
        market.trim()
    };
    let locale = if locale.trim().is_empty() {
        "und"
    } else {
        locale.trim()
    };
    let rows = sqlx::query(
        r#"
        SELECT
            p.page_node_key,
            p.scope_signature,
            COALESCE(p.parent_page_node_key, '') AS parent_page_node_key,
            p.page_type_key,
            p.canonical_slug,
            p.canonical_url_path,
            p.hierarchy_depth,
            p.menu_group,
            COALESCE(k.market, '') AS market,
            COALESCE(k.locale, '') AS locale,
            k.country_code,
            k.visa_type,
            k.applicant_profile
        FROM site.page_nodes p
        LEFT JOIN site.keyword_clusters k ON k.cluster_key = p.keyword_cluster_key
        WHERE p.lifecycle_state NOT IN ('blocked', 'deprecated')
        ORDER BY p.scope_signature, p.hierarchy_depth, p.canonical_url_path
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;
    let pages = rows
        .into_iter()
        .map(|row| ActivePageForNavigation {
            page_node_key: row.get("page_node_key"),
            scope_signature: row.get("scope_signature"),
            parent_page_node_key: row.get("parent_page_node_key"),
            page_type_key: row.get("page_type_key"),
            canonical_slug: row.get("canonical_slug"),
            canonical_url_path: row.get("canonical_url_path"),
            hierarchy_depth: row.get("hierarchy_depth"),
            menu_group: row.get("menu_group"),
            market: row.get("market"),
            locale: row.get("locale"),
            country_code: row.get("country_code"),
            visa_type: row.get("visa_type"),
            applicant_profile: row.get("applicant_profile"),
        })
        .collect::<Vec<_>>();
    if pages.is_empty() {
        return Ok(GlobalNavigationReport::default());
    }

    let mut scope_page_counts = BTreeMap::<String, i32>::new();
    let mut scope_fields_by_signature = BTreeMap::<
        String,
        (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >::new();
    for page in &pages {
        *scope_page_counts
            .entry(page.scope_signature.clone())
            .or_default() += 1;
        scope_fields_by_signature
            .entry(page.scope_signature.clone())
            .or_insert_with(|| {
                (
                    if page.market.trim().is_empty() {
                        market.to_string()
                    } else {
                        page.market.clone()
                    },
                    if page.locale.trim().is_empty() {
                        locale.to_string()
                    } else {
                        page.locale.clone()
                    },
                    page.country_code.clone(),
                    page.visa_type.clone(),
                    page.applicant_profile.clone(),
                )
            });
    }

    let mut scope_count = 0u64;
    for (
        scope_signature,
        (scope_market, scope_locale, country_code, visa_type, applicant_profile),
    ) in &scope_fields_by_signature
    {
        sqlx::query(
            r#"
            INSERT INTO site.site_scopes
                (scope_signature, market, locale, country_code, visa_type, applicant_profile,
                 page_count, status, last_reconciled_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'active', now())
            ON CONFLICT (scope_signature) DO UPDATE
            SET market = EXCLUDED.market,
                locale = EXCLUDED.locale,
                country_code = EXCLUDED.country_code,
                visa_type = EXCLUDED.visa_type,
                applicant_profile = EXCLUDED.applicant_profile,
                page_count = EXCLUDED.page_count,
                status = 'active',
                last_reconciled_at = now(),
                updated_at = now()
            "#,
        )
        .bind(scope_signature)
        .bind(scope_market)
        .bind(scope_locale)
        .bind(country_code)
        .bind(visa_type)
        .bind(applicant_profile)
        .bind(*scope_page_counts.get(scope_signature).unwrap_or(&0))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        scope_count += 1;
    }

    let navigation_tree_key = primitives::seo::seo_artifact_key(
        "navigation_tree",
        &[market, locale, "navigation_policy@1"],
    );
    sqlx::query(
        r#"
        INSERT INTO site.navigation_trees
            (navigation_tree_key, market, locale, tree_version, policy_version, status, generated_at)
        VALUES ($1, $2, $3, 1, 'navigation_policy@1', 'active', now())
        ON CONFLICT (navigation_tree_key) DO UPDATE
        SET status = 'active',
            generated_at = now(),
            updated_at = now()
        "#,
    )
    .bind(&navigation_tree_key)
    .bind(market)
    .bind(locale)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    sqlx::query("DELETE FROM site.navigation_items WHERE navigation_tree_key = $1")
        .bind(&navigation_tree_key)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;

    let mut silo_group_by_menu = BTreeMap::<String, String>::new();
    for page in &pages {
        let menu_group = if page.menu_group.trim().is_empty() {
            "global".to_string()
        } else {
            page.menu_group.clone()
        };
        if silo_group_by_menu.contains_key(&menu_group) {
            continue;
        }
        let silo_group_key = primitives::seo::seo_artifact_key("silo_group", &[&menu_group]);
        sqlx::query(
            r#"
            INSERT INTO site.silo_groups
                (silo_group_key, scope_signature, group_type, label, canonical_url_path, sort_order, status)
            VALUES ($1, $2, 'directory', $3, '', 1000, 'active')
            ON CONFLICT (silo_group_key) DO UPDATE
            SET label = EXCLUDED.label,
                status = 'active',
                updated_at = now()
            "#,
        )
        .bind(&silo_group_key)
        .bind(blank_as_none(&page.scope_signature))
        .bind(label_from_slug(&menu_group.replace(':', "-")))
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        silo_group_by_menu.insert(menu_group, silo_group_key);
    }

    let page_item_key = pages
        .iter()
        .map(|page| {
            (
                page.page_node_key.clone(),
                primitives::seo::seo_artifact_key(
                    "navigation_item",
                    &[&navigation_tree_key, &page.page_node_key],
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut page_item_count = 0u64;
    let mut seen_tree_url_labels = std::collections::BTreeSet::<(String, String)>::new();
    for (idx, page) in pages.iter().enumerate() {
        let label = label_from_page(page);
        let tree_url_label = (page.canonical_url_path.clone(), label.clone());
        if !seen_tree_url_labels.insert(tree_url_label) {
            continue;
        }
        let item_key = page_item_key
            .get(&page.page_node_key)
            .cloned()
            .unwrap_or_else(|| {
                primitives::seo::seo_artifact_key(
                    "navigation_item",
                    &[&navigation_tree_key, &page.page_node_key],
                )
            });
        let parent_item_key = page_item_key
            .get(&page.parent_page_node_key)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty());
        let silo_group_key = silo_group_by_menu
            .get(if page.menu_group.trim().is_empty() {
                "global"
            } else {
                page.menu_group.as_str()
            })
            .map(String::as_str);
        sqlx::query(
            r#"
            INSERT INTO site.navigation_items
                (navigation_item_key, navigation_tree_key, parent_item_key, silo_group_key,
                 page_node_key, scope_signature, item_type, label, url_path,
                 hierarchy_depth, sort_order, status)
            VALUES ($1, $2, $3, $4, $5, $6, 'page', $7, $8, $9, $10, 'active')
            ON CONFLICT (navigation_item_key) DO UPDATE
            SET parent_item_key = EXCLUDED.parent_item_key,
                silo_group_key = EXCLUDED.silo_group_key,
                label = EXCLUDED.label,
                url_path = EXCLUDED.url_path,
                hierarchy_depth = EXCLUDED.hierarchy_depth,
                sort_order = EXCLUDED.sort_order,
                status = 'active',
                updated_at = now()
            "#,
        )
        .bind(&item_key)
        .bind(&navigation_tree_key)
        .bind(parent_item_key)
        .bind(silo_group_key)
        .bind(&page.page_node_key)
        .bind(&page.scope_signature)
        .bind(label)
        .bind(&page.canonical_url_path)
        .bind(page.hierarchy_depth)
        .bind(idx as i32)
        .execute(pool)
        .await
        .map_err(classify_sqlx)?;
        page_item_count += 1;
    }

    let rebuild_plan_key =
        primitives::seo::seo_artifact_key("global_rebuild_plan", &[&navigation_tree_key, reason]);
    sqlx::query(
        r#"
        INSERT INTO site.global_rebuild_plan
            (rebuild_plan_key, trigger_type, priority, reason_payload, status)
        VALUES ($1, 'navigation_reconciled', 2, $2, 'queued')
        ON CONFLICT (rebuild_plan_key) DO UPDATE
        SET reason_payload = EXCLUDED.reason_payload,
            status = 'queued',
            updated_at = now()
        "#,
    )
    .bind(&rebuild_plan_key)
    .bind(Json(json!({
        "reason": reason,
        "navigation_tree_key": navigation_tree_key,
        "scope_count": scope_count,
        "page_item_count": page_item_count,
    })))
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;

    Ok(GlobalNavigationReport {
        navigation_tree_key,
        scope_count,
        page_item_count,
        silo_group_count: silo_group_by_menu.len() as u64,
        rebuild_plan_count: 1,
    })
}

fn label_from_page(page: &ActivePageForNavigation) -> String {
    let source = if page.canonical_slug.trim().is_empty() {
        page.canonical_url_path
            .trim_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("page")
    } else {
        page.canonical_slug.as_str()
    };
    let mut label = label_from_slug(source);
    match page.page_type_key.as_str() {
        "country_hub_page" if !label.to_ascii_lowercase().contains("visa") => {
            label.push_str(" Visa");
        }
        "fee_page" if !label.to_ascii_lowercase().contains("fee") => label.push_str(" Fees"),
        "timeline_page" if !label.to_ascii_lowercase().contains("time") => {
            label.push_str(" Timeline");
        }
        _ => {}
    }
    label
}

fn label_from_slug(value: &str) -> String {
    let label = value
        .trim_matches('/')
        .split(['-', '_', '/', ':'])
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if label.is_empty() {
        "Page".to_string()
    } else {
        label
    }
}
