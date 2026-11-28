// Native stdio MCP adapter for stateless authoring, offline receipt
// inspection and a strictly read-only, separately credentialed native V2
// operator status bridge. No provider calls, Temporal Start, arbitrary file
// reads, dynamic manifest loading or second DAG runner are reachable here.

const SITE_ARCHITECT_MCP_PROTOCOL: &str = "2025-11-25";
// Two sealed plans and two V2 receipts may each reach the existing 512 KiB
// DAG boundary; allow envelope overhead, without accepting unbounded input.
const SITE_ARCHITECT_MCP_MAX_MESSAGE_BYTES: usize = 3 * 1024 * 1024;
// Recipe authoring is much smaller than the transport envelope needed for
// comparing two already bounded saved V2 receipts. Enforce the distinction
// on the server, not just in the advertised MCP JSON Schema.
const SITE_ARCHITECT_MCP_AUTHORING_MAX_BYTES: usize = 2 * SITE_ARCHITECT_RECIPE_MAX_BYTES as usize + 4096;
const SITE_ARCHITECT_MCP_MAX_RECIPE_EDITS: usize = 128;
const SITE_ARCHITECT_MCP_MAX_SCOPE_ID_BYTES: usize = 128;
const SITE_ARCHITECT_MCP_OPERATOR_REPLY_MAX_BYTES: u64 = 4096;
const SITE_ARCHITECT_MCP_OPERATOR_REQUEST_MAX_BYTES: usize = 256;

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct McpRunStatus {
    canonical_run_id: uuid::Uuid,
}

/// The private reader sends a redacted, bounded status—not a provider receipt.
/// Keep a strict wire DTO: a compromised transport must not smuggle arbitrary
/// strings or fields into the model's conversation.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpOwnerRunReport {
    operation: String,
    status: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    canonical_run_id: Option<uuid::Uuid>,
    #[serde(default)]
    parent_workflow_run_id: Option<String>,
    #[serde(default)]
    plan_hash: Option<String>,
    #[serde(default)]
    parent_start_sha256: Option<String>,
    #[serde(default)]
    step_count: Option<usize>,
    #[serde(default)]
    owner_result_count: Option<usize>,
    #[serde(default)]
    result_blake3: Option<String>,
    #[serde(default)]
    governance_currently_active: Option<bool>,
    #[serde(default)]
    temporal_history_attestation: Option<String>,
    #[serde(default)]
    owner_receipt_attestation: Option<String>,
    #[serde(default)]
    source_authentication: Option<String>,
    next_action: String,
}

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

