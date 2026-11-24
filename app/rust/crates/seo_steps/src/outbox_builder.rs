/// Типизированные строители outbox-событий.
///
/// Внутри seo_steps не используются ad hoc JSON runtime-contracts.
/// Runtime wire для outbox строится строго через protobuf payload bytes.
use contracts::generated::alegria::sync::v1::{
    Neo4jRuleUpsertPayload, QdrantDeleteCommand, QdrantEntityPayload, QdrantUpsertCommand,
    RuleRoleTypeV1,
};
use primitives::hash::{blake3_hex, content_hash_v1};
use primitives::qdrant_point_id::qdrant_point_id_v1;
use prost::Message;
use runtime_models::RuleRoleType;

#[derive(Debug, Clone)]
pub struct OutboxEnvelope {
    pub aggregate_type: String,
    pub aggregate_key: String,
    pub target_system: String,
    pub event_type: String,
    pub payload_type: String,
    pub schema_version: i32,
    pub idempotency_key: String,
    pub payload_bytes: Vec<u8>,
}

impl OutboxEnvelope {
    pub fn payload_bytes(&self) -> &[u8] {
        &self.payload_bytes
    }
}

fn encode_payload<T: Message>(value: &T) -> Vec<u8> {
    value.encode_to_vec()
}

fn make_idempotency_key(aggregate_key: &str, event_type: &str, payload_bytes: &[u8]) -> String {
    let payload_hash = blake3_hex(payload_bytes);
    content_hash_v1(&format!("{aggregate_key}|{event_type}|{payload_hash}"))
}

fn sync_role_type(role_type: RuleRoleType) -> i32 {
    match role_type {
        RuleRoleType::MustProvide => RuleRoleTypeV1::MustProvide as i32,
        RuleRoleType::MustPay => RuleRoleTypeV1::MustPay as i32,
        RuleRoleType::MustSatisfy => RuleRoleTypeV1::MustSatisfy as i32,
        RuleRoleType::Allows => RuleRoleTypeV1::Allows as i32,
        RuleRoleType::Forbids => RuleRoleTypeV1::Forbids as i32,
        RuleRoleType::Timeline => RuleRoleTypeV1::Timeline as i32,
        RuleRoleType::DocumentRequired => RuleRoleTypeV1::DocumentRequired as i32,
        RuleRoleType::EligibilityRule => RuleRoleTypeV1::EligibilityRule as i32,
        RuleRoleType::FeeItem => RuleRoleTypeV1::FeeItem as i32,
        RuleRoleType::TimelineItem => RuleRoleTypeV1::TimelineItem as i32,
        RuleRoleType::WhereToApply => RuleRoleTypeV1::WhereToApply as i32,
        RuleRoleType::AppointmentRule => RuleRoleTypeV1::AppointmentRule as i32,
        RuleRoleType::FormRequired => RuleRoleTypeV1::FormRequired as i32,
        RuleRoleType::Step => RuleRoleTypeV1::Step as i32,
    }
}

pub fn neo4j_rule_upserted(rule_instance_id: &str, context_key: &str) -> OutboxEnvelope {
    let payload_bytes = encode_payload(&Neo4jRuleUpsertPayload {
        rule_instance_id: rule_instance_id.to_string(),
        context_key: context_key.to_string(),
    });
    OutboxEnvelope {
        aggregate_type: "rule_instance".to_string(),
        aggregate_key: rule_instance_id.to_string(),
        target_system: "neo4j".to_string(),
        event_type: "RuleInstanceUpserted".to_string(),
        payload_type: "alegria.outbox.neo4j_rule_upserted.v1".to_string(),
        schema_version: 1,
        idempotency_key: make_idempotency_key(
            rule_instance_id,
            "RuleInstanceUpserted",
            &payload_bytes,
        ),
        payload_bytes,
    }
}

