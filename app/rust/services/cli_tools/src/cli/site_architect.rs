use std::io::Read;

use seo_application::operation_adapter::{
    semantic_extraction_owner_schemas, semantic_extraction_registry,
};
use seo_application::site_architect::{
    compare_executions, describe_tools, diff_recipes, inspect_execution, inspect_operation,
    installed_semantic_operation_catalog, preview_recipe, ExecutionPlan, OperationDagReceiptV1,
    OperationRef, RecipeParameters, SiteArchitectCatalog, SiteRecipe,
};

const SITE_ARCHITECT_SAVED_MAX_BYTES: u64 = 512 * 1024;

const SITE_ARCHITECT_RECIPE_MAX_BYTES: u64 = 256 * 1024;

fn site_architect_problem(code: &str, message: &str, next_action: &str) -> Value {
    json!({
        "status": "invalid",
        "code": code,
        "message": message,
        "next_action": next_action,
    })
}

/// A single bounded stdin payload removes temporary files from ChatGPT/CLI
/// authoring, but never grants a runtime or provider execution capability.
fn site_architect_open_input(path: &str) -> std::io::Result<Box<dyn Read>> {
    if path == "-" {
        Ok(Box::new(std::io::stdin()))
    } else {
        Ok(Box::new(fs::File::open(path)?))
    }
}

fn site_architect_bounded_bytes(reader: impl Read, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn site_architect_reused_stdin(paths: &[&str]) -> bool {
    paths.iter().filter(|path| **path == "-").count() > 1
}

fn site_architect_stdin_conflict() -> Value {
    site_architect_problem(
        "stdin_reused",
        "Only one input may read stdin ('-') in the same siteArchitect command.",
        "Pass one JSON input via '-' and the remaining input files by explicit paths.",
    )
}

fn site_architect_read_recipe(path: &str) -> std::result::Result<SiteRecipe, Value> {
    let reader = site_architect_open_input(path).map_err(|_| {
        site_architect_problem(
            "recipe_file_unavailable",
            "Cannot open the requested recipe input.",
            "Check recipe file access or pass '-' to read a single JSON payload from stdin.",
        )
    })?;
    let bytes = site_architect_bounded_bytes(reader, SITE_ARCHITECT_RECIPE_MAX_BYTES).map_err(|error| {
        site_architect_problem(
            "recipe_read_failed",
            &format!("Cannot read recipe input: {error}"),
            "Make the recipe readable and retry.",
        )
    })?;
    if bytes.len() as u64 > SITE_ARCHITECT_RECIPE_MAX_BYTES {
        return Err(site_architect_problem(
            "recipe_too_large",
            "Recipe exceeds the 256 KiB input limit.",
            "Split the research into smaller, independently checked recipes.",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        site_architect_problem(
            "recipe_invalid_json",
            &error.to_string(),
            "Use the versioned SiteRecipe JSON format; unknown fields are forbidden.",
        )
    })
}

/// Individual settings use the same 256 KiB authoring bound as a recipe.
/// Never include the supplied bytes in the error or diagnostic response.
fn site_architect_read_parameters(path: &str) -> std::result::Result<RecipeParameters, Value> {
    let reader = site_architect_open_input(path).map_err(|_| {
        site_architect_problem(
            "parameters_file_unavailable",
            "Cannot open the requested operation settings input.",
            "Provide a readable JSON object or '-' for bounded stdin.",
        )
    })?;
    let bytes = site_architect_bounded_bytes(reader, SITE_ARCHITECT_RECIPE_MAX_BYTES).map_err(|_| {
        site_architect_problem(
            "parameters_read_failed",
            "Cannot read operation settings.",
            "Check file access and retry.",
        )
    })?;
    if bytes.len() as u64 > SITE_ARCHITECT_RECIPE_MAX_BYTES {
        return Err(site_architect_problem(
            "parameters_too_large",
            "Operation settings exceed the 256 KiB authoring limit.",
            "Reduce the settings payload before validation.",
        ));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        site_architect_problem(
            "parameters_invalid_json",
            "Operation settings must be a JSON object.",
            "Use the exact installed owner's parameter JSON schema.",
        )
    })?;
    if !value.is_object() {
        return Err(site_architect_problem(
            "parameters_not_object",
            "Operation settings must be a JSON object.",
            "Use the exact installed owner's parameter JSON schema.",
        ));
    }
    Ok(value.into())
}

/// Do not reflect provider settings, unknown operation IDs, source locators,
/// or user-controlled error strings into ChatGPT/CLI diagnostics.
fn site_architect_safe_operation_problem(error: &seo_application::site_architect::SiteArchitectError) -> Value {
    let problem = error.problem();
    site_architect_problem(
        problem.code,
        "The requested operation or settings cannot be verified against the installed trusted catalog.",
        problem.next_action,
    )
}

/// Authored step names and source locators are untrusted. Keep typed error
/// codes and the exact offending *ordinary* step identifier, but never echo
/// provider parameters, URLs or arbitrary strings from PlanError::Display.
fn site_architect_safe_recipe_problem(error: &seo_application::site_architect::SiteArchitectError) -> Value {
    let problem = error.problem();
    let mut output = site_architect_problem(
        problem.code,
        "The recipe failed an installed typed operation, parameter or DAG check.",
        problem.next_action,
    );
    if let Some(binding) = problem.incompatible_input {
        output["incompatible_input"] = json!(binding);
    }
    if let Some(step) = problem.step {
        if let Ok(Value::String(name)) = serde_json::to_value(&step) {
            if !name.is_empty()
                && name.len() <= 64
                && name.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                })
            {
                output["step"] = Value::String(name);
            }
        }
    }
    output
}

