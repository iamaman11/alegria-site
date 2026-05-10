use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

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

static MONEY_RE: OnceLock<Regex> = OnceLock::new();
static DAY_RE: OnceLock<Regex> = OnceLock::new();
static DATE_RE: OnceLock<Regex> = OnceLock::new();

fn money_re() -> &'static Regex {
    MONEY_RE.get_or_init(|| Regex::new(r"(?i)\b\d+(?:[.,]\d+)?\s*(?:eur|€|евро)\b").unwrap())
}
fn day_re() -> &'static Regex {
    DAY_RE.get_or_init(|| Regex::new(r"(?i)\b\d+\s*(?:дн|дней|day|days)\b").unwrap())
}
fn date_re() -> &'static Regex {
    DATE_RE.get_or_init(|| Regex::new(r"\b\d{1,2}[./-]\d{1,2}[./-]\d{2,4}\b").unwrap())
}

pub fn execute(input: &EntitySpanInput) -> EntitySpanOutput {
    let text = input.raw_text.clone();
    let lowered = text.to_lowercase();
    let mut mentions: Vec<EntityMention> = Vec::new();

    for m in money_re().find_iter(&text) {
        mentions.push(EntityMention {
            raw_text: m.as_str().to_string(),
            entity_type: "fee".to_string(),
            has_numeric: true,
            is_central: true,
            confidence: 0.95,
        });
    }
    for m in day_re().find_iter(&text) {
        mentions.push(EntityMention {
            raw_text: m.as_str().to_string(),
            entity_type: "timeline".to_string(),
            has_numeric: true,
            is_central: true,
            confidence: 0.93,
        });
    }
    for m in date_re().find_iter(&text) {
        mentions.push(EntityMention {
            raw_text: m.as_str().to_string(),
            entity_type: "date".to_string(),
            has_numeric: true,
            is_central: false,
            confidence: 0.90,
        });
    }

    for k in ["паспорт", "страхов", "анкет", "декларац", "справк"]
    {
        if lowered.contains(k) {
            mentions.push(EntityMention {
                raw_text: k.to_string(),
                entity_type: "concept".to_string(),
                has_numeric: false,
                is_central: true,
                confidence: 0.82,
            });
        }
    }
    for k in ["vfs", "посольств", "консульств", "визовый центр"] {
        if lowered.contains(k) {
            mentions.push(EntityMention {
                raw_text: k.to_string(),
                entity_type: "organization".to_string(),
                has_numeric: false,
                is_central: true,
                confidence: 0.80,
            });
        }
    }
    for k in ["ип", "self-employed", "студент", "пенсионер", "дети"] {
        if lowered.contains(k) {
            mentions.push(EntityMention {
                raw_text: k.to_string(),
                entity_type: "profile".to_string(),
                has_numeric: false,
                is_central: false,
                confidence: 0.78,
            });
        }
    }

    EntitySpanOutput { mentions }
}