pub fn qdrant_rule_upsert(
    run_id: &str,
    rule_instance_id: &str,
    context_key: &str,
    rule_type_key: &str,
    concept_key: &str,
    role_type: &str,
    source_key: &str,
    collection_name: &str,
    vector: Vec<f64>,
) -> OutboxEnvelope {
    let vector_size = vector.len() as u32;
    let payload_bytes = encode_payload(&QdrantUpsertCommand {
        event_id: String::new(),
        collection_name: collection_name.to_string(),
        entity_type: "rule_instance".to_string(),
        entity_key: rule_instance_id.to_string(),
        point_id: qdrant_point_id_v1(collection_name, "rule_instance", rule_instance_id),
        vector: vector.into_iter().map(|v| v as f32).collect(),
        payload: Some(QdrantEntityPayload {
            rule_instance_id: rule_instance_id.to_string(),
            context_key: context_key.to_string(),
            rule_type_key: rule_type_key.to_string(),
            concept_key: concept_key.to_string(),
            role_type: sync_role_type(RuleRoleType::parse(role_type).unwrap_or_default()),
            source_key: source_key.to_string(),
        }),
        distance: "cosine".to_string(),
        vector_size,
        metadata: Default::default(),
    });
    OutboxEnvelope {
        aggregate_type: "rule_instance".to_string(),
        aggregate_key: rule_instance_id.to_string(),
        target_system: "qdrant".to_string(),
        event_type: "QdrantUpsertCommand".to_string(),
        payload_type: "alegria.outbox.qdrant_upsert_command.v1".to_string(),
        schema_version: 1,
        idempotency_key: make_idempotency_key(
            &format!("{run_id}|{rule_instance_id}"),
            "QdrantUpsertCommand",
            &payload_bytes,
        ),
        payload_bytes,
    }
}

/// A repeated demotion within the same run has one effect, but a later
/// re-verification followed by another demotion must enqueue a new tombstone.
pub fn qdrant_rule_delete(
    run_id: &str,
    rule_instance_id: &str,
    collection_name: &str,
) -> OutboxEnvelope {
    let payload_bytes = encode_payload(&QdrantDeleteCommand {
        collection_name: collection_name.to_string(),
        entity_type: "rule_instance".to_string(),
        entity_key: rule_instance_id.to_string(),
        point_id: qdrant_point_id_v1(collection_name, "rule_instance", rule_instance_id),
    });
    OutboxEnvelope {
        aggregate_type: "rule_instance".to_string(),
        aggregate_key: rule_instance_id.to_string(),
        target_system: "qdrant".to_string(),
        event_type: "QdrantDeleteCommand".to_string(),
        payload_type: "alegria.outbox.qdrant_delete_command.v1".to_string(),
        schema_version: 1,
        idempotency_key: make_idempotency_key(
            &format!("{run_id}|{rule_instance_id}"),
            "QdrantDeleteCommand",
            &payload_bytes,
        ),
        payload_bytes,
    }
}

#[cfg(test)]
mod projection_lifecycle_tests {
    use super::*;

    #[test]
    fn tombstones_are_idempotent_within_run_but_not_across_reverify_cycles() {
        let first = qdrant_rule_delete("run-1", "rule-1", "verified_rules_4");
        let replay = qdrant_rule_delete("run-1", "rule-1", "verified_rules_4");
        let later = qdrant_rule_delete("run-2", "rule-1", "verified_rules_4");
        assert_eq!(first.idempotency_key, replay.idempotency_key);
        assert_ne!(first.idempotency_key, later.idempotency_key);
        assert_eq!(first.event_type, "QdrantDeleteCommand");
        let command = QdrantDeleteCommand::decode(first.payload_bytes()).unwrap();
        assert_eq!(command.entity_key, "rule-1");
        assert_eq!(command.collection_name, "verified_rules_4");
    }

    #[test]
    fn reverified_rule_reenqueues_the_same_embedding_for_a_new_run() {
        fn event(run: &str) -> OutboxEnvelope {
            qdrant_rule_upsert(
                run,
                "rule-1",
                "ctx",
                "fee",
                "fee",
                "must_pay",
                "source-1",
                "verified_rules_4",
                vec![0.1, 0.2],
            )
        }
        assert_eq!(
            event("run-1").idempotency_key,
            event("run-1").idempotency_key
        );
        assert_ne!(
            event("run-1").idempotency_key,
            event("run-2").idempotency_key
        );
    }
}
