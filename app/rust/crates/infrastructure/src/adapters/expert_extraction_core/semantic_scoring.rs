fn stage_record<T: Serialize, U: Serialize>(
    stage_name: &str,
    section: &RawSectionRecord,
    input: &T,
    output: &U,
    status: ExpertStageStatus,
    error_class: Option<&str>,
) -> ExpertStageRecord {
    let input_json = serde_json::to_vec(input).unwrap_or_default();
    let output_json = serde_json::to_vec(output).unwrap_or_default();
    ExpertStageRecord {
        stage_name: stage_name.to_string(),
        schema_version: EXPERT_CORE_SCHEMA_VERSION,
        idempotency_key: blake3_hex(
            format!(
                "expert_extraction_core|{}|{}|{}|{}",
                stage_name, section.id, section.content_hash, section.heading_path
            )
            .as_bytes(),
        ),
        input_hash: content_hash_v1(std::str::from_utf8(&input_json).unwrap_or("")),
        output_hash: content_hash_v1(std::str::from_utf8(&output_json).unwrap_or("")),
        status,
        error_class: error_class.map(str::to_string),
    }
}

fn dominant_layers(text: &str) -> Vec<String> {
    let scores = layer_scores_for_text(text);
    let max_score = scores.values().copied().fold(0.0_f32, f32::max);
    let threshold = if max_score >= 0.55 {
        max_score * 0.55
    } else {
        0.20
    };
    let mut layers = scores
        .iter()
        .filter(|(_, score)| **score >= threshold && **score > 0.0)
        .map(|(layer, _)| layer.clone())
        .collect::<Vec<_>>();
    if layers.is_empty() {
        layers.push("procedural".to_string());
    }
    layers
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
        normalized_marker_score(text, &["seo", "serp", "ранжир", "ключев"]),
    );
    scores.insert(
        "commercial".to_string(),
        normalized_marker_score(text, &["консультац", "заказать", "услуга", "под ключ"]),
    );
    scores
}

fn page_mode_hint(section: &RawSectionRecord) -> String {
    let text = format!(
        "{}\n{}\n{}",
        section.source_url, section.heading_path, section.content_md
    )
    .to_lowercase();
    if text.contains("sitemap") || text.contains("directory") || text.contains("каталог") {
        "directory_page".to_string()
    } else if text.contains("breadcrumb") || text.contains("menu") || text.contains("навигац")
    {
        "menu_page".to_string()
    } else if text.contains("privacy") || text.contains("cookie") || text.contains("login") {
        "utility_page".to_string()
    } else if text.contains("consultation") || text.contains("book now") {
        "landing_page".to_string()
    } else {
        "content_page".to_string()
    }
}

fn summarize(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(180).collect())
        .unwrap_or_default()
}

fn global_entities(text: &str) -> Vec<String> {
    let lowered = text.to_lowercase();
    let mut entities = Vec::new();
    for token in [
        "passport",
        "паспорт",
        "insurance",
        "страхов",
        "vfs",
        "посольств",
    ] {
        if lowered.contains(token) {
            entities.push(token.to_string());
        }
    }
    entities.sort();
    entities.dedup();
    entities
}

fn section_is_noise(section: &RawSectionRecord) -> bool {
    let lowered_type = section.section_type.to_ascii_lowercase();
    lowered_type.contains("footer")
        || lowered_type.contains("nav")
        || lowered_type.contains("toc")
        || lowered_type.contains("menu")
}

fn weighted_family_score(inputs: &[(&str, f32)], markers: &[&str]) -> f32 {
    if markers.is_empty() {
        return 0.0;
    }
    inputs.iter().fold(0.0_f32, |acc, (text, weight)| {
        let lowered = text.to_lowercase();
        if markers.iter().any(|marker| lowered.contains(marker)) {
            acc + *weight
        } else {
            acc
        }
    })
}

fn push_hint_if_supported(
    target: &mut Vec<String>,
    hint: &str,
    score: f32,
    threshold: f32,
    strong_support: bool,
) {
    if score >= threshold || strong_support {
        target.push(hint.to_string());
    }
}

fn page_context_profile(sections: &[RawSectionRecord], text: &str) -> WholePageContextProfile {
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
    let combined = text.to_lowercase();
    let framing_inputs = [
        (source_url.as_str(), 0.35_f32),
        (headings.as_str(), 0.35_f32),
        (primary_content.as_str(), 0.70_f32),
        (secondary_content.as_str(), 0.10_f32),
        (combined.as_str(), 0.20_f32),
    ];

    let mut country_hints = Vec::new();
    for (hint, tokens, threshold) in [
        ("ES", &["spain", "spanish", "испан", "españa"][..], 0.65_f32),
        ("PL", &["poland", "polish", "польш", "polska"][..], 0.65_f32),
        ("FR", &["france", "french", "франц"][..], 0.65_f32),
        (
            "DE",
            &["germany", "german", "герман", "deutschland"][..],
            0.65_f32,
        ),
    ] {
        let score = weighted_family_score(&framing_inputs, tokens);
        let strong_support = tokens
            .iter()
            .any(|token| source_url.contains(token) || headings.contains(token));
        push_hint_if_supported(&mut country_hints, hint, score, threshold, strong_support);
    }

    let mut visa_type_hints = Vec::new();
    for (hint, tokens, threshold) in [
        (
            "tourist",
            &["tourist", "tourism", "турист", "шенген"][..],
            0.60_f32,
        ),
        (
            "work",
            &["work visa", "рабоч", "employment visa"][..],
            0.60_f32,
        ),
        (
            "student",
            &["student visa", "study visa", "учеб", "student"][..],
            0.60_f32,
        ),
    ] {
        let score = weighted_family_score(&framing_inputs, tokens);
        let strong_support = tokens
            .iter()
            .any(|token| source_url.contains(token) || headings.contains(token));
        push_hint_if_supported(&mut visa_type_hints, hint, score, threshold, strong_support);
    }

    let mut authority_hints = Vec::new();
    for (hint, tokens, threshold) in [
        (
            "consulate",
            &["consulate", "consular", "консуль", "посольств"][..],
            0.55_f32,
        ),
        (
            "visa_center",
            &["vfs", "visa center", "визов"][..],
            0.55_f32,
        ),
        (
            "government",
            &["ministry", "gov.", ".gov", "government", "министер"][..],
            0.55_f32,
        ),
    ] {
        let score = weighted_family_score(&framing_inputs, tokens);
        let strong_support = tokens
            .iter()
            .any(|token| source_url.contains(token) || headings.contains(token));
        push_hint_if_supported(&mut authority_hints, hint, score, threshold, strong_support);
    }

    country_hints.sort();
    country_hints.dedup();
    visa_type_hints.sort();
    visa_type_hints.dedup();
    authority_hints.sort();
    authority_hints.dedup();

    WholePageContextProfile {
        country_hints,
        visa_type_hints,
        authority_hints,
    }
}