fn site_architect_operation_inspection(
    catalog: &SiteArchitectCatalog,
    operation: &OperationRef,
    parameters: Option<&RecipeParameters>,
) -> std::result::Result<Value, Value> {
    inspect_operation(catalog, operation, parameters)
        .map(|inspection| json!(inspection))
        .map_err(|error| site_architect_safe_operation_problem(&error))
}

/// A deliberately *local* readiness accounting. The installed owner catalog
/// can prove schemas and pure-plan composition, but it cannot prove an
/// authenticated ChatGPT user, live provider, independent issuer, a consumed
/// one-use grant, source bytes or a Temporal execution. Keep those gates
/// explicit even if every owner has a valid schema. Do not hard-code the 52
/// product-matrix entries as if they were installed executable operations.
fn site_architect_launch_readiness(
    operations: &[seo_application::site_architect::ToolDescription],
) -> Value {
    let verified_catalog =
        seo_application::site_architect::summarize_local_execution_readiness(operations);
    json!({
        "status":"catalog_and_authoring_only",
        "typed_operation_readiness":&verified_catalog,
        "counts":{
            "installed_versioned_operations":verified_catalog.installed_versioned_operations,
            "with_parameter_schema":verified_catalog.installed_versioned_operations - verified_catalog.with_missing_parameter_definition,
            "with_local_typed_parameter_check":verified_catalog.with_installed_parameter_definition,
            "requiring_independent_owner_validate_only":verified_catalog.requiring_independent_owner_parameter_validation,
            "user_start_verified_by_this_adapter":verified_catalog.user_start_verified
        },
        "composition":{
            "single_operation":"typed_preview_only",
            "multi_operation":"typed_dag_preview_only",
            "selection_order":"caller_selected_subject_to_typed_input_dependencies",
            "execution_order":"not_verified"
        },
        "one_step_launch_request":{
            "tool":"site_architect_one_step",
            "status":"blocked_authenticated_start_gateway_not_configured",
            "scope":"one_typed_recipe_step",
            "mutating_temporal_start":false,
            "authenticated_oauth_resource_server":"not_installed_in_stdio_adapter",
            "remote_run_one":"separate_oauth_protected_request_id_contract"
        },
        "launch_gates":{
            "authenticated_chatgpt_principal_and_tenant_acl":"not_proven_here",
            "budget_and_one_use_grant":"not_proven_here",
            "exact_source_bytes_and_command_attestation":"not_proven_here",
            "native_owner_validate_only_and_effect_admission":"not_proven_here",
            "temporal_start_and_server_history":"not_proven_here",
            "provider_receipt_and_metering":"not_proven_here"
        },
        "mcp_start_exposed":false,
        "next_action":"Complete independent user authorization, atomic one-use grant reservation, owner/source attestation and exact Temporal History checks before enabling mutating ChatGPT Start. Catalog declarations cannot certify any user-ready operation."
    })
}

