use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    ArtifactSchema, ArtifactTypeId, EffectId, IdempotencyPolicy, Operation, OperationContract,
    OperationId, OperationOwnerId, OperationRef, PortId,
};
use seo_domain::extraction::operations::LayerRoute;
use serde_json::json;

use super::*;
use crate::operation_adapter::semantic_extraction_registry;
use crate::site_lab::{installed_semantic_lab_catalog, OperationGateV1};

fn registry() -> SiteLabCatalog {
    installed_semantic_lab_catalog(
        semantic_extraction_registry()
            .expect("semantic catalog must be installed")
            .catalog()
            .clone(),
    )
    .expect("trusted semantic parameters")
}

fn remote_operation() -> OperationRef {
    OperationRef {
        id: OperationId::from("future.search_demand"),
        revision: 1,
    }
}

fn register_remote_contract(catalog: &mut SiteLabCatalog) {
    catalog
        .register_contract(OperationContract {
            reference: remote_operation(),
            owner: OperationOwnerId::from("future_provider"),
            inputs: BTreeMap::from([(
                PortId::from("input"),
                ArtifactSchema {
                    id: ArtifactTypeId::from("research.query"),
                    revision: 1,
                },
            )]),
            outputs: BTreeMap::from([(
                PortId::from("output"),
                ArtifactSchema {
                    id: ArtifactTypeId::from("research.results"),
                    revision: 1,
                },
            )]),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::from([EffectId::from("provider.read")]),
            idempotency: IdempotencyPolicy::AtMostOnce,
        })
        .expect("trusted independent remote contract");
}

#[test]
fn individual_operation_inspection_returns_exact_installed_metadata_without_executing() {
    let catalog = registry();
    let operation = LayerRoute::contract().reference;
    let parameters = RecipeParameters::from(json!({}));
    let report = inspect_operation(&catalog, &operation, Some(&parameters)).unwrap();
    assert_eq!(report.tool.operation, operation);
    assert_eq!(report.tool.owner, "semantic_extraction");
    assert!(report.tool.contract_hash.len() > 16);
    assert_eq!(report.status, "registered_operation_inspection_only");
    assert_eq!(report.input_bindings, "not_checked");
    assert_eq!(report.owner_validate_only, "required_not_run");
    assert_eq!(report.operator_admission, "not_issued_by_inspection");
    assert_eq!(report.executable_owner, "not_verified");
    assert_eq!(report.provider_connectivity, "not_checked");
    assert_eq!(report.execution, "not_started");
    assert_eq!(report.execution_readiness, report.tool.execution_readiness);
    assert_eq!(report.execution_readiness.schema_revision, 1);
    assert_eq!(report.execution_readiness.status, "catalog_preview_only");
    assert_eq!(
        report.execution_readiness.parameter_contract,
        OperationGateV1::DefinitionInstalled
    );
    assert_eq!(
        report.execution_readiness.typed_output_contract,
        OperationGateV1::DefinitionInstalled
    );
    assert_eq!(
        report.execution_readiness.owner_validate_only_history,
        OperationGateV1::NotVerified
    );
    assert_eq!(
        report.execution_readiness.authenticated_principal_tenant_acl,
        OperationGateV1::NotVerified
    );
    assert_eq!(
        report.execution_readiness.bounded_one_use_request_grant,
        OperationGateV1::NotVerified
    );
    assert_eq!(
        report.execution_readiness.temporal_start_and_server_history,
        OperationGateV1::NotVerified
    );
    assert_eq!(
        report.execution_readiness.provider_usage_and_budget_receipt,
        OperationGateV1::NotApplicable
    );
    assert!(!report.execution_readiness.user_start_verified);
    assert!(!report.execution_readiness.group_execution_verified);
    let checked = report.checked_parameters.unwrap();
    assert_eq!(checked.status, "typed_configuration_checked");
    assert_eq!(checked.verification, "typed_local");
    assert_eq!(checked.parameter_hash.len(), 64);
}

#[test]
fn local_dto_rejects_unrecognized_configuration_and_does_not_echo_values() {
    let catalog = registry();
    let operation = LayerRoute::contract().reference;
    let secret = "test-only-secret-must-not-be-echoed";
    let parameters = RecipeParameters::from(json!({"api_key": secret}));
    let error = inspect_operation(&catalog, &operation, Some(&parameters))
        .err()
        .expect("unrecognized DTO values must be rejected");
    let problem = error.problem();
    assert_eq!(problem.code, "operation_parameters_invalid");
    assert!(!problem.message.contains(secret));
    assert!(!problem.next_action.contains(secret));
}

