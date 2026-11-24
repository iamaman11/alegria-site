//! Page context hints and source framing, independent of runtime adapters.

use super::scoring::dominant_layers;
use super::{WholePageContextProfile, WholePageSection};

pub(super) fn summarize(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(180).collect())
        .unwrap_or_default()
}

pub(super) fn global_entities(text: &str) -> Vec<String> {
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

pub(super) fn section_is_noise(section: &WholePageSection) -> bool {
    let lowered_type = section.section_type.to_ascii_lowercase();
    lowered_type.contains("footer")
        || lowered_type.contains("nav")
        || lowered_type.contains("toc")
        || lowered_type.contains("menu")
}

pub(super) fn weighted_family_score(inputs: &[(&str, f32)], markers: &[&str]) -> f32 {
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

pub(super) fn page_context_profile(
    sections: &[WholePageSection],
    text: &str,
) -> WholePageContextProfile {
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

pub(super) fn merge_unique_strings(
    left: &mut Vec<String>,
    right: impl IntoIterator<Item = String>,
) {
    for item in right {
        if !left.contains(&item) {
            left.push(item);
        }
    }
}

pub(super) fn supportive_section_count_for_layer(
    sections: &[WholePageSection],
    layer: &str,
) -> usize {
    sections
        .iter()
        .filter(|section| {
            dominant_layers(&section.content_md)
                .iter()
                .any(|value| value == layer)
        })
        .count()
}
