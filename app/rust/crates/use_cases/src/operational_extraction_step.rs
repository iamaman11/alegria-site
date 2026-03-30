use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalExtractionInput {
    pub section_id: String,
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalEntity {
    pub entity_kind: String,
    pub value: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalExtractionOutput {
    pub entities: Vec<OperationalEntity>,
}

pub fn execute(input: &OperationalExtractionInput) -> OperationalExtractionOutput {
    let lowered = input.raw_text.to_lowercase();
    let mut entities = Vec::new();

    if lowered.contains("график") || lowered.contains("schedule") || lowered.contains("время работы") {
        entities.push(OperationalEntity {
            entity_kind: "office_schedule".to_string(),
            value: "office_schedule_detected".to_string(),
            confidence: 0.88,
        });
    }
    if lowered.contains("выходн") || lowered.contains("holiday") || lowered.contains("closed") {
        entities.push(OperationalEntity {
            entity_kind: "closure_notice".to_string(),
            value: "closure_detected".to_string(),
            confidence: 0.86,
        });
    }
    if lowered.contains("запись") || lowered.contains("appointment") || lowered.contains("slot") {
        entities.push(OperationalEntity {
            entity_kind: "appointment_rule".to_string(),
            value: "appointment_detected".to_string(),
            confidence: 0.83,
        });
    }

    OperationalExtractionOutput { entities }
}