#[test]
fn unknown_operations_and_missing_owner_schema_fail_closed() {
    let mut catalog = registry();
    let unknown = remote_operation();
    assert_eq!(
        inspect_operation(&catalog, &unknown, None)
            .err()
            .expect("unregistered operation must be rejected")
            .problem()
            .code,
        "operation_unavailable"
    );
    register_remote_contract(&mut catalog);
    let report = inspect_operation(&catalog, &unknown, None).unwrap();
    assert!(report.tool.parameters.is_none());
    assert_eq!(report.tool.readiness, "missing_parameter_schema");
    assert_eq!(report.executable_owner, "not_verified");
    assert_eq!(
        report.execution_readiness.parameter_contract,
        OperationGateV1::DefinitionMissing
    );
    assert_eq!(
        report.execution_readiness.provider_usage_and_budget_receipt,
        OperationGateV1::NotVerified
    );
    assert!(!report.execution_readiness.user_start_verified);
    assert_eq!(
        inspect_operation(&catalog, &unknown, Some(&RecipeParameters::default()))
            .err()
            .expect("parameter schema required")
            .problem()
            .code,
        "operation_parameter_schema_missing"
    );
}

#[test]
fn remote_owner_settings_need_validate_only_and_new_settings_change_digest() {
    let mut catalog = registry();
    register_remote_contract(&mut catalog);
    let operation = remote_operation();
    let fingerprint = catalog
        .catalog()
        .contract(&operation)
        .unwrap()
        .fingerprint()
        .unwrap();
    catalog
        .register_remote(
            &operation,
            &fingerprint,
            json!({"type":"object","properties":{"region":{"type":"string"}}}),
        )
        .unwrap();
    let east = RecipeParameters::from(json!({"region": "east"}));
    let west = RecipeParameters::from(json!({"region": "west"}));
    let first = inspect_operation(&catalog, &operation, Some(&east)).unwrap();
    let second = inspect_operation(&catalog, &operation, Some(&west)).unwrap();
    assert_eq!(first.tool.readiness, "requires_owner_admission");
    assert_eq!(first.owner_validate_only, "required_not_run");
    assert_eq!(first.operator_admission, "not_issued_by_inspection");
    assert_eq!(first.executable_owner, "not_verified");
    assert_eq!(
        first.execution_readiness.parameter_contract,
        OperationGateV1::IndependentOwnerValidationRequired
    );
    assert_eq!(
        first.execution_readiness.provider_usage_and_budget_receipt,
        OperationGateV1::NotVerified
    );
    assert_eq!(
        first.execution_readiness.typed_failure_wire,
        OperationGateV1::NotVerified
    );
    assert!(!first.execution_readiness.user_start_verified);
    let visible = serde_json::to_string(&first).unwrap();
    assert!(!visible.contains("east"));
    assert!(!visible.contains("west"));
    assert_eq!(
        first.checked_parameters.as_ref().unwrap().status,
        "schema_shape_checked_only"
    );
    assert_eq!(
        first.checked_parameters.as_ref().unwrap().verification,
        "requires_owner_validate_only"
    );
    assert_ne!(
        first.checked_parameters.unwrap().parameter_hash,
        second.checked_parameters.unwrap().parameter_hash
    );
    let invalid = RecipeParameters::from(json!(["array"]));
    assert_eq!(
        inspect_operation(&catalog, &operation, Some(&invalid))
            .err()
            .expect("non-object parameters must be rejected")
            .problem()
            .code,
        "operation_parameters_invalid"
    );
}

#[test]
fn every_installed_catalog_operation_stays_nonexecuting_without_independent_evidence() {
    let catalog = registry();
    let tools = describe_tools(&catalog).unwrap();
    assert!(!tools.is_empty());
    for tool in tools {
        let readiness = &tool.execution_readiness;
        assert_eq!(readiness.status, "catalog_preview_only");
        assert_eq!(
            readiness.installed_runtime_owner_route,
            OperationGateV1::NotVerified
        );
        assert_eq!(
            readiness.bounded_one_use_request_grant,
            OperationGateV1::NotVerified
        );
        assert!(!readiness.user_start_verified);
        assert!(!readiness.group_execution_verified);
    }
}