/// The one-step recipe is the smallest typed DAG authoring graph, not a
/// Temporal execution preflight or parallel single-operation executor. This
/// reads only declarations; actual source bytes, Temporal History, owner
/// ValidateOnly and SQL admission are absent.
fn site_architect_one_step_preview(
    recipe: &SiteRecipe,
    catalog: &SiteArchitectCatalog,
) -> std::result::Result<Value, Value> {
    if recipe.steps.len() != 1 {
        return Err(site_architect_problem(
            "one_step_required",
            "Individual operation check requires exactly one step in the recipe.",
            "Use a one-step recipe or site_architect_check for a multi-step scenario.",
        ));
    }
    let step = &recipe.steps[0];
    let operation =
        site_architect_operation_inspection(catalog, &step.operation, Some(&step.parameters))?;
    let preview = preview_recipe(recipe, catalog)
        .map_err(|error| site_architect_safe_recipe_problem(&error))?;
    Ok(json!({
        "status": "single_step_preview_only",
        "operation": operation,
        "preview": preview,
        "source_authentication": "not_checked",
        "owner_validate_only": "required_not_run",
        "operator_admission": "not_issued",
        "temporal_history_attestation": "not_checked",
        "user_start_admission": "not_available_in_read_only_adapter",
        "batch_execution": "not_started",
        "execution": "not_started",
        "next_action": "Verify exact source and command bytes through the native V2 preflight, execute independent owner ValidateOnly and obtain operator admission before any Temporal effects.",
    }))
}

fn site_architect_print<T: serde::Serialize>(report: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(report)?);
    Ok(())
}

