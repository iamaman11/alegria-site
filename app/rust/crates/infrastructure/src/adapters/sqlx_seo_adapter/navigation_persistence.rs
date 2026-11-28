pub const NAVIGATION_NOTIFY_CHANNEL: &str = "alegria_navigation_changed";

/// Ignore timestamps and write activity: Global Reconcile may replace the
/// same tree many times without changing a single rendered navigation item.
async fn navigation_projection_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    navigation_tree_key: &str,
) -> Result<serde_json::Value, DomainError> {
    sqlx::query_scalar(
        r#"
        SELECT COALESCE(jsonb_agg(jsonb_build_array(
            i.page_node_key, i.parent_item_key, i.silo_group_key,
            i.label, i.url_path, i.hierarchy_depth, i.sort_order, i.status,
            COALESCE(g.sort_order, 1000), COALESCE(g.status, '')
        ) ORDER BY i.navigation_item_key), '[]'::jsonb)
        FROM site.navigation_items i
        LEFT JOIN site.silo_groups g ON g.silo_group_key = i.silo_group_key
        WHERE i.navigation_tree_key = $1
        "#,
    )
    .bind(navigation_tree_key)
    .fetch_one(&mut **tx)
    .await
    .map_err(classify_sqlx)
}

/// The scope tuple is authoritative even when a planned page has no keyword
/// cluster. Do not infer an unknown page's market from the current caller.
/// This metadata is also consumed by CMS and Temporal rebuild dispatch.
pub async fn upsert_site_scope_identity_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scope_signature: &str,
    market: &str,
    locale: &str,
    country_code: &str,
    visa_type: &str,
    applicant_profile: &str,
) -> Result<(), DomainError> {
    if scope_signature.trim().is_empty() || market.trim().is_empty() || locale.trim().is_empty() {
        return Err(DomainError::ContractViolation {
            message: "global navigation requires authoritative scope signature, market and locale"
                .to_string(),
        });
    }
    sqlx::query(
        r#"
        INSERT INTO site.site_scopes
            (scope_signature, market, locale, country_code, visa_type, applicant_profile,
             page_count, status, last_reconciled_at)
        VALUES ($1, $2, $3, $4, $5, $6, 0, 'active', now())
        ON CONFLICT (scope_signature) DO UPDATE
        SET market = EXCLUDED.market, locale = EXCLUDED.locale,
            country_code = EXCLUDED.country_code, visa_type = EXCLUDED.visa_type,
            applicant_profile = EXCLUDED.applicant_profile,
            updated_at = now()
        "#,
    )
    .bind(scope_signature)
    .bind(market)
    .bind(locale)
    .bind(blank_as_none(country_code))
    .bind(blank_as_none(visa_type))
    .bind(blank_as_none(applicant_profile))
    .execute(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub async fn persist_global_navigation_from_active_pages(
    pool: &PgPool,
    market: &str,
    locale: &str,
) -> Result<GlobalNavigationReport, DomainError> {
    let mut tx = pool.begin().await.map_err(classify_sqlx)?;
    let report = persist_global_navigation_from_active_pages_in_tx(&mut tx, market, locale).await?;
    tx.commit().await.map_err(classify_sqlx)?;
    Ok(report)
}

/// Compose navigation with the surrounding Global Reconcile transaction.
/// Its page snapshot sees IA, Link and retirement changes in that same tx.
pub async fn persist_global_navigation_from_active_pages_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    market: &str,
    locale: &str,
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
    let navigation_tree_key = seo_domain::artifact_identity::seo_artifact_key(
        "navigation_tree",
        &[market, locale, "navigation_policy@1"],
    );
    // Lock the release's tree BEFORE reading its page snapshot. Otherwise
    // two Global Reconciles for distinct scopes of one market can both read
    // stale snapshots, then sequentially overwrite one another's menu.
    sqlx::query(
        r#"
        INSERT INTO site.navigation_trees
            (navigation_tree_key, market, locale, tree_version, policy_version, status, generated_at)
        VALUES ($1, $2, $3, 1, 'navigation_policy@1', 'active', now())
        ON CONFLICT (navigation_tree_key) DO UPDATE
        SET status = 'active', generated_at = now(), updated_at = now()
        "#,
    )
    .bind(&navigation_tree_key)
    .bind(market)
    .bind(locale)
    .execute(&mut **tx)
    .await
    .map_err(classify_sqlx)?;
    let previous_navigation = navigation_projection_in_tx(tx, &navigation_tree_key).await?;
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
            s.market,
            s.locale,
            COALESCE(s.country_code, k.country_code) AS country_code,
            COALESCE(s.visa_type, k.visa_type) AS visa_type,
            COALESCE(s.applicant_profile, k.applicant_profile) AS applicant_profile
        FROM site.page_nodes p
        JOIN site.site_scopes s ON s.scope_signature = p.scope_signature
        LEFT JOIN site.keyword_clusters k ON k.cluster_key = p.keyword_cluster_key
        WHERE p.lifecycle_state NOT IN ('blocked', 'deprecated')
          AND s.market = $1 AND s.locale = $2
        ORDER BY p.scope_signature, p.hierarchy_depth, p.canonical_url_path, p.page_node_key
        "#,
    )
    .bind(market)
    .bind(locale)
    .fetch_all(&mut **tx)
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
    // Zero active pages is itself a valid revision. Reconcile an empty tree
    // instead of leaving the previous menu visible after all pages retire.

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

    // The reconciled tree is one result: readers must never observe the tree
    // after its old items were deleted but before its replacement is complete.
    // This also rolls back scope counts and silo groups if a late write fails.
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
        .execute(&mut **tx)
        .await
        .map_err(classify_sqlx)?;
        scope_count += 1;
    }
    // An empty snapshot is a real revision: retire only scopes of this
    // market/locale whose last active page disappeared, preserving all other
    // markets (and their dispatch metadata).
    let active_scope_keys = scope_fields_by_signature
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    sqlx::query(
        r#"
        UPDATE site.site_scopes
        SET page_count = 0, status = 'deprecated',
            last_reconciled_at = now(), updated_at = now()
        WHERE market = $1 AND locale = $2
          AND NOT (scope_signature = ANY($3))
          AND (status <> 'deprecated' OR page_count <> 0)
        "#,
    )
    .bind(market)
    .bind(locale)
    .bind(&active_scope_keys)
    .execute(&mut **tx)
    .await
    .map_err(classify_sqlx)?;

    let previous_groups = sqlx::query(
        "DELETE FROM site.navigation_items WHERE navigation_tree_key = $1 RETURNING silo_group_key",
    )
    .bind(&navigation_tree_key)
    .fetch_all(&mut **tx)
    .await
    .map_err(classify_sqlx)?
    .into_iter()
    .filter_map(|row| row.get::<Option<String>, _>("silo_group_key"))
    .collect::<Vec<_>>();

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
        let silo_group_key =
            seo_domain::artifact_identity::seo_artifact_key("silo_group", &[&menu_group]);
        sqlx::query(
            r#"
            INSERT INTO site.silo_groups
                (silo_group_key, scope_signature, group_type, label, canonical_url_path, sort_order, status)
            VALUES ($1, $2, 'directory', $3, '', 1000, 'active')
            ON CONFLICT (silo_group_key) DO UPDATE
            SET scope_signature = NULL,
                label = EXCLUDED.label,
                status = 'active',
                updated_at = now()
            "#,
        )
        .bind(&silo_group_key)
        // A menu-group label is a shared dictionary entry. Its owner is the
        // per-market navigation tree/item, not the first arbitrary scope
        // that happened to insert the same group in this global table.
        .bind(Option::<&str>::None)
        .bind(seo_domain::navigation::label_from_slug(&menu_group.replace(':', "-")))
        .execute(&mut **tx)
        .await
        .map_err(classify_sqlx)?;
        silo_group_by_menu.insert(menu_group, silo_group_key);
    }

    let page_item_key = pages
        .iter()
        .map(|page| {
            (
                page.page_node_key.clone(),
                seo_domain::artifact_identity::seo_artifact_key(
                    "navigation_item",
                    &[&navigation_tree_key, &page.page_node_key],
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut page_item_count = 0u64;
    let mut seen_tree_url_labels = std::collections::BTreeSet::<(String, String)>::new();
    for (idx, page) in pages.iter().enumerate() {
        let label = seo_domain::navigation::label_for_page(
            &page.page_type_key,
            &page.canonical_slug,
            &page.canonical_url_path,
        );
        let tree_url_label = (page.canonical_url_path.clone(), label.clone());
        if !seen_tree_url_labels.insert(tree_url_label) {
            continue;
        }
        let item_key = page_item_key
            .get(&page.page_node_key)
            .cloned()
            .unwrap_or_else(|| {
                seo_domain::artifact_identity::seo_artifact_key(
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
        .execute(&mut **tx)
        .await
        .map_err(classify_sqlx)?;
        page_item_count += 1;
    }

    // Shared menu labels are a dictionary. Retire only groups affected by
    // this tree's replacement; unrelated markets must not cause global
    // scans or change state during a scoped rebuild.
    if !previous_groups.is_empty() {
        sqlx::query(
            r#"
            UPDATE site.silo_groups g
            SET status = 'deprecated', updated_at = now()
            WHERE g.silo_group_key = ANY($1) AND g.status = 'active'
              AND NOT EXISTS (
                  SELECT 1 FROM site.navigation_items i
                  JOIN site.navigation_trees t ON t.navigation_tree_key = i.navigation_tree_key
                  WHERE i.silo_group_key = g.silo_group_key
                    AND i.status = 'active' AND t.status = 'active'
              )
            "#,
        )
        .bind(&previous_groups)
        .execute(&mut **tx)
        .await
        .map_err(classify_sqlx)?;
    }

    if navigation_projection_in_tx(tx, &navigation_tree_key).await? != previous_navigation {
        // PostgreSQL emits this only on COMMIT. A failed/rolled-back planning
        // transaction cannot trigger an external HTML refresh.
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(NAVIGATION_NOTIFY_CHANNEL)
            .bind(&navigation_tree_key)
            .execute(&mut **tx)
            .await
            .map_err(classify_sqlx)?;
    }

    Ok(GlobalNavigationReport {
        navigation_tree_key,
        scope_count,
        page_item_count,
        silo_group_count: silo_group_by_menu.len() as u64,
    })
}