/// A trusted one-step draft scaffold. The author supplies only identity and
/// the exact installed operation reference; input source identities, owner
/// routes and provider parameters cannot be invented by this helper.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpPrepareStep {
    tenant_id: String,
    run_id: String,
    step_id: String,
    operation: seo_application::site_architect::OperationRef,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpOneRecipe {
    recipe: seo_application::site_architect::SiteRecipe,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpOneOperation {
    operation: seo_application::site_architect::OperationRef,
    #[serde(default)]
    parameters: Option<seo_application::site_architect::RecipeParameters>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpEditRecipe {
    recipe: seo_application::site_architect::SiteRecipe,
    changes: Vec<seo_application::site_architect::RecipeChange>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpTwoRecipes {
    first: seo_application::site_architect::SiteRecipe,
    second: seo_application::site_architect::SiteRecipe,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpSavedExecution {
    plan: seo_application::site_architect::ExecutionPlan,
    receipt: seo_application::site_architect::OperationDagReceiptV1,
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
    batch: seo_application::site_architect::ConsumerNeedEvidenceBatchV1,
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
    mcp_object_schema(
        json!({
            "schema": mcp_operation_schema(),
            "provenance":{"type":"string"}
        }),
        &["schema", "provenance"],
    )
}

fn mcp_step_schema() -> Value {
    mcp_object_schema(
        json!({
            "id":{"type":"string","minLength":1},
            "operation":mcp_operation_schema(),
            "inputs":{"type":"object","additionalProperties":mcp_input_schema()},
            "parameters":{"type":"object","description":"Owner schema in site_architect_tools"},
            "after":{"type":"array","uniqueItems":true,"items":{"type":"string"}}
        }),
        &["id", "operation", "inputs"],
    )
}

fn mcp_recipe_schema() -> Value {
    mcp_object_schema(
        json!({
            "schema_revision":{"const":2},
            "scope":mcp_object_schema(json!({
                "tenant_id":{"type":"string","minLength":1},
                "run_id":{"type":"string","minLength":1}
            }), &["tenant_id","run_id"]),
            "sources":{"type":"object","additionalProperties":mcp_source_schema()},
            "steps":{"type":"array","items":mcp_step_schema()}
        }),
        &["schema_revision", "scope", "steps"],
    )
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
    mcp_object_schema(
        json!({
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
        }),
        &["plan", "receipt"],
    )
}

fn mcp_need_evidence_schema() -> Value {
    // This schema is for MCP discovery; strongly typed, deny-unknown-fields
    // Rust DTOs and strict size/hash/scope checks remain authoritative.
    let source = mcp_object_schema(
        json!({
            "source_id":{"type":"string"}, "tenant_id":{"type":"string"},
            "source_run_id":{"type":"string"}, "retrieval_channel":{"type":"string"},
            "representation":{"enum":["primary_document","search_snippet","derived_summary","mirrored_document"]},
            "primary_origin_id":{"type":"string"}, "primary_document_id":{"type":"string"},
            "provenance_locator":{"type":"string"}, "document_payload_hash":{"type":"string"}
        }),
        &[
            "source_id",
            "tenant_id",
            "source_run_id",
            "retrieval_channel",
            "representation",
            "primary_origin_id",
            "primary_document_id",
            "provenance_locator",
            "document_payload_hash",
        ],
    );
    let observation = mcp_object_schema(
        json!({
            "observation_id":{"type":"string"}, "need_key":{"type":"string"},
            "asserted_need":{"type":"string"}, "context":{"type":"string"},
            "excerpt":{"type":"string"}, "excerpt_hash":{"type":"string"}, "source":source
        }),
        &[
            "observation_id",
            "need_key",
            "asserted_need",
            "context",
            "excerpt",
            "excerpt_hash",
            "source",
        ],
    );
    let hypothesis = mcp_object_schema(
        json!({
            "need_key":{"type":"string"}, "proposed_site_change":{"type":"string"},
            "expected_benefit":{"type":"string"}
        }),
        &["need_key", "proposed_site_change", "expected_benefit"],
    );
    mcp_object_schema(
        json!({"batch":mcp_object_schema(json!({
        "schema_revision":{"const":1},
        "scope":mcp_object_schema(json!({"tenant_id":{"type":"string"},"run_id":{"type":"string"}}), &["tenant_id","run_id"]),
        "observations":{"type":"array","maxItems":128,"items":observation},
        "hypotheses":{"type":"array","minItems":1,"maxItems":32,"items":hypothesis}
    }), &["schema_revision","scope","observations","hypotheses"])}),
        &["batch"],
    )
}

fn mcp_site_architect_tools() -> Value {
    json!({"tools":[
        {
            "name":"site_architect_tools", "title":"Available siteArchitect operations",
            "description":"Product action matrix by area and exact revision, plus installed owner contracts, typed ports, settings and readiness.",
            "inputSchema":mcp_object_schema(json!({}), &[]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_architect_operation", "title":"Inspect one installed operation",
            "description":"Versioned ports, settings schema and local/owner-only validation status. No provider call or execution; settings never echoed.",
            "inputSchema":mcp_object_schema(json!({
                "operation":mcp_operation_schema(),
                "parameters":{"type":"object","description":"Owner settings; omit to inspect only"}
            }), &["operation"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_architect_one_step", "title":"Check one operation as a Temporal recipe",
            "description":"Typed single-step DAG preview, not admission. User ACL, one-use grant, sources, owner ValidateOnly and History unverified.",
            "inputSchema":mcp_object_schema(json!({"recipe":mcp_recipe_schema()}), &["recipe"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_architect_run_status", "title":"Read an independently authorized Temporal operation status",
            "description":"Canonical UUID read-only status via preconfigured tenant-bound Unix operator. Checks server Temporal History, not provider receipt/source; no start/retry.",
            "inputSchema":mcp_object_schema(json!({
                "canonical_run_id":{"type":"string","format":"uuid","minLength":36,"maxLength":36}
            }), &["canonical_run_id"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_architect_new", "title":"Create a blank research recipe",
            "description":"Create a blank scoped recipe; drafts cannot run.",
            "inputSchema":mcp_object_schema(json!({"tenant_id":{"type":"string","minLength":1},"run_id":{"type":"string","minLength":1}}), &["tenant_id","run_id"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_architect_prepare_step", "title":"Prepare a typed one-step draft",
            "description":"Select an exact trusted operation contract; return an unbound one-step draft and versioned required input artifact identities. No owner DTO validation, source or Start is invented.",
            "inputSchema":mcp_object_schema(json!({
                "tenant_id":{"type":"string","minLength":1,"maxLength":128},
                "run_id":{"type":"string","minLength":1,"maxLength":128},
                "step_id":{"type":"string","minLength":1,"maxLength":64},
                "operation":mcp_operation_schema()
            }), &["tenant_id","run_id","step_id","operation"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_architect_edit", "title":"Edit typed research steps",
            "description":"Atomically edit typed recipe and preview; no auto-connections or executable grant.",
            "inputSchema":mcp_object_schema(json!({
                "recipe":mcp_recipe_schema(),
                "changes":{"type":"array","minItems":1,"maxItems":128,"items":mcp_edit_schema()}
            }), &["recipe","changes"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_architect_check", "title":"Validate a research recipe",
            "description":"Validate installed contracts, typed DAG ports/dependencies/settings locally; non-admitted preview.",
            "inputSchema":mcp_object_schema(json!({"recipe":mcp_recipe_schema()}), &["recipe"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_architect_diff", "title":"Compare research recipe hypotheses",
            "description":"Hash-only typed recipe comparison; no live provider data or measured SEO outcomes.",
            "inputSchema":mcp_object_schema(json!({"first":mcp_recipe_schema(),"second":mcp_recipe_schema()}), &["first","second"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_architect_connections", "title":"Find compatible typed step connections",
            "description":"Exact-schema source/upstream binding hints, excluding immediate cycles. Not validated or granted.",
            "inputSchema":mcp_object_schema(json!({"recipe":mcp_recipe_schema()}), &["recipe"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_architect_inspect", "title":"Inspect a saved typed execution receipt",
            "description":"Locally inspect sealed V2 plan/receipt JSON, never file paths. Hash diagnostics; no external attestation.",
            "inputSchema":mcp_saved_execution_schema(),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_architect_compare_results", "title":"Compare two saved execution receipts",
            "description":"Compare two sealed V2 receipts, same tenant and exact sources. Hash-only; History unverified. No paths.",
            "inputSchema":mcp_object_schema(json!({"first":mcp_saved_execution_schema(),"second":mcp_saved_execution_schema()}), &["first","second"]),
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
        {
            "name":"site_architect_evidence", "title":"Assess independent consumer needs evidence",
            "description":"Group independent claimed primary consumer sources, excluding snippets/mirrors. No source/provider attestation.",
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

fn mcp_site_architect_authoring_payload_too_large(payload: &impl serde::Serialize) -> bool {
    serde_json::to_vec(payload).map_or(true, |bytes| {
        bytes.len() > SITE_ARCHITECT_RECIPE_MAX_BYTES as usize
    })
}

fn mcp_site_architect_authoring_envelope_too_large(payload: &impl serde::Serialize) -> bool {
    serde_json::to_vec(payload).map_or(true, |bytes| bytes.len() > SITE_ARCHITECT_MCP_AUTHORING_MAX_BYTES)
}

fn mcp_site_architect_authoring_problem() -> Value {
    site_architect_problem(
        "authoring_payload_too_large",
        "siteArchitect authoring input exceeds the bounded recipe and edit budget.",
        "Limit each recipe to 256 KiB, each edit batch to 128 changes and split large experiments into separate recipes.",
    )
}

fn mcp_site_architect_scope_id_is_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= SITE_ARCHITECT_MCP_MAX_SCOPE_ID_BYTES
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

fn mcp_operator_read_unavailable() -> Value {
    site_architect_problem(
        "operator_read_unavailable_or_untrusted",
        "The separately credentialed read-only Temporal operator is unavailable or rejected the request.",
        "Ask the operator to check the private Unix bridge, tenant-scoped read role, independent governance and actual server Histories. A refused read does not prove a Workflow has not started.",
    )
}

fn mcp_operator_read_result(
    response: &[u8],
    canonical_run_id: uuid::Uuid,
) -> Option<(Value, bool)> {
    if response.is_empty() || response.len() > SITE_ARCHITECT_MCP_OPERATOR_REPLY_MAX_BYTES as usize {
        return None;
    }
    let report: McpOwnerRunReport = serde_json::from_slice(response).ok()?;
    if report.operation != "read_operation_dag_run_v2" || report.next_action.len() > 512 {
        return None;
    }
    if report.status == "refused" {
        if report.code.as_deref() != Some("run_unavailable_or_untrusted")
            || report.canonical_run_id.is_some()
        {
            return None;
        }
        return Some((mcp_operator_read_unavailable(), true));
    }
    if report.code.is_some()
        || report.canonical_run_id != Some(canonical_run_id)
        || report.step_count != Some(1)
        || report.owner_result_count.is_none_or(|count| count > 1)
        || report.governance_currently_active.is_none()
        || report.source_authentication.as_deref() != Some("not_independently_attested")
        || report.owner_receipt_attestation.as_deref() != Some("not_independently_verified")
    {
        return None;
    }
    let workflow_run_id = report.parent_workflow_run_id.as_deref()?;
    let parsed_workflow_run_id = uuid::Uuid::parse_str(workflow_run_id).ok()?;
    if parsed_workflow_run_id.is_nil() || parsed_workflow_run_id.to_string() != workflow_run_id {
        return None;
    }
    let is_sha = |sha: &str| {
        sha.len() == 64
            && sha
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    let plan_hash = report.plan_hash.as_deref()?;
    let start_hash = report.parent_start_sha256.as_deref()?;
    if !is_sha(plan_hash)
        || !is_sha(start_hash)
        || report
            .result_blake3
            .as_deref()
            .is_some_and(|hash| !is_sha(hash))
    {
        return None;
    }
    let (next_action, valid_count, requires_result, expected_history_proof) = match report.status.as_str() {
        "temporal_running" => (
            "Wait for the same native Temporal Workflow; read this business run again after completion.",
            0,
            false,
            "exact_server_parent_start_only",
        ),
        "temporal_completed_owner_unverified" => (
            "Obtain independently attested owner SQL/provider receipt and source evidence before using the result as SEO evidence.",
            1,
            true,
            "exact_server_parent_validation_and_effect_child_histories",
        ),
        "temporal_failed" | "temporal_canceled" | "temporal_timed_out"
        | "temporal_terminated" | "temporal_continued_as_new" => (
            "Inspect the exact native Temporal terminal History with the installed operator; never blindly retry an uncertain external effect.",
            0,
            false,
            "exact_server_parent_start_and_terminal_event",
        ),
        _ => return None,
    };
    if report.owner_result_count != Some(valid_count)
        || requires_result != report.result_blake3.is_some()
        || report.temporal_history_attestation.as_deref() != Some(expected_history_proof)
    {
        return None;
    }
    Some((
        json!({
            "status": report.status,
            "operation":"read_operation_dag_run_v2",
            "canonical_run_id":canonical_run_id,
            "parent_workflow_run_id":workflow_run_id,
            "plan_hash":plan_hash,
            "parent_start_sha256":start_hash,
            "step_count":1,
            "owner_result_count":valid_count,
            "result_blake3":report.result_blake3,
            "governance_currently_active":report.governance_currently_active,
            "temporal_history_attestation":report.temporal_history_attestation,
            "owner_receipt_attestation":"not_independently_verified",
            "source_authentication":"not_independently_attested",
            "execution_control":"read_only_no_start_or_retry",
            "next_action":next_action,
        }),
        false,
    ))
}

#[cfg(unix)]
fn mcp_operator_socket_read(locator: &McpRunStatus) -> Option<Vec<u8>> {
    use std::io::{Read, Write};
    use std::os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    };
    use std::time::Duration;

    // Both names belong exclusively to the installed MCP process. The model
    // supplies only canonical_run_id. An empty or user-settable socket path in
    // tool arguments must never be converted into privileged owner authority.
    let configured_path = std::env::var("OPERATION_DAG_READ_V2_SOCKET_PATH").ok()?;
    let expected_uid_text = std::env::var("OPERATION_DAG_READ_V2_OPERATOR_UID").ok()?;
    let expected_uid = expected_uid_text.parse::<u32>().ok()?;
    if expected_uid.to_string() != expected_uid_text
        || configured_path.len() > 100
        || !configured_path.starts_with('/')
    {
        return None;
    }
    let path = std::path::Path::new(&configured_path);
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return None;
    }
    let parent = path.parent()?;
    let parent_info = std::fs::symlink_metadata(parent).ok()?;
    let socket_info = std::fs::symlink_metadata(path).ok()?;
    if !parent_info.file_type().is_dir()
        || parent_info.uid() != expected_uid
        || parent_info.mode() & 0o7777 != 0o2710
        || !socket_info.file_type().is_socket()
        || socket_info.uid() != expected_uid
        || socket_info.gid() != parent_info.gid()
        || socket_info.mode() & 0o777 != 0o660
    {
        return None;
    }
    // No symlink, group/world-writeable parent or attacker-replaced ancestor.
    // The leaf directory and socket are owned by a separate, trusted Unix UID;
    // this unprivileged MCP client cannot alter the operator's private DSN.
    for ancestor in parent.ancestors().skip(1) {
        let metadata = std::fs::symlink_metadata(ancestor).ok()?;
        if !metadata.file_type().is_dir() || metadata.mode() & 0o022 != 0 {
            return None;
        }
    }
    let mut socket = UnixStream::connect(path).ok()?;
    socket
        .set_read_timeout(Some(Duration::from_secs(37)))
        .ok()?;
    socket
        .set_write_timeout(Some(Duration::from_secs(3)))
        .ok()?;
    let mut request = serde_json::to_vec(locator).ok()?;
    request.push(b'\n');
    if request.len() > SITE_ARCHITECT_MCP_OPERATOR_REQUEST_MAX_BYTES {
        return None;
    }
    socket.write_all(&request).ok()?;
    socket.shutdown(std::net::Shutdown::Write).ok()?;
    let mut result = Vec::new();
    socket
        .take(SITE_ARCHITECT_MCP_OPERATOR_REPLY_MAX_BYTES + 1)
        .read_to_end(&mut result)
        .ok()?;
    if result.len() > SITE_ARCHITECT_MCP_OPERATOR_REPLY_MAX_BYTES as usize
        || result.last() != Some(&b'\n')
        || result[..result.len() - 1].contains(&b'\n')
    {
        return None;
    }
    result.pop();
    Some(result)
}

fn mcp_site_architect_run_status(args: Value) -> (Value, bool) {
    if serde_json::to_vec(&args).map_or(true, |bytes| {
        bytes.len() > SITE_ARCHITECT_MCP_OPERATOR_REQUEST_MAX_BYTES
    }) {
        return (
            mcp_bad_arguments("bounded UUID-only locator required"),
            true,
        );
    }
    let request: McpRunStatus = match serde_json::from_value(args.clone()) {
        Ok(parsed) => parsed,
        Err(_) => {
            return (
                mcp_bad_arguments("canonical UUID-only locator required"),
                true,
            )
        }
    };
    if request.canonical_run_id.is_nil()
        || args.get("canonical_run_id").and_then(Value::as_str)
            != Some(request.canonical_run_id.to_string().as_str())
    {
        return (mcp_bad_arguments("noncanonical business UUID"), true);
    }
    #[cfg(unix)]
    if let Some(response) = mcp_operator_socket_read(&request) {
        if let Some((result, is_error)) =
            mcp_operator_read_result(&response, request.canonical_run_id)
        {
            return (result, is_error);
        }
    }
    (mcp_operator_read_unavailable(), true)
}

fn mcp_site_architect_call(
    name: &str,
    args: Value,
    catalog: &seo_application::site_architect::SiteArchitectCatalog,
) -> (Value, bool) {
    use seo_application::site_architect::{
        compare_executions, describe_tools, diff_recipes, inspect_execution,
        preview_consumer_need_evidence, preview_recipe, revise_recipe, suggest_compatible_inputs,
        SiteRecipe,
    };

    // `tools/list` declares limits for model convenience, but the MCP caller
    // is untrusted and can bypass every JSON Schema annotation. This protects
    // the edit/diff/evidence paths without changing the larger saved-result
    // envelope or granting any path/Temporal/provider capability.
    if matches!(
        name,
        "site_architect_operation"
            | "site_architect_one_step"
            | "site_architect_new"
            | "site_architect_prepare_step"
            | "site_architect_edit"
            | "site_architect_check"
            | "site_architect_diff"
            | "site_architect_connections"
            | "site_architect_evidence"
    ) && mcp_site_architect_authoring_envelope_too_large(&args)
    {
        return (mcp_site_architect_authoring_problem(), true);
    }
    // RecipeChange deliberately has no Serialize implementation. Bound the
    // client's original JSON edit array before deserializing it into the
    // strongly typed, deserialize-only edit commands.
    if name == "site_architect_edit"
        && args
            .get("changes")
            .is_some_and(|changes| mcp_site_architect_authoring_payload_too_large(changes))
    {
        return (mcp_site_architect_authoring_problem(), true);
    }

    match name {
        "site_architect_tools" => {
            let parsed: std::result::Result<serde_json::Map<String, Value>, _> =
                serde_json::from_value(if args.is_null() { json!({}) } else { args });
            match parsed {
                Ok(fields) if fields.is_empty() => match site_architect_tools_report(catalog) {
                    Ok(report) => (report, false),
                    Err(problem) => (problem, true),
                },
                _ => (mcp_bad_arguments("site_architect_tools takes an empty arguments object"), true),
            }
        }
        "site_architect_operation" => match serde_json::from_value::<McpOneOperation>(args) {
            Ok(input) if mcp_site_architect_authoring_payload_too_large(&input.operation)
                || input.parameters.as_ref().is_some_and(mcp_site_architect_authoring_payload_too_large) =>
            {
                (site_architect_problem("operation_settings_too_large", "Operation identifier or settings exceed the authoring limit.", "Use a bounded versioned operation reference and at most 256 KiB of JSON settings."), true)
            }
            Ok(input) => match site_architect_operation_inspection(
                catalog,
                &input.operation,
                input.parameters.as_ref(),
            ) {
                Ok(inspection) => (inspection, false),
                Err(problem) => (problem, true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_architect_one_step" => match serde_json::from_value::<McpOneRecipe>(args) {
            Ok(input) if mcp_site_architect_authoring_payload_too_large(&input.recipe) =>
            {
                (site_architect_problem("one_step_recipe_too_large", "One-step recipe exceeds the 256 KiB authoring limit.", "Reduce the recipe input before checking this operation."), true)
            }
            Ok(input) => match site_architect_one_step_preview(&input.recipe, catalog) {
                Ok(preview) => (preview, false),
                Err(problem) => (problem, true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_architect_run_status" => mcp_site_architect_run_status(args),
        "site_architect_new" => match serde_json::from_value::<McpNewRecipe>(args) {
            Ok(input) if mcp_site_architect_scope_id_is_valid(&input.tenant_id)
                && mcp_site_architect_scope_id_is_valid(&input.run_id) => {
                let recipe = SiteRecipe {
                    schema_revision: seo_application::site_architect::SITE_RECIPE_SCHEMA_REVISION,
                    scope: seo_application::site_architect::RunScope {tenant_id:input.tenant_id,run_id:input.run_id},
                    sources: Default::default(), steps: Vec::new(),
                };
                (json!({"status":"draft_unchecked","recipe":recipe,"execution":"not_started"}), false)
            }
            Ok(_) => (mcp_bad_arguments("tenant_id and run_id must be bounded identifiers"), true),
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_architect_prepare_step" => match serde_json::from_value::<McpPrepareStep>(args) {
            Ok(input)
                if mcp_site_architect_scope_id_is_valid(&input.tenant_id)
                    && mcp_site_architect_scope_id_is_valid(&input.run_id)
                    && !input.step_id.is_empty()
                    && input.step_id.len() <= 64
                    && input.step_id.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                    }) =>
            {
                match seo_application::site_architect::inspect_operation(
                    catalog, &input.operation, None,
                ) {
                    Ok(inspection) => {
                        let required_input_bindings: Vec<Value> = inspection.tool.inputs.iter()
                            .map(|(port, schema)| json!({
                                "port":port,
                                "schema":schema,
                                "binding":"unbound",
                                "owner_dto_validation":"required_not_run",
                                "source_authentication":"not_checked"
                            })).collect();
                        let has_required_inputs = !required_input_bindings.is_empty();
                        let step_id = match serde_json::from_value(json!(input.step_id)) {
                            Ok(id) => id,
                            Err(_) => return (mcp_bad_arguments("invalid typed step identifier"), true),
                        };
                        let recipe = SiteRecipe {
                            schema_revision: seo_application::site_architect::SITE_RECIPE_SCHEMA_REVISION,
                            scope: seo_application::site_architect::RunScope {
                                tenant_id:input.tenant_id,
                                run_id:input.run_id,
                            },
                            sources:Default::default(),
                            steps:vec![seo_application::site_architect::RecipeStep {
                                id:step_id,
                                operation:input.operation,
                                inputs:Default::default(),
                                parameters:Default::default(),
                                after:Default::default(),
                            }],
                        };
                        (json!({
                            "status":if has_required_inputs {"draft_input_bindings_required"} else {"draft_pending_local_check"},
                            "recipe":recipe,
                            "required_input_bindings":required_input_bindings,
                            "parameter_schema":inspection.tool.parameters,
                            "output_ports":inspection.tool.outputs,
                            "executable_owner":"not_verified",
                            "parameters":"not_checked",
                            "owner_validate_only":"required_not_run",
                            "source_authentication":"not_checked",
                            "operator_admission":"not_issued",
                            "execution":"not_started",
                            "next_action":"Bind each required input through site_architect_edit with a real source identity or compatible Produced step; set owner parameters, then call site_architect_check. Runtime authorization and independent ValidateOnly are still required before Start."
                        }), false)
                    },
                    Err(error) => (site_architect_safe_operation_problem(&error), true),
                }
            }
            Ok(_) => (mcp_bad_arguments("bounded scope and step identifiers required"), true),
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_architect_edit" => match serde_json::from_value::<McpEditRecipe>(args) {
            Ok(input) if input.changes.len() > SITE_ARCHITECT_MCP_MAX_RECIPE_EDITS
                || mcp_site_architect_authoring_payload_too_large(&input.recipe) =>
            {
                (mcp_site_architect_authoring_problem(), true)
            }
            Ok(input) => match revise_recipe(&input.recipe, &input.changes) {
                Ok(recipe) if mcp_site_architect_authoring_payload_too_large(&recipe) =>
                    (mcp_site_architect_authoring_problem(), true),
                Ok(recipe) => match preview_recipe(&recipe, catalog) {
                    Ok(preview) => (json!({"status":mcp_edit_preview_status(preview.status),"recipe":recipe,"preview":preview}), false),
                    Err(error) => (json!({"status":"draft_needs_changes","recipe":recipe,"problem":site_architect_safe_recipe_problem(&error),"execution":"not_started"}), false),
                },
                Err(_) => (json!({"status":"invalid","code":"recipe_edit_rejected","message":"The requested edit cannot be applied to this versioned recipe.","next_action":"Fix the referenced step or batch; no partial edits were applied."}), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_architect_check" => match serde_json::from_value::<McpOneRecipe>(args) {
            Ok(input) if mcp_site_architect_authoring_payload_too_large(&input.recipe) =>
                (mcp_site_architect_authoring_problem(), true),
            Ok(input) => match preview_recipe(&input.recipe, catalog) {
                Ok(preview) => (json!(preview), false),
                Err(error) => (site_architect_safe_recipe_problem(&error), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_architect_diff" => match serde_json::from_value::<McpTwoRecipes>(args) {
            Ok(input) if mcp_site_architect_authoring_payload_too_large(&input.first)
                || mcp_site_architect_authoring_payload_too_large(&input.second) =>
            {
                (mcp_site_architect_authoring_problem(), true)
            }
            Ok(input) => match diff_recipes(&input.first, &input.second, catalog) {
                Ok(diff) => (json!(diff), false),
                Err(error) => (site_architect_safe_recipe_problem(&error), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_architect_connections" => match serde_json::from_value::<McpOneRecipe>(args) {
            Ok(input) if mcp_site_architect_authoring_payload_too_large(&input.recipe) =>
                (mcp_site_architect_authoring_problem(), true),
            Ok(input) => match suggest_compatible_inputs(&input.recipe, catalog) {
                Ok(preview) => (json!(preview), false),
                Err(error) => (site_architect_safe_recipe_problem(&error), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_architect_inspect" => match serde_json::from_value::<McpSavedExecution>(args) {
            Ok(input) => match inspect_execution(&input.plan, &input.receipt, catalog.catalog()) {
                Ok(inspection) => (json!(inspection), false),
                Err(error) => (json!(error.problem()), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_architect_compare_results" => match serde_json::from_value::<McpCompareExecutions>(args) {
            Ok(input) => match compare_executions(
                &input.first.plan, &input.first.receipt,
                &input.second.plan, &input.second.receipt, catalog.catalog(),
            ) {
                Ok(comparison) => (json!(comparison), false),
                Err(error) => (json!(error.problem()), true),
            },
            Err(error) => (mcp_bad_arguments(error), true),
        },
        "site_architect_evidence" => match serde_json::from_value::<McpNeedEvidence>(args) {
            Ok(input) if mcp_site_architect_authoring_payload_too_large(&input.batch) =>
                (mcp_site_architect_authoring_problem(), true),
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
        _ => (json!({"status":"invalid","code":"tool_unavailable","message":"Unknown siteArchitect MCP tool","next_action":"Call tools/list to discover available tools."}), true),
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
fn mcp_site_architect_message(
    request: Value,
    catalog: &seo_application::site_architect::SiteArchitectCatalog,
) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str);
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || method.is_none() {
        return Some(mcp_error(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid JSON-RPC request",
        ));
    }
    // Clients send initialization and other notifications without an id.
    let id = id?;
    let result = match method? {
        "initialize" => {
            json!({"protocolVersion":SITE_ARCHITECT_MCP_PROTOCOL,
                "capabilities":{"tools":{"listChanged":false}},
                "serverInfo":{"name":"alegria-siteArchitect","version":env!("CARGO_PKG_VERSION")},
                "instructions":"Read-only siteArchitect: trusted catalog, typed DAG authoring, a fail-closed one-step launch request diagnostic, saved-receipt inspection and an optional independently credentialed server-Temporal-History status reader. A trusted ChatGPT OAuth resource server is NOT configured in stdio MCP. No user authorization issuer, one-use grant consume, Temporal Start, provider effects or admission is exposed."})
        }
        "ping" => json!({}),
        "tools/list" => mcp_site_architect_tools(),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or(Value::Null);
            return Some(match serde_json::from_value::<McpCall>(params) {
                Ok(call) => {
                    let (data, is_error) = mcp_site_architect_call(&call.name, call.arguments, catalog);
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
        if line.len().saturating_add(take) > SITE_ARCHITECT_MCP_MAX_MESSAGE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "MCP message exceeds 3 MiB boundary",
            ));
        }
        line.extend_from_slice(&available[..take]);
        input.consume(take);
        if end.is_some() {
            return Ok(Some(line));
        }
    }
}

fn serve_site_architect_mcp<R: std::io::BufRead, W: std::io::Write>(
    mut input: R,
    mut output: W,
    catalog: &seo_application::site_architect::SiteArchitectCatalog,
) -> Result<()> {
    while let Some(line) = mcp_bounded_line(&mut input)? {
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let response = match serde_json::from_slice::<Value>(&line) {
            Ok(request) => mcp_site_architect_message(request, catalog),
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

fn mcp_site_architect_server() -> Result<i32> {
    let catalog = site_architect_installed_catalog()?;
    serve_site_architect_mcp(std::io::stdin().lock(), std::io::stdout().lock(), &catalog)?;
    Ok(0)
}

#[cfg(test)]
mod site_architect_mcp_tests {
    use super::*;

    fn catalog() -> seo_application::site_architect::SiteArchitectCatalog {
        let registry = seo_application::operation_adapter::semantic_extraction_registry().unwrap();
        seo_application::site_architect::installed_semantic_operation_catalog(registry.catalog().clone())
            .unwrap()
    }

    fn tool_call(name: &str, arguments: Value) -> Value {
        mcp_site_architect_message(
            json!({
                "jsonrpc":"2.0", "id":9, "method":"tools/call",
                "params":{"name":name,"arguments":arguments}
            }),
            &catalog(),
        )
        .unwrap()
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
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"site_architect_tools\",\"arguments\":{}}}\n"
        );
        let mut output = Vec::new();
        serve_site_architect_mcp(std::io::Cursor::new(lines), &mut output, &catalog()).unwrap();
        let replies: Vec<Value> = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["result"]["serverInfo"]["name"], "alegria-siteArchitect");
        assert_eq!(
            replies[0]["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
        assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 13);
        assert!(replies[1]["result"]["tools"].as_array().unwrap().iter().all(|tool| {
            tool["name"].as_str().is_some_and(|name| name.starts_with("site_architect_"))
        }));
        assert!(
            replies[1]["result"].to_string().len() < 24_576,
            "tools/list should fit a compact model context"
        );
        for name in [
            "site_architect_operation",
            "site_architect_one_step",
            "site_architect_prepare_step",
            "site_architect_run_status",
            "site_architect_connections",
            "site_architect_inspect",
            "site_architect_compare_results",
            "site_architect_evidence",
        ] {
            assert!(replies[1]["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == name));
        }
        assert_eq!(
            replies[2]["result"]["structuredContent"]["operations"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        let readiness = &replies[2]["result"]["structuredContent"]["launch_readiness"];
        assert_eq!(readiness["counts"]["installed_versioned_operations"], 5);
        assert_eq!(readiness["counts"]["user_start_verified_by_this_adapter"], 0);
        assert_eq!(readiness["mcp_start_exposed"], false);
        assert_eq!(
            readiness["one_step_launch_request"]["status"],
            "blocked_authenticated_start_gateway_not_configured"
        );
        assert_eq!(
            readiness["one_step_launch_request"]["mutating_temporal_start"],
            false
        );
        assert_eq!(
            readiness["launch_gates"]["budget_and_one_use_grant"],
            "not_proven_here"
        );
        assert_eq!(replies[2]["result"]["isError"], false);
    }

    #[test]
    fn mcp_connections_stays_a_non_executable_hint() {
        let draft = tool_call(
            "site_architect_new",
            json!({"tenant_id":"tenant","run_id":"draft"}),
        );
        let recipe = draft["result"]["structuredContent"]["recipe"].clone();
        let hint = tool_call("site_architect_connections", json!({"recipe":recipe}));
        assert_eq!(hint["result"]["isError"], false);
        assert_eq!(
            hint["result"]["structuredContent"]["status"],
            "schema_candidates_only"
        );
        assert_eq!(
            hint["result"]["structuredContent"]["execution"],
            "not_started"
        );
        assert!(hint["result"]["structuredContent"]["ports"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn mcp_operation_reports_exact_installed_metadata_without_execution() {
        let response = tool_call(
            "site_architect_operation",
            json!({
                "operation":{"id":"semantic.entity_spans","revision":1},
                "parameters":{}
            }),
        );
        let result = &response["result"]["structuredContent"];
        assert_eq!(response["result"]["isError"], false);
        assert_eq!(result["status"], "registered_operation_inspection_only");
        assert_eq!(result["tool"]["operation"]["id"], "semantic.entity_spans");
        assert_eq!(result["execution"], "not_started");
        assert_eq!(result["provider_connectivity"], "not_checked");
        assert_eq!(result["checked_parameters"]["verification"], "typed_local");

        let unknown = tool_call(
            "site_architect_operation",
            json!({
                "operation":{"id":"unregistered_secret-SENTINEL","revision":99},
                "parameters":{}
            }),
        );
        assert_eq!(
            unknown["result"]["structuredContent"]["code"],
            "operation_unavailable"
        );
        assert_eq!(unknown["result"]["isError"], true);
        assert!(!unknown.to_string().contains("unregistered_secret-SENTINEL"));
    }

    #[test]
    fn mcp_prepare_step_uses_installed_contract_and_leaves_every_source_unbound() {
        let response = tool_call(
            "site_architect_prepare_step",
            json!({
                "tenant_id":"lab",
                "run_id":"study-1",
                "step_id":"extract",
                "operation":{"id":"semantic.entity_spans","revision":1}
            }),
        );
        assert_eq!(response["result"]["isError"], false);
        let prepared = &response["result"]["structuredContent"];
        assert_eq!(prepared["status"], "draft_input_bindings_required");
        assert_eq!(prepared["recipe"]["schema_revision"], 2);
        assert_eq!(prepared["recipe"]["steps"][0]["id"], "extract");
        assert_eq!(prepared["recipe"]["steps"][0]["operation"]["id"], "semantic.entity_spans");
        assert_eq!(prepared["recipe"]["steps"][0]["inputs"], json!({}));
        assert_eq!(prepared["recipe"]["steps"][0]["parameters"], json!({}));
        assert_eq!(prepared["recipe"]["sources"], json!({}));
        let bindings = prepared["required_input_bindings"].as_array().unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0]["port"], "input");
        assert_eq!(bindings[0]["schema"]["id"], "semantic.entity_span_input");
        assert_eq!(bindings[0]["binding"], "unbound");
        assert_eq!(bindings[0]["owner_dto_validation"], "required_not_run");
        assert_eq!(bindings[0]["source_authentication"], "not_checked");
        assert_eq!(prepared["executable_owner"], "not_verified");
        assert_eq!(prepared["operator_admission"], "not_issued");
        assert_eq!(prepared["execution"], "not_started");

        let unbound = tool_call("site_architect_one_step", json!({
            "recipe":prepared["recipe"]
        }));
        assert_eq!(unbound["result"]["isError"], true,
            "a prepared draft is not an executable plan without actual input bindings");
    }

    #[test]
    fn mcp_prepare_step_rejects_uninstalled_routes_and_forged_authority() {
        let normal = json!({
            "tenant_id":"lab", "run_id":"draft", "step_id":"s1",
            "operation":{"id":"semantic.entity_spans","revision":1}
        });
        let mut unknown = normal.clone();
        unknown["operation"]["id"] = json!("uninstalled-SECRET-SENTINEL");
        let result = tool_call("site_architect_prepare_step", unknown);
        assert_eq!(result["result"]["isError"], true);
        assert_eq!(result["result"]["structuredContent"]["code"], "operation_unavailable");
        assert!(!result.to_string().contains("SECRET-SENTINEL"));

        for (key, value) in [
            ("task_queue", json!("private-worker-SENTINEL")),
            ("parameters", json!({"bearer_token":"SECRET-SENTINEL"})),
            ("operator_admission", json!("forged")),
        ] {
            let mut forged = normal.clone();
            forged[key] = value;
            let denied = tool_call("site_architect_prepare_step", forged);
            assert_eq!(denied["result"]["isError"], true, "{key}");
            assert_eq!(denied["result"]["structuredContent"]["code"], "invalid_tool_arguments");
            assert!(!denied.to_string().contains("SENTINEL"));
        }
        let mut invalid_step = normal;
        invalid_step["step_id"] = json!("private/SENTINEL");
        let denied = tool_call("site_architect_prepare_step", invalid_step);
        assert_eq!(denied["result"]["isError"], true);
        assert!(!denied.to_string().contains("SENTINEL"));
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
        let checked = tool_call("site_architect_one_step", json!({"recipe":recipe.clone()}));
        let report = &checked["result"]["structuredContent"];
        assert_eq!(checked["result"]["isError"], false);
        assert_eq!(report["status"], "single_step_preview_only");
        assert_eq!(report["execution"], "not_started");
        assert_eq!(report["owner_validate_only"], "required_not_run");
        assert_eq!(report["operator_admission"], "not_issued");
        assert_eq!(report["user_start_admission"], "not_available_in_read_only_adapter");
        assert_eq!(report["batch_execution"], "not_started");
        assert_eq!(report["temporal_history_attestation"], "not_checked");

        let mut empty = recipe.clone();
        empty["steps"] = json!([]);
        let rejected = tool_call("site_architect_one_step", json!({"recipe":empty}));
        assert_eq!(rejected["result"]["isError"], true);
        assert_eq!(
            rejected["result"]["structuredContent"]["code"],
            "one_step_required"
        );

        let mut secret_settings = recipe;
        secret_settings["steps"][0]["parameters"] =
            json!({"provider_token":"SENTINEL-PRIVATE-KEY"});
        let bad = tool_call("site_architect_one_step", json!({"recipe":secret_settings}));
        assert_eq!(bad["result"]["isError"], true);
        assert!(!bad.to_string().contains("SENTINEL-PRIVATE-KEY"));
    }

    #[test]
    fn mcp_one_step_rejects_forged_authority_and_does_not_shadow_remote_run_one() {
        assert!(mcp_site_architect_tools()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["name"] != "site_architect_run_one"));
        let recipe = json!({
            "schema_revision":2,
            "scope":{"tenant_id":"caller-supplied-is-not-an-acl","run_id":"one-step"},
            "sources":{"mentions":{"schema":{"id":"semantic.entity_span_input","revision":1},"provenance":"unverified:test"}},
            "steps":[{"id":"extract","operation":{"id":"semantic.entity_spans","revision":1},
                "inputs":{"input":{"source":"external","artifact":"mentions"}},"parameters":{}}]
        });

        let response = tool_call("site_architect_one_step", json!({"recipe":recipe.clone()}));
        let result = &response["result"]["structuredContent"];
        assert_eq!(response["result"]["isError"], false);
        assert_eq!(result["status"], "single_step_preview_only");
        assert_eq!(result["user_start_admission"], "not_available_in_read_only_adapter");
        assert_eq!(result["operator_admission"], "not_issued");
        assert_eq!(result["execution"], "not_started");

        // None of these strings may be upgraded into authority by supplying
        // them as tool-call arguments. Reject them rather than ignore them.
        for property in [
            "principal_id", "tenant_id", "bearer_token", "grant_id", "temporal_url",
            "operator_socket_path", "source_path", "authenticated_context",
        ] {
            let mut args = json!({"recipe":recipe.clone()});
            args[property] = json!("SENTINEL-FORGED-AUTHORITY");
            let rejected = tool_call("site_architect_one_step", args);
            assert_eq!(rejected["result"]["isError"], true);
            assert_eq!(
                rejected["result"]["structuredContent"]["code"],
                "invalid_tool_arguments",
                "{property} must never be interpreted as authority"
            );
            assert!(!rejected.to_string().contains("SENTINEL-FORGED-AUTHORITY"));
        }

        let mut no_steps = recipe.clone();
        no_steps["steps"] = json!([]);
        let zero = tool_call("site_architect_one_step", json!({"recipe":no_steps}));
        assert_eq!(zero["result"]["structuredContent"]["code"], "one_step_required");

        let mut two_steps = recipe.clone();
        two_steps["steps"].as_array_mut().unwrap().push(recipe["steps"][0].clone());
        let two = tool_call("site_architect_one_step", json!({"recipe":two_steps}));
        assert_eq!(two["result"]["structuredContent"]["code"], "one_step_required");

        let mut token_in_settings = recipe.clone();
        token_in_settings["steps"][0]["parameters"] =
            json!({"bearer_token":"SENTINEL-PRIVATE-KEY"});
        let rejected = tool_call("site_architect_one_step", json!({"recipe":token_in_settings}));
        assert_eq!(rejected["result"]["isError"], true);
        assert!(!rejected.to_string().contains("SENTINEL-PRIVATE-KEY"));

        // The only `run_one` contract is the authenticated remote request-ID
        // tool; the local stdio adapter must not advertise a homonym.
        let unsupported = tool_call("site_architect_run_one", json!({"recipe":recipe}));
        assert_eq!(unsupported["result"]["isError"], true);
    }

    fn safe_owner_report(canonical_run_id: uuid::Uuid) -> Value {
        json!({
            "operation":"read_operation_dag_run_v2",
            "status":"temporal_completed_owner_unverified",
            "canonical_run_id":canonical_run_id,
            "parent_workflow_run_id":"134ad0e9-4673-4d94-9ff7-0d2f999ad8c9",
            "plan_hash":"a".repeat(64),
            "parent_start_sha256":"b".repeat(64),
            "step_count":1,
            "owner_result_count":1,
            "result_blake3":"c".repeat(64),
            "governance_currently_active":true,
            "temporal_history_attestation":"exact_server_parent_validation_and_effect_child_histories",
            "owner_receipt_attestation":"not_independently_verified",
            "source_authentication":"not_independently_attested",
            "next_action":"Ask separately credentialed operator for provider attestations."
        })
    }

    #[test]
    fn mcp_read_status_requires_canonical_business_uuid_and_never_accepts_paths() {
        for invalid in [
            json!({"canonical_run_id":"not-a-uuid"}),
            json!({"canonical_run_id":"00000000-0000-0000-0000-000000000000"}),
            json!({"canonical_run_id":"134AD0E9-4673-4D94-9FF7-0D2F999AD8C9"}),
            json!({"canonical_run_id":"134ad0e9-4673-4d94-9ff7-0d2f999ad8c9","socket_path":"/private/SENTINEL"}),
            json!({"canonical_run_id":"134ad0e9-4673-4d94-9ff7-0d2f999ad8c9","tenant_id":"other"}),
            json!({"run_id":"134ad0e9-4673-4d94-9ff7-0d2f999ad8c9"}),
        ] {
            let report = tool_call("site_architect_run_status", invalid);
            assert_eq!(report["result"]["isError"], true);
            assert_eq!(
                report["result"]["structuredContent"]["code"],
                "invalid_tool_arguments"
            );
            assert!(!report.to_string().contains("/private/SENTINEL"));
        }
    }

    #[test]
    fn mcp_read_status_only_accepts_verified_one_step_history_summary() {
        let run_id = uuid::Uuid::parse_str("c37c2193-e0ed-4d79-96c7-c097a64f81e5").unwrap();
        let good = serde_json::to_vec(&safe_owner_report(run_id)).unwrap();
        let (status, is_error) = mcp_operator_read_result(&good, run_id).unwrap();
        assert!(!is_error);
        assert_eq!(status["status"], "temporal_completed_owner_unverified");
        assert_eq!(status["owner_result_count"], 1);
        assert_eq!(status["execution_control"], "read_only_no_start_or_retry");
        assert_eq!(
            status["owner_receipt_attestation"],
            "not_independently_verified"
        );
        assert_eq!(
            status["source_authentication"],
            "not_independently_attested"
        );

        let mut report = safe_owner_report(run_id);
        report["code"] = json!("success");
        assert!(mcp_operator_read_result(&serde_json::to_vec(&report).unwrap(), run_id).is_none());
        report = safe_owner_report(run_id);
        report["status"] = json!("provider_receipt_attested");
        assert!(mcp_operator_read_result(&serde_json::to_vec(&report).unwrap(), run_id).is_none());
        report = safe_owner_report(run_id);
        report["source_authentication"] = json!("authenticated");
        assert!(mcp_operator_read_result(&serde_json::to_vec(&report).unwrap(), run_id).is_none());
        report = safe_owner_report(run_id);
        report["canonical_run_id"] = json!("92f02cf7-a725-4209-8247-24295cc1e303");
        assert!(mcp_operator_read_result(&serde_json::to_vec(&report).unwrap(), run_id).is_none());
        report = safe_owner_report(run_id);
        report["untrusted_secret"] = json!("SENTINEL-PRIVATE-API-KEY");
        assert!(mcp_operator_read_result(&serde_json::to_vec(&report).unwrap(), run_id).is_none());
    }

    #[test]
    fn mcp_read_status_collapses_refused_missing_and_foreign_tenant() {
        let run_id = uuid::Uuid::new_v4();
        let refused = json!({
            "operation":"read_operation_dag_run_v2",
            "status":"refused",
            "code":"run_unavailable_or_untrusted",
            "next_action":"Unknown private SQL/Temporal diagnostic"
        });
        let (status, is_error) =
            mcp_operator_read_result(&serde_json::to_vec(&refused).unwrap(), run_id).unwrap();
        assert!(is_error);
        assert_eq!(status["code"], "operator_read_unavailable_or_untrusted");
        assert!(!status.to_string().contains("Unknown private SQL"));

        let (result, failure) = mcp_site_architect_run_status(json!({
            "canonical_run_id":run_id,
        }));
        assert!(failure);
        assert_eq!(result["code"], "operator_read_unavailable_or_untrusted");
        assert!(!result.to_string().contains(&run_id.to_string()));
        assert!(!result.to_string().contains("tenant_id"));
    }

    #[test]
    fn mcp_individual_operation_limits_authored_parameters_to_256_kib() {
        let too_large = tool_call(
            "site_architect_operation",
            json!({
                "operation":{"id":"semantic.entity_spans","revision":1},
                "parameters":{"untrusted_value":"x".repeat(256 * 1024)}
            }),
        );
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
            (
                "site_architect_new",
                json!({"tenant_id":"lab","run_id":huge.clone()}),
            ),
            ("site_architect_check", json!({"recipe":{"extra":huge.clone()}})),
            (
                "site_architect_diff",
                json!({"first":{"extra":huge.clone()},"second":{}}),
            ),
            (
                "site_architect_connections",
                json!({"recipe":{"extra":huge.clone()}}),
            ),
            ("site_architect_evidence", json!({"batch":{"extra":huge}})),
        ] {
            let response = tool_call(name, arguments);
            assert_eq!(response["result"]["isError"], true, "{name}");
            assert_eq!(
                response["result"]["structuredContent"]["code"], "authoring_payload_too_large",
                "{name}"
            );
            assert!(!response.to_string().contains("SENTINEL-PRIVATE-JSON"));
        }
    }

    #[test]
    fn mcp_edit_enforces_change_count_and_never_echoes_failing_step_values() {
        let draft = tool_call("site_architect_new", json!({"tenant_id":"lab","run_id":"draft"}));
        let recipe = draft["result"]["structuredContent"]["recipe"].clone();
        let changes: Vec<Value> = (0..129)
            .map(|_| json!({"action":"remove_step","step":"absent"}))
            .collect();
        let oversized = tool_call(
            "site_architect_edit",
            json!({"recipe":recipe.clone(),"changes":changes}),
        );
        assert_eq!(oversized["result"]["isError"], true);
        assert_eq!(
            oversized["result"]["structuredContent"]["code"],
            "authoring_payload_too_large"
        );

        let sensitive = "SENTINEL-PRIVATE-STEP-VALUE";
        let failed = tool_call(
            "site_architect_edit",
            json!({"recipe":recipe,"changes":[
                {"action":"remove_step","step":sensitive}
            ]}),
        );
        assert_eq!(
            failed["result"]["structuredContent"]["code"],
            "recipe_edit_rejected"
        );
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
            ("site_architect_check", json!({"recipe":invalid_recipe.clone()})),
            ("site_architect_connections", json!({"recipe":invalid_recipe})),
        ] {
            let response = tool_call(name, arguments);
            assert_eq!(response["result"]["isError"], true);
            assert_eq!(
                response["result"]["structuredContent"]["code"],
                "operation_unavailable"
            );
            assert!(!response.to_string().contains(sensitive));
        }
    }

    #[test]
    fn mcp_port_mismatch_reports_expected_schema_and_redacts_the_authored_source() {
        let sensitive = "SENTINEL-PRIVATE-SOURCE";
        let response = tool_call(
            "site_architect_check",
            json!({"recipe":{
                "schema_revision":2,
                "scope":{"tenant_id":"tenant","run_id":"draft"},
                "sources":{
                    "source":{"schema":{"id":sensitive,"revision":9},"provenance":sensitive}
                },
                "steps":[{
                    "id":sensitive,
                    "operation":{"id":"semantic.entity_spans","revision":1},
                    "inputs":{"input":{"source":"external","artifact":"source"}}
                }]
            }}),
        );
        assert_eq!(response["result"]["isError"], true);
        let problem = &response["result"]["structuredContent"];
        assert_eq!(problem["code"], "incompatible_ports");
        assert_eq!(
            problem["incompatible_input"],
            json!({
                "step_index":0,
                "operation":{"id":"semantic.entity_spans","revision":1},
                "port":"input",
                "expected":{"id":"semantic.entity_span_input","revision":1},
                "actual":{"source":"external","declared_revision":9,"same_type":false}
            })
        );
        assert!(!response.to_string().contains(sensitive));
    }

    #[test]
    fn mcp_scope_identifiers_are_bounded_and_do_not_echo_invalid_inputs() {
        let sensitive = "SENTINEL-PRIVATE-SCOPE".repeat(12);
        let invalid = tool_call(
            "site_architect_new",
            json!({"tenant_id":sensitive,"run_id":"draft"}),
        );
        assert_eq!(invalid["result"]["isError"], true);
        assert_eq!(
            invalid["result"]["structuredContent"]["code"],
            "invalid_tool_arguments"
        );
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
        let preview = tool_call("site_architect_evidence", json!({"batch":batch}));
        assert_eq!(preview["result"]["isError"], false);
        assert_eq!(
            preview["result"]["structuredContent"]["status"],
            "structural_only_unattested"
        );
        assert_eq!(
            preview["result"]["structuredContent"]["assessments"][0]["status"],
            "insufficient_evidence"
        );
        assert_eq!(
            preview["result"]["structuredContent"]["assessments"][0]["source_authentication"],
            "not_checked"
        );
        assert!(!preview
            .to_string()
            .contains("synthetic evidence for page-a"));
    }

    #[test]
    fn mcp_edit_never_marks_pending_remote_owner_validation_as_validated() {
        assert_eq!(
            mcp_edit_preview_status("valid_preview_only"),
            "draft_validated"
        );
        assert_eq!(
            mcp_edit_preview_status("pending_owner_parameter_validation"),
            "draft_pending_owner_parameter_validation"
        );
        assert_eq!(
            mcp_edit_preview_status("unknown_future_owner_state"),
            "draft_unverified"
        );
    }

    #[test]
    fn mcp_receipt_rejects_forgery_without_claiming_temporal_attestation() {
        let saved = forged_saved_execution();
        let inspected = tool_call("site_architect_inspect", saved.clone());
        assert_eq!(inspected["result"]["isError"], true);
        assert_eq!(
            inspected["result"]["structuredContent"]["code"],
            "saved_plan_invalid"
        );

        let compared = tool_call(
            "site_architect_compare_results",
            json!({"first":saved,"second":forged_saved_execution()}),
        );
        assert_eq!(compared["result"]["isError"], true);
        assert_eq!(
            compared["result"]["structuredContent"]["code"],
            "saved_plan_invalid"
        );
        assert!(!compared
            .to_string()
            .contains("same_source_bytes_structural_result_comparison"));
    }

    #[test]
    fn mcp_receipt_arguments_reject_paths_unknown_fields_and_secret_reflection() {
        for name in ["site_architect_inspect", "site_architect_compare_results"] {
            let path = tool_call(
                name,
                json!({"plan_file":"/etc/shadow","receipt_file":"/private/input"}),
            );
            assert_eq!(
                path["result"]["structuredContent"]["code"],
                "invalid_tool_arguments"
            );
            assert!(!path.to_string().contains("/etc/shadow"));

            let mut forged = forged_saved_execution();
            forged["secret_from_untrusted_client"] = json!("SENTINEL-CREDENTIAL-DO-NOT-LOG");
            let invalid = if name == "site_architect_inspect" {
                tool_call(name, forged)
            } else {
                tool_call(
                    name,
                    json!({"first":forged,"second":forged_saved_execution()}),
                )
            };
            assert_eq!(invalid["result"]["isError"], true);
            assert_eq!(
                invalid["result"]["structuredContent"]["code"],
                "invalid_tool_arguments"
            );
            assert!(!invalid
                .to_string()
                .contains("SENTINEL-CREDENTIAL-DO-NOT-LOG"));
        }
    }

    #[test]
    fn mcp_edit_returns_draft_and_real_validation_errors() {
        let new = mcp_site_architect_message(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"site_architect_new","arguments":{"tenant_id":"lab","run_id":"test"}}}),
            &catalog(),
        )
        .unwrap();
        let recipe = new["result"]["structuredContent"]["recipe"].clone();
        assert_eq!(recipe["steps"].as_array().unwrap().len(), 0);
        let edit = mcp_site_architect_message(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"site_architect_edit","arguments":{"recipe":recipe,"changes":[
                {"action":"add_step","step":{"id":"extract","operation":{"id":"semantic.entity_spans","revision":1},"inputs":{}}}
            ]}}}), &catalog()).unwrap();
        assert_eq!(
            edit["result"]["structuredContent"]["status"],
            "draft_needs_changes"
        );
        assert_eq!(
            edit["result"]["structuredContent"]["recipe"]["steps"][0]["id"],
            "extract"
        );
        assert_eq!(edit["result"]["isError"], false);
        let invalid = mcp_site_architect_message(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
            "name":"site_architect_edit","arguments":{"recipe":edit["result"]["structuredContent"]["recipe"],"changes":[
                {"action":"add_step","step":{"id":"extract","operation":{"id":"semantic.entity_spans","revision":1},"inputs":{}}}
            ]}}}), &catalog()).unwrap();
        assert_eq!(
            invalid["result"]["structuredContent"]["code"],
            "recipe_edit_rejected"
        );
        assert_eq!(invalid["result"]["isError"], true);
    }

    #[test]
    fn mcp_rejects_unknown_methods_and_oversized_lines() {
        let unavailable = mcp_site_architect_message(
            json!({"jsonrpc":"2.0","id":8,"method":"execution/start"}),
            &catalog(),
        )
        .unwrap();
        assert_eq!(unavailable["error"]["code"], -32601);

        // A caller's claimed grant, role or tenant is never a valid way to
        // promote this read-only MCP authoring process into a Start issuer.
        for name in [
            "site_architect_run",
            "site_architect_request_grant",
            "site_architect_start_one_step",
        ] {
            let refused = tool_call(
                name,
                json!({
                    "request_id":"1285d5a3-afee-40aa-b334-7a47a43d6dd7",
                    "tenant_id":"SENTINEL-FOREIGN-TENANT",
                    "operator_socket":"/private/SENTINEL-OPERATOR-SOCKET",
                    "role":"admin"
                }),
            );
            assert_eq!(refused["result"]["isError"], true);
            assert_eq!(
                refused["result"]["structuredContent"]["code"],
                "tool_unavailable"
            );
            assert!(!refused.to_string().contains("SENTINEL"));
        }
        let oversized = vec![b'a'; SITE_ARCHITECT_MCP_MAX_MESSAGE_BYTES + 1];
        assert!(mcp_bounded_line(&mut std::io::Cursor::new(oversized)).is_err());
    }
}
