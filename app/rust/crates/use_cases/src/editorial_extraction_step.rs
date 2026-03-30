use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorialExtractionInput {
    pub section_id: String,
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorialTopic {
    pub topic_type: String,
    pub topic_key_candidate: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorialExtractionOutput {
    pub topics: Vec<EditorialTopic>,
}

fn to_key(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .replace('ё', "е")
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>()
}

pub fn execute(input: &EditorialExtractionInput) -> EditorialExtractionOutput {
    let lowered = input.raw_text.to_lowercase();
    let mut topics = Vec::new();

    if lowered.contains("как ") || lowered.contains("почему") || lowered.contains("что делать") {
        topics.push(EditorialTopic {
            topic_type: "faq_topic".to_string(),
            topic_key_candidate: to_key("faq_user_question"),
            confidence: 0.82,
        });
    }
    if lowered.contains("ошибк") || lowered.contains("отказ") || lowered.contains("проблем") {
        topics.push(EditorialTopic {
            topic_type: "pain_point".to_string(),
            topic_key_candidate: to_key("visa_refusal_pain_point"),
            confidence: 0.86,
        });
    }
    if lowered.contains("ип") || lowered.contains("self-employed") || lowered.contains("самозанят") {
        topics.push(EditorialTopic {
            topic_type: "profile_topic".to_string(),
            topic_key_candidate: to_key("self_employed_guidance"),
            confidence: 0.84,
        });
    }

    EditorialExtractionOutput { topics }
}
