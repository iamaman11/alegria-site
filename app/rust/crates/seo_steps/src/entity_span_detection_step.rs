use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySpanInput {
    pub section_id: String,
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityMention {
    pub raw_text: String,
    pub entity_type: String,
    pub has_numeric: bool,
    pub is_central: bool,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySpanOutput {
    pub mentions: Vec<EntityMention>,
}

fn normalized(s: &str) -> String {
    s.trim().to_lowercase().replace('ё', "е")
}

fn trim_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| {
            matches!(
                ch,
                ',' | '.' | ';' | ':' | '!' | '?' | '(' | ')' | '[' | ']' | '"' | '\''
            )
        })
        .to_string()
}

fn token_pairs(text: &str) -> Vec<(String, String)> {
    text.split_whitespace()
        .filter_map(|raw| {
            let cleaned = trim_token(raw);
            if cleaned.is_empty() {
                None
            } else {
                Some((raw.to_string(), cleaned))
            }
        })
        .collect()
}

fn is_decimal_token(token: &str) -> bool {
    let mut seen_digit = false;
    let mut seen_separator = false;
    for ch in token.chars() {
        if ch.is_ascii_digit() {
            seen_digit = true;
            continue;
        }
        if (ch == '.' || ch == ',') && !seen_separator {
            seen_separator = true;
            continue;
        }
        return false;
    }
    seen_digit
}

fn is_fee_unit(token: &str) -> bool {
    matches!(normalized(token).as_str(), "eur" | "€" | "евро")
}

fn is_day_unit(token: &str) -> bool {
    matches!(normalized(token).as_str(), "дн" | "дней" | "day" | "days")
}

fn is_date_token(token: &str) -> bool {
    let trimmed = trim_token(token);
    let parts = trimmed
        .split(['.', '/', '-'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() != 3 {
        return false;
    }
    let valid_lengths = [1usize, 2usize, 4usize];
    parts.iter().all(|part| {
        part.chars().all(|ch| ch.is_ascii_digit()) && valid_lengths.contains(&part.len())
    })
}

fn push_phrase_mentions(
    mentions: &mut Vec<EntityMention>,
    lowered: &str,
    phrases: &[(&str, &str, bool, bool, f32)],
) {
    for (needle, entity_type, has_numeric, is_central, confidence) in phrases {
        if lowered.contains(needle) {
            mentions.push(EntityMention {
                raw_text: (*needle).to_string(),
                entity_type: (*entity_type).to_string(),
                has_numeric: *has_numeric,
                is_central: *is_central,
                confidence: *confidence,
            });
        }
    }
}

pub fn execute(input: &EntitySpanInput) -> EntitySpanOutput {
    let lowered = normalized(&input.raw_text);
    let tokens = token_pairs(&input.raw_text);
    let mut mentions: Vec<EntityMention> = Vec::new();

    for window in tokens.windows(2) {
        let left = &window[0].1;
        let right = &window[1].1;
        if is_decimal_token(left) && is_fee_unit(right) {
            mentions.push(EntityMention {
                raw_text: format!("{} {}", left, right),
                entity_type: "fee".to_string(),
                has_numeric: true,
                is_central: true,
                confidence: 0.95,
            });
        }
        if is_decimal_token(left) && is_day_unit(right) {
            mentions.push(EntityMention {
                raw_text: format!("{} {}", left, right),
                entity_type: "timeline".to_string(),
                has_numeric: true,
                is_central: true,
                confidence: 0.93,
            });
        }
    }

    for (_, cleaned) in &tokens {
        if is_date_token(cleaned) {
            mentions.push(EntityMention {
                raw_text: cleaned.clone(),
                entity_type: "date".to_string(),
                has_numeric: true,
                is_central: false,
                confidence: 0.90,
            });
        }
    }

    push_phrase_mentions(
        &mut mentions,
        &lowered,
        &[
            ("паспорт", "concept", false, true, 0.82),
            ("passport", "concept", false, true, 0.82),
            ("страхов", "concept", false, true, 0.82),
            ("insurance", "concept", false, true, 0.82),
            ("анкет", "concept", false, true, 0.82),
            ("декларац", "concept", false, true, 0.82),
            ("справк", "concept", false, true, 0.82),
            ("vfs", "organization", false, true, 0.80),
            ("посольств", "organization", false, true, 0.80),
            ("консульств", "organization", false, true, 0.80),
            ("визовый центр", "organization", false, true, 0.80),
            ("ип", "profile", false, false, 0.78),
            ("self-employed", "profile", false, false, 0.78),
            ("студент", "profile", false, false, 0.78),
            ("пенсионер", "profile", false, false, 0.78),
            ("дети", "profile", false, false, 0.78),
        ],
    );

    EntitySpanOutput { mentions }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_fee_timeline_date_and_concepts_without_regex() {
        let output = execute(&EntitySpanInput {
            section_id: "section-1".to_string(),
            raw_text:
                "Passport required. Fee 80 EUR. Processing time 15 days. Apply before 12/06/2026."
                    .to_string(),
        });
        let entity_types = output
            .mentions
            .iter()
            .map(|mention| mention.entity_type.as_str())
            .collect::<Vec<_>>();
        assert!(entity_types.contains(&"fee"));
        assert!(entity_types.contains(&"timeline"));
        assert!(entity_types.contains(&"date"));
        assert!(entity_types.contains(&"concept"));
    }

    #[test]
    fn irrelevant_footer_without_supported_tokens_does_not_emit_numeric_mentions() {
        let output = execute(&EntitySpanInput {
            section_id: "section-1".to_string(),
            raw_text: "Footer: contact us for updates and newsletter.".to_string(),
        });
        assert!(!output.mentions.iter().any(|mention| mention.has_numeric));
    }
}
