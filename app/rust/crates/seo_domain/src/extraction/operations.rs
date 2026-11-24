//! Real, pure semantic extraction operations exposed to a composable runner.
//!
//! These contracts describe existing domain algorithms. Their implementations
//! remain here, independent of Temporal DTOs, SQLx and provider adapters.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use operation_protocol::{
    ArtifactSchema, IdempotencyPolicy, Operation, OperationContract, OperationId,
    OperationOwnerId, OperationRef, PortId,
};

use super::{editorial_topics, entity_spans, layer_routing, operational_entities};
use crate::canonical_mapping;

/// A local, side-effect-free implementation with a real Rust input and output.
/// A future effectful operation will use an owner-specific adapter, not this
/// trait or an I/O call hidden inside a domain function.
pub trait PureExtractionOperation: Operation<Failure = Infallible> {
    fn evaluate(input: &Self::Input) -> Self::Output;
}

fn pure_contract(id: &str, input_schema: &str, output_schema: &str) -> OperationContract {
    OperationContract {
        reference: OperationRef {
            id: OperationId::from(id),
            revision: 1,
        },
        owner: OperationOwnerId::from("semantic_extraction"),
        inputs: BTreeMap::from([(
            PortId::from("input"),
            ArtifactSchema {
                id: input_schema.into(),
                revision: 1,
            },
        )]),
        outputs: BTreeMap::from([(
            PortId::from("output"),
            ArtifactSchema {
                id: output_schema.into(),
                revision: 1,
            },
        )]),
        effects: BTreeSet::new(),
        idempotency: IdempotencyPolicy::Pure,
    }
}

pub struct LayerRoute;

impl Operation for LayerRoute {
    type Input = layer_routing::LayerRouterInput;
    type Output = layer_routing::LayerRouterOutput;
    type Failure = Infallible;

    fn contract() -> OperationContract {
        pure_contract(
            "semantic.layer_route",
            "semantic.layer_router_input",
            "semantic.layer_router_output",
        )
    }
}

impl PureExtractionOperation for LayerRoute {
    fn evaluate(input: &Self::Input) -> Self::Output {
        layer_routing::execute(input)
    }
}

pub struct EntitySpans;

impl Operation for EntitySpans {
    type Input = entity_spans::EntitySpanInput;
    type Output = entity_spans::EntitySpanOutput;
    type Failure = Infallible;

    fn contract() -> OperationContract {
        pure_contract(
            "semantic.entity_spans",
            "semantic.entity_span_input",
            "semantic.entity_span_output",
        )
    }
}

impl PureExtractionOperation for EntitySpans {
    fn evaluate(input: &Self::Input) -> Self::Output {
        entity_spans::execute(input)
    }
}

/// A real downstream operation consuming an earlier node's typed entity-span
/// artifact. Its pure alias/lexicon decision precedes the separate Qdrant
/// retrieval capability; a vector suggestion never certifies Truth itself.
pub struct CanonicalizeMentions;

impl Operation for CanonicalizeMentions {
    type Input = entity_spans::EntitySpanOutput;
    type Output = canonical_mapping::CanonicalMappingOutput;
    type Failure = Infallible;

    fn contract() -> OperationContract {
        pure_contract(
            "semantic.canonicalize_mentions",
            "semantic.entity_span_output",
            "semantic.canonical_mapping_output",
        )
    }
}

impl PureExtractionOperation for CanonicalizeMentions {
    fn evaluate(input: &Self::Input) -> Self::Output {
        let mentions = input
            .mentions
            .iter()
            .map(|mention| canonical_mapping::MentionForMapping {
                raw_text: mention.raw_text.clone(),
                entity_type: mention.entity_type.clone(),
            })
            .collect::<Vec<_>>();
        canonical_mapping::map_mentions(&mentions)
    }
}

pub struct EditorialTopics;

impl Operation for EditorialTopics {
    type Input = editorial_topics::EditorialExtractionInput;
    type Output = editorial_topics::EditorialExtractionOutput;
    type Failure = Infallible;

    fn contract() -> OperationContract {
        pure_contract(
            "semantic.editorial_topics",
            "semantic.editorial_extraction_input",
            "semantic.editorial_extraction_output",
        )
    }
}

impl PureExtractionOperation for EditorialTopics {
    fn evaluate(input: &Self::Input) -> Self::Output {
        editorial_topics::execute(input)
    }
}

/// A fourth existing production operation proves extension through its owner
/// and the runner's registration only. Neither DAG kernel nor existing
/// operation implementations need to change.
pub struct OperationalEntities;

impl Operation for OperationalEntities {
    type Input = operational_entities::OperationalExtractionInput;
    type Output = operational_entities::OperationalExtractionOutput;
    type Failure = Infallible;

    fn contract() -> OperationContract {
        pure_contract(
            "semantic.operational_entities",
            "semantic.operational_extraction_input",
            "semantic.operational_extraction_output",
        )
    }
}

impl PureExtractionOperation for OperationalEntities {
    fn evaluate(input: &Self::Input) -> Self::Output {
        operational_entities::execute(input)
    }
}
