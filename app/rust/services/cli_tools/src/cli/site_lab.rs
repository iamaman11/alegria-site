use std::io::Read;

use seo_application::operation_adapter::semantic_extraction_registry;
use seo_application::site_lab::{
    compare_executions, describe_tools, diff_recipes, inspect_execution,
    installed_semantic_lab_catalog, preview_recipe, ExecutionPlan, OperationDagReceiptV1,
    SiteRecipe,
};

const SITE_LAB_SAVED_MAX_BYTES: u64 = 512 * 1024;

const SITE_LAB_RECIPE_MAX_BYTES: u64 = 256 * 1024;

fn site_lab_problem(code: &str, message: &str, next_action: &str) -> Value {
    json!({
        "status": "invalid",
        "code": code,
        "message": message,
        "next_action": next_action,
    })
}

fn site_lab_read_recipe(path: &str) -> std::result::Result<SiteRecipe, Value> {
    let file = fs::File::open(path).map_err(|error| {
        site_lab_problem(
            "recipe_file_unavailable",
            &format!("Cannot open recipe: {error}"),
            "Check --recipe-file/--first-file/--second-file and local file access.",
        )
    })?;
    let mut bytes = Vec::new();
    file.take(SITE_LAB_RECIPE_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            site_lab_problem(
                "recipe_read_failed",
                &format!("Cannot read recipe: {error}"),
                "Make the recipe readable and retry.",
            )
        })?;
    if bytes.len() as u64 > SITE_LAB_RECIPE_MAX_BYTES {
        return Err(site_lab_problem(
            "recipe_too_large",
            "Recipe exceeds the 256 KiB input limit.",
            "Split the research into smaller, independently checked recipes.",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        site_lab_problem(
            "recipe_invalid_json",
            &error.to_string(),
            "Use the versioned SiteRecipe JSON format; unknown fields are forbidden.",
        )
    })
}

fn site_lab_print<T: serde::Serialize>(report: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(report)?);
    Ok(())
}

fn site_lab_read_saved<T: serde::de::DeserializeOwned>(
    path: &str,
) -> std::result::Result<T, Value> {
    let file = fs::File::open(path).map_err(|_| {
        site_lab_problem(
            "saved_file_unavailable",
            "Cannot open the requested saved plan or receipt.",
            "Pass a readable --plan-file or --receipt-file from a completed run.",
        )
    })?;
    let mut bytes = Vec::new();
    file.take(SITE_LAB_SAVED_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            site_lab_problem(
                "saved_read_failed",
                "Cannot read the saved file.",
                "Check file permissions and retry.",
            )
        })?;
    if bytes.len() as u64 > SITE_LAB_SAVED_MAX_BYTES {
        return Err(site_lab_problem(
            "saved_file_too_large",
            "Saved input exceeds the existing 512 KiB DAG envelope budget.",
            "Do not truncate a receipt; inspect the original bounded Temporal result.",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        site_lab_problem(
            "saved_invalid_json",
            "Saved input is not a valid, versioned plan or receipt.",
            "Export the original typed plan and V2 receipt without editing their contents.",
        )
    })
}

fn site_lab_read_execution(
    plan_file: &str,
    receipt_file: &str,
) -> std::result::Result<(ExecutionPlan, OperationDagReceiptV1), Value> {
    Ok((
        site_lab_read_saved(plan_file)?,
        site_lab_read_saved(receipt_file)?,
    ))
}

