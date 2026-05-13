use contracts::generated::alegria::temporal::v1::{
    CannibalizationConflictState, IaBuildInputPayload, IaBuildOutputPayload, PageBlueprintState,
    PageNodeState,
};

use crate::seo_step_support::{artifact_key, scope_signature, slug};

fn locale_lang(locale: &str) -> String {
    locale
        .split(['-', '_'])
        .next()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or("und")
        .to_ascii_lowercase()
}

fn page_type_for_keyword(seed_keyword: &str) -> &'static str {
    let query = seed_keyword.to_ascii_lowercase();
    if query.contains("document") || query.contains("requirements") || query.contains("документ")
    {
        "requirement_page"
    } else if query.contains("cost")
        || query.contains("fee")
        || query.contains("price")
        || query.contains("стоим")
    {
        "fee_page"
    } else if query.contains("time") || query.contains("processing") || query.contains("срок") {
        "timeline_page"
    } else if query.contains("faq") || query.contains("вопрос") {
        "faq_page"
    } else {
        "detail_page"
    }
}

fn country_slug(country_code: &str) -> String {
    match country_code.to_ascii_uppercase().as_str() {
        "ES" => "spain".to_string(),
        "TH" => "thailand".to_string(),
        "PL" => "poland".to_string(),
        "FR" => "france".to_string(),
        "IT" => "italy".to_string(),
        "DE" => "germany".to_string(),
        "GR" => "greece".to_string(),
        "PT" => "portugal".to_string(),
        "CZ" => "czech-republic".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

fn page_type_path_segment(page_type: &str, canonical_slug: &str) -> String {
    match page_type {
        "requirement_page" => "requirements".to_string(),
        "fee_page" => "fees".to_string(),
        "timeline_page" => "processing-times".to_string(),
        "faq_page" => "faq".to_string(),
        "checklist_page" => "checklist".to_string(),
        "troubleshooting_page" => format!("problems/{canonical_slug}"),
        "comparison_page" => format!("compare/{canonical_slug}"),
        "supporting_editorial" => format!("guides/{canonical_slug}"),
        _ => canonical_slug.to_string(),
    }
}

fn required_sections_for_page_type(page_type: &str) -> Vec<String> {
    let roles: &[&str] = match page_type {
        "fee_page" => &["overview", "fees", "faq", "related_pages", "cta_disclaimer"],
        "timeline_page" => &[
            "overview",
            "timing",
            "process",
            "faq",
            "related_pages",
            "cta_disclaimer",
        ],
        "requirement_page" | "checklist_page" => &[
            "overview",
            "who_fits",
            "documents",
            "process",
            "where_to_apply",
            "faq",
            "related_pages",
            "cta_disclaimer",
        ],
        "faq_page" => &["overview", "faq", "related_pages", "cta_disclaimer"],
        _ => &[
            "overview",
            "who_fits",
            "documents",
            "process",
            "fees",
            "timing",
            "where_to_apply",
            "faq",
            "related_pages",
            "cta_disclaimer",
        ],
    };
    roles.iter().map(|role| (*role).to_string()).collect()
}

fn canonical_prefix(input: &IaBuildInputPayload) -> String {
    let Some(scope) = input.scope.as_ref() else {
        return "/und/visa/all/all".to_string();
    };
    format!(
        "/{}/visa/{}/{}",
        locale_lang(&scope.locale),
        country_slug(&scope.country_code),
        slug(&scope.visa_type)
    )
}

fn country_prefix(input: &IaBuildInputPayload) -> String {
    let Some(scope) = input.scope.as_ref() else {
        return "/und/visa/all".to_string();
    };
    format!(
        "/{}/visa/{}",
        locale_lang(&scope.locale),
        country_slug(&scope.country_code)
    )
}

fn push_blueprint(
    page_blueprints: &mut Vec<PageBlueprintState>,
    page_type_key: &str,
    dominant_intent: &str,
    scope_class: &str,
    canonical_url_family: &str,
) -> String {
    let blueprint_key = artifact_key(
        "page_blueprint",
        &[page_type_key, dominant_intent, scope_class, "1"],
    );
    if !page_blueprints
        .iter()
        .any(|blueprint| blueprint.blueprint_key == blueprint_key)
    {
        page_blueprints.push(PageBlueprintState {
            blueprint_key: blueprint_key.clone(),
            page_type_key: page_type_key.to_string(),
            dominant_intent: dominant_intent.to_string(),
            scope_class: scope_class.to_string(),
            blueprint_version: 1,
            status: "active".to_string(),
            required_sections: required_sections_for_page_type(page_type_key),
            title_pattern: format!("{{country}} {{visa_type}} {page_type_key}"),
            canonical_url_family: canonical_url_family.to_string(),
        });
    }
    blueprint_key
}

pub fn execute(input: &IaBuildInputPayload) -> IaBuildOutputPayload {
    let fallback_scope = scope_signature(input.scope.as_ref());
    let mut page_nodes = Vec::new();
    let mut page_blueprints = Vec::new();
    let mut canonical_owner: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut cannibalization_conflicts = Vec::new();

    let country_prefix = country_prefix(input);
    let canonical_prefix = canonical_prefix(input);
    let country_segment = input
        .scope
        .as_ref()
        .map(|scope| country_slug(&scope.country_code))
        .unwrap_or_else(|| "all".to_string());
    let country_hub_slug = input
        .scope
        .as_ref()
        .map(|scope| slug(&format!("{} visa", country_slug(&scope.country_code))))
        .unwrap_or_else(|| "visa-country-hub".to_string());
    let visa_hub_slug = input
        .scope
        .as_ref()
        .map(|scope| {
            slug(&format!(
                "{} {} visa",
                country_slug(&scope.country_code),
                scope.visa_type
            ))
        })
        .unwrap_or_else(|| "visa-hub".to_string());

    let country_blueprint_key = push_blueprint(
        &mut page_blueprints,
        "country_hub_page",
        "informational",
        "country_visa_scope",
        "visa_country_silo",
    );
    let country_page_node_key = artifact_key(
        "page_node",
        &[
            &fallback_scope,
            "country_hub_page",
            "informational",
            &country_hub_slug,
        ],
    );
    page_nodes.push(PageNodeState {
        page_node_key: country_page_node_key.clone(),
        scope_signature: fallback_scope.clone(),
        keyword_cluster_key: String::new(),
        blueprint_key: country_blueprint_key,
        page_type_key: "country_hub_page".to_string(),
        dominant_intent: "informational".to_string(),
        canonical_slug: country_hub_slug,
        canonical_url_path: format!("{country_prefix}/"),
        lifecycle_state: "planned".to_string(),
        parent_page_node_key: String::new(),
        hierarchy_depth: 2,
        menu_group: format!("visa:{country_segment}"),
        breadcrumb_policy: "path_segments".to_string(),
        canonical_url_family: "visa_country_silo".to_string(),
    });

    let hub_blueprint_key = push_blueprint(
        &mut page_blueprints,
        "hub_page",
        "informational",
        "visa_scope",
        "visa_type_silo",
    );
    let hub_page_node_key = artifact_key(
        "page_node",
        &[&fallback_scope, "hub_page", "informational", &visa_hub_slug],
    );
    page_nodes.push(PageNodeState {
        page_node_key: hub_page_node_key.clone(),
        scope_signature: fallback_scope.clone(),
        keyword_cluster_key: String::new(),
        blueprint_key: hub_blueprint_key,
        page_type_key: "hub_page".to_string(),
        dominant_intent: "informational".to_string(),
        canonical_slug: visa_hub_slug,
        canonical_url_path: format!("{canonical_prefix}/"),
        lifecycle_state: "planned".to_string(),
        parent_page_node_key: country_page_node_key,
        hierarchy_depth: 3,
        menu_group: format!("visa:{country_segment}"),
        breadcrumb_policy: "path_segments".to_string(),
        canonical_url_family: "visa_type_silo".to_string(),
    });

    for cluster in &input.keyword_clusters {
        let scope = if cluster.scope_signature.is_empty() {
            fallback_scope.clone()
        } else {
            cluster.scope_signature.clone()
        };
        let canonical_slug = slug(&cluster.seed_keyword);
        let page_type_key = page_type_for_keyword(&cluster.seed_keyword).to_string();
        let path_segment = page_type_path_segment(&page_type_key, &canonical_slug);
        let canonical_url_path = format!("{canonical_prefix}/{path_segment}/");
        let dominant_intent = if cluster.dominant_intent.is_empty() {
            "informational".to_string()
        } else {
            cluster.dominant_intent.clone()
        };
        let blueprint_key = push_blueprint(
            &mut page_blueprints,
            &page_type_key,
            &dominant_intent,
            "visa_scope",
            "visa_type_leaf",
        );
        let page_node_key = artifact_key(
            "page_node",
            &[&scope, &page_type_key, &dominant_intent, &canonical_slug],
        );

        if let Some(existing_owner) =
            canonical_owner.insert(canonical_url_path.clone(), page_node_key.clone())
        {
            cannibalization_conflicts.push(CannibalizationConflictState {
                conflict_key: artifact_key(
                    "cannibalization_conflict",
                    &[
                        &scope,
                        &existing_owner,
                        &page_node_key,
                        "duplicate_canonical_url",
                    ],
                ),
                scope_signature: scope.clone(),
                page_key_a: existing_owner,
                page_key_b: page_node_key.clone(),
                conflict_reason: "duplicate_canonical_url".to_string(),
                severity: "blocking".to_string(),
                status: "open".to_string(),
            });
        }
        page_nodes.push(PageNodeState {
            page_node_key,
            scope_signature: scope,
            keyword_cluster_key: cluster.cluster_key.clone(),
            blueprint_key,
            page_type_key,
            dominant_intent,
            canonical_slug,
            canonical_url_path,
            lifecycle_state: "planned".to_string(),
            parent_page_node_key: hub_page_node_key.clone(),
            hierarchy_depth: 4,
            menu_group: format!("visa:{country_segment}"),
            breadcrumb_policy: "path_segments".to_string(),
            canonical_url_family: "visa_type_leaf".to_string(),
        });
    }

    IaBuildOutputPayload {
        page_nodes,
        page_blueprints,
        cannibalization_conflicts,
    }
}