fn site_architect_read_saved<T: serde::de::DeserializeOwned>(
    path: &str,
) -> std::result::Result<T, Value> {
    let reader = site_architect_open_input(path).map_err(|_| {
        site_architect_problem(
            "saved_file_unavailable",
            "Cannot open the requested saved plan or receipt input.",
            "Pass a readable saved file or '-' for one bounded stdin input.",
        )
    })?;
    let bytes = site_architect_bounded_bytes(reader, SITE_ARCHITECT_SAVED_MAX_BYTES).map_err(|_| {
        site_architect_problem(
            "saved_read_failed",
            "Cannot read the saved file.",
            "Check file permissions and retry.",
        )
    })?;
    if bytes.len() as u64 > SITE_ARCHITECT_SAVED_MAX_BYTES {
        return Err(site_architect_problem(
            "saved_file_too_large",
            "Saved input exceeds the existing 512 KiB DAG envelope budget.",
            "Do not truncate a receipt; inspect the original bounded Temporal result.",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        site_architect_problem(
            "saved_invalid_json",
            "Saved input is not a valid, versioned plan or receipt.",
            "Export the original typed plan and V2 receipt without editing their contents.",
        )
    })
}

fn site_architect_read_execution(
    plan_file: &str,
    receipt_file: &str,
) -> std::result::Result<(ExecutionPlan, OperationDagReceiptV1), Value> {
    Ok((
        site_architect_read_saved(plan_file)?,
        site_architect_read_saved(receipt_file)?,
    ))
}

/// Match the native Temporal worker's operator-owned trust root, never a
/// path/manifest supplied by MCP tool arguments or a recipe. An installed
/// remote owner remains subject to independent ValidateOnly and admission.
fn site_architect_installed_catalog() -> Result<SiteArchitectCatalog> {
    // The registry is compiled from actual, trusted owner operations. User
    // recipes cannot inject contracts, owner routes, credentials or task queues.
    let registry = semantic_extraction_registry().context("register siteArchitect operations")?;
    let mut installed = installed_semantic_operation_catalog(registry.catalog().clone())
        .context("register trusted siteArchitect parameter contracts")?;
    if let Some(path) = std::env::var_os("TEMPORAL_OPERATION_DAG_CATALOG_PATH") {
        let bundle = trusted_owner_catalog::load_trusted_owner_bundle(std::path::Path::new(&path))
            .context("read operator-owned Temporal owner routes for siteArchitect")?;
        let compiled_schemas = semantic_extraction_owner_schemas()?;
        for contract in bundle.routes.catalog().contracts() {
            if let Some(local) = installed.catalog().contract(&contract.reference) {
                anyhow::ensure!(
                    local == contract,
                    "trusted owner route conflicts with a built-in siteArchitect operation"
                );
                let schema = compiled_schemas.get(&contract.reference)
                    .context("compiled semantic owner DTO schema missing")?;
                bundle.verify_caller_schema(&contract.reference, schema)?;
                anyhow::ensure!(
                    bundle.parameter_schemas.get(&contract.reference)
                        == Some(&json!({"type": "object", "additionalProperties": false})),
                    "installed semantic settings differ from the compiled empty DTO contract"
                );
                // The same operation retains its stricter compiled authoring
                // validator. The operator catalog supplies the native route.
                continue;
            }
            installed.register_contract(contract.clone())?;
            if let Some(schema) = bundle.parameter_schemas.get(&contract.reference) {
                installed.register_remote(&contract.reference, &contract.fingerprint()?, schema.clone())?;
            }
        }
    }
    Ok(installed)
}

fn site_architect_tools_report(
    catalog: &SiteArchitectCatalog,
) -> std::result::Result<Value, Value> {
    let operations = describe_tools(catalog).map_err(|error| json!(error.problem()))?;
    let matrix = seo_application::site_architect::matrix::operation_matrix(&operations)
        .map_err(|error| site_architect_problem(
            "operation_matrix_invalid",
            &error.to_string(),
            "Correct the bundled product matrix.",
        ))?;
    Ok(json!({
        "status": "catalog_preview_only",
        "launch_readiness": site_architect_launch_readiness(&operations),
        "operations": operations,
        "matrix": matrix,
        "execution": "not_started"
    }))
}

fn site_architect_command(action: SiteArchitectCommand) -> Result<i32> {
    let installed = site_architect_installed_catalog()?;
    let catalog = &installed;

    match action {
        SiteArchitectCommand::Tools => {
            match site_architect_tools_report(catalog) {
                Ok(report) => site_architect_print(&report)?,
                Err(problem) => {
                    site_architect_print(&problem)?;
                    return Ok(2);
                }
            }
        }
        SiteArchitectCommand::Operation {
            id,
            revision,
            parameters_file,
        } => {
            if id.trim().is_empty() || id.len() > 256 || revision == 0 {
                site_architect_print(&site_architect_problem(
                    "operation_reference_invalid",
                    "Operation identifier or revision is invalid.",
                    "Select an exact installed operation from site-architect tools.",
                ))?;
                return Ok(2);
            }
            let operation: OperationRef =
                match serde_json::from_value(json!({"id": id, "revision": revision})) {
                    Ok(reference) => reference,
                    Err(_) => {
                        site_architect_print(&site_architect_problem(
                            "operation_reference_invalid",
                            "Operation identifier or revision is invalid.",
                            "Select an exact installed operation from site-architect tools.",
                        ))?;
                        return Ok(2);
                    }
                };
            let parameters = match parameters_file.as_deref() {
                Some(path) => match site_architect_read_parameters(path) {
                    Ok(parameters) => Some(parameters),
                    Err(problem) => {
                        site_architect_print(&problem)?;
                        return Ok(2);
                    }
                },
                None => None,
            };
            match site_architect_operation_inspection(catalog, &operation, parameters.as_ref()) {
                Ok(report) => site_architect_print(&report)?,
                Err(problem) => {
                    site_architect_print(&problem)?;
                    return Ok(2);
                }
            }
        }
        SiteArchitectCommand::OneStep { recipe_file } => {
            let recipe = match site_architect_read_recipe(&recipe_file) {
                Ok(recipe) => recipe,
                Err(problem) => {
                    site_architect_print(&problem)?;
                    return Ok(2);
                }
            };
            match site_architect_one_step_preview(&recipe, catalog) {
                Ok(report) => site_architect_print(&report)?,
                Err(problem) => {
                    site_architect_print(&problem)?;
                    return Ok(2);
                }
            }
        }
        SiteArchitectCommand::Check { recipe_file } => {
            let recipe = match site_architect_read_recipe(&recipe_file) {
                Ok(recipe) => recipe,
                Err(problem) => {
                    site_architect_print(&problem)?;
                    return Ok(2);
                }
            };
            match preview_recipe(&recipe, catalog) {
                Ok(report) => site_architect_print(&report)?,
                Err(error) => {
                    site_architect_print(&site_architect_safe_recipe_problem(&error))?;
                    return Ok(2);
                }
            }
        }
        SiteArchitectCommand::Diff {
            first_file,
            second_file,
        } => {
            if site_architect_reused_stdin(&[&first_file, &second_file]) {
                site_architect_print(&site_architect_stdin_conflict())?;
                return Ok(2);
            }
            let first = match site_architect_read_recipe(&first_file) {
                Ok(recipe) => recipe,
                Err(problem) => {
                    site_architect_print(&problem)?;
                    return Ok(2);
                }
            };
            let second = match site_architect_read_recipe(&second_file) {
                Ok(recipe) => recipe,
                Err(problem) => {
                    site_architect_print(&problem)?;
                    return Ok(2);
                }
            };
            match diff_recipes(&first, &second, catalog) {
                Ok(report) => site_architect_print(&report)?,
                Err(error) => {
                    site_architect_print(&site_architect_safe_recipe_problem(&error))?;
                    return Ok(2);
                }
            }
        }
        SiteArchitectCommand::Inspect {
            plan_file,
            receipt_file,
        } => {
            if site_architect_reused_stdin(&[&plan_file, &receipt_file]) {
                site_architect_print(&site_architect_stdin_conflict())?;
                return Ok(2);
            }
            let (plan, receipt) = match site_architect_read_execution(&plan_file, &receipt_file) {
                Ok(saved) => saved,
                Err(problem) => {
                    site_architect_print(&problem)?;
                    return Ok(2);
                }
            };
            match inspect_execution(&plan, &receipt, catalog.catalog()) {
                Ok(report) => site_architect_print(&report)?,
                Err(error) => {
                    site_architect_print(&error.problem())?;
                    return Ok(2);
                }
            }
        }
        SiteArchitectCommand::CompareResults {
            first_plan_file,
            first_receipt_file,
            second_plan_file,
            second_receipt_file,
        } => {
            if site_architect_reused_stdin(&[
                &first_plan_file,
                &first_receipt_file,
                &second_plan_file,
                &second_receipt_file,
            ]) {
                site_architect_print(&site_architect_stdin_conflict())?;
                return Ok(2);
            }
            let (first_plan, first_receipt) =
                match site_architect_read_execution(&first_plan_file, &first_receipt_file) {
                    Ok(saved) => saved,
                    Err(problem) => {
                        site_architect_print(&problem)?;
                        return Ok(2);
                    }
                };
            let (second_plan, second_receipt) =
                match site_architect_read_execution(&second_plan_file, &second_receipt_file) {
                    Ok(saved) => saved,
                    Err(problem) => {
                        site_architect_print(&problem)?;
                        return Ok(2);
                    }
                };
            match compare_executions(
                &first_plan,
                &first_receipt,
                &second_plan,
                &second_receipt,
                catalog.catalog(),
            ) {
                Ok(report) => site_architect_print(&report)?,
                Err(error) => {
                    site_architect_print(&error.problem())?;
                    return Ok(2);
                }
            }
        }
    }
    Ok(0)
}

#[cfg(test)]
mod site_architect_cli_tests {
    use super::*;

    #[test]
    fn native_architect_commands_are_parseable_without_runtime_configuration() {
        for args in [
            vec!["cli_tools", "site-architect", "tools"],
            vec![
                "cli_tools",
                "site-architect",
                "operation",
                "--id",
                "semantic.entity_spans",
                "--revision",
                "1",
            ],
            vec![
                "cli_tools",
                "site-architect",
                "operation",
                "--id",
                "semantic.entity_spans",
                "--revision",
                "1",
                "--parameters-file",
                "-",
            ],
            vec!["cli_tools", "site-architect", "one-step", "--recipe-file", "-"],
            vec!["cli_tools", "site-architect", "check", "--recipe-file", "recipe.json"],
            vec!["cli_tools", "site-architect", "check", "--recipe-file", "-"],
            vec![
                "cli_tools",
                "site-architect",
                "diff",
                "--first-file",
                "first.json",
                "--second-file",
                "second.json",
            ],
            vec![
                "cli_tools", "site-architect", "inspect", "--plan-file", "plan.json",
                "--receipt-file", "receipt.json",
            ],
            vec![
                "cli_tools", "site-architect", "compare-results", "--first-plan-file",
                "a-plan.json", "--first-receipt-file", "a-receipt.json",
                "--second-plan-file", "b-plan.json", "--second-receipt-file", "b-receipt.json",
            ],
        ] {
            let parsed = Cli::try_parse_from(args).unwrap();
            assert!(matches!(parsed.command, Command::SiteArchitect { .. }));
        }
        for args in [
            vec!["cli_tools", "unknown-command", "tools"],
            vec!["cli_tools", "mcp-unknown"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
        assert!(Cli::try_parse_from(["cli_tools", "mcp-site-architect"]).is_ok());
    }

    #[test]
    fn parse_errors_do_not_leak_recipe_bytes() {
        let problem = site_architect_problem(
            "recipe_invalid_json",
            "expected a JSON object at line 1 column 1",
            "Correct the input.",
        );
        assert_eq!(problem["status"], "invalid");
        assert_eq!(problem["code"], "recipe_invalid_json");
    }

    #[test]
    fn recipe_json_rejects_runtime_credentials_and_supplied_contracts() {
        let raw = json!({
            "schema_revision": 2,
            "scope": { "tenant_id": "lab", "run_id": "hypothesis-1" },
            "steps": [],
        });
        for (field, attempted_value) in [
            ("task_queue", json!("untrusted-queue")),
            ("pinned_contracts", json!([])),
            ("provider_credentials", json!({})),
        ] {
            let mut forged = raw.clone();
            forged[field] = attempted_value;
            assert!(
                serde_json::from_value::<SiteRecipe>(forged).is_err(),
                "recipe unexpectedly accepted {field}"
            );
        }
    }

    #[test]
    fn machine_json_stdin_is_single_use_and_has_explicit_budgets() {
        assert!(!site_architect_reused_stdin(&["-", "a.json"]));
        assert!(!site_architect_reused_stdin(&["a.json", "b.json"]));
        assert!(site_architect_reused_stdin(&["-", "-", "c.json"]));
        assert_eq!(site_architect_stdin_conflict()["code"], "stdin_reused");

        let max = SITE_ARCHITECT_RECIPE_MAX_BYTES as usize;
        let exact = site_architect_bounded_bytes(
            std::io::Cursor::new(vec![b'x'; max]),
            SITE_ARCHITECT_RECIPE_MAX_BYTES,
        )
        .unwrap();
        assert_eq!(exact.len(), max);
        let oversized = site_architect_bounded_bytes(
            std::io::Cursor::new(vec![b'x'; max + 10]),
            SITE_ARCHITECT_RECIPE_MAX_BYTES,
        )
        .unwrap();
        assert_eq!(oversized.len(), max + 1);
    }

    #[test]
    fn single_operation_is_inspectable_but_not_executable() {
        let catalog = site_architect_installed_catalog().unwrap();
        let operation: OperationRef = serde_json::from_value(json!({
            "id": "semantic.entity_spans", "revision": 1
        }))
        .unwrap();
        let profile =
            site_architect_operation_inspection(&catalog, &operation, Some(&RecipeParameters::default()))
                .unwrap();
        assert_eq!(profile["status"], "registered_operation_inspection_only");
        assert_eq!(profile["execution"], "not_started");
        assert_eq!(profile["provider_connectivity"], "not_checked");
        assert_eq!(
            profile["checked_parameters"]["status"],
            "typed_configuration_checked"
        );
        assert_eq!(profile["checked_parameters"]["verification"], "typed_local");
        assert_eq!(profile["operator_admission"], "not_issued_by_inspection");
    }

    #[test]
    fn readiness_is_derived_from_installed_catalog_without_inventing_user_start() {
        let catalog = site_architect_installed_catalog().unwrap();
        let operations = describe_tools(&catalog).unwrap();
        let readiness = site_architect_launch_readiness(&operations);
        assert_eq!(readiness["status"], "catalog_and_authoring_only");
        assert_eq!(
            readiness["counts"]["installed_versioned_operations"],
            operations.len()
        );
        assert_eq!(readiness["counts"]["user_start_verified_by_this_adapter"], 0);
        assert_eq!(readiness["mcp_start_exposed"], false);
        assert_eq!(readiness["composition"]["single_operation"], "typed_preview_only");
        assert_eq!(readiness["composition"]["multi_operation"], "typed_dag_preview_only");
        assert_eq!(
            readiness["launch_gates"]["authenticated_chatgpt_principal_and_tenant_acl"],
            "not_proven_here"
        );
        assert_eq!(readiness["launch_gates"]["provider_receipt_and_metering"], "not_proven_here");
    }

    #[test]
    fn one_step_is_exactly_one_native_recipe_and_cannot_start_temporal() {
        let catalog = site_architect_installed_catalog().unwrap();
        let recipe: SiteRecipe = serde_json::from_value(json!({
            "schema_revision":2,
            "scope":{"tenant_id":"lab","run_id":"single-test"},
            "sources":{"mentions":{"schema":{"id":"semantic.entity_span_input","revision":1},"provenance":"unverified:test"}},
            "steps":[{"id":"extract","operation":{"id":"semantic.entity_spans","revision":1},
                "inputs":{"input":{"source":"external","artifact":"mentions"}},"parameters":{}}]
        })).unwrap();
        let result = site_architect_one_step_preview(&recipe, &catalog).unwrap();
        assert_eq!(result["status"], "single_step_preview_only");
        assert_eq!(result["execution"], "not_started");
        assert_eq!(result["source_authentication"], "not_checked");
        assert_eq!(result["temporal_history_attestation"], "not_checked");
        assert_eq!(result["operator_admission"], "not_issued");
        assert_eq!(
            result["preview"]["ordered_steps"].as_array().unwrap().len(),
            1
        );

        let mut two_steps = recipe.clone();
        two_steps.steps.push(recipe.steps[0].clone());
        assert_eq!(
            site_architect_one_step_preview(&two_steps, &catalog).unwrap_err()["code"],
            "one_step_required"
        );
        let mut none = recipe;
        none.steps.clear();
        assert_eq!(
            site_architect_one_step_preview(&none, &catalog).unwrap_err()["code"],
            "one_step_required"
        );
    }

    #[test]
    fn individual_operation_rejects_unknown_settings_without_reflecting_secrets() {
        let catalog = site_architect_installed_catalog().unwrap();
        let operation: OperationRef = serde_json::from_value(json!({
            "id": "semantic.entity_spans", "revision": 1
        }))
        .unwrap();
        let parameters =
            RecipeParameters::from(json!({"provider_token":"SENTINEL-PRIVATE-API-KEY"}));
        let error =
            site_architect_operation_inspection(&catalog, &operation, Some(&parameters)).unwrap_err();
        assert_eq!(error["status"], "invalid");
        assert_eq!(error["code"], "operation_parameters_invalid");
        assert!(!error.to_string().contains("SENTINEL-PRIVATE-API-KEY"));
    }
}
