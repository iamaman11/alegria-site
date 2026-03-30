use primitives::hash::content_hash_v1;
use runtime_models::RuleRoleType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TripleBuilderInput {
    pub section_id: String,
    pub procedural_rules: Vec<ProceduralRuleForTriple>,
    pub operational_entities: Vec<OperationalEntityForTriple>,
    pub editorial_topics: Vec<EditorialTopicForTriple>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProceduralRuleForTriple {
    pub rule_key: String,
    pub role_type: RuleRoleType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalEntityForTriple {
    pub entity_kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorialTopicForTriple {
    pub topic_type: String,
    pub topic_key_candidate: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltTriple {
    pub triple_id: String,
    pub subject_key: String,
    pub relation_type: String,
    pub object_key: String,
    pub layer: String,
    pub evidence_section_id: String,
    pub confidence: f32,
    pub numeric_tokens: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TripleBuilderOutput {
    pub triples: Vec<BuiltTriple>,
}

fn triple_id(subject: &str, rel: &str, object: &str) -> String {
    content_hash_v1(format!("{subject}|{rel}|{object}").as_str())
}

pub fn execute(input: &TripleBuilderInput) -> TripleBuilderOutput {
    let mut triples = Vec::new();
    for r in &input.procedural_rules {
        let subject = format!("ri:{}", r.rule_key);
        let object = format!("concept:{}", r.rule_key);
        triples.push(BuiltTriple {
            triple_id: triple_id(&subject, "ABOUT", &object),
            subject_key: subject,
            relation_type: "ABOUT".to_string(),
            object_key: object,
            layer: "procedural".to_string(),
            evidence_section_id: input.section_id.clone(),
            confidence: 0.9,
            numeric_tokens: Vec::new(),
        });
    }
    for o in &input.operational_entities {
        let subject = format!("office:{}", o.entity_kind);
        let object = format!("op:{}", o.value);
        triples.push(BuiltTriple {
            triple_id: triple_id(&subject, "HAS_NOTICE", &object),
            subject_key: subject,
            relation_type: "HAS_NOTICE".to_string(),
            object_key: object,
            layer: "operational".to_string(),
            evidence_section_id: input.section_id.clone(),
            confidence: 0.85,
            numeric_tokens: Vec::new(),
        });
    }
    for e in &input.editorial_topics {
        let subject = format!("section:{}", input.section_id);
        let object = format!("topic:{}", e.topic_key_candidate);
        triples.push(BuiltTriple {
            triple_id: triple_id(&subject, "MENTIONS_TOPIC", &object),
            subject_key: subject.clone(),
            relation_type: "MENTIONS_TOPIC".to_string(),
            object_key: object.clone(),
            layer: "editorial".to_string(),
            evidence_section_id: input.section_id.clone(),
            confidence: 0.82,
            numeric_tokens: Vec::new(),
        });
    }
    TripleBuilderOutput { triples }
}
