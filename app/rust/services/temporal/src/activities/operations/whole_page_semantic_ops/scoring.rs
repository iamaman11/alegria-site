fn dominant_layers(text: &str) -> Vec<String> {
    let scores = layer_scores_for_text(text);
    let mut layers = select_layers_from_scores(&scores);
    if layers.is_empty() {
        layers.push("procedural".to_string());
    }
    layers
}

#[derive(Debug, Clone)]
struct WholePageDeterministicSnapshot {
    page_mode_hint: String,
    page_mode_confidence: f32,
    dominant_layers: Vec<String>,
    layer_scores: BTreeMap<String, f32>,
    page_summary: String,
    page_context_profile: WholePageContextProfile,
    mixed_section_ids: Vec<i64>,
    global_entities: Vec<String>,
    uncertainty_flags: Vec<String>,
    reason_codes: Vec<String>,
}

fn normalized_marker_score(text: &str, markers: &[&str]) -> f32 {
    if markers.is_empty() {
        return 0.0;
    }
    let lowered = text.to_lowercase();
    let hits = markers
        .iter()
        .filter(|marker| lowered.contains(**marker))
        .count() as f32;
    hits / markers.len() as f32
}

fn marker_hit_count(text: &str, markers: &[&str]) -> usize {
    let lowered = text.to_lowercase();
    markers
        .iter()
        .filter(|marker| lowered.contains(**marker))
        .count()
}

fn layer_marker_score(text: &str, markers: &[&str], multi_hit_floor: f32) -> f32 {
    let normalized = normalized_marker_score(text, markers);
    if marker_hit_count(text, markers) >= 2 {
        normalized.max(multi_hit_floor)
    } else {
        normalized
    }
}

fn layer_scores_for_text(text: &str) -> BTreeMap<String, f32> {
    let mut scores = BTreeMap::new();
    scores.insert(
        "procedural".to_string(),
        layer_marker_score(
            text,
            &[
                "passport",
                "паспорт",
                "insurance",
                "страхов",
                "application form",
                "анкет",
                "requirements",
                "document",
                "fee",
                "eur",
                "сбор",
                "visa",
                "виза",
            ],
            0.24,
        ),
    );
    scores.insert(
        "operational".to_string(),
        layer_marker_score(
            text,
            &[
                "schedule",
                "график",
                "holiday",
                "appointment",
                "booking",
                "submission window",
                "office hours",
                "запись",
                "время работы",
            ],
            0.24,
        ),
    );
    scores.insert(
        "editorial".to_string(),
        layer_marker_score(
            text,
            &[
                "faq",
                "why",
                "mistakes",
                "avoid",
                "what to do",
                "что делать",
                "почему",
                "ошибк",
                "отказ",
                "проблем",
            ],
            0.24,
        ),
    );
    scores.insert(
        "seo".to_string(),
        normalized_marker_score(
            text,
            &["seo", "serp", "ключев", "ranking", "organic traffic"],
        ),
    );
    scores.insert(
        "commercial".to_string(),
        normalized_marker_score(
            text,
            &[
                "consultation",
                "book now",
                "услуга",
                "под ключ",
                "заказать",
                "service package",
            ],
        ),
    );
    scores
}

fn select_layers_from_scores(scores: &BTreeMap<String, f32>) -> Vec<String> {
    let max_score = scores.values().copied().fold(0.0_f32, f32::max);
    let threshold = if max_score >= 0.55 {
        max_score * 0.55
    } else {
        0.20
    };
    scores
        .iter()
        .filter(|(_, score)| **score >= threshold && **score > 0.0)
        .map(|(layer, _)| layer.clone())
        .collect()
}

