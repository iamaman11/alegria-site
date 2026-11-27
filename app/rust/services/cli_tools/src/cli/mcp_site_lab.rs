// Native stdio MCP adapter for stateless authoring and offline receipt
// inspection. It uses the same trusted catalog and checks as `site-lab`;
// no provider calls, Temporal Start, arbitrary file reads, dynamic manifest
// loading, or independent DAG runner.

const SITE_LAB_MCP_PROTOCOL: &str = "2025-11-25";
// Two sealed plans and two V2 receipts may each reach the existing 512 KiB
// DAG boundary; allow envelope overhead, without accepting unbounded input.
const SITE_LAB_MCP_MAX_MESSAGE_BYTES: usize = 3 * 1024 * 1024;
// Recipe authoring is much smaller than the transport envelope needed for
// comparing two already bounded saved V2 receipts. Enforce the distinction
// on the server, not just in the advertised MCP JSON Schema.
const SITE_LAB_MCP_AUTHORING_MAX_BYTES: usize = 2 * SITE_LAB_RECIPE_MAX_BYTES as usize + 4096;
const SITE_LAB_MCP_MAX_RECIPE_EDITS: usize = 128;
const SITE_LAB_MCP_MAX_SCOPE_ID_BYTES: usize = 128;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpCall {
    name: String,
    #[serde(default)]
    arguments: Value,
    #[serde(rename = "_meta", default)]
    _meta: Option<Value>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpNewRecipe {
    tenant_id: String,
    run_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpOneRecipe {
    recipe: seo_application::site_lab::SiteRecipe,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpOneOperation {
    operation: seo_application::site_lab::OperationRef,
    #[serde(default)]
    parameters: Option<seo_application::site_lab::RecipeParameters>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpEditRecipe {
    recipe: seo_application::site_lab::SiteRecipe,
    changes: Vec<seo_application::site_lab::RecipeChange>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpTwoRecipes {
    first: seo_application::site_lab::SiteRecipe,
    second: seo_application::site_lab::SiteRecipe,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpSavedExecution {
    plan: seo_application::site_lab::ExecutionPlan,
    receipt: seo_application::site_lab::OperationDagReceiptV1,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpCompareExecutions {
    first: McpSavedExecution,
    second: McpSavedExecution,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpNeedEvidence {
    batch: seo_application::site_lab::ConsumerNeedEvidenceBatchV1,
}

fn mcp_object_schema(fields: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": fields,
        "required": required,
        "additionalProperties": false,
    })
}

fn mcp_operation_schema() -> Value {
    mcp_object_schema(
        json!({"id":{"type":"string","minLength":1},"revision":{"type":"integer","minimum":1}}),
        &["id", "revision"],
    )
}

fn mcp_input_schema() -> Value {
    json!({"oneOf":[
        mcp_object_schema(json!({"source":{"const":"external"},"artifact":{"type":"string"}}), &["source","artifact"]),
        mcp_object_schema(json!({"source":{"const":"produced"},"step":{"type":"string"},"port":{"type":"string"}}), &["source","step","port"])
    ]})
}

fn mcp_source_schema() -> Value {
    mcp_object_schema(json!({
        "schema": mcp_operation_schema(),
        "provenance":{"type":"string"}
    }), &["schema", "provenance"])
}

fn mcp_step_schema() -> Value {
    mcp_object_schema(json!({
        "id":{"type":"string","minLength":1},
        "operation":mcp_operation_schema(),
        "inputs":{"type":"object","additionalProperties":mcp_input_schema()},
        "parameters":{"type":"object","description":"Owner-specific settings; see operation parameters.json_schema in site_lab_tools"},
        "after":{"type":"array","uniqueItems":true,"items":{"type":"string"}}
    }), &["id","operation","inputs"])
}

fn mcp_recipe_schema() -> Value {
    mcp_object_schema(json!({
        "schema_revision":{"const":2},
        "scope":mcp_object_schema(json!({
            "tenant_id":{"type":"string","minLength":1},
            "run_id":{"type":"string","minLength":1}
        }), &["tenant_id","run_id"]),
        "sources":{"type":"object","additionalProperties":mcp_source_schema()},
        "steps":{"type":"array","items":mcp_step_schema()}
    }), &["schema_revision","scope","steps"])
}

fn mcp_edit_schema() -> Value {
    json!({"oneOf":[
        mcp_object_schema(json!({"action":{"const":"add_step"},"step":mcp_step_schema()}), &["action","step"]),
        mcp_object_schema(json!({"action":{"const":"remove_step"},"step":{"type":"string"}}), &["action","step"]),
        mcp_object_schema(json!({"action":{"const":"set_operation"},"step":{"type":"string"},"operation":mcp_operation_schema()}), &["action","step","operation"]),
        mcp_object_schema(json!({"action":{"const":"set_parameters"},"step":{"type":"string"},"parameters":{"type":"object"}}), &["action","step","parameters"]),
        mcp_object_schema(json!({"action":{"const":"set_input"},"step":{"type":"string"},"port":{"type":"string"},"input":mcp_input_schema()}), &["action","step","port","input"]),
        mcp_object_schema(json!({"action":{"const":"remove_input"},"step":{"type":"string"},"port":{"type":"string"}}), &["action","step","port"]),
        mcp_object_schema(json!({"action":{"const":"set_after"},"step":{"type":"string"},"after":{"type":"array","uniqueItems":true,"items":{"type":"string"}}}), &["action","step","after"]),
        mcp_object_schema(json!({"action":{"const":"upsert_source"},"artifact":{"type":"string"},"source":mcp_source_schema()}), &["action","artifact","source"]),
        mcp_object_schema(json!({"action":{"const":"remove_source"},"artifact":{"type":"string"}}), &["action","artifact"])
    ]})
}

fn mcp_saved_execution_schema() -> Value {
    // Short introspection schema, with the exact deeply nested Rust DTO
    // enforced after deserialization. Only complete typed JSON values work;
    // there is deliberately no path, URL, provider ID or receipt loader.
    mcp_object_schema(json!({
        "plan":mcp_object_schema(json!({
            "schema_revision":{"type":"integer"},
            "scope":{"type":"object"},
            "allowed_effects":{"type":"array"},
            "external_artifacts":{"type":"object"},
            "pinned_contracts":{"type":"array"},
            "nodes":{"type":"array"},
            "hash":{"type":"string"}
        }), &["schema_revision","scope","allowed_effects","external_artifacts","pinned_contracts","nodes","hash"]),
        "receipt":mcp_object_schema(json!({
            "plan_hash":{"type":"string"},
            "order":{"type":"array"},
            "artifacts":{"type":"object"},
            "produced_evidence":{"type":"object"},
            "owner_results":{"type":"object"}
        }), &["plan_hash","order","artifacts","produced_evidence","owner_results"])
    }), &["plan","receipt"])
}

fn mcp_need_evidence_schema() -> Value {
    // This schema is for MCP discovery; strongly typed, deny-unknown-fields
    // Rust DTOs and strict size/hash/scope checks remain authoritative.
    let source = mcp_object_schema(json!({
        "source_id":{"type":"string"}, "tenant_id":{"type":"string"},
        "source_run_id":{"type":"string"}, "retrieval_channel":{"type":"string"},
        "representation":{"enum":["primary_document","search_snippet","derived_summary","mirrored_document"]},
        "primary_origin_id":{"type":"string"}, "primary_document_id":{"type":"string"},
        "provenance_locator":{"type":"string"}, "document_payload_hash":{"type":"string"}
    }), &["source_id","tenant_id","source_run_id","retrieval_channel","representation","primary_origin_id","primary_document_id","provenance_locator","document_payload_hash"]);
    let observation = mcp_object_schema(json!({
        "observation_id":{"type":"string"}, "need_key":{"type":"string"},
        "asserted_need":{"type":"string"}, "context":{"type":"string"},
        "excerpt":{"type":"string"}, "excerpt_hash":{"type":"string"}, "source":source
    }), &["observation_id","need_key","asserted_need","context","excerpt","excerpt_hash","source"]);
    let hypothesis = mcp_object_schema(json!({
        "need_key":{"type":"string"}, "proposed_site_change":{"type":"string"},
        "expected_benefit":{"type":"string"}
    }), &["need_key","proposed_site_change","expected_benefit"]);
    mcp_object_schema(json!({"batch":mcp_object_schema(json!({
        "schema_revision":{"const":1},
        "scope":mcp_object_schema(json!({"tenant_id":{"type":"string"},"run_id":{"type":"string"}}), &["tenant_id","run_id"]),
        "observations":{"type":"array","maxItems":128,"items":observation},
        "hypotheses":{"type":"array","minItems":1,"maxItems":32,"items":hypothesis}
    }), &["schema_revision","scope","observations","hypotheses"])}), &["batch"])
}

fn mcp_site_lab_tools() -> Value {
    json!({"tools":[
        {
            "name":"site_lab_tools", "title":"Available Site Lab operations",
            "description":"Discover only trusted installed operations, owner, typed ports, version, effects and exact provider parameter schema. Catalog presence never proves provider connectivity or permission to execute.",
            "inputSchema":mcp_object_schema(json!({}), &[]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_lab_operation", "title":"Inspect one installed operation",
            "description":"Inspect exact versioned owner ports, advanced parameter schema and admission status. Optional parameters get local typed validation or explicitly pending remote ValidateOnly. No provider connection, actual inputs or execution; settings never echoed.",
            "inputSchema":mcp_object_schema(json!({
                "operation":mcp_operation_schema(),
                "parameters":{"type":"object","description":"Optional owner-specific settings; omitted settings mean inspect only"}
            }), &["operation"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_lab_one_step", "title":"Check one operation as a Temporal recipe",
            "description":"Check a recipe with exactly one step using the existing typed DAG plan validator, and show exact installed owner settings and input dependencies. Authoring preview only, NOT the native Temporal Start preflight: exact source bytes, owner ValidateOnly, admission and History remain unverified.",
            "inputSchema":mcp_object_schema(json!({"recipe":mcp_recipe_schema()}), &["recipe"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_lab_new", "title":"Create a blank research recipe",
            "description":"Create an empty versioned SiteRecipe draft with a run scope. Add sources and steps with site_lab_edit; an empty draft cannot run.",
            "inputSchema":mcp_object_schema(json!({"tenant_id":{"type":"string","minLength":1},"run_id":{"type":"string","minLength":1}}), &["tenant_id","run_id"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_lab_edit", "title":"Edit typed research steps",
            "description":"Apply atomic stateless step/source/port/parameter changes and return the full new recipe plus preview or actionable validation problem. No auto-connections; provider parameters retain their exact owner schema. A draft is never an executable grant.",
            "inputSchema":mcp_object_schema(json!({
                "recipe":mcp_recipe_schema(),
                "changes":{"type":"array","minItems":1,"maxItems":128,"items":mcp_edit_schema()}
            }), &["recipe","changes"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_lab_check", "title":"Validate a research recipe",
            "description":"Check the exact installed owner contracts, typed ports, graph dependencies and local parameter DTOs via the existing V2 DAG validator. Produces only a non-admitted preview.",
            "inputSchema":mcp_object_schema(json!({"recipe":mcp_recipe_schema()}), &["recipe"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_lab_diff", "title":"Compare research recipe hypotheses",
            "description":"Compare two validated recipes by pinned graph and typed parameter hashes; no live provider data or measured SEO outcomes.",
            "inputSchema":mcp_object_schema(json!({"first":mcp_recipe_schema(),"second":mcp_recipe_schema()}), &["first","second"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_lab_connections", "title":"Find compatible typed step connections",
            "description":"Suggest candidate source and upstream output bindings for every step input port by exact schema, excluding immediate dependency cycles. Suggestions are not a validated recipe or an execution grant.",
            "inputSchema":mcp_object_schema(json!({"recipe":mcp_recipe_schema()}), &["recipe"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_lab_inspect", "title":"Inspect a saved typed execution receipt",
            "description":"Locally check a complete sealed V2 plan and receipt against trusted contracts. Input typed JSON objects directly, never filesystem paths. Returns hashes, step diagnostics and explicit unverified Temporal/provider/source attestation; no payloads.",
            "inputSchema":mcp_saved_execution_schema(),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_lab_compare_results", "title":"Compare two saved execution receipts",
            "description":"Locally inspect and compare two distinct typed V2 plan/receipt pairs, only with equal tenant and exact original source bytes. Hash-only structural differences, not measured SEO or Temporal History attestation. No file paths.",
            "inputSchema":mcp_object_schema(json!({"first":mcp_saved_execution_schema(),"second":mcp_saved_execution_schema()}), &["first","second"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_lab_evidence", "title":"Assess independent consumer needs evidence",
            "description":"Group typed consumer-need observations and site hypotheses; count only distinct claimed primary publisher/document/content sources, never SERP snippets or mirrors. Structural candidate only: no provider, source or Temporal attestation.",
            "inputSchema":mcp_need_evidence_schema(),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        }
    ]})
}

fn mcp_bad_arguments(_error: impl std::fmt::Display) -> Value {
    json!({
        "status":"invalid", "code":"invalid_tool_arguments",
        "message":"Tool arguments are not compatible with the declared typed schema.",
        "next_action":"Use this MCP tool's inputSchema and the installed owner parameter schema."
    })
}

fn mcp_site_lab_authoring_payload_too_large(payload: &impl serde::Serialize) -> bool {
    serde_json::to_vec(payload)
        .map_or(true, |bytes| bytes.len() > SITE_LAB_RECIPE_MAX_BYTES as usize)
}

fn mcp_site_lab_authoring_envelope_too_large(payload: &impl serde::Serialize) -> bool {
    serde_json::to_vec(payload)
        .map_or(true, |bytes| bytes.len() > SITE_LAB_MCP_AUTHORING_MAX_BYTES)
}

fn mcp_site_lab_authoring_problem() -> Value {
    site_lab_problem(
        "authoring_payload_too_large",
        "Site Lab authoring input exceeds the bounded recipe and edit budget.",
        "Limit each recipe to 256 KiB, each edit batch to 128 changes and split large experiments into separate recipes.",
    )
}

fn mcp_site_lab_scope_id_is_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= SITE_LAB_MCP_MAX_SCOPE_ID_BYTES
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn mcp_edit_preview_status(preview_status: &str) -> &'static str {
    match preview_status {
        "valid_preview_only" => "draft_validated",
        "pending_owner_parameter_validation" => "draft_pending_owner_parameter_validation",
        // An unknown future preview state must not be upgraded to validated.
        _ => "draft_unverified",
    }
}

fn mcp_site_lab_call(name: &str, args: Value, catalog: &seo_application::site_lab::SiteLabCatalog) -> (Value, bool) {
    use seo_application::site_lab::{
        compare_executions, describe_tools, diff_recipes, inspect_execution, preview_recipe,
        preview_consumer_need_evidence, revise_recipe, suggest_compatible_inputs, SiteRecipe,
    };

    // `tools/list` declares limits for model convenience, but the MCP caller
    // is untrusted and can bypass every JSON Schema annotation. This protects
    // the edit/diff/evidence paths without changing the larger saved-result
    // envelope or granting any path/Temporal/provider capability.
    if matches!(name,
        "site_lab_operation" | "site_lab_one_step" | "site_lab_new" |
        "site_lab_edit" | "site_lab_check" | "site_lab_diff" |
        "site_lab_connections" | "site_lab_evidence"
    ) && mcp_site_lab_authoring_envelope_too_large(&args) {
        return (mcp_site_lab_authoring_problem(), true);
    }
    // RecipeChange deliberately has no Serialize implementation. Bound the
    // client's original JSON edit array before deserializing it into the
    // strongly typed, deserialize-only edit commands.
    if name == "site_lab_edit"
        && args
            .get("changes")
            .is_some_and(|changes| mcp_site_lab_authoring_payload_too_large(changes))
    {
        return (mcp_site_lab_authoring_problem(), true);
    }

    match name {
        "site_lab_tools" => {
            let parsed: std::result::Result<serde_json::Map<String, Value>, _> =
                serde_json::from_value(if args.is_null() { json!({}) } else { args });
            match parsed {
                Ok(fields) if fields.is_empty() => match describe_tools(catalog) {
                    Ok(operations) => (json!({"status":"catalog_preview_only","operations":operations,"execution":"not_started"}), false),
                    Err(error) => (json!(error.problem()), true),
                },
                _ => (mcp_bad_arguments("site_lab_tools takes an empty arguments object"), true),
            }
        }
        "site_lab_operation" => match serde_json::from_value::<McpOneOperation>(args) {
            Ok(input) if mcp_site_lab_authoring_payload_too_large(&input.operation)
                || input.parameters.as_ref().is_some_and(mcp_site_lab_authoring_payload_too_large) =>
            {
                (site_lab_problem("operation_settings_too_large", "Operation identifier or settings exceed the authoring limit.", "Use a bounded versioned operation reference and at most 256 KiB of JSON settings."), true)
            }
            Ok(input) => match site_lab_operation_inspection(
                catalog,
                &input.operation,
                input.parameters.as_ref(),
            ) {
                Ok(inspection) => (inspection, false),
                Err(problem) => (problem, true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_lab_one_step" => match serde_json::from_value::<McpOneRecipe>(args) {
            Ok(input) if mcp_site_lab_authoring_payload_too_large(&input.recipe) =>
            {
                (site_lab_problem("one_step_recipe_too_large", "One-step recipe exceeds the 256 KiB authoring limit.", "Reduce the recipe input before checking this operation."), true)
            }
            Ok(input) => match site_lab_one_step_preview(&input.recipe, catalog) {
                Ok(preview) => (preview, false),
                Err(problem) => (problem, true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_lab_new" => match serde_json::from_value::<McpNewRecipe>(args) {
            Ok(input) if mcp_site_lab_scope_id_is_valid(&input.tenant_id)
                && mcp_site_lab_scope_id_is_valid(&input.run_id) => {
                let recipe = SiteRecipe {
                    schema_revision: seo_application::site_lab::SITE_RECIPE_SCHEMA_REVISION,
                    scope: seo_application::site_lab::RunScope {tenant_id:input.tenant_id,run_id:input.run_id},
                    sources: Default::default(), steps: Vec::new(),
                };
                (json!({"status":"draft_unchecked","recipe":recipe,"execution":"not_started"}), false)
            }
            Ok(_) => (mcp_bad_arguments("tenant_id and run_id must be bounded identifiers"), true),
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_lab_edit" => match serde_json::from_value::<McpEditRecipe>(args) {
            Ok(input) if input.changes.len() > SITE_LAB_MCP_MAX_RECIPE_EDITS
                || mcp_site_lab_authoring_payload_too_large(&input.recipe) =>
            {
                (mcp_site_lab_authoring_problem(), true)
            }
            Ok(input) => match revise_recipe(&input.recipe, &input.changes) {
                Ok(recipe) if mcp_site_lab_authoring_payload_too_large(&recipe) =>
                    (mcp_site_lab_authoring_problem(), true),
                Ok(recipe) => match preview_recipe(&recipe, catalog) {
                    Ok(preview) => (json!({"status":mcp_edit_preview_status(preview.status),"recipe":recipe,"preview":preview}), false),
                    Err(error) => (json!({"status":"draft_needs_changes","recipe":recipe,"problem":site_lab_safe_recipe_problem(&error),"execution":"not_started"}), false),
                },
                Err(_) => (json!({"status":"invalid","code":"recipe_edit_rejected","message":"The requested edit cannot be applied to this versioned recipe.","next_action":"Fix the referenced step or batch; no partial edits were applied."}), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_lab_check" => match serde_json::from_value::<McpOneRecipe>(args) {
            Ok(input) if mcp_site_lab_authoring_payload_too_large(&input.recipe) =>
                (mcp_site_lab_authoring_problem(), true),
            Ok(input) => match preview_recipe(&input.recipe, catalog) {
                Ok(preview) => (json!(preview), false),
                Err(error) => (site_lab_safe_recipe_problem(&error), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_lab_diff" => match serde_json::from_value::<McpTwoRecipes>(args) {
            Ok(input) if mcp_site_lab_authoring_payload_too_large(&input.first)
                || mcp_site_lab_authoring_payload_too_large(&input.second) =>
            {
                (mcp_site_lab_authoring_problem(), true)
            }
            Ok(input) => match diff_recipes(&input.first, &input.second, catalog) {
                Ok(diff) => (json!(diff), false),
                Err(error) => (site_lab_safe_recipe_problem(&error), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_lab_connections" => match serde_json::from_value::<McpOneRecipe>(args) {
            Ok(input) if mcp_site_lab_authoring_payload_too_large(&input.recipe) =>
                (mcp_site_lab_authoring_problem(), true),
            Ok(input) => match suggest_compatible_inputs(&input.recipe, catalog) {
                Ok(preview) => (json!(preview), false),
                Err(error) => (site_lab_safe_recipe_problem(&error), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_lab_inspect" => match serde_json::from_value::<McpSavedExecution>(args) {
            Ok(input) => match inspect_execution(&input.plan, &input.receipt, catalog.catalog()) {
                Ok(inspection) => (json!(inspection), false),
                Err(error) => (json!(error.problem()), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_lab_compare_results" => match serde_json::from_value::<McpCompareExecutions>(args) {
            Ok(input) => match compare_executions(
                &input.first.plan, &input.first.receipt,
                &input.second.plan, &input.second.receipt, catalog.catalog(),
            ) {
                Ok(comparison) => (json!(comparison), false),
                Err(error) => (json!(error.problem()), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_lab_evidence" => match serde_json::from_value::<McpNeedEvidence>(args) {
            Ok(input) if mcp_site_lab_authoring_payload_too_large(&input.batch) =>
                (mcp_site_lab_authoring_problem(), true),
            Ok(input) => match preview_consumer_need_evidence(&input.batch) {
                Ok(assessment) => (json!(assessment), false),
                Err(error) => (json!({
                    "status":"invalid", "code":"consumer_evidence_invalid",
                    "message":"Consumer evidence does not satisfy the installed typed source, hash or scope contract.",
                    "next_action":"Correct the typed source identities, hashes, tenant scope or bounded observations. Separate source/owner attestation remains required."
                }), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        _ => (json!({"status":"invalid","code":"tool_unavailable","message":"Unknown Site Lab MCP tool","next_action":"Call tools/list to discover available tools."}), true),
    }
}

fn mcp_tool_result(data: Value, is_error: bool) -> Value {
    let text = data.to_string();
    json!({
        "content":[{"type":"text","text":text}],
        "structuredContent":data,
        "isError":is_error
    })
}

fn mcp_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn mcp_reply(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

/// One JSON-RPC message in, optionally one message out. Notifications produce
/// no response. No network/file IO is reachable through this dispatcher.
fn mcp_site_lab_message(request: Value, catalog: &seo_application::site_lab::SiteLabCatalog) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str);
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || method.is_none() {
        return Some(mcp_error(id.unwrap_or(Value::Null), -32600, "Invalid JSON-RPC request"));
    }
    // Clients send initialization and other notifications without an id.
    let id = id?;
    let result = match method? {
        "initialize" => {
            json!({"protocolVersion":SITE_LAB_MCP_PROTOCOL,
                "capabilities":{"tools":{"listChanged":false}},
                "serverInfo":{"name":"alegria-site-lab","version":env!("CARGO_PKG_VERSION")},
                "instructions":"Stateless recipe authoring and offline inspection of typed saved V2 receipts only. No Temporal Start, History attestation, provider effects or admission is exposed."})
        }
        "ping" => json!({}),
        "tools/list" => mcp_site_lab_tools(),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or(Value::Null);
            return Some(match serde_json::from_value::<McpCall>(params) {
                Ok(call) => {
                    let (data, is_error) = mcp_site_lab_call(&call.name, call.arguments, catalog);
                    mcp_reply(id, mcp_tool_result(data, is_error))
                }
                Err(_) => mcp_error(id, -32602, "Invalid tools/call parameters"),
            });
        }
        _ => return Some(mcp_error(id, -32601, "Method not found")),
    };
    Some(mcp_reply(id, result))
}

fn mcp_bounded_line<R: std::io::BufRead>(input: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let end = available.iter().position(|byte| *byte == b'\n');
        let take = end.map_or(available.len(), |position| position + 1);
        if line.len().saturating_add(take) > SITE_LAB_MCP_MAX_MESSAGE_BYTES {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "MCP message exceeds 3 MiB boundary"));
        }
        line.extend_from_slice(&available[..take]);
        input.consume(take);
        if end.is_some() {
            return Ok(Some(line));
        }
    }
}

fn serve_site_lab_mcp<R: std::io::BufRead, W: std::io::Write>(
    mut input: R,
    mut output: W,
    catalog: &seo_application::site_lab::SiteLabCatalog,
) -> Result<()> {
    while let Some(line) = mcp_bounded_line(&mut input)? {
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let response = match serde_json::from_slice::<Value>(&line) {
            Ok(request) => mcp_site_lab_message(request, catalog),
            Err(_) => Some(mcp_error(Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

fn mcp_site_lab_server() -> Result<i32> {
    let catalog = site_lab_installed_catalog()?;
    serve_site_lab_mcp(std::io::stdin().lock(), std::io::stdout().lock(), &catalog)?;
    Ok(0)
}

#[cfg(test)]
mod site_lab_mcp_tests {
    use super::*;

    fn catalog() -> seo_application::site_lab::SiteLabCatalog {
        let registry = seo_application::operation_adapter::semantic_extraction_registry().unwrap();
        seo_application::site_lab::installed_semantic_lab_catalog(registry.catalog().clone()).unwrap()
    }

    fn tool_call(name: &str, arguments: Value) -> Value {
        mcp_site_lab_message(json!({
            "jsonrpc":"2.0", "id":9, "method":"tools/call",
            "params":{"name":name,"arguments":arguments}
        }), &catalog()).unwrap()
    }

    fn forged_saved_execution() -> Value {
        json!({
            "plan":{
                "schema_revision":2,
                "scope":{"tenant_id":"tenant","run_id":"forged-run"},
                "allowed_effects":[],
                "external_artifacts":{},
                "pinned_contracts":[],
                "nodes":[],
                "hash":"forged-hash"
            },
            "receipt":{
                "plan_hash":"forged-hash",
                "order":[],
                "artifacts":{},
                "produced_evidence":{},
                "owner_results":{}
            }
        })
    }

    #[test]
    fn mcp_initialization_and_catalog_are_protocol_clean() {
        let lines = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"site_lab_tools\",\"arguments\":{}}}\n"
        );
        let mut output = Vec::new();
        serve_site_lab_mcp(std::io::Cursor::new(lines), &mut output, &catalog()).unwrap();
        let replies: Vec<Value> = output.split(|byte| *byte == b'\n').filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap()).collect();
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["result"]["capabilities"]["tools"]["listChanged"], false);
        assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 11);
        assert!(replies[1]["result"].to_string().len() < 24_576, "tools/list should fit a compact model context");
        for name in [
            "site_lab_operation",
            "site_lab_one_step",
            "site_lab_connections",
            "site_lab_inspect",
            "site_lab_compare_results",
            "site_lab_evidence",
        ] {
            assert!(replies[1]["result"]["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == name));
        }
        assert_eq!(replies[2]["result"]["structuredContent"]["operations"].as_array().unwrap().len(), 5);
        assert_eq!(replies[2]["result"]["isError"], false);
    }

    #[test]
    fn mcp_connections_stays_a_non_executable_hint() {
        let draft = tool_call("site_lab_new", json!({"tenant_id":"tenant","run_id":"draft"}));
        let recipe = draft["result"]["structuredContent"]["recipe"].clone();
        let hint = tool_call("site_lab_connections", json!({"recipe":recipe}));
        assert_eq!(hint["result"]["isError"], false);
        assert_eq!(hint["result"]["structuredContent"]["status"], "schema_candidates_only");
        assert_eq!(hint["result"]["structuredContent"]["execution"], "not_started");
        assert!(hint["result"]["structuredContent"]["ports"].as_array().unwrap().is_empty());
    }

    #[test]
    fn mcp_operation_reports_exact_installed_metadata_without_execution() {
        let response = tool_call("site_lab_operation", json!({
            "operation":{"id":"semantic.entity_spans","revision":1},
            "parameters":{}
        }));
        let result = &response["result"]["structuredContent"];
        assert_eq!(response["result"]["isError"], false);
        assert_eq!(result["status"], "registered_operation_inspection_only");
        assert_eq!(
            result["tool"]["operation"]["id"],
            "semantic.entity_spans"
        );
        assert_eq!(result["execution"], "not_started");
        assert_eq!(result["provider_connectivity"], "not_checked");
        assert_eq!(
            result["checked_parameters"]["verification"],
            "typed_local"
        );

        let unknown = tool_call("site_lab_operation", json!({
            "operation":{"id":"unregistered_secret-SENTINEL","revision":99},
            "parameters":{}
        }));
        assert_eq!(
            unknown["result"]["structuredContent"]["code"],
            "operation_unavailable"
        );
        assert_eq!(unknown["result"]["isError"], true);
        assert!(!unknown.to_string().contains("unregistered_secret-SENTINEL"));
    }

    #[test]
    fn mcp_one_step_is_a_native_dag_check_not_a_hidden_runner() {
        let recipe = json!({
            "schema_revision":2,
            "scope":{"tenant_id":"lab","run_id":"test-single"},
            "sources":{"mentions":{"schema":{"id":"semantic.entity_span_input","revision":1},"provenance":"unverified:test"}},
            "steps":[{"id":"extract","operation":{"id":"semantic.entity_spans","revision":1},
                "inputs":{"input":{"source":"external","artifact":"mentions"}},"parameters":{}}]
        });
        let checked = tool_call("site_lab_one_step", json!({"recipe":recipe.clone()}));
        let report = &checked["result"]["structuredContent"];
        assert_eq!(checked["result"]["isError"], false);
        assert_eq!(report["status"], "single_step_preview_only");
        assert_eq!(report["execution"], "not_started");
        assert_eq!(report["owner_validate_only"], "required_not_run");
        assert_eq!(report["operator_admission"], "not_issued");
        assert_eq!(report["temporal_history_attestation"], "not_checked");

        let mut empty = recipe.clone();
        empty["steps"] = json!([]);
        let rejected = tool_call("site_lab_one_step", json!({"recipe":empty}));
        assert_eq!(rejected["result"]["isError"], true);
        assert_eq!(
            rejected["result"]["structuredContent"]["code"],
            "one_step_required"
        );

        let mut secret_settings = recipe;
        secret_settings["steps"][0]["parameters"] = json!({"provider_token":"SENTINEL-PRIVATE-KEY"});
        let bad = tool_call("site_lab_one_step", json!({"recipe":secret_settings}));
        assert_eq!(bad["result"]["isError"], true);
        assert!(!bad.to_string().contains("SENTINEL-PRIVATE-KEY"));
    }

    #[test]
    fn mcp_individual_operation_limits_authored_parameters_to_256_kib() {
        let too_large = tool_call("site_lab_operation", json!({
            "operation":{"id":"semantic.entity_spans","revision":1},
            "parameters":{"untrusted_value":"x".repeat(256 * 1024)}
        }));
        assert_eq!(too_large["result"]["isError"], true);
        assert_eq!(
            too_large["result"]["structuredContent"]["code"],
            "operation_settings_too_large"
        );
        assert!(too_large.to_string().len() < 2048);
    }

    #[test]
    fn mcp_all_authoring_paths_enforce_bounded_inputs_before_processing() {
        let huge = "SENTINEL-PRIVATE-JSON".repeat(28 * 1024);
        for (name, arguments) in [
            ("site_lab_new", json!({"tenant_id":"lab","run_id":huge.clone()})),
            ("site_lab_check", json!({"recipe":{"extra":huge.clone()}})),
            ("site_lab_diff", json!({"first":{"extra":huge.clone()},"second":{}})),
            ("site_lab_connections", json!({"recipe":{"extra":huge.clone()}})),
            ("site_lab_evidence", json!({"batch":{"extra":huge}})),
        ] {
            let response = tool_call(name, arguments);
            assert_eq!(response["result"]["isError"], true, "{name}");
            assert_eq!(
                response["result"]["structuredContent"]["code"],
                "authoring_payload_too_large",
                "{name}"
            );
            assert!(!response.to_string().contains("SENTINEL-PRIVATE-JSON"));
        }
    }

    #[test]
    fn mcp_edit_enforces_change_count_and_never_echoes_failing_step_values() {
        let draft = tool_call("site_lab_new", json!({"tenant_id":"lab","run_id":"draft"}));
        let recipe = draft["result"]["structuredContent"]["recipe"].clone();
        let changes: Vec<Value> = (0..129)
            .map(|_| json!({"action":"remove_step","step":"absent"}))
            .collect();
        let oversized = tool_call("site_lab_edit", json!({"recipe":recipe.clone(),"changes":changes}));
        assert_eq!(oversized["result"]["isError"], true);
        assert_eq!(oversized["result"]["structuredContent"]["code"], "authoring_payload_too_large");

        let sensitive = "SENTINEL-PRIVATE-STEP-VALUE";
        let failed = tool_call("site_lab_edit", json!({"recipe":recipe,"changes":[
            {"action":"remove_step","step":sensitive}
        ]}));
        assert_eq!(failed["result"]["structuredContent"]["code"], "recipe_edit_rejected");
        assert!(!failed.to_string().contains(sensitive));
    }

    #[test]
    fn mcp_recipe_errors_keep_typed_codes_without_leaking_author_supplied_ids() {
        let sensitive = "SENTINEL-PRIVATE-OPERATION-VALUE";
        let invalid_recipe = json!({
            "schema_revision":2,
            "scope":{"tenant_id":"tenant","run_id":"draft"},
            "sources":{},
            "steps":[{"id":"extract","operation":{"id":sensitive,"revision":1},"inputs":{}}]
        });
        for (name, arguments) in [
            ("site_lab_check", json!({"recipe":invalid_recipe.clone()})),
            ("site_lab_connections", json!({"recipe":invalid_recipe})),
        ] {
            let response = tool_call(name, arguments);
            assert_eq!(response["result"]["isError"], true);
            assert_eq!(response["result"]["structuredContent"]["code"], "operation_unavailable");
            assert!(!response.to_string().contains(sensitive));
        }
    }

    #[test]
    fn mcp_scope_identifiers_are_bounded_and_do_not_echo_invalid_inputs() {
        let sensitive = "SENTINEL-PRIVATE-SCOPE".repeat(12);
        let invalid = tool_call("site_lab_new", json!({"tenant_id":sensitive,"run_id":"draft"}));
        assert_eq!(invalid["result"]["isError"], true);
        assert_eq!(invalid["result"]["structuredContent"]["code"], "invalid_tool_arguments");
        assert!(!invalid.to_string().contains("SENTINEL-PRIVATE-SCOPE"));
    }

    fn evidence_observation(id: &str, kind: &str, origin: &str, document: &str) -> Value {
        let excerpt = format!("synthetic evidence for {id}");
        json!({
            "observation_id":id,"need_key":"price-clarity",
            "asserted_need":"Know final price", "context":"Synthetic test evidence",
            "excerpt":excerpt, "excerpt_hash":primitives::hash::blake3_hex(excerpt.as_bytes()),
            "source":{
                "source_id":id, "tenant_id":"tenant-a", "source_run_id":"source-fixture",
                "retrieval_channel":"synthetic-test", "representation":kind,
                "primary_origin_id":origin,"primary_document_id":document,
                "provenance_locator":format!("synthetic://test/{id}"),
                "document_payload_hash":primitives::hash::blake3_hex(document.as_bytes())
            }
        })
    }

    #[test]
    fn mcp_need_evidence_never_claims_attested_source_or_site_outcome() {
        let batch = json!({
            "schema_revision":1,
            "scope":{"tenant_id":"tenant-a","run_id":"research-one"},
            "observations":[
                evidence_observation("serp-a", "search_snippet", "origin-a", "doc-a"),
                evidence_observation("page-a", "primary_document", "origin-a", "doc-a")
            ],
            "hypotheses":[{"need_key":"price-clarity","proposed_site_change":"Show full pricing","expected_benefit":"Clearer comparison"}]
        });
        let preview = tool_call("site_lab_evidence", json!({"batch":batch}));
        assert_eq!(preview["result"]["isError"], false);
        assert_eq!(preview["result"]["structuredContent"]["status"], "structural_only_unattested");
        assert_eq!(preview["result"]["structuredContent"]["assessments"][0]["status"], "insufficient_evidence");
        assert_eq!(preview["result"]["structuredContent"]["assessments"][0]["source_authentication"], "not_checked");
        assert!(!preview.to_string().contains("synthetic evidence for page-a"));
    }

    #[test]
    fn mcp_edit_never_marks_pending_remote_owner_validation_as_validated() {
        assert_eq!(mcp_edit_preview_status("valid_preview_only"), "draft_validated");
        assert_eq!(
            mcp_edit_preview_status("pending_owner_parameter_validation"),
            "draft_pending_owner_parameter_validation"
        );
        assert_eq!(mcp_edit_preview_status("unknown_future_owner_state"), "draft_unverified");
    }

    #[test]
    fn mcp_receipt_rejects_forgery_without_claiming_temporal_attestation() {
        let saved = forged_saved_execution();
        let inspected = tool_call("site_lab_inspect", saved.clone());
        assert_eq!(inspected["result"]["isError"], true);
        assert_eq!(inspected["result"]["structuredContent"]["code"], "saved_plan_invalid");

        let compared = tool_call("site_lab_compare_results", json!({"first":saved,"second":forged_saved_execution()}));
        assert_eq!(compared["result"]["isError"], true);
        assert_eq!(compared["result"]["structuredContent"]["code"], "saved_plan_invalid");
        assert!(!compared.to_string().contains("same_source_bytes_structural_result_comparison"));
    }

    #[test]
    fn mcp_receipt_arguments_reject_paths_unknown_fields_and_secret_reflection() {
        for name in ["site_lab_inspect", "site_lab_compare_results"] {
            let path = tool_call(name, json!({"plan_file":"/etc/shadow","receipt_file":"/private/input"}));
            assert_eq!(path["result"]["structuredContent"]["code"], "invalid_tool_arguments");
            assert!(!path.to_string().contains("/etc/shadow"));

            let mut forged = forged_saved_execution();
            forged["secret_from_untrusted_client"] = json!("SENTINEL-CREDENTIAL-DO-NOT-LOG");
            let invalid = if name == "site_lab_inspect" {
                tool_call(name, forged)
            } else {
                tool_call(name, json!({"first":forged,"second":forged_saved_execution()}))
            };
            assert_eq!(invalid["result"]["isError"], true);
            assert_eq!(invalid["result"]["structuredContent"]["code"], "invalid_tool_arguments");
            assert!(!invalid.to_string().contains("SENTINEL-CREDENTIAL-DO-NOT-LOG"));
        }
    }

    #[test]
    fn mcp_edit_returns_draft_and_real_validation_errors() {
        let new = mcp_site_lab_message(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"site_lab_new","arguments":{"tenant_id":"lab","run_id":"test"}}}), &catalog()).unwrap();
        let recipe = new["result"]["structuredContent"]["recipe"].clone();
        assert_eq!(recipe["steps"].as_array().unwrap().len(), 0);
        let edit = mcp_site_lab_message(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"site_lab_edit","arguments":{"recipe":recipe,"changes":[
                {"action":"add_step","step":{"id":"extract","operation":{"id":"semantic.entity_spans","revision":1},"inputs":{}}}
            ]}}}), &catalog()).unwrap();
        assert_eq!(edit["result"]["structuredContent"]["status"], "draft_needs_changes");
        assert_eq!(edit["result"]["structuredContent"]["recipe"]["steps"][0]["id"], "extract");
        assert_eq!(edit["result"]["isError"], false);
        let invalid = mcp_site_lab_message(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
            "name":"site_lab_edit","arguments":{"recipe":edit["result"]["structuredContent"]["recipe"],"changes":[
                {"action":"add_step","step":{"id":"extract","operation":{"id":"semantic.entity_spans","revision":1},"inputs":{}}}
            ]}}}), &catalog()).unwrap();
        assert_eq!(invalid["result"]["structuredContent"]["code"], "recipe_edit_rejected");
        assert_eq!(invalid["result"]["isError"], true);
    }

    #[test]
    fn mcp_rejects_unknown_methods_and_oversized_lines() {
        let unavailable = mcp_site_lab_message(json!({"jsonrpc":"2.0","id":8,"method":"execution/start"}), &catalog()).unwrap();
        assert_eq!(unavailable["error"]["code"], -32601);
        let oversized = vec![b'a'; SITE_LAB_MCP_MAX_MESSAGE_BYTES + 1];
        assert!(mcp_bounded_line(&mut std::io::Cursor::new(oversized)).is_err());
    }
}