fn site_lab_command(action: SiteLabCommand) -> Result<i32> {
    // The registry is compiled from actual, trusted owner operations. User
    // recipes cannot inject contracts, owner routes, credentials or task queues.
    let registry = semantic_extraction_registry().context("register Site Lab operations")?;
    let installed = installed_semantic_lab_catalog(registry.catalog().clone())
        .context("register trusted Site Lab parameter contracts")?;
    let catalog = &installed;

    match action {
        SiteLabCommand::Tools => {
            site_lab_print(&json!({
                "status": "catalog_preview_only",
                "operations": describe_tools(catalog)?,
                "execution": "not_started",
                "note": "Declared tools are not provider connectivity or owner admission proof.",
            }))?;
        }
        SiteLabCommand::Check { recipe_file } => {
            let recipe = match site_lab_read_recipe(&recipe_file) {
                Ok(recipe) => recipe,
                Err(problem) => {
                    site_lab_print(&problem)?;
                    return Ok(2);
                }
            };
            match preview_recipe(&recipe, catalog) {
                Ok(report) => site_lab_print(&report)?,
                Err(error) => {
                    site_lab_print(&error.problem())?;
                    return Ok(2);
                }
            }
        }
        SiteLabCommand::Diff {
            first_file,
            second_file,
        } => {
            let first = match site_lab_read_recipe(&first_file) {
                Ok(recipe) => recipe,
                Err(problem) => {
                    site_lab_print(&problem)?;
                    return Ok(2);
                }
            };
            let second = match site_lab_read_recipe(&second_file) {
                Ok(recipe) => recipe,
                Err(problem) => {
                    site_lab_print(&problem)?;
                    return Ok(2);
                }
            };
            match diff_recipes(&first, &second, catalog) {
                Ok(report) => site_lab_print(&report)?,
                Err(error) => {
                    site_lab_print(&error.problem())?;
                    return Ok(2);
                }
            }
        }
        SiteLabCommand::Inspect {
            plan_file,
            receipt_file,
        } => {
            let (plan, receipt) = match site_lab_read_execution(&plan_file, &receipt_file) {
                Ok(saved) => saved,
                Err(problem) => {
                    site_lab_print(&problem)?;
                    return Ok(2);
                }
            };
            match inspect_execution(&plan, &receipt, catalog.catalog()) {
                Ok(report) => site_lab_print(&report)?,
                Err(error) => {
                    site_lab_print(&error.problem())?;
                    return Ok(2);
                }
            }
        }
        SiteLabCommand::CompareResults {
            first_plan_file,
            first_receipt_file,
            second_plan_file,
            second_receipt_file,
        } => {
            let (first_plan, first_receipt) =
                match site_lab_read_execution(&first_plan_file, &first_receipt_file) {
                    Ok(saved) => saved,
                    Err(problem) => {
                        site_lab_print(&problem)?;
                        return Ok(2);
                    }
                };
            let (second_plan, second_receipt) =
                match site_lab_read_execution(&second_plan_file, &second_receipt_file) {
                    Ok(saved) => saved,
                    Err(problem) => {
                        site_lab_print(&problem)?;
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
                Ok(report) => site_lab_print(&report)?,
                Err(error) => {
                    site_lab_print(&error.problem())?;
                    return Ok(2);
                }
            }
        }
    }
    Ok(0)
}

#[cfg(test)]
mod site_lab_cli_tests {
    use super::*;

    #[test]
    fn native_lab_commands_are_parseable_without_runtime_configuration() {
        for args in [
            vec!["cli_tools", "site-lab", "tools"],
            vec!["cli_tools", "site-lab", "check", "--recipe-file", "recipe.json"],
            vec![
                "cli_tools",
                "site-lab",
                "diff",
                "--first-file",
                "first.json",
                "--second-file",
                "second.json",
            ],
            vec![
                "cli_tools", "site-lab", "inspect", "--plan-file", "plan.json",
                "--receipt-file", "receipt.json",
            ],
            vec![
                "cli_tools", "site-lab", "compare-results", "--first-plan-file",
                "a-plan.json", "--first-receipt-file", "a-receipt.json",
                "--second-plan-file", "b-plan.json", "--second-receipt-file", "b-receipt.json",
            ],
        ] {
            let parsed = Cli::try_parse_from(args).unwrap();
            assert!(matches!(parsed.command, Command::SiteLab { .. }));
        }
    }

    #[test]
    fn parse_errors_do_not_leak_recipe_bytes() {
        let problem = site_lab_problem(
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
}
