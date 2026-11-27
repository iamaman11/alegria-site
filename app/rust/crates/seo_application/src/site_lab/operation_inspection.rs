//! Individual operation inspection through exactly the same trusted catalog
//! used by multi-step Site Lab recipes. This cannot start an Activity, grant
//! operator admission or prove that provider credentials are connected.

use operation_protocol::OperationRef;
use serde::Serialize;

use super::{
    describe_tools, RecipeParameters, SiteLabCatalog, SiteLabError, ToolDescription,
};

/// A digest of the installed owner's typed parameter check, never a copy of
/// the author's provider settings or a claim that ValidateOnly has run.
#[derive(Debug, Serialize)]
pub struct CheckedOperationParameters {
    pub status: &'static str,
    pub parameter_hash: String,
    pub verification: &'static str,
}

/// One operation, separately inspectable before it is connected to a recipe.
/// Existing `ToolDescription` is the sole catalog metadata representation;
/// this report adds explicit current checks without inferring runtime access.
#[derive(Debug, Serialize)]
pub struct OperationInspection {
    pub status: &'static str,
    pub tool: ToolDescription,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_parameters: Option<CheckedOperationParameters>,
    pub input_bindings: &'static str,
    pub owner_validate_only: &'static str,
    pub operator_admission: &'static str,
    pub executable_owner: &'static str,
    pub provider_connectivity: &'static str,
    pub execution: &'static str,
    pub next_action: &'static str,
}

/// Discover a single installed operation and, optionally, validate its exact
/// configuration against the installed owner's local DTO or trusted schema.
/// Remote schemas only prove JSON-object shape: owner ValidateOnly must check
/// all advanced settings before any external effect can be authorized.
pub fn inspect_operation(
    catalog: &SiteLabCatalog,
    operation: &OperationRef,
    parameters: Option<&RecipeParameters>,
) -> Result<OperationInspection, SiteLabError> {
    let tool = describe_tools(catalog)?
        .into_iter()
        .find(|tool| &tool.operation == operation)
        .ok_or_else(|| SiteLabError::UnavailableOperation(operation.clone()))?;

    let checked_parameters = parameters
        .map(|parameters| {
            let checked = catalog.validate(operation, parameters)?;
            Ok::<CheckedOperationParameters, SiteLabError>(CheckedOperationParameters {
                status: if checked.verification == "typed_local" {
                    "typed_configuration_checked"
                } else {
                    "schema_shape_checked_only"
                },
                parameter_hash: checked.digest,
                verification: checked.verification,
            })
        })
        .transpose()?;

    let missing_schema = tool.parameters.is_none();
    let needs_owner_validation = tool
        .parameters
        .as_ref()
        .is_some_and(|parameters| parameters.verification == "requires_owner_validate_only");
    let has_effects = !tool.effects.is_empty();

    Ok(OperationInspection {
        status: "registered_operation_inspection_only",
        tool,
        checked_parameters,
        input_bindings: "not_checked",
        // Even a pure local DTO check is not the native V2 ValidateOnly Child.
        // Every eventual V2 execution schedules that child independently.
        owner_validate_only: "required_not_run",
        operator_admission: "not_issued_by_inspection",
        executable_owner: "not_verified",
        provider_connectivity: "not_checked",
        execution: "not_started",
        next_action: if missing_schema {
            "Install this owner's trusted parameter schema before configuring or executing it."
        } else if needs_owner_validation {
            "Validate parameters with the owner's real ValidateOnly Child, bind the exact typed inputs, then obtain independent admission before execution."
        } else if has_effects {
            "Bind inputs, check a one-step V2 recipe, and obtain independent owner admission before execution."
        } else {
            "Bind the exact input artifacts and check a one-step recipe with the existing V2 DAG validator."
        },
    })
}

#[cfg(test)]
mod tests;
