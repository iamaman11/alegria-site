fn json_text<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(serde_json::Value::as_str)
}

fn verified_support_fragment(
    role_type: &str,
    concept_label: &str,
    params: &serde_json::Value,
) -> String {
    match role_type {
        "document_required" | "must_provide" | "form_required" => {
            let mut fragment = format!("Required document: {concept_label}.");
            if json_text(params, "subtype").is_some_and(|v| !v.trim().is_empty()) {
                fragment = format!(
                    "Required document: {concept_label} ({}) .",
                    json_text(params, "subtype").unwrap_or_default()
                );
            }
            fragment.replace(" )", ")")
        }
        "fee_item" | "must_pay" => {
            let amount = params
                .get("amount")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let currency = json_text(params, "currency").unwrap_or("EUR");
            if amount > 0.0 {
                format!("Fee item: {amount:.2} {currency} for {concept_label}.")
            } else {
                format!("Fee item: {concept_label}.")
            }
        }
        "timeline_item" | "timeline" => {
            let days = params
                .get("days")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default();
            if days > 0 {
                format!("Processing timeline: {concept_label} takes {days} days.")
            } else {
                format!("Processing timeline: {concept_label}.")
            }
        }
        "where_to_apply" => {
            let channel = json_text(params, "channel").unwrap_or("official route");
            let location = json_text(params, "location_key").unwrap_or(concept_label);
            format!("Where to apply: {location} via {channel}.")
        }
        "appointment_rule" => format!("Appointment rule: {concept_label}."),
        "eligibility_rule" | "must_satisfy" => format!("Eligibility rule: {concept_label}."),
        "step" => {
            let idx = params
                .get("step_index")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default();
            if idx > 0 {
                format!("Application step {idx}: {concept_label}.")
            } else {
                format!("Application step: {concept_label}.")
            }
        }
        "allows" => format!("Allowed condition: {concept_label}."),
        "forbids" => format!("Forbidden condition: {concept_label}."),
        _ => format!("Verified rule: {concept_label}."),
    }
}

