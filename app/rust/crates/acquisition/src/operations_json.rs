//! Tests for the explicitly versioned acquisition wire boundary.
use super::*;
use crate::source_relevance::PageMode;
use operation_protocol::ContractCatalog;

fn execute_one_typed<O: PureAcquisitionOperation>(wire: &str) -> O::Output {
    // Standalone owner-level typed evaluation. The external Site Lab
    // runtime does not currently register acquisition operations.
    let input: O::Input = serde_json::from_str(wire).unwrap();
    let result = O::evaluate(&input);
    serde_json::from_slice(&serde_json::to_vec(&result).unwrap()).unwrap()
}

#[test]
fn acquisition_contracts_are_distinct_pure_owner_operations() {
    let mut catalog = ContractCatalog::default();
    catalog.register::<ClassifySourcePage>().unwrap();
    catalog.register::<EvaluateDomBlocks>().unwrap();
    let class = ClassifySourcePage::contract();
    let blocks = EvaluateDomBlocks::contract();
    assert_eq!(class.owner.as_str(), "acquisition");
    assert_eq!(blocks.owner.as_str(), "acquisition");
    assert_ne!(class.reference, blocks.reference);
    assert_ne!(class.inputs, blocks.inputs);
    for contract in [&class, &blocks] {
        assert!(contract.effects.is_empty());
        assert!(contract.invalidates.is_empty());
        assert_eq!(contract.idempotency, IdempotencyPolicy::Pure);
        assert_eq!(contract.reference.revision, 1);
        assert_eq!(contract.inputs.len(), 1);
        assert_eq!(contract.outputs.len(), 1);
        assert!(catalog.contract(&contract.reference).is_some());
    }
}

#[test]
fn page_classifier_runs_independently_with_exact_typed_input() {
    let result = execute_one_typed::<ClassifySourcePage>(
        r#"{"url":"https://example.org/menu","title":"Menu","raw_text":"Visa fee EUR 25"}"#,
    );
    assert_eq!(result.page_mode, PageMode::MenuPage);
    assert!(!result.allow_editorial_extraction);
    assert!(!result.allow_structural_extraction);

    assert!(serde_json::from_str::<ClassifySourcePageInput>(
        r#"{"url":"https://example.org","title":"Title","raw_text":"x","ignored_setting":true}"#,
    )
    .is_err());

    // Historical Temporal payloads stay permissive: strictness is added only
    // to the new ACQ owner contract, avoiding frozen V1 replay breakage.
    assert!(serde_json::from_str::<PageUtilityClassifierInput>(
        r#"{"url":"https://example.org","title":"Title","raw_text":"x","old_activity_field":true}"#,
    )
    .is_ok());
}

#[test]
fn block_relevance_runs_independently_and_rejects_unknown_parameters() {
    let result = execute_one_typed::<EvaluateDomBlocks>(
        r#"{"blocks":[{"dom_block_id":"main","block_role":"content_main","text":"Visa fee EUR 25"},{"dom_block_id":"nav","block_role":"navigation","text":"Visa fee EUR 25"}]}"#,
    );
    assert_eq!(result.blocks.len(), 2);
    assert!(result.blocks[0].allow_extraction);
    assert!(!result.blocks[1].allow_extraction);
    assert!(result.blocks[0].content_relevance_score > result.blocks[1].content_relevance_score);
    assert_eq!(result.blocks[1].block_role, BlockRole::Navigation);

    assert!(serde_json::from_str::<DomBlockRelevanceInput>(
        r#"{"blocks":[{"dom_block_id":"main","block_role":"content_main","text":"text","undocumented":1}]}"#,
    )
    .is_err());
    assert!(serde_json::from_str::<DomBlockRelevanceInput>(
        r#"{"blocks":[],"tenant_id":"forged"}"#,
    )
    .is_err());
    assert!(serde_json::from_str::<DomBlockInput>(
        r#"{"dom_block_id":"main","block_role":"content_main","text":"Visa", "old_activity_field":true}"#,
    )
    .is_ok());
}
