//! Deterministic semantic classification of acquired source text.
//!
//! These algorithms identify candidate mentions, lanes, and topics. They do not
//! decide source authority or truth, perform I/O, or depend on activity DTOs.
//! Both the Temporal workflow and the in-process extraction adapter use these
//! same domain types; the runtime payload registry keeps its established IDs.

pub mod editorial_topics;
pub mod entity_spans;
pub mod layer_routing;
pub mod operational_entities;

#[cfg(test)]
mod tests {
    use super::{editorial_topics, entity_spans, layer_routing, operational_entities};

    #[test]
    fn multilingual_factual_section_flows_through_routing_and_extraction() {
        let text = concat!(
            "Паспорт и страховка обязательны. Сбор 80 EUR, срок 15 дней. ",
            "График работы и выходные. Почему отказ?"
        );
        let routed = layer_routing::execute(&layer_routing::LayerRouterInput {
            section_id: "section-visa".into(),
            heading_text: "Документы, паспорт и сбор".into(),
            raw_text: text.into(),
            source_tier: "official".into(),
            block_type: "instruction".into(),
        });
        assert_eq!(routed.primary_layer, "procedural");
        assert!(routed.confidence >= 0.60);
        assert!(!routed.needs_hitl);
        assert!(routed.hitl_reason.is_none());

        let mentions = entity_spans::execute(&entity_spans::EntitySpanInput {
            section_id: "section-visa".into(),
            raw_text: text.into(),
        });
        assert!(mentions
            .mentions
            .iter()
            .any(|mention| mention.entity_type == "fee" && mention.raw_text == "80 EUR"));
        assert!(mentions
            .mentions
            .iter()
            .any(|mention| mention.entity_type == "timeline" && mention.raw_text == "15 дней"));
        assert!(mentions
            .mentions
            .iter()
            .any(|mention| {
                mention.entity_type == "concept" && mention.raw_text == "паспорт"
            }));

        let operational =
            operational_entities::execute(&operational_entities::OperationalExtractionInput {
                section_id: "section-visa".into(),
                raw_text: text.into(),
            });
        let entity_kinds: Vec<_> = operational
            .entities
            .iter()
            .map(|entity| entity.entity_kind.as_str())
            .collect();
        assert_eq!(entity_kinds, ["office_schedule", "closure_notice"]);

        let editorial = editorial_topics::execute(&editorial_topics::EditorialExtractionInput {
            section_id: "section-visa".into(),
            raw_text: text.into(),
        });
        let topic_types: Vec<_> = editorial
            .topics
            .iter()
            .map(|topic| topic.topic_type.as_str())
            .collect();
        assert_eq!(topic_types, ["faq_topic", "pain_point"]);
    }

    #[test]
    fn undecidable_section_requires_hitl_and_yields_no_candidate_facts() {
        let text = "Footer: contact us for newsletter updates.";
        let routed = layer_routing::execute(&layer_routing::LayerRouterInput {
            section_id: "footer".into(),
            heading_text: "Contact".into(),
            raw_text: text.into(),
            source_tier: "unknown".into(),
            block_type: "other".into(),
        });
        assert_eq!(routed.primary_layer, "procedural");
        assert_eq!(routed.confidence, 0.15);
        assert!(routed.needs_hitl);
        assert_eq!(routed.hitl_reason.as_deref(), Some("low_layer_confidence"));
        assert!(routed.secondary_layers.is_empty());

        assert!(entity_spans::execute(&entity_spans::EntitySpanInput {
            section_id: "footer".into(),
            raw_text: text.into(),
        })
        .mentions
        .is_empty());
        assert!(operational_entities::execute(&operational_entities::OperationalExtractionInput {
            section_id: "footer".into(),
            raw_text: text.into(),
        })
        .entities
        .is_empty());
        assert!(editorial_topics::execute(&editorial_topics::EditorialExtractionInput {
            section_id: "footer".into(),
            raw_text: text.into(),
        })
        .topics
        .is_empty());
    }

    #[test]
    fn high_signal_text_retains_ranked_secondary_lanes() {
        let routed = layer_routing::execute(&layer_routing::LayerRouterInput {
            section_id: "mixed".into(),
            heading_text: "документ паспорт сбор fee eur must".into(),
            raw_text: concat!(
                "schedule график выходные holiday ",
                "faq как почему совет ошибка pain"
            )
            .into(),
            source_tier: "ключев seo serp ранжир интент".into(),
            block_type: concat!(
                "услуга стоимость услуги под ключ ",
                "консультация заказать"
            )
            .into(),
        });
        assert_eq!(routed.primary_layer, "procedural");
        assert!(routed.confidence > 0.80);
        assert_eq!(routed.secondary_layers.len(), 4);
        for pair in routed.secondary_layers.windows(2) {
            assert!(pair[0].confidence >= pair[1].confidence);
        }
        assert!(routed.secondary_layers.iter().all(|lane| lane.confidence >= 0.50));
    }
}
