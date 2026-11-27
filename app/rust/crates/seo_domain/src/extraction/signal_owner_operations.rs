//! Two separately callable pure EX-07 owner contracts.
//!
//! These contract declarations are deliberately owner-only. They are not
//! registered into the historical five-operation linked runner, nor do they
//! grant provider effects or Temporal Child deployment readiness.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use operation_protocol::{
    ArtifactSchema, IdempotencyPolicy, Operation, OperationContract, OperationId, OperationOwnerId,
    OperationRef, PortId,
};

use super::{commercial_signals, operations::PureExtractionOperation, seo_signals};

fn pure_signal_contract(
    id: &str,
    owner: &str,
    input_artifact: &str,
    output_artifact: &str,
) -> OperationContract {
    OperationContract {
        reference: OperationRef {
            id: OperationId::from(id),
            revision: 1,
        },
        owner: OperationOwnerId::from(owner),
        inputs: BTreeMap::from([(
            PortId::from("input"),
            ArtifactSchema {
                id: input_artifact.into(),
                revision: 1,
            },
        )]),
        outputs: BTreeMap::from([(
            PortId::from("output"),
            ArtifactSchema {
                id: output_artifact.into(),
                revision: 1,
            },
        )]),
        invalidates: BTreeSet::new(),
        effects: BTreeSet::new(),
        idempotency: IdempotencyPolicy::Pure,
    }
}

/// SEO ranking/keyword signals; intentionally distinct from commercial policy.
pub struct ExtractSeoSignals;

impl Operation for ExtractSeoSignals {
    type Input = seo_signals::SeoSignalInput;
    type Output = seo_signals::SeoSignalOutput;
    type Failure = Infallible;

    fn contract() -> OperationContract {
        pure_signal_contract(
            "semantic.seo_signals",
            "seo_signal_extraction",
            "semantic.seo_signal_input",
            "semantic.seo_signal_output",
        )
    }
}

impl PureExtractionOperation for ExtractSeoSignals {
    fn evaluate(input: &Self::Input) -> Self::Output {
        seo_signals::execute(input)
    }
}

/// Commercial service/price signals; separate input/output schema and owner.
pub struct ExtractCommercialSignals;

impl Operation for ExtractCommercialSignals {
    type Input = commercial_signals::CommercialSignalInput;
    type Output = commercial_signals::CommercialSignalOutput;
    type Failure = Infallible;

    fn contract() -> OperationContract {
        pure_signal_contract(
            "semantic.commercial_signals",
            "commercial_signal_extraction",
            "semantic.commercial_signal_input",
            "semantic.commercial_signal_output",
        )
    }
}

impl PureExtractionOperation for ExtractCommercialSignals {
    fn evaluate(input: &Self::Input) -> Self::Output {
        commercial_signals::execute(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_infallible<O: Operation<Failure = Infallible>>() {}

    #[test]
    fn signal_contracts_are_distinct_stable_pure_owner_only() {
        assert_infallible::<ExtractSeoSignals>();
        assert_infallible::<ExtractCommercialSignals>();
        let seo = ExtractSeoSignals::contract();
        let commercial = ExtractCommercialSignals::contract();
        assert_eq!(seo.reference.id, OperationId::from("semantic.seo_signals"));
        assert_eq!(
            commercial.reference.id,
            OperationId::from("semantic.commercial_signals")
        );
        assert_eq!(seo.reference.revision, 1);
        assert_eq!(commercial.reference.revision, 1);
        assert_ne!(seo.owner, commercial.owner);
        assert_ne!(
            seo.fingerprint().unwrap(),
            commercial.fingerprint().unwrap()
        );
        assert_eq!(
            seo.fingerprint().unwrap(),
            ExtractSeoSignals::contract().fingerprint().unwrap()
        );
        assert_eq!(
            commercial.fingerprint().unwrap(),
            ExtractCommercialSignals::contract().fingerprint().unwrap()
        );
        for contract in [seo, commercial] {
            assert_eq!(contract.idempotency, IdempotencyPolicy::Pure);
            assert!(contract.effects.is_empty());
            assert!(contract.invalidates.is_empty());
            assert_eq!(contract.inputs.len(), 1);
            assert_eq!(contract.outputs.len(), 1);
            assert!(contract.inputs.contains_key(&PortId::from("input")));
            assert!(contract.outputs.contains_key(&PortId::from("output")));
        }
    }

    #[test]
    fn domain_algorithms_do_not_cross_assign_signal_types() {
        let seo = ExtractSeoSignals::evaluate(&seo_signals::SeoSignalInput {
            section_id: "one".into(),
            raw_text: "SEO serp keywords".into(),
        });
        let commercial =
            ExtractCommercialSignals::evaluate(&commercial_signals::CommercialSignalInput {
                section_id: "two".into(),
                raw_text: "Service price".into(),
            });
        assert!(seo
            .signals
            .iter()
            .all(|signal| signal.signal_type.ends_with("signal")));
        assert!(commercial
            .signals
            .iter()
            .all(|signal| signal.signal_type == "service_offer"
                || signal.signal_type == "commercial_price"));
    }
}