async fn resolve_verified_fact_support(
    pool: &PgPool,
    context_key: &str,
    applicant_profile: &str,
) -> Result<applicability::ResolvedVerifiedSupportBundle<SeoVerifiedFactSupportState>, DomainError>
{
    let rows = sqlx::query(
        r#"
        SELECT
            r.rule_instance_id,
            r.role_type,
            r.params,
            r.effective_from::text AS effective_from,
            r.effective_to::text AS effective_to,
            COALESCE(r.freshness_class, 'unknown') AS freshness_class,
            COALESCE(c.label_ru, c.concept_key, r.concept_key) AS concept_label,
            COALESCE(s.source_label, '') AS source_label,
            COALESCE(s.source_type, 'editorial') AS source_type,
            COALESCE(s.trust_level, 3) AS trust_level,
            EXISTS (
                SELECT 1
                FROM verified.rule_instance_profiles rp_any
                WHERE rp_any.rule_instance_id = r.rule_instance_id
            ) AS has_profile_overrides,
            EXISTS (
                SELECT 1
                FROM verified.rule_instance_profiles rp_apply
                WHERE rp_apply.rule_instance_id = r.rule_instance_id
                  AND rp_apply.profile_key = $2
                  AND rp_apply.applicability = 'applies'
            ) AS has_apply_profile,
            EXISTS (
                SELECT 1
                FROM verified.rule_instance_profiles rp_conditional
                WHERE rp_conditional.rule_instance_id = r.rule_instance_id
                  AND rp_conditional.profile_key = $2
                  AND rp_conditional.applicability = 'conditional'
            ) AS has_conditional_profile,
            EXISTS (
                SELECT 1
                FROM verified.rule_instance_profiles rp_exclude
                WHERE rp_exclude.rule_instance_id = r.rule_instance_id
                  AND rp_exclude.profile_key = $2
                  AND rp_exclude.applicability = 'excludes'
            ) AS has_exclude_profile,
            EXISTS (
                SELECT 1
                FROM verified.rule_exceptions re
                WHERE re.rule_instance_id = r.rule_instance_id
                  AND re.status = 'verified'
                  AND re.profile_key = $2
                  AND re.override_kind = 'waive'
            ) AS has_waive_exception,
            EXISTS (
                SELECT 1
                FROM verified.rule_exceptions re
                WHERE re.rule_instance_id = r.rule_instance_id
                  AND re.status = 'verified'
                  AND re.profile_key = $2
                  AND re.override_kind = 'remove_requirement'
            ) AS has_remove_exception,
            EXISTS (
                SELECT 1
                FROM verified.rule_exceptions re
                WHERE re.rule_instance_id = r.rule_instance_id
                  AND re.status = 'verified'
                  AND re.profile_key = $2
                  AND re.override_kind = 'replace_value'
            ) AS has_replace_exception,
            EXISTS (
                SELECT 1
                FROM verified.rule_exceptions re
                WHERE re.rule_instance_id = r.rule_instance_id
                  AND re.status = 'verified'
                  AND re.profile_key = $2
                  AND re.override_kind = 'add_requirement'
            ) AS has_add_requirement_exception
        FROM verified.rule_instances r
        LEFT JOIN kb.concepts c ON c.concept_key = r.concept_key
        LEFT JOIN kb.sources s ON s.source_key = r.source_key
        WHERE r.context_key = $1
          AND r.status = 'verified'
          AND COALESCE(r.publish_admissibility, 'not_admissible') = 'admissible'
          AND r.source_key IS NOT NULL
          AND r.source_key <> ''
          AND r.evidence_section_id IS NOT NULL
          AND COALESCE(r.evidence_quote, '') <> ''
          AND COALESCE(r.source_snapshot_hash, '') <> ''
        ORDER BY r.role_type, r.rule_instance_id
        "#,
    )
    .bind(context_key)
    .bind(applicant_profile)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut candidates = Vec::new();
    for row in rows {
        let rule_instance_id: String = row.get("rule_instance_id");
        let has_profile_overrides: bool = row.get("has_profile_overrides");
        let has_apply_profile: bool = row.get("has_apply_profile");
        let has_conditional_profile: bool = row.get("has_conditional_profile");
        let has_exclude_profile: bool = row.get("has_exclude_profile");
        let has_waive_exception: bool = row.get("has_waive_exception");
        let has_remove_exception: bool = row.get("has_remove_exception");
        let has_replace_exception: bool = row.get("has_replace_exception");
        let has_add_requirement_exception: bool = row.get("has_add_requirement_exception");
        let params: Json<serde_json::Value> = row.get("params");
        let role_type: String = row.get("role_type");
        let concept_label: String = row.get("concept_label");
        let source_type: String = row.get("source_type");
        let trust_level: i32 = row.get("trust_level");
        let source_tier = match (source_type.as_str(), trust_level) {
            ("government", _) | (_, 5) => "official",
            ("vfs", _) | (_, 4) => "regulated_partner",
            ("internal", _) => "internal_verified",
            ("niche_agency", _) => "industry_reference",
            _ => "editorial_reference",
        };
        let effective_to = row
            .get::<Option<String>, _>("effective_to")
            .unwrap_or_default();
        let freshness_class: String = row.get("freshness_class");
        let support = SeoVerifiedFactSupportState {
            fragment_text: verified_support_fragment(&role_type, &concept_label, &params.0),
            support_ref: rule_instance_id.clone(),
            role_type,
            source_label: row.get("source_label"),
            source_tier: source_tier.to_string(),
            freshness_class,
            observed_at: row
                .get::<Option<String>, _>("effective_from")
                .unwrap_or_default(),
            valid_until: effective_to,
        };
        candidates.push((
            applicability::ApplicabilityRuleCandidate {
                rule_instance_id,
                has_profile_overrides,
                has_apply_profile,
                has_conditional_profile,
                has_exclude_profile,
                has_waive_exception,
                has_remove_exception,
                has_replace_exception,
                has_add_requirement_exception,
            },
            support,
        ));
    }
    Ok(applicability::resolve_support_candidates(
        applicant_profile,
        candidates,
    ))
}

pub async fn load_verified_support_bundle(
    pool: &PgPool,
    run_id: &str,
    context_key: &str,
    scope_signature: &str,
    applicant_profile: &str,
) -> Result<Vec<SeoVerifiedFactSupportState>, DomainError> {
    non_empty(context_key, "context_key")?;
    non_empty(scope_signature, "scope_signature")?;
    let normalized_profile = validate_applicant_profile_reference(pool, applicant_profile).await?;
    let resolution = resolve_verified_fact_support(pool, context_key, &normalized_profile).await?;
    if resolution.included.is_empty() {
        return Err(validation_failure(format!(
            "verified support bundle is empty for context_key `{context_key}` and applicant_profile `{normalized_profile}`"
        )));
    }
    let support_count = resolution.included.len();
    let supports = resolution.included;
    let excluded_rule_count = resolution.excluded_rules.len();
    let excluded_rules = resolution
        .excluded_rules
        .iter()
        .map(|diagnostic| {
            json!({
                "rule_instance_id": diagnostic.rule_instance_id,
                "reason_code": diagnostic.reason_code,
                "detail": diagnostic.detail,
            })
        })
        .collect::<Vec<_>>();
    let unresolved_rule_count = resolution.unresolved_rules.len();
    let unresolved_rules = resolution
        .unresolved_rules
        .iter()
        .map(|diagnostic| {
            json!({
                "rule_instance_id": diagnostic.rule_instance_id,
                "reason_code": diagnostic.reason_code,
                "detail": diagnostic.detail,
            })
        })
        .collect::<Vec<_>>();
    let applied_override_count = resolution.applied_overrides.len();
    let applied_overrides = resolution
        .applied_overrides
        .iter()
        .map(|diagnostic| {
            json!({
                "rule_instance_id": diagnostic.rule_instance_id,
                "reason_code": diagnostic.reason_code,
                "detail": diagnostic.detail,
            })
        })
        .collect::<Vec<_>>();
    upsert_runtime_json_blob(
        pool,
        run_id,
        "verified_support_bundle",
        "alegria.temporal.v1.SeoVerifiedSupportBundleJson",
        &json!({
            "context_key": context_key,
            "scope_signature": scope_signature,
            "applicant_profile": normalized_profile,
            "support_count": support_count,
            "supports": supports.clone(),
            "excluded_rule_count": excluded_rule_count,
            "excluded_rules": excluded_rules,
            "unresolved_rule_count": unresolved_rule_count,
            "unresolved_rules": unresolved_rules,
            "applied_override_count": applied_override_count,
            "applied_overrides": applied_overrides,
        }),
    )
    .await?;
    Ok(supports)
}