fn page_mode_scores(
    _text: &str,
    sections: &[raw_crawl_adapter::RawSectionRecord],
) -> BTreeMap<String, f32> {
    let mut scores = BTreeMap::new();
    let source_url = sections
        .first()
        .map(|section| section.source_url.to_lowercase())
        .unwrap_or_default();
    let headings = sections
        .iter()
        .map(|section| section.heading_path.trim())
        .filter(|heading| !heading.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let primary_content = sections
        .iter()
        .filter(|section| !section_is_noise(section))
        .map(|section| section.content_md.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    let secondary_content = sections
        .iter()
        .filter(|section| section_is_noise(section))
        .map(|section| section.content_md.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    let framing_inputs = [
        (source_url.as_str(), 0.25_f32),
        (headings.as_str(), 0.25_f32),
        (primary_content.as_str(), 0.55_f32),
        (secondary_content.as_str(), 0.15_f32),
    ];
    let nav_sections = sections
        .iter()
        .filter(|section| {
            let kind = section.section_type.to_ascii_lowercase();
            kind.contains("nav") || kind.contains("toc")
        })
        .count() as f32;
    let footer_sections = sections
        .iter()
        .filter(|section| section.section_type.to_ascii_lowercase().contains("footer"))
        .count() as f32;
    let total_sections = sections.len().max(1) as f32;
    let content_support = weighted_family_score(
        &framing_inputs,
        &[
            "visa",
            "виза",
            "requirements",
            "document",
            "passport",
            "appointment",
            "faq",
            "guide",
        ],
    );
    let utility_support = weighted_family_score(
        &framing_inputs,
        &["privacy", "cookie", "login", "terms", "policy"],
    );
    scores.insert(
        "directory_page".to_string(),
        weighted_family_score(
            &framing_inputs,
            &["sitemap", "directory", "каталог", "index page"],
        ) + (nav_sections / total_sections) * 0.10,
    );
    scores.insert(
        "menu_page".to_string(),
        weighted_family_score(
            &framing_inputs,
            &["breadcrumb", "menu", "навигац", "sidebar"],
        ) + (nav_sections / total_sections) * 0.50,
    );
    scores.insert(
        "utility_page".to_string(),
        (utility_support + (footer_sections / total_sections) * 0.15 - content_support.min(0.35))
            .max(0.0),
    );
    scores.insert(
        "landing_page".to_string(),
        weighted_family_score(
            &framing_inputs,
            &["consultation", "book now", "услуга", "под ключ", "cta"],
        ),
    );
    scores.insert(
        "content_page".to_string(),
        (0.40 + content_support + ((total_sections - footer_sections) / total_sections) * 0.10
            - (nav_sections / total_sections) * 0.30)
            .clamp(0.0, 1.0),
    );
    scores
}

fn select_page_mode_and_confidence(scores: &BTreeMap<String, f32>) -> (String, f32) {
    let mut ranked = scores.iter().collect::<Vec<_>>();
    ranked.sort_by(|lhs, rhs| rhs.1.total_cmp(lhs.1).then_with(|| lhs.0.cmp(rhs.0)));
    if let Some((mode, score)) = ranked.first() {
        let runner_up = ranked.get(1).map(|(_, value)| **value).unwrap_or(0.0);
        let confidence = (*score - runner_up).max(0.15) + 0.5;
        (mode.to_string(), confidence.min(0.95))
    } else {
        ("content_page".to_string(), 0.5)
    }
}

fn section_mixed_layer_score(text: &str) -> usize {
    layer_scores_for_text(text)
        .values()
        .filter(|score| **score >= 0.20)
        .count()
}

fn build_page_sketch(
    page_id: i64,
    sections: &[raw_crawl_adapter::RawSectionRecord],
    snapshot: &WholePageDeterministicSnapshot,
) -> String {
    let source_url = sections
        .first()
        .map(|section| section.source_url.as_str())
        .unwrap_or_default();
    let headings = sections
        .iter()
        .map(|section| section.heading_path.trim())
        .filter(|heading| !heading.is_empty())
        .take(6)
        .collect::<Vec<_>>();
    let excerpts = sections
        .iter()
        .filter(|section| !section.content_md.trim().is_empty())
        .take(3)
        .map(|section| {
            let excerpt = section.content_md.chars().take(220).collect::<String>();
            format!(
                "section:{} type:{} text:{}",
                section.id, section.section_type, excerpt
            )
        })
        .collect::<Vec<_>>();
    let mut section_histogram = BTreeMap::new();
    for section in sections {
        *section_histogram
            .entry(section.section_type.to_ascii_lowercase())
            .or_insert(0usize) += 1;
    }
    let section_histogram = section_histogram
        .into_iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "page_id:{page_id}\nsource_url:{source_url}\nheadings:{}\nsection_histogram:{}\npage_mode_hint:{}\ndominant_layers:{}\ncontext_country:{}\ncontext_visa:{}\ncontext_authority:{}\nglobal_entities:{}\nsummary:{}\nexcerpts:\n{}",
        headings.join(" | "),
        section_histogram,
        snapshot.page_mode_hint,
        snapshot.dominant_layers.join("|"),
        snapshot.page_context_profile.country_hints.join("|"),
        snapshot.page_context_profile.visa_type_hints.join("|"),
        snapshot.page_context_profile.authority_hints.join("|"),
        snapshot.global_entities.join("|"),
        snapshot.page_summary,
        excerpts.join("\n")
    )
}

fn deterministic_whole_page_snapshot(
    sections: &[raw_crawl_adapter::RawSectionRecord],
    combined: &str,
) -> WholePageDeterministicSnapshot {
    let layer_scores = layer_scores_for_text(combined);
    let dominant_layers = select_layers_from_scores(&layer_scores);
    let page_mode_score_map = page_mode_scores(combined, sections);
    let (page_mode_hint, page_mode_confidence) =
        select_page_mode_and_confidence(&page_mode_score_map);
    let mixed_section_ids = sections
        .iter()
        .filter_map(|section| {
            if section_mixed_layer_score(&section.content_md) >= 2 {
                Some(section.id)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let mut reason_codes = Vec::new();
    reason_codes.push(format!("page_mode:{}", page_mode_hint));
    if !mixed_section_ids.is_empty() {
        reason_codes.push("mixed_section_detected".to_string());
    }
    let mut uncertainty_flags = Vec::new();
    if page_mode_confidence < 0.65 {
        uncertainty_flags.push("low_page_mode_confidence".to_string());
    }
    if dominant_layers.len() > 1 {
        uncertainty_flags.push("multi_layer_page".to_string());
    }
    let page_context_profile = page_context_profile(sections, combined);
    for hint in &page_context_profile.country_hints {
        reason_codes.push(format!("context_country:{hint}"));
    }
    for hint in &page_context_profile.visa_type_hints {
        reason_codes.push(format!("context_visa:{hint}"));
    }
    for hint in &page_context_profile.authority_hints {
        reason_codes.push(format!("context_authority:{hint}"));
    }
    reason_codes.sort();
    reason_codes.dedup();
    WholePageDeterministicSnapshot {
        page_mode_hint,
        page_mode_confidence,
        dominant_layers,
        layer_scores,
        page_summary: summarize(combined),
        page_context_profile,
        mixed_section_ids,
        global_entities: global_entities(combined),
        uncertainty_flags,
        reason_codes,
    }
}