pub async fn resolve_rebuild_impacts(
    pool: &PgPool,
    changed_truth_keys: &[String],
) -> Result<BTreeMap<String, Value>, DomainError> {
    if changed_truth_keys.is_empty() {
        return Ok(BTreeMap::new());
    }

    let truth_support_refs = changed_truth_keys
        .iter()
        .map(|key| {
            key.strip_prefix("verified.rule_instance:")
                .or_else(|| key.strip_prefix("truth_support:"))
                .unwrap_or(key.as_str())
                .to_string()
        })
        .collect::<Vec<_>>();
    let blueprint_refs = changed_truth_keys
        .iter()
        .filter_map(|key| {
            key.strip_prefix("site.page_blueprint:")
                .or_else(|| key.strip_prefix("blueprint:"))
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let template_refs = changed_truth_keys
        .iter()
        .filter_map(|key| {
            key.strip_prefix("site.section_template:")
                .or_else(|| key.strip_prefix("template:"))
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let serp_query_refs = changed_truth_keys
        .iter()
        .filter_map(|key| {
            key.strip_prefix("serp.query:")
                .or_else(|| key.strip_prefix("serp_pattern:"))
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let source_refs = changed_truth_keys
        .iter()
        .filter_map(|key| {
            key.strip_prefix("source_key:")
                .or_else(|| key.strip_prefix("source:"))
                .or_else(|| key.strip_prefix("raw.source:"))
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let navigation_refs = changed_truth_keys
        .iter()
        .filter_map(|key| {
            key.strip_prefix("navigation_state:")
                .or_else(|| key.strip_prefix("nav_state:"))
                .or_else(|| key.strip_prefix("site.navigation:"))
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let keyword_cluster_refs = if serp_query_refs.is_empty() {
        Vec::new()
    } else {
        sqlx::query(
            r#"
            SELECT cluster_key
            FROM site.keyword_clusters
            WHERE seed_keyword = ANY($1)
            "#,
        )
        .bind(&serp_query_refs)
        .fetch_all(pool)
        .await
        .map_err(classify_sqlx)?
        .into_iter()
        .map(|row| row.get::<String, _>("cluster_key"))
        .collect::<Vec<_>>()
    };

    let rows = sqlx::query(
        r#"
        SELECT page_node_key, dependency_type, dependency_ref, reason_package
        FROM monitoring.seo_rebuild_dependencies
        WHERE status = 'active'
          AND (
            (dependency_type = 'truth_support' AND dependency_ref = ANY($1))
            OR (dependency_type = 'blueprint' AND dependency_ref = ANY($2))
            OR (dependency_type = 'section_template' AND dependency_ref = ANY($3))
            OR (dependency_type = 'serp_query' AND dependency_ref = ANY($4))
            OR (dependency_type = 'keyword_cluster' AND dependency_ref = ANY($5))
            OR (dependency_type = 'source_provenance' AND dependency_ref = ANY($6))
            OR (dependency_type = 'navigation_state' AND dependency_ref = ANY($7))
          )
        ORDER BY page_node_key, dependency_type, dependency_ref
        "#,
    )
    .bind(&truth_support_refs)
    .bind(&blueprint_refs)
    .bind(&template_refs)
    .bind(&serp_query_refs)
    .bind(&keyword_cluster_refs)
    .bind(&source_refs)
    .bind(&navigation_refs)
    .fetch_all(pool)
    .await
    .map_err(classify_sqlx)?;

    let mut impacted = BTreeMap::<String, Value>::new();
    for row in rows {
        let page_node_key: String = row.get("page_node_key");
        let dependency_type: String = row.get("dependency_type");
        let dependency_ref: String = row.get("dependency_ref");
        let reason_package: Json<Value> = row.get("reason_package");
        impacted
            .entry(page_node_key)
            .and_modify(|reason| {
                if let Some(reasons) = reason
                    .get_mut("matched_dependencies")
                    .and_then(Value::as_array_mut)
                {
                    reasons.push(json!({
                        "dependency_type": dependency_type,
                        "dependency_ref": dependency_ref,
                    }));
                }
            })
            .or_insert(json!({
                "matched_dependencies": [{
                    "dependency_type": dependency_type,
                    "dependency_ref": dependency_ref,
                }],
                "seed_reason_package": reason_package.0,
            }));
    }
    Ok(impacted)
}

